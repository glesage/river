import { test, expect, Locator, Page } from "@playwright/test";

// Coverage for the SECONDARY loading indicator (docs/plans/loading-indicators.md):
// five small dots riding a wave inside the connection pill, shown the moment
// any background work starts (connecting, reconnecting, loading or re-syncing
// rooms, a request nobody is waiting on) and held at least 1s. They fade in
// and out over 300ms while the pill's label glides aside to make room.
//
// The dots span stays mounted so it can fade out; `data-active="true"` is
// what "showing" means.
//
// The pill is mounted twice (rooms rail, and the mobile no-room screen) and
// both copies share their test ids, so `:visible` picks the one the user sees.
//
// PREMISES (see `example_data.rs::install_test_hooks`):
//   - A no-sync build starts `Disconnected` and never writes `SYNC_STATUS` on
//     its own, and a no-sync `Disconnected` is NOT reconnecting, so the pill
//     is idle at load.
//   - This build's `ROOMS_LOAD_STATE` sits at `Loading` forever, so
//     `setSyncStatus("connected")` alone is background work (rooms loading).
//     `setRoomsLoadState("loaded")` ends it (and clears ROOMS / CURRENT_ROOM
//     as a side effect, which these tests don't depend on).
//   - `beginBackgroundRequest()` / `endBackgroundRequest()` record and settle
//     a request nobody is waiting on; `awaitRoomUpdate()` +
//     `sendRoomUpdate()` record one the user IS waiting on.

const VISIBLE_PILL = '[data-testid="connection-status-indicator"]:visible';
// The visible pill's dots span, showing or not.
const DOTS_SPAN = `${VISIBLE_PILL} [data-testid="connection-activity-dots"]`;
// The same span, only while it is showing.
const VISIBLE_DOTS = `${DOTS_SPAN}[data-active="true"]`;

async function waitForApp(page: Page) {
  await page.waitForSelector(".app-root", { timeout: 30_000 });
  await expect(page.locator('[data-testid="rooms-rail"]')).toHaveCount(1);
}

async function hook(page: Page, name: string, arg?: string) {
  await page.evaluate(
    ({ name, arg }) => {
      (window as any).__riverTest[name](arg);
    },
    { name, arg }
  );
}

// Connected with nothing going on in the background.
async function quietConnected(page: Page) {
  await hook(page, "setSyncStatus", "connected");
  await hook(page, "setRoomsLoadState", "loaded");
  await expect(page.locator(VISIBLE_DOTS)).toHaveCount(0, { timeout: 3_000 });
}

// Logs { t, present } each time the visible pill's dots come or go, measured
// in the page so round-trip latency stays out of the timing assertions.
async function recordDots(
  page: Page
): Promise<() => Promise<{ t: number; present: boolean }[]>> {
  await page.evaluate(() => {
    const w = window as any;
    const isPresent = () =>
      Array.from(
        document.querySelectorAll('[data-testid="connection-activity-dots"]')
      ).some(
        (el) =>
          el.getAttribute("data-active") === "true" &&
          (el.closest('[data-testid="connection-status-indicator"]') as HTMLElement)
            .offsetParent !== null
      );
    w.__dotsLog = [{ t: performance.now(), present: isPresent() }];
    let last = isPresent();
    new MutationObserver(() => {
      const now = isPresent();
      if (now !== last) {
        last = now;
        w.__dotsLog.push({ t: performance.now(), present: now });
      }
    }).observe(document.body, {
      childList: true,
      subtree: true,
      // The span stays mounted; only this attribute says it is showing.
      attributes: true,
      attributeFilter: ["data-active"],
    });
  });
  return async () => page.evaluate(() => (window as any).__dotsLog);
}

async function pill(page: Page): Promise<Locator> {
  const visible = page.locator(VISIBLE_PILL);
  await expect(visible).toHaveCount(1);
  return visible;
}

type Transition = { property: string; duration: number };

// Start a background request, and in the same page task wait for the dots
// span to open, then report the transitions running on it. Done in-page so a
// slow test runner can't miss a 300ms transition.
async function openDots(page: Page): Promise<Transition[]> {
  return page.evaluate(async () => {
    const w = window as any;
    const pill = Array.from(
      document.querySelectorAll('[data-testid="connection-status-indicator"]')
    ).find((el) => (el as HTMLElement).offsetParent !== null)!;
    const span = pill.querySelector('[data-testid="connection-activity-dots"]')!;
    const opened = new Promise<void>((resolve) => {
      new MutationObserver((_, observer) => {
        if (span.getAttribute("data-active") === "true") {
          observer.disconnect();
          resolve();
        }
      }).observe(span, { attributes: true, attributeFilter: ["data-active"] });
    });
    w.__riverTest.beginBackgroundRequest();
    await opened;
    return span.getAnimations().map((a: any) => ({
      property: a.transitionProperty,
      duration: Number(a.effect.getTiming().duration),
    }));
  });
}

// Settle the background request, wait out the gate's 1s hold until the span
// starts closing, and report the transitions running on it.
async function closeDots(page: Page): Promise<Transition[]> {
  return page.evaluate(async () => {
    const w = window as any;
    const pill = Array.from(
      document.querySelectorAll('[data-testid="connection-status-indicator"]')
    ).find((el) => (el as HTMLElement).offsetParent !== null)!;
    const span = pill.querySelector('[data-testid="connection-activity-dots"]')!;
    const closing = new Promise<void>((resolve) => {
      new MutationObserver((_, observer) => {
        if (span.getAttribute("data-active") === "false") {
          observer.disconnect();
          resolve();
        }
      }).observe(span, { attributes: true, attributeFilter: ["data-active"] });
    });
    w.__riverTest.endBackgroundRequest();
    await closing;
    return span.getAnimations().map((a: any) => ({
      property: a.transitionProperty,
      duration: Number(a.effect.getTiming().duration),
    }));
  });
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
      await waitForApp(page);
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
      await waitForApp(page);
      const readLog = await recordDots(page);

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
      // Why it is busy, for devtools and as the pill's tooltip.
      await expect(visible).toHaveAttribute("data-busy-reason", "connecting");
      await expect(visible).toHaveAttribute("title", "Connecting to Freenet…");

      // No debounce: on screen within a couple of frames of the change.
      const t0 = await page.evaluate(() => (window as any).__t0);
      const shown = (await readLog()).find((e) => e.present);
      expect(shown).toBeTruthy();
      expect(shown!.t - t0).toBeLessThan(250);
    });

    test("the pill's status dot and label are unchanged by the dots", async ({
      page,
    }) => {
      // connection-status-indicator.spec.ts reads the pill's FIRST div as its
      // status dot and its whole text as the label; the dots must not
      // disturb either.
      await page.goto("/");
      await waitForApp(page);
      await hook(page, "setSyncStatus", "connecting");
      const visible = await pill(page);
      await expect(page.locator(VISIBLE_DOTS)).toHaveCount(1);

      await expect(visible.locator("div").first()).toHaveClass(/bg-yellow-500/);
      expect(((await visible.textContent()) ?? "").trim()).toBe("Connecting...");
    });

    test("rooms loading while connected is background work", async ({
      page,
    }) => {
      await page.goto("/");
      await waitForApp(page);
      await hook(page, "setSyncStatus", "connected");
      await expect(page.locator(VISIBLE_DOTS)).toHaveCount(1);
      await expect(await pill(page)).toHaveAttribute(
        "data-busy-reason",
        "loading-rooms"
      );

      await hook(page, "setRoomsLoadState", "loaded");
      await expect(page.locator(VISIBLE_DOTS)).toHaveCount(0, {
        timeout: 3_000,
      });
    });

    test("a background request shows the dots, held at least 1s", async ({
      page,
    }) => {
      await page.goto("/");
      await waitForApp(page);
      await quietConnected(page);
      const readLog = await recordDots(page);

      await hook(page, "beginBackgroundRequest");
      await expect(page.locator(VISIBLE_DOTS)).toHaveCount(1);
      await expect(await pill(page)).toHaveAttribute(
        "data-busy-reason",
        "requests"
      );
      await hook(page, "endBackgroundRequest");
      await expect(page.locator(VISIBLE_DOTS)).toHaveCount(0, {
        timeout: 3_000,
      });

      const log = await readLog();
      const shown = log.find((e) => e.present);
      expect(shown).toBeTruthy();
      const hidden = log.find((e) => !e.present && e.t > shown!.t);
      expect(hidden).toBeTruthy();
      expect(hidden!.t - shown!.t).toBeGreaterThanOrEqual(950);
      expect(hidden!.t - shown!.t).toBeLessThan(2_000);
    });

    test("a request the user is waiting on is not background work", async ({
      page,
    }) => {
      await page.goto("/");
      await waitForApp(page);
      await quietConnected(page);

      await hook(page, "awaitRoomUpdate");
      await hook(page, "sendRoomUpdate");
      // The primary dots show instead (after their 500ms debounce).
      await expect(page.getByTestId("network-activity-indicator")).toBeVisible();
      await expect(page.locator(VISIBLE_DOTS)).toHaveCount(0);
    });

    test("an error or a no-sync disconnect shows no dots", async ({ page }) => {
      await page.goto("/");
      await waitForApp(page);
      for (const state of ["error", "disconnected"]) {
        await hook(page, "setSyncStatus", state);
        await page.waitForTimeout(1_200);
        await expect(page.locator(VISIBLE_DOTS), state).toHaveCount(0);
      }
    });

    test("reduced motion keeps the dots still", async ({ page }) => {
      await page.emulateMedia({ reducedMotion: "reduce" });
      await page.goto("/");
      await waitForApp(page);
      await hook(page, "setSyncStatus", "connecting");
      const dot = page.locator(`${VISIBLE_DOTS} .pill-activity-dot`).first();
      await expect(dot).toBeVisible();
      expect(await dot.evaluate((el) => getComputedStyle(el).animationName)).toBe(
        "river-flow-shimmer"
      );
    });

    // Status stays `connected` in the tests below (a background request
    // drives the dots), so the label's text, and therefore its own width,
    // never changes: any sideways move is the dots making room.

    test("the dots fade in and out over 300ms", async ({ page }) => {
      await page.goto("/");
      await waitForApp(page);
      await quietConnected(page);
      const span = page.locator(DOTS_SPAN);
      await expect(span).toHaveAttribute("data-active", "false");

      const opening = await openDots(page);
      expect(durationOf(opening, "opacity"), "opacity fades in").toBe(300);
      expect(durationOf(opening, "width"), "the space opens").toBe(300);
      await expect
        .poll(() => span.evaluate((el) => getComputedStyle(el).opacity))
        .toBe("1");
      await expect
        .poll(() => span.evaluate((el) => getComputedStyle(el).width))
        .toBe("23px");

      // Still mounted while it fades out, rather than vanishing.
      const closing = await closeDots(page);
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
      await waitForApp(page);
      await quietConnected(page);
      // The pill's first span is its label (its first div is the status dot).
      const label = page.locator(VISIBLE_PILL).locator("span").first();
      await expect(label).toHaveText("Connected");
      const labelX = () => label.evaluate((el) => el.getBoundingClientRect().x);
      const idleX = await labelX();

      // Open the dots, then pause their transitions halfway and seek there:
      // a deterministic mid-glide sample, where a wall-clock one could flake.
      const midX = await page.evaluate(async () => {
        const w = window as any;
        const pill = Array.from(
          document.querySelectorAll('[data-testid="connection-status-indicator"]')
        ).find((el) => (el as HTMLElement).offsetParent !== null)!;
        const span = pill.querySelector('[data-testid="connection-activity-dots"]')!;
        const label = pill.querySelector("span")!;
        const opened = new Promise<void>((resolve) => {
          new MutationObserver((_, observer) => {
            if (span.getAttribute("data-active") === "true") {
              observer.disconnect();
              resolve();
            }
          }).observe(span, { attributes: true, attributeFilter: ["data-active"] });
        });
        w.__riverTest.beginBackgroundRequest();
        await opened;
        const running = span.getAnimations();
        running.forEach((a) => {
          a.pause();
          a.currentTime = 150;
        });
        const x = label.getBoundingClientRect().x;
        running.forEach((a) => a.finish());
        return x;
      });

      // Open: half of the 23px dots + 6px margin, since the pill centres.
      await expect.poll(labelX).toBeLessThan(idleX - 13);
      const openX = await labelX();
      expect(idleX - openX).toBeGreaterThan(13);
      expect(idleX - openX).toBeLessThan(16);
      expect(midX, "mid-glide, the label is on its way").toBeLessThan(idleX - 0.5);
      expect(midX, "mid-glide, the label is not there yet").toBeGreaterThan(
        openX + 0.5
      );

      // Closed again (after the 1s hold and the 300ms fade): back home.
      await hook(page, "endBackgroundRequest");
      await expect.poll(labelX, { timeout: 3_000 }).toBeGreaterThan(idleX - 1);
      expect(await labelX()).toBeLessThan(idleX + 1);
    });

    test("five dots ride one wave", async ({ page }) => {
      await page.goto("/");
      await waitForApp(page);
      await hook(page, "setSyncStatus", "connecting");
      const delays = await page
        .locator(`${VISIBLE_DOTS} .pill-activity-dot`)
        .evaluateAll((els) =>
          els.map((el) => parseFloat(getComputedStyle(el).animationDelay))
        );
      expect(delays).toHaveLength(5);
      // Each dot a step behind its left neighbour: the crest travels. Fails
      // if the `--i` wiring breaks and they bob in unison.
      for (let i = 1; i < delays.length; i++) {
        expect(delays[i]).toBeGreaterThan(delays[i - 1]);
      }
    });

    test("reduced motion fades the dots without moving the label", async ({
      page,
    }) => {
      await page.emulateMedia({ reducedMotion: "reduce" });
      await page.goto("/");
      await waitForApp(page);
      const property = await page
        .locator(DOTS_SPAN)
        .evaluate((el) => getComputedStyle(el).transitionProperty);
      expect(property).toContain("opacity");
      expect(property).not.toContain("width");
    });
  });
}
