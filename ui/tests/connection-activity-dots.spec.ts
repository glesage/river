import { test, expect, Locator, Page } from "@playwright/test";
import { waitForApp } from "./example-room";
import { expectShimmerInPlace } from "./motion";
import { callRiverTest, RiverTestWindow } from "./river-test";
import { recordDomState } from "./dom-state-recorder";

// The dots stay mounted for fade-out; `data-active` determines visibility.
// Both pill copies share test IDs, so `:visible` selects the active placement.
// No-sync stays Disconnected with rooms Loading until the test hooks change it.
// `setRoomsLoadState("loaded")` also clears ROOMS / CURRENT_ROOM.

const VISIBLE_PILL = '[data-testid="connection-status-indicator"]:visible';

const DOTS_SPAN = `${VISIBLE_PILL} [data-testid="connection-activity-dots"]`;

const VISIBLE_DOTS = `${DOTS_SPAN}[data-active="true"]`;

// The pill lives in the Rooms rail, so wait for that as well as the shell.
async function waitForAppAndRail(page: Page) {
  await waitForApp(page);
  await expect(page.locator('[data-testid="rooms-rail"]')).toHaveCount(1);
}

async function quietConnected(page: Page) {
  await callRiverTest(page, "setSyncStatus", "connected");
  await callRiverTest(page, "setRoomsLoadState", "loaded");
  await expect(page.locator(VISIBLE_DOTS)).toHaveCount(0, { timeout: 3_000 });
}

// Active dots in the laid-out pill copy; the dots stay mounted while inactive.
function recordDots(page: Page) {
  return recordDomState(page, {
    selector: '[data-testid="connection-activity-dots"][data-active="true"]',
    visibleWithin: '[data-testid="connection-status-indicator"]',
    attributes: ["data-active"],
  });
}

async function pill(page: Page): Promise<Locator> {
  const visible = page.locator(VISIBLE_PILL);
  await expect(visible).toHaveCount(1);
  return visible;
}

type Transition = { property: string; duration: number };

type DotsTransition = { transitions: Transition[]; labelX?: number };

// Observe, trigger and capture in one browser task so a slow runner cannot
// miss the 300ms fade. With `seekMs`, also measure the label mid-transition.
async function captureDotsTransition(
  page: Page,
  active: boolean,
  seekMs?: number
): Promise<DotsTransition> {
  return page.evaluate(
    async ({ active, seekMs }) => {
      const hooks = (window as unknown as RiverTestWindow).__riverTest;
      const pill = Array.from(
        document.querySelectorAll<HTMLElement>('[data-testid="connection-status-indicator"]')
      ).find((el) => el.offsetParent !== null);
      const span = pill?.querySelector('[data-testid="connection-activity-dots"]');
      if (!pill || !span) throw new Error("no visible connection pill with activity dots");

      const want = String(active);
      let observer: MutationObserver | undefined;
      let timer: number | undefined;
      const reached = new Promise<void>((resolve, reject) => {
        observer = new MutationObserver(() => {
          if (span.getAttribute("data-active") === want) resolve();
        });
        observer.observe(span, { attributes: true, attributeFilter: ["data-active"] });
        timer = window.setTimeout(
          () => reject(new Error(`the dots never reached data-active="${want}"`)),
          5_000
        );
      });
      try {
        if (active) hooks.beginBackgroundRequest();
        else hooks.endBackgroundRequest();
        await reached;
      } finally {
        observer?.disconnect();
        window.clearTimeout(timer);
      }

      const running = span.getAnimations();
      const transitions = running.map((a) => ({
        property: (a as CSSTransition).transitionProperty,
        duration: Number(a.effect!.getTiming().duration),
      }));
      if (seekMs === undefined) return { transitions };

      running.forEach((a) => {
        a.pause();
        a.currentTime = seekMs;
      });
      const labelX = pill.querySelector("span")!.getBoundingClientRect().x;
      running.forEach((a) => a.finish());
      return { transitions, labelX };
    },
    { active, seekMs }
  );
}

function durationOf(transitions: Transition[], property: string) {
  return transitions.find((t) => t.property === property)?.duration;
}

for (const { label, viewport } of [
  { label: "mobile", viewport: { width: 390, height: 844 } },
  { label: "desktop", viewport: { width: 1280, height: 800 } },
]) {
  test.describe(`Connection pill activity dots (${label})`, () => {
    test.use({ viewport });

    test("an idle no-sync build shows no dots", async ({ page }) => {
      await page.goto("/");
      await waitForAppAndRail(page);
      await page.waitForTimeout(1_500);
      await expect(page.locator(VISIBLE_DOTS)).toHaveCount(0);
      const visible = await pill(page);
      await expect(visible).toHaveAttribute("aria-busy", "false");
      await expect(visible).not.toHaveAttribute("data-busy-reason", /.*/);
      await expect(visible).not.toHaveAttribute("title", /.*/);
    });

    test("connecting shows the dots at once, inside the pill", async ({
      page,
    }) => {
      await page.goto("/");
      await waitForAppAndRail(page);
      const recorder = await recordDots(page);
      try {
        await page.evaluate(() => {
          const w = window as any;
          w.__t0 = performance.now();
          w.__riverTest.setSyncStatus("connecting");
        });
        const dots = page.locator(VISIBLE_DOTS);
        await expect(dots).toHaveCount(1);
        await expect(dots.locator(".pill-activity-dot")).toHaveCount(5);
        const visible = await pill(page);
        await expect(visible).toHaveAttribute("aria-busy", "true");

        await expect(visible).toHaveAttribute("data-busy-reason", "connecting");
        await expect(visible).toHaveAttribute("title", "Connecting to Freenet");


        const t0 = await page.evaluate(() => (window as any).__t0);
        const shown = (await recorder.read()).find((e) => e.present);
        expect(shown).toBeTruthy();
        expect(shown!.t - t0).toBeLessThan(250);
      } finally {
        await recorder.dispose();
      }
    });

    test("the pill's status dot and label are unchanged by the dots", async ({
      page,
    }) => {
      // Preserve the selectors used by connection-status-indicator.spec.ts.
      await page.goto("/");
      await waitForAppAndRail(page);
      await callRiverTest(page, "setSyncStatus", "connecting");
      const visible = await pill(page);
      await expect(page.locator(VISIBLE_DOTS)).toHaveCount(1);

      await expect(visible.locator("div").first()).toHaveClass(/bg-yellow-500/);
      expect(((await visible.textContent()) ?? "").trim()).toBe("Connecting");
    });

    test("rooms loading while connected is background work", async ({
      page,
    }) => {
      await page.goto("/");
      await waitForAppAndRail(page);
      await callRiverTest(page, "setSyncStatus", "connected");
      await expect(page.locator(VISIBLE_DOTS)).toHaveCount(1);
      await expect(await pill(page)).toHaveAttribute(
        "data-busy-reason",
        "loading-rooms"
      );

      await callRiverTest(page, "setRoomsLoadState", "loaded");
      await expect(page.locator(VISIBLE_DOTS)).toHaveCount(0, {
        timeout: 3_000,
      });
    });

    test("a background request shows the dots, held at least 1s", async ({
      page,
    }) => {
      await page.goto("/");
      await waitForAppAndRail(page);
      await quietConnected(page);
      const recorder = await recordDots(page);
      try {
        await callRiverTest(page, "beginBackgroundRequest");
        await expect(page.locator(VISIBLE_DOTS)).toHaveCount(1);
        await expect(await pill(page)).toHaveAttribute(
          "data-busy-reason",
          "requests"
        );
        await callRiverTest(page, "endBackgroundRequest");
        await expect(page.locator(VISIBLE_DOTS)).toHaveCount(0, {
          timeout: 3_000,
        });

        const log = await recorder.read();
        const shown = log.find((e) => e.present);
        expect(shown).toBeTruthy();
        const hidden = log.find((e) => !e.present && e.t > shown!.t);
        expect(hidden).toBeTruthy();
        expect(hidden!.t - shown!.t).toBeGreaterThanOrEqual(950);
        expect(hidden!.t - shown!.t).toBeLessThan(2_000);
      } finally {
        await recorder.dispose();
      }
    });

    test("a request the user is waiting on is not background work", async ({
      page,
    }) => {
      await page.goto("/");
      await waitForAppAndRail(page);
      await quietConnected(page);

      await callRiverTest(page, "awaitRoomUpdate");
      await callRiverTest(page, "sendRoomUpdate");

      await expect(page.getByTestId("network-activity-indicator")).toBeVisible();
      await expect(page.locator(VISIBLE_DOTS)).toHaveCount(0);
    });

    test("an error or a no-sync disconnect shows no dots", async ({ page }) => {
      await page.goto("/");
      await waitForAppAndRail(page);
      for (const state of ["error", "disconnected"] as const) {
        await callRiverTest(page, "setSyncStatus", state);
        await page.waitForTimeout(1_200);
        await expect(page.locator(VISIBLE_DOTS), state).toHaveCount(0);
      }
    });

    test("reduced motion keeps the dots still: opacity moves, position does not", async ({
      page,
    }) => {
      await page.emulateMedia({ reducedMotion: "reduce" });
      await page.goto("/");
      await waitForAppAndRail(page);
      await callRiverTest(page, "setSyncStatus", "connecting");
      const dot = page.locator(`${VISIBLE_DOTS} .pill-activity-dot`).first();
      await expect(dot).toBeVisible();

      await expectShimmerInPlace(dot, 2_000);
    });

    // Keep the label text fixed to isolate movement caused by the dots.

    test("the dots fade in and out over 300ms", async ({ page }) => {
      await page.goto("/");
      await waitForAppAndRail(page);
      await quietConnected(page);
      const span = page.locator(DOTS_SPAN);
      await expect(span).toHaveAttribute("data-active", "false");

      const { transitions: opening } = await captureDotsTransition(page, true);
      expect(durationOf(opening, "opacity"), "opacity fades in").toBe(300);
      expect(durationOf(opening, "width"), "the space opens").toBe(300);
      await expect
        .poll(() => span.evaluate((el) => getComputedStyle(el).opacity))
        .toBe("1");
      await expect
        .poll(() => span.evaluate((el) => getComputedStyle(el).width))
        .toBe("23px");


      const { transitions: closing } = await captureDotsTransition(page, false);
      expect(durationOf(closing, "opacity"), "opacity fades out").toBe(300);
      expect(durationOf(closing, "width"), "the space closes").toBe(300);
      await expect
        .poll(() => span.evaluate((el) => getComputedStyle(el).opacity))
        .toBe("0");
      await expect
        .poll(() => span.evaluate((el) => getComputedStyle(el).width))
        .toBe("0px");
    });

    test("the label glides left as the dots open, and back as they close", async ({
      page,
    }) => {
      await page.goto("/");
      await waitForAppAndRail(page);
      await quietConnected(page);

      const label = page.locator(VISIBLE_PILL).locator("span").first();
      await expect(label).toHaveText("Connected");
      const labelX = () => label.evaluate((el) => el.getBoundingClientRect().x);
      const idleX = await labelX();

      // Seek to mid-transition to avoid flaky wall-clock sampling.
      const midX = (await captureDotsTransition(page, true, 150)).labelX!;

      // Open: half of the 23px dots + 6px margin, since the pill centres.
      await expect.poll(labelX).toBeLessThan(idleX - 13);
      const openX = await labelX();
      expect(idleX - openX).toBeGreaterThan(13);
      expect(idleX - openX).toBeLessThan(16);
      expect(midX, "mid-glide, the label is on its way").toBeLessThan(idleX - 0.5);
      expect(midX, "mid-glide, the label is not there yet").toBeGreaterThan(
        openX + 0.5
      );


      await callRiverTest(page, "endBackgroundRequest");
      await expect.poll(labelX, { timeout: 3_000 }).toBeGreaterThan(idleX - 1);
      expect(await labelX()).toBeLessThan(idleX + 1);
    });

    test("five dots ride one wave", async ({ page }) => {
      await page.goto("/");
      await waitForAppAndRail(page);
      await callRiverTest(page, "setSyncStatus", "connecting");
      const delays = await page
        .locator(`${VISIBLE_DOTS} .pill-activity-dot`)
        .evaluateAll((els) =>
          els.map((el) => parseFloat(getComputedStyle(el).animationDelay))
        );
      expect(delays).toHaveLength(5);

      for (let i = 1; i < delays.length; i++) {
        expect(delays[i]).toBeGreaterThan(delays[i - 1]);
      }
    });

    test("reduced motion fades the dots without moving the label", async ({
      page,
    }) => {
      await page.emulateMedia({ reducedMotion: "reduce" });
      await page.goto("/");
      await waitForAppAndRail(page);
      const property = await page
        .locator(DOTS_SPAN)
        .evaluate((el) => getComputedStyle(el).transitionProperty);
      expect(property).toContain("opacity");
      expect(property).not.toContain("width");
    });
  });
}
