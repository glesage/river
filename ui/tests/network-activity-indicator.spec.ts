import { test, expect, Page } from "@playwright/test";
import { waitForApp, openRoomWithComposer } from "./example-room";
import { expectShimmerInPlace } from "./motion";

// Coverage for the PRIMARY loading indicator:
// ten accent-blue dots shown only while the user is waiting on the node's
// reply to something they did (sending, reacting, creating or joining a room,
// saving a setting). Background work (connecting, loading or re-syncing rooms,
// refreshes) never shows them: that belongs to the small dots in the
// connection pill (connection-activity-dots.spec.ts). The dots are debounced
// 500ms, so a quick reply shows nothing, and held at least 1s once shown.
//
// PLACEMENT: inside the chat section, never fixed to the window. With a room
// open the dots dock at the bottom of the message history, just above the
// composer, and the history gains bottom padding while they show so they never
// cover the last message. With no room open they sit in the flow of the
// no-room screen.
//
// PREMISES (see `example_data.rs::install_test_hooks`):
//   - A no-sync build starts `Disconnected` and never writes `SYNC_STATUS` on
//     its own. The primary needs `Connected`, so every busy test first calls
//     `setSyncStatus("connected")`.
//   - `beginUserAction(kind?)` / `endUserAction()` hold a scoped user action
//     (kind: "sending" (default) | "saving" | "creating-room").
//   - `awaitRoomUpdate()`, `sendRoomUpdate()`, `answerRoomUpdate()`,
//     `failRoomUpdate()` walk a room change through the real attach-on-send
//     path: queued, carried by an UPDATE, then answered or failed.
//   - `beginBackgroundRequest()` / `endBackgroundRequest()` record and settle
//     a request nobody is waiting on.
//
// Timing rules: appearing takes ~500ms (the default `toBeVisible()` timeout
// absorbs it); disappearing can take up to 1s (the hold), so every "gone"
// assertion uses `toHaveCount(0, { timeout: 3_000 })`.
//
// State/timer/content coverage below (gate behaviour, hold timing, reasons,
// the wave's own motion) runs once per Playwright project: none of it varies
// with screen size. Responsive placement — docking above the composer, the
// history's spacer, tap-through geometry, and the mobile-only panel-hiding
// case — runs once per viewport size further down, since those are the only
// things here that actually depend on layout.

const INDICATOR_TESTID = "network-activity-indicator";
const DOT_TESTID = "network-activity-dot";
const STATUS_TESTID = "network-activity-status";

async function setSyncStatus(page: Page, state: string) {
  await page.evaluate((s) => {
    (window as any).__riverTest.setSyncStatus(s);
  }, state);
}

async function hook(page: Page, name: string, arg?: string) {
  await page.evaluate(
    ({ name, arg }) => {
      (window as any).__riverTest[name](arg);
    },
    { name, arg }
  );
}

// Connected, then a scoped user action: the primary's busy state.
async function startUserAction(page: Page, kind?: string) {
  await setSyncStatus(page, "connected");
  await hook(page, "beginUserAction", kind);
}

async function endUserAction(page: Page) {
  await hook(page, "endUserAction");
}

// Wait until the gate is Idle, not merely "no dots in the DOM": a count-0
// check alone can pass while a debounce is still pending and the dots are
// about to appear. Past the debounce, a pending show has either mounted (and
// the count waits out its hold) or been cancelled.
async function settleIdle(page: Page) {
  await page.waitForTimeout(600);
  await expect(page.getByTestId(INDICATOR_TESTID)).toHaveCount(0, {
    timeout: 3_000,
  });
}

type PresenceEvent = { t: number; present: boolean };

// Installs a MutationObserver on document.body that logs
// { t: performance.now(), present } to window.__presenceLog every time
// whether the indicator is in the DOM flips — including the initial state.
// Measuring from inside the page (rather than polling from Playwright) keeps
// round-trip latency out of the millisecond-level assertions below. Returns
// a function that reads the log back.
async function recordPresence(
  page: Page
): Promise<() => Promise<PresenceEvent[]>> {
  await page.evaluate((testid) => {
    const w = window as any;
    w.__presenceLog = [];
    const isPresent = () =>
      document.querySelector(`[data-testid="${testid}"]`) !== null;
    let last = isPresent();
    w.__presenceLog.push({ t: performance.now(), present: last });
    const observer = new MutationObserver(() => {
      const now = isPresent();
      if (now !== last) {
        last = now;
        w.__presenceLog.push({ t: performance.now(), present: now });
      }
    });
    observer.observe(document.body, { childList: true, subtree: true });
    // Kept reachable only so it isn't a mystery global if inspected; nothing
    // here needs to disconnect it (the page is torn down with the test).
    w.__presenceObserver = observer;
  }, INDICATOR_TESTID);

  return async (): Promise<PresenceEvent[]> =>
    page.evaluate(() => (window as any).__presenceLog);
}

// First logged transition to `present`, at or after `after`. Log entries
// alternate (the indicator mounts/unmounts, it never "flickers" in place),
// so this is enough to pull out a specific show/hide pair.
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
    // This is the premise every other spec in the suite relies on: without
    // a test-hook nudge, a no-sync example build never shows the dots, so
    // their appearance elsewhere in the suite cannot be blamed on load
    // noise from this feature.
    await page.goto("/");
    await waitForApp(page);
    await page.waitForTimeout(1_500);

    await expect(page.getByTestId(INDICATOR_TESTID)).toHaveCount(0);
    await expect(page.getByTestId(STATUS_TESTID)).toHaveText("");
  });

  test("a load shorter than 500ms never shows the dots", async ({ page }) => {
    await page.goto("/");
    await waitForApp(page);
    const readLog = await recordPresence(page);

    await setSyncStatus(page, "connected");
    // 300ms: past the old 200ms debounce, so this fails if it regresses.
    await page.evaluate(() => {
      const w = window as any;
      w.__riverTest.beginUserAction();
      setTimeout(() => w.__riverTest.endUserAction(), 300);
    });

    await page.waitForTimeout(2_000);

    const log = await readLog();
    expect(log.some((e) => e.present)).toBe(false);
  });

  test("the dots appear only after the 500ms debounce", async ({ page }) => {
    await page.goto("/");
    await waitForApp(page);
    await setSyncStatus(page, "connected");
    const readLog = await recordPresence(page);

    await page.evaluate(() => {
      const w = window as any;
      w.__t0 = performance.now();
      w.__riverTest.beginUserAction();
    });

    await expect(page.getByTestId(INDICATOR_TESTID)).toBeVisible();

    const t0 = await page.evaluate(() => (window as any).__t0);
    const log = await readLog();
    const shown = findTransition(log, true);
    expect(shown).toBeTruthy();

    const delta = shown!.t - t0;
    // 5ms slack for timer rounding. t0 is taken before the hook's own
    // `defer`, so the true gap can only be larger, never smaller.
    expect(delta).toBeGreaterThanOrEqual(495);
    // Generous upper bound so a stalled timer doesn't pass as "debounced".
    expect(delta).toBeLessThan(1_500);
  });

  test("once shown, the dots stay for at least 1s", async ({ page }) => {
    await page.goto("/");
    await waitForApp(page);
    const readLog = await recordPresence(page);

    await startUserAction(page);
    await expect(page.getByTestId(INDICATOR_TESTID)).toBeVisible();

    await endUserAction(page);
    await expect(page.getByTestId(INDICATOR_TESTID)).toHaveCount(0, {
      timeout: 3_000,
    });

    const log = await readLog();
    const shown = findTransition(log, true);
    expect(shown).toBeTruthy();
    const hidden = findTransition(log, false, shown!.t);
    expect(hidden).toBeTruthy();

    const delta = hidden!.t - shown!.t;
    expect(delta).toBeGreaterThanOrEqual(950);
    expect(delta).toBeLessThan(2_000);
  });

  test("a load longer than 1s hides as soon as it ends", async ({ page }) => {
    await page.goto("/");
    await waitForApp(page);
    const readLog = await recordPresence(page);

    await startUserAction(page);
    await expect(page.getByTestId(INDICATOR_TESTID)).toBeVisible();
    // Already visible for longer than the 1s minimum before we end it.
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
    const log = await readLog();
    const shown = findTransition(log, true);
    expect(shown).toBeTruthy();
    const hidden = findTransition(log, false, shown!.t);
    expect(hidden).toBeTruthy();

    const delta = hidden!.t - t1;
    expect(delta).toBeGreaterThanOrEqual(0);
    expect(delta).toBeLessThan(500);
  });

  test("busy again during the hold does not extend it past the original minimum", async ({
    page,
  }) => {
    await page.goto("/");
    await waitForApp(page);
    const readLog = await recordPresence(page);

    await startUserAction(page);
    await expect(page.getByTestId(INDICATOR_TESTID)).toBeVisible();

    // 800ms, not less: if the hold clock were reset by the second busy
    // spell, the dots would hide ~shown_t + 800 + 1000, comfortably past the
    // bound below. A shorter wait leaves that mutation within ~50ms of it.
    await endUserAction(page);
    await page.waitForTimeout(800);
    await hook(page, "beginUserAction");
    await page.waitForTimeout(200);
    await endUserAction(page);

    await expect(page.getByTestId(INDICATOR_TESTID)).toHaveCount(0, {
      timeout: 3_000,
    });

    const log = await readLog();
    const shown = findTransition(log, true);
    expect(shown).toBeTruthy();
    const hidden = findTransition(log, false, shown!.t);
    expect(hidden).toBeTruthy();

    // The "clock not reset" rule: total visible time is bounded by the
    // ORIGINAL shown_t + the minimum, plus slack — not by the last busy
    // spell restarting a fresh 1s.
    expect(hidden!.t).toBeLessThanOrEqual(shown!.t + 1_000 + 500);
  });

  test("dots ride one wave, and each one visibly varies", async ({ page }) => {
    await page.goto("/");
    await waitForApp(page);
    await startUserAction(page);
    await expect(page.getByTestId(INDICATOR_TESTID)).toBeVisible();
    const dots = page.getByTestId(DOT_TESTID);
    await expect(dots).toHaveCount(10);

    // Every dot runs two animations (`.river-flow-dot` in main.css): the
    // travelling wave (1.5s) and a slower per-dot swell (2.2-3.4s). Told
    // apart by duration, not by keyframe name or shorthand order.
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

    // One wave: every dot rides the same 1.5s period ...
    for (const w of waves) {
      expect(w.duration).toBeGreaterThanOrEqual(1_490);
      expect(w.duration).toBeLessThanOrEqual(1_510);
    }
    // ... with the crest travelling strictly left to right. Fails if the
    // `--i` wiring breaks and the dots move in unison, or if the random
    // nudge ever grows enough to reorder them.
    for (let i = 1; i < waves.length; i++) {
      expect(waves[i].delay).toBeGreaterThan(waves[i - 1].delay);
    }

    // Each dot also swells on its own period, so the row never repeats.
    expect(new Set(swells.map((s) => s.duration)).size).toBeGreaterThan(1);

    // "Each one visibly varies": freeze every dot's swell at its start and
    // its wave at the same phase, then read the rendered offset. Identical
    // amplitude and curve would collapse these to one value.
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
        // The same phase (a quarter into its own cycle), not the same
        // clock time: the delay stagger alone would already differ, and is
        // covered above.
        wave.currentTime = waveDelay + 1_500 / 4;
        ys.push(el.getBoundingClientRect().y);
      }
      // Resume real playback; nothing after this reads these dots again.
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
    // main.css: `river-flow-shimmer`, a 3s opacity-only cycle.
    await expectShimmerInPlace(page.getByTestId(DOT_TESTID).first(), 3_000);
  });

  test("a user action shows nothing unless connected", async ({ page }) => {
    // Connecting and reconnecting are background work for the pill; the
    // primary only follows what the user waits on from a live node.
    await page.goto("/");
    await waitForApp(page);
    await hook(page, "beginUserAction");

    for (const state of ["connecting", "disconnected", "error"]) {
      await setSyncStatus(page, state);
      // The wait gives a wrong implementation time to show up.
      await page.waitForTimeout(1_000);
      await expect(page.getByTestId(INDICATOR_TESTID), state).toHaveCount(0);
    }
  });

  test("the hold keeps the reason", async ({ page }) => {
    await page.goto("/");
    await waitForApp(page);

    await startUserAction(page, "saving");
    const indicator = page.getByTestId(INDICATOR_TESTID);
    await expect(indicator).toBeVisible();
    await expect(indicator).toHaveAttribute("data-reason", "saving");

    await endUserAction(page);
    // Still visible and still reporting "saving" while held — not blank.
    // The wait is what makes this a check on the hold: without it the
    // assertion can pass in the few ms before a hold-less build unmounts
    // the dots.
    await page.waitForTimeout(500);
    await expect(indicator).toHaveCount(1);
    await expect(indicator).toHaveAttribute("data-reason", "saving");

    await expect(indicator).toHaveCount(0, { timeout: 3_000 });
  });

  test("background work never shows the primary dots", async ({ page }) => {
    // Connected with rooms still loading (this build's ROOMS_LOAD_STATE
    // sits at `Loading`) plus an outstanding background request: busy for
    // the pill, idle for the primary.
    await page.goto("/");
    await waitForApp(page);
    await setSyncStatus(page, "connected");
    await hook(page, "beginBackgroundRequest");

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
    await setSyncStatus(page, "connected");
    await settleIdle(page);

    // Queued, then carried by an UPDATE.
    await hook(page, "awaitRoomUpdate");
    await hook(page, "sendRoomUpdate");
    const indicator = page.getByTestId(INDICATOR_TESTID);
    await expect(indicator).toBeVisible();
    await expect(indicator).toHaveAttribute("data-reason", "sending");

    // Still waiting on the node.
    await page.waitForTimeout(1_200);
    await expect(indicator).toBeVisible();

    await hook(page, "answerRoomUpdate");
    await expect(indicator).toHaveCount(0, { timeout: 3_000 });
  });

  test("a rejected UPDATE ends the wait", async ({ page }) => {
    await page.goto("/");
    await waitForApp(page);
    await setSyncStatus(page, "connected");
    await settleIdle(page);

    await hook(page, "awaitRoomUpdate");
    await hook(page, "sendRoomUpdate");
    const indicator = page.getByTestId(INDICATOR_TESTID);
    await expect(indicator).toBeVisible();

    await hook(page, "failRoomUpdate");
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
    // The everyday case the debounce exists for: most replies come back
    // well under 500ms, so they must never cause a visible blip.
    await page.goto("/");
    await waitForApp(page);
    await setSyncStatus(page, "connected");
    await settleIdle(page);

    const readLog = await recordPresence(page);
    await page.evaluate(() => {
      const w = window as any;
      w.__riverTest.awaitRoomUpdate();
      w.__riverTest.sendRoomUpdate();
      setTimeout(() => w.__riverTest.answerRoomUpdate(), 100);
    });

    await page.waitForTimeout(2_000);

    const log = await readLog();
    expect(log.some((e) => e.present)).toBe(false);
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

      // In the flow of the chat section, not fixed to the window.
      const position = await indicator.evaluate(
        (el) => getComputedStyle(el).position
      );
      expect(position).not.toBe("fixed");

      // Centred in the same column as the Welcome copy.
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
      // Below the Welcome copy, as part of it.
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

      // 12px above the composer's top edge.
      const gap = composerBox!.y - (box!.y + box!.height);
      expect(gap).toBeGreaterThanOrEqual(11);
      expect(gap).toBeLessThanOrEqual(13);

      // Centred on the chat column the composer spans.
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

      // The spacer opens (it eases over 150ms) ...
      await expect.poll(spacerHeight).toBeGreaterThanOrEqual(20);

      // ... and the reader, pinned to the bottom when the room opened, stays
      // pinned, with the same 12px between the last message (which ends where
      // the spacer starts) and the dots as between the dots and the composer.
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
      // No `force`: if `pointer-events: none` or the indicator's geometry
      // regressed, this click would either hang on actionability or land on
      // the indicator instead of the button.
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

    // Switch to the rooms panel: the chat section is hidden below 768px,
    // and the dots go with it rather than floating over the room list.
    await page.getByTestId("hamburger-rooms-button").click();
    await expect(page.getByTestId("room-list")).toBeVisible();
    await expect(indicator).toBeHidden();
  });
});
