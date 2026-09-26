import { test, expect, Page } from "@playwright/test";
import { waitForApp, openRoomWithComposer } from "./example-room";
import { expectShimmerInPlace } from "./motion";
import { callRiverTest, UserActionKind } from "./river-test";
import { recordDomState, PresenceEvent } from "./dom-state-recorder";

// No-sync stays Disconnected until a test hook changes it; the primary
// indicator requires Connected. Room-update hooks exercise the real
// attach-on-send path without a node.

const INDICATOR_TESTID = "network-activity-indicator";
const DOT_TESTID = "network-activity-dot";
const STATUS_TESTID = "network-activity-status";

async function startUserAction(page: Page, kind?: UserActionKind) {
  await callRiverTest(page, "setSyncStatus", "connected");
  await callRiverTest(page, "beginUserAction", kind);
}

async function endUserAction(page: Page) {
  await callRiverTest(page, "endUserAction");
}

// Wait past the debounce: a count-0 check alone can pass before dots appear.
async function settleIdle(page: Page) {
  await page.waitForTimeout(600);
  await expect(page.getByTestId(INDICATOR_TESTID)).toHaveCount(0, {
    timeout: 3_000,
  });
}

// DOM presence, not visibility: the indicator unmounts when idle.
function recordPresence(page: Page) {
  return recordDomState(page, { selector: `[data-testid="${INDICATOR_TESTID}"]` });
}


function findTransition(
  log: PresenceEvent[],
  present: boolean,
  after = -Infinity
): PresenceEvent | undefined {
  return log.find((e) => e.present === present && e.t > after);
}

type Rect = { x: number; y: number; width: number; height: number };

function boxesIntersect(a: Rect, b: Rect): boolean {
  return !(
    a.x + a.width <= b.x ||
    b.x + b.width <= a.x ||
    a.y + a.height <= b.y ||
    b.y + b.height <= a.y
  );
}

test.describe("Network activity indicator", () => {
  test("idle no-sync build renders no indicator", async ({ page }) => {

    await page.goto("/");
    await waitForApp(page);
    await page.waitForTimeout(1_500);

    await expect(page.getByTestId(INDICATOR_TESTID)).toHaveCount(0);
    await expect(page.getByTestId(STATUS_TESTID)).toHaveText("");
  });

  test("a load shorter than 500ms never shows the dots", async ({ page }) => {
    await page.goto("/");
    await waitForApp(page);
    const recorder = await recordPresence(page);
    try {
      await callRiverTest(page, "setSyncStatus", "connected");
      // 300ms: past the old 200ms debounce, so this fails if it regresses.
      await page.evaluate(() => {
        const w = window as any;
        w.__riverTest.beginUserAction();
        setTimeout(() => w.__riverTest.endUserAction(), 300);
      });

      await page.waitForTimeout(2_000);

      const log = await recorder.read();
      expect(log.some((e) => e.present)).toBe(false);
    } finally {
      await recorder.dispose();
    }
  });

  test("the dots appear after the 500ms debounce, then hold for 1s with their reason", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForApp(page);
    await callRiverTest(page, "setSyncStatus", "connected");
    const recorder = await recordPresence(page);
    try {
      await page.evaluate(() => {
        const w = window as any;
        w.__t0 = performance.now();
        w.__riverTest.beginUserAction("saving");
      });
      const indicator = page.getByTestId(INDICATOR_TESTID);
      await expect(indicator).toBeVisible();
      await expect(indicator).toHaveAttribute("data-reason", "saving");

      // End at once, so the rest of the visible time is the hold.
      await endUserAction(page);
      // Without this wait, assertions could pass before a hold-less build unmounts.
      await page.waitForTimeout(500);
      await expect(indicator).toHaveCount(1);
      await expect(indicator).toHaveAttribute("data-reason", "saving");

      await expect(indicator).toHaveCount(0, { timeout: 3_000 });

      const t0 = await page.evaluate(() => (window as any).__t0);
      const log = await recorder.read();
      const shown = findTransition(log, true);
      expect(shown).toBeTruthy();
      const hidden = findTransition(log, false, shown!.t);
      expect(hidden).toBeTruthy();

      const debounce = shown!.t - t0;
      // 5ms slack for timer rounding. t0 is taken before the hook's own
      // `defer`, so the true gap can only be larger, never smaller.
      expect(debounce).toBeGreaterThanOrEqual(495);
      // Generous upper bound so a stalled timer doesn't pass as "debounced".
      expect(debounce).toBeLessThan(1_500);

      const visible = hidden!.t - shown!.t;
      expect(visible).toBeGreaterThanOrEqual(950);
      expect(visible).toBeLessThan(2_000);
    } finally {
      await recorder.dispose();
    }
  });

  test("a load longer than 1s hides as soon as it ends", async ({ page }) => {
    await page.goto("/");
    await waitForApp(page);
    const recorder = await recordPresence(page);
    try {
      await startUserAction(page);
      await expect(page.getByTestId(INDICATOR_TESTID)).toBeVisible();

      await page.waitForTimeout(2_500);

      await page.evaluate(() => {
        const w = window as any;
        w.__t1 = performance.now();
        w.__riverTest.endUserAction();
      });

      await expect(page.getByTestId(INDICATOR_TESTID)).toHaveCount(0, {
        timeout: 3_000,
      });

      const t1 = await page.evaluate(() => (window as any).__t1);
      const log = await recorder.read();
      const shown = findTransition(log, true);
      expect(shown).toBeTruthy();
      const hidden = findTransition(log, false, shown!.t);
      expect(hidden).toBeTruthy();

      const delta = hidden!.t - t1;
      expect(delta).toBeGreaterThanOrEqual(0);
      expect(delta).toBeLessThan(500);
    } finally {
      await recorder.dispose();
    }
  });

  test("busy again during the hold does not extend it past the original minimum", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForApp(page);
    const recorder = await recordPresence(page);
    try {
      await startUserAction(page);
      await expect(page.getByTestId(INDICATOR_TESTID)).toBeVisible();

      // 800ms, not less: if the hold clock were reset by the second busy
      // spell, the dots would hide ~shown_t + 800 + 1000, comfortably past the
      // bound below. A shorter wait leaves that mutation within ~50ms of it.
      await endUserAction(page);
      await page.waitForTimeout(800);
      await callRiverTest(page, "beginUserAction");
      await page.waitForTimeout(200);
      await endUserAction(page);

      await expect(page.getByTestId(INDICATOR_TESTID)).toHaveCount(0, {
        timeout: 3_000,
      });

      const log = await recorder.read();
      const shown = findTransition(log, true);
      expect(shown).toBeTruthy();
      const hidden = findTransition(log, false, shown!.t);
      expect(hidden).toBeTruthy();


      expect(hidden!.t).toBeLessThanOrEqual(shown!.t + 1_000 + 500);
    } finally {
      await recorder.dispose();
    }
  });

  test("dots ride one wave, and each one visibly varies", async ({ page }) => {
    await page.goto("/");
    await waitForApp(page);
    await startUserAction(page);
    await expect(page.getByTestId(INDICATOR_TESTID)).toBeVisible();
    const dots = page.getByTestId(DOT_TESTID);
    await expect(dots).toHaveCount(10);

    // Distinguish wave (1.5s) from swell (2.2–3.4s) without relying on animation order.
    const isWave = (t: { duration: number }) => Math.abs(t.duration - 1_500) < 50;

    const perDot = await dots.evaluateAll((els) =>
      els.map((el) =>
        (el as HTMLElement).getAnimations().map((a) => {
          const timing = a.effect!.getComputedTiming();
          return { duration: Number(timing.duration), delay: Number(timing.delay) };
        })
      )
    );
    expect(perDot).toHaveLength(10);
    for (const anims of perDot) {
      expect(anims).toHaveLength(2);
    }

    const waves = perDot.map((anims) => {
      const matches = anims.filter(isWave);
      expect(matches).toHaveLength(1);
      return matches[0];
    });
    const swells = perDot.map((anims) => {
      const matches = anims.filter((a) => !isWave(a));
      expect(matches).toHaveLength(1);
      return matches[0];
    });


    for (const w of waves) {
      expect(w.duration).toBeGreaterThanOrEqual(1_490);
      expect(w.duration).toBeLessThanOrEqual(1_510);
    }

    for (let i = 1; i < waves.length; i++) {
      expect(waves[i].delay).toBeGreaterThan(waves[i - 1].delay);
    }


    expect(new Set(swells.map((s) => s.duration)).size).toBeGreaterThan(1);

    // Freeze swell and align wave phases to isolate amplitude/curve variation.
    const offsets = await dots.evaluateAll((els) => {
      const ys: number[] = [];
      for (const el of els as HTMLElement[]) {
        const anims = el.getAnimations();
        const wave = anims.find(
          (a) => Math.abs(Number(a.effect!.getComputedTiming().duration) - 1_500) < 50
        )!;
        const swell = anims.find((a) => a !== wave)!;
        const waveDelay = Number(wave.effect!.getComputedTiming().delay);
        swell.pause();
        swell.currentTime = 0;
        wave.pause();
        // Account for stagger so it cannot masquerade as amplitude variation.
        wave.currentTime = waveDelay + 1_500 / 4;
        ys.push(el.getBoundingClientRect().y);
      }

      for (const el of els as HTMLElement[]) {
        el.getAnimations().forEach((a) => a.play());
      }
      return ys;
    });
    // Layout rounding alone spreads identical dots by up to ~0.5px; the real
    // variation gives a couple of px.
    const spread = Math.max(...offsets) - Math.min(...offsets);
    expect(spread).toBeGreaterThan(1);
  });

  test("reduced motion swaps the wave for a shimmer", async ({ page }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    await page.goto("/");
    await waitForApp(page);
    await startUserAction(page);

    await expect(page.getByTestId(INDICATOR_TESTID)).toBeVisible();

    await expectShimmerInPlace(page.getByTestId(DOT_TESTID).first(), 3_000);
  });

  test("a user action shows nothing unless connected", async ({ page }) => {

    await page.goto("/");
    await waitForApp(page);
    await callRiverTest(page, "beginUserAction");

    for (const state of ["connecting", "disconnected", "error"] as const) {
      await callRiverTest(page, "setSyncStatus", state);
      // The wait gives a wrong implementation time to show up.
      await page.waitForTimeout(1_000);
      await expect(page.getByTestId(INDICATOR_TESTID), state).toHaveCount(0);
    }
  });

  test("background work never shows the primary dots", async ({ page }) => {
    // ROOMS_LOAD_STATE also stays Loading in this build.
    await page.goto("/");
    await waitForApp(page);
    await callRiverTest(page, "setSyncStatus", "connected");
    await callRiverTest(page, "beginBackgroundRequest");

    await page.waitForTimeout(1_500);
    await expect(page.getByTestId(INDICATOR_TESTID)).toHaveCount(0);
    await expect(page.getByTestId(STATUS_TESTID)).toHaveText("");
  });
});

test.describe("User actions", () => {
  test("a room change keeps the dots until the node answers its UPDATE", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForApp(page);
    await callRiverTest(page, "setSyncStatus", "connected");
    await settleIdle(page);


    await callRiverTest(page, "awaitRoomUpdate");
    await callRiverTest(page, "sendRoomUpdate");
    const indicator = page.getByTestId(INDICATOR_TESTID);
    await expect(indicator).toBeVisible();
    await expect(indicator).toHaveAttribute("data-reason", "sending");


    await page.waitForTimeout(1_200);
    await expect(indicator).toBeVisible();

    await callRiverTest(page, "answerRoomUpdate");
    await expect(indicator).toHaveCount(0, { timeout: 3_000 });
  });

  test("a rejected UPDATE ends the wait", async ({ page }) => {
    await page.goto("/");
    await waitForApp(page);
    await callRiverTest(page, "setSyncStatus", "connected");
    await settleIdle(page);

    await callRiverTest(page, "awaitRoomUpdate");
    await callRiverTest(page, "sendRoomUpdate");
    const indicator = page.getByTestId(INDICATOR_TESTID);
    await expect(indicator).toBeVisible();

    await callRiverTest(page, "failRoomUpdate");
    await expect(indicator).toHaveCount(0, { timeout: 3_000 });
  });

  test("creating a room has its own reason", async ({ page }) => {
    await page.goto("/");
    await waitForApp(page);
    await startUserAction(page, "creating-room");
    const indicator = page.getByTestId(INDICATOR_TESTID);
    await expect(indicator).toBeVisible();
    await expect(indicator).toHaveAttribute("data-reason", "creating-room");
    await expect(page.getByTestId(STATUS_TESTID)).toContainText(
      "Creating room"
    );
  });

  test("a reply faster than 500ms shows nothing", async ({ page }) => {

    await page.goto("/");
    await waitForApp(page);
    await callRiverTest(page, "setSyncStatus", "connected");
    await settleIdle(page);

    const recorder = await recordPresence(page);
    try {
      await page.evaluate(() => {
        const w = window as any;
        w.__riverTest.awaitRoomUpdate();
        w.__riverTest.sendRoomUpdate();
        setTimeout(() => w.__riverTest.answerRoomUpdate(), 100);
      });

      await page.waitForTimeout(2_000);

      const log = await recorder.read();
      expect(log.some((e) => e.present)).toBe(false);
    } finally {
      await recorder.dispose();
    }
  });
});

for (const { label, viewport, isMobile } of [
  { label: "mobile", viewport: { width: 390, height: 844 }, isMobile: true },
  { label: "desktop", viewport: { width: 1280, height: 800 }, isMobile: false },
]) {
  test.describe(`Network activity indicator placement (${label})`, () => {
    test.use({ viewport });

    test("with no room open, ten dots sit in the no-room screen, not on the window", async ({
      page,
    }) => {
      await page.goto("/");
      await waitForApp(page);
      await startUserAction(page);

      const indicator = page.getByTestId(INDICATOR_TESTID);
      await expect(indicator).toBeVisible();
      await expect(indicator).toHaveAttribute("data-reason", "sending");
      await expect(page.getByTestId(DOT_TESTID)).toHaveCount(10);


      const position = await indicator.evaluate(
        (el) => getComputedStyle(el).position
      );
      expect(position).not.toBe("fixed");


      const heading = page.getByRole("heading", { name: "Welcome to River" });
      const [box, headingBox] = await Promise.all([
        indicator.boundingBox(),
        heading.boundingBox(),
      ]);
      expect(box).toBeTruthy();
      expect(headingBox).toBeTruthy();
      const centre = box!.x + box!.width / 2;
      const headingCentre = headingBox!.x + headingBox!.width / 2;
      expect(Math.abs(centre - headingCentre)).toBeLessThanOrEqual(2);

      expect(box!.y).toBeGreaterThan(headingBox!.y + headingBox!.height);

      await expect(page.getByTestId(STATUS_TESTID)).toContainText("Sending");
    });

    test("with a room open, the dots dock just above the composer", async ({
      page,
    }) => {
      await page.goto("/");
      await waitForApp(page);
      await openRoomWithComposer(page, isMobile);
      await startUserAction(page);

      const indicator = page.getByTestId(INDICATOR_TESTID);
      await expect(indicator).toBeVisible();
      await expect(indicator).toHaveCount(1);

      const position = await indicator.evaluate(
        (el) => getComputedStyle(el).position
      );
      expect(position).toBe("absolute");

      const composer = page.getByTestId("message-composer");
      const [box, composerBox] = await Promise.all([
        indicator.boundingBox(),
        composer.boundingBox(),
      ]);
      expect(box).toBeTruthy();
      expect(composerBox).toBeTruthy();


      const gap = composerBox!.y - (box!.y + box!.height);
      expect(gap).toBeGreaterThanOrEqual(11);
      expect(gap).toBeLessThanOrEqual(13);


      const centre = box!.x + box!.width / 2;
      const composerCentre = composerBox!.x + composerBox!.width / 2;
      expect(Math.abs(centre - composerCentre)).toBeLessThanOrEqual(2);
    });

    test("the history makes room for the docked dots, then gives it back", async ({
      page,
    }) => {
      await page.goto("/");
      await waitForApp(page);
      await openRoomWithComposer(page, isMobile);

      const spacer = page.getByTestId("chat-activity-spacer");
      const spacerHeight = () =>
        spacer.evaluate((el) => el.getBoundingClientRect().height);
      expect(await spacerHeight()).toBe(0);

      await startUserAction(page);
      const indicator = page.getByTestId(INDICATOR_TESTID);
      await expect(indicator).toBeVisible();

      // Poll through the spacer's 150ms transition.
      await expect.poll(spacerHeight).toBeGreaterThanOrEqual(20);

      // The spacer's top is the last message's bottom; room opening pins the scroll.
      const clearance = async () => {
        const [lastMessageBottom, dotsTop] = await Promise.all([
          spacer.evaluate((el) => el.getBoundingClientRect().top),
          indicator.evaluate((el) => el.getBoundingClientRect().top),
        ]);
        return dotsTop - lastMessageBottom;
      };
      await expect.poll(clearance).toBeGreaterThanOrEqual(11);
      expect(await clearance()).toBeLessThanOrEqual(13.5);

      await endUserAction(page);
      await expect(indicator).toHaveCount(0, { timeout: 3_000 });
      await expect.poll(spacerHeight).toBe(0);
    });

    test("does not intercept taps", async ({ page }) => {
      await page.goto("/");
      await waitForApp(page);
      await startUserAction(page);

      const indicator = page.getByTestId(INDICATOR_TESTID);
      await expect(indicator).toBeVisible();

      const pointerEvents = await indicator.evaluate(
        (el) => getComputedStyle(el).pointerEvents
      );
      expect(pointerEvents).toBe("none");

      await openRoomWithComposer(page, isMobile);

      const messageInput = page.getByTestId("message-input");
      await expect(messageInput).toBeVisible({ timeout: 5_000 });

      const indicatorBox = await indicator.boundingBox();
      const inputBox = await messageInput.boundingBox();
      expect(indicatorBox).toBeTruthy();
      expect(inputBox).toBeTruthy();
      expect(boxesIntersect(indicatorBox as Rect, inputBox as Rect)).toBe(
        false
      );

      const marker = `network-activity-tap-check-${label}`;
      await messageInput.fill(marker);
      // Do not force: normal actionability must catch intercepted clicks.
      await page.getByTestId("send-message-button").click();
      await expect(page.getByText(marker)).toBeVisible({ timeout: 5_000 });
    });
  });
}

test.describe("Network activity indicator (mobile-only panel behaviour)", () => {
  test.use({ viewport: { width: 390, height: 844 } });

  test("mobile: the dots belong to the chat, so the rooms panel does not show them", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForApp(page);
    await openRoomWithComposer(page, true);
    await startUserAction(page);

    const indicator = page.getByTestId(INDICATOR_TESTID);
    await expect(indicator).toBeVisible();


    await page.getByTestId("hamburger-rooms-button").click();
    await expect(page.getByTestId("room-list")).toBeVisible();
    await expect(indicator).toBeHidden();
  });
});
