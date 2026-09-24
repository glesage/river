import { test, expect, Page } from "@playwright/test";
import { selectListedRoom } from "./example-room";

// Coverage for the network activity indicator
// (docs/plans/2026-09-24-network-activity-indicator.md): ten accent-blue
// dots, fixed at the bottom-centre of the page, that appear while River is
// connecting, reconnecting, joining a room, loading/syncing rooms, or
// (Phase 2) has a GET/UPDATE in flight. It is debounced 200ms — a load that
// finishes sooner shows nothing — and held on screen for at least 1.5s (one
// full CSS wave period) once shown, so a short load still reads as one
// ripple rather than a blip.
//
// PREMISES this spec relies on (see the plan's "Premises of the no-sync
// example build" and `example_data.rs::install_test_hooks`):
//   - `SYNC_STATUS` starts at `Disconnected`, and a `no-sync` build never
//     writes it again on its own ("Disconnected" only means "reconnecting"
//     in a sync build). So the indicator is idle at load, and
//     `window.__riverTest.setSyncStatus(state)` (state: "connecting" |
//     "connected" | "disconnected" | "error") is the only way to reach its
//     busy states here.
//   - `window.__riverTest.setRoomsLoadState(state)` (already used by
//     rooms-loading-state.spec.ts) ALSO clears ROOMS and CURRENT_ROOM as a
//     side effect. Tests below only call it where that side effect doesn't
//     matter to what's being asserted.
//   - `ROOMS_LOAD_STATE` sits at its `Loading` default forever in this
//     build — nothing in the example fixture advances it — which is the
//     premise the "connected with rooms still loading" test depends on.
//   - Phase 2's `window.__riverTest.beginActivity(kind)` /
//     `endActivity(kind)` (kind: "fetch" | "send") drive the in-flight
//     tracker directly, independent of any real network traffic.
//
// Two timing rules apply to every test below (plan Task 9):
//   - Appearing takes ~200ms after the triggering hook call. The default
//     `toBeVisible()` timeout (5s) already absorbs that, so no special
//     handling is needed for "the dots should show up" assertions.
//   - Disappearing can take up to 1.5s after the load ends (the minimum-
//     visible hold). Every "gone" assertion therefore uses
//     `toHaveCount(0, { timeout: 3_000 })` rather than the default —  a bare
//     `toHaveCount(0)` would race the hold and flake.

const INDICATOR_TESTID = "network-activity-indicator";
const DOT_TESTID = "network-activity-dot";
const STATUS_TESTID = "network-activity-status";

async function waitForApp(page: Page) {
  await page.waitForSelector(".app-root", { timeout: 30_000 });
  await expect(page.locator("aside, .app-root button")).not.toHaveCount(0);
}

async function setSyncStatus(page: Page, state: string) {
  await page.evaluate((s) => {
    (window as any).__riverTest.setSyncStatus(s);
  }, state);
}

async function setLoadState(page: Page, state: string) {
  await page.evaluate((s) => {
    (window as any).__riverTest.setRoomsLoadState(s);
  }, state);
}

// Wait until the gate is Idle, not merely "no dots in the DOM": a count-0
// check alone can pass while a debounce is still pending and the dots are
// about to appear. Past the debounce, a pending show has either mounted (and
// the count waits out its hold) or been cancelled.
async function settleIdle(page: Page) {
  await page.waitForTimeout(300);
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

for (const { label, viewport, isMobile } of [
  { label: "mobile", viewport: { width: 390, height: 844 }, isMobile: true },
  { label: "desktop", viewport: { width: 1280, height: 800 }, isMobile: false },
]) {
  test.describe(`Network activity indicator (${label})`, () => {
    test.use({ viewport });

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

    test("a load shorter than 200ms never shows the dots", async ({
      page,
    }) => {
      await page.goto("/");
      await waitForApp(page);
      const readLog = await recordPresence(page);

      await page.evaluate(() => {
        const w = window as any;
        w.__riverTest.setSyncStatus("connecting");
        setTimeout(() => w.__riverTest.setSyncStatus("disconnected"), 100);
      });

      await page.waitForTimeout(2_000);

      const log = await readLog();
      expect(log.some((e) => e.present)).toBe(false);
    });

    test("the dots appear only after the 200ms debounce", async ({
      page,
    }) => {
      await page.goto("/");
      await waitForApp(page);
      const readLog = await recordPresence(page);

      await page.evaluate(() => {
        const w = window as any;
        w.__t0 = performance.now();
        w.__riverTest.setSyncStatus("connecting");
      });

      await expect(page.getByTestId(INDICATOR_TESTID)).toBeVisible();

      const t0 = await page.evaluate(() => (window as any).__t0);
      const log = await readLog();
      const shown = findTransition(log, true);
      expect(shown).toBeTruthy();

      const delta = shown!.t - t0;
      // 5ms slack for timer rounding. t0 is taken before the hook's own
      // `defer`, so the true gap can only be larger, never smaller.
      expect(delta).toBeGreaterThanOrEqual(195);
      // Generous upper bound so a stalled timer doesn't pass as "debounced".
      expect(delta).toBeLessThan(1_000);
    });

    test("once shown, the dots stay for at least 1.5s", async ({ page }) => {
      await page.goto("/");
      await waitForApp(page);
      const readLog = await recordPresence(page);

      await setSyncStatus(page, "connecting");
      await expect(page.getByTestId(INDICATOR_TESTID)).toBeVisible();

      await setSyncStatus(page, "disconnected");
      await expect(page.getByTestId(INDICATOR_TESTID)).toHaveCount(0, {
        timeout: 3_000,
      });

      const log = await readLog();
      const shown = findTransition(log, true);
      expect(shown).toBeTruthy();
      const hidden = findTransition(log, false, shown!.t);
      expect(hidden).toBeTruthy();

      const delta = hidden!.t - shown!.t;
      expect(delta).toBeGreaterThanOrEqual(1_450);
      expect(delta).toBeLessThan(2_500);
    });

    test("a load longer than 1.5s hides as soon as it ends", async ({
      page,
    }) => {
      await page.goto("/");
      await waitForApp(page);
      const readLog = await recordPresence(page);

      await setSyncStatus(page, "connecting");
      await expect(page.getByTestId(INDICATOR_TESTID)).toBeVisible();
      // Already visible for longer than the 1.5s minimum before we end it.
      await page.waitForTimeout(2_500);

      await page.evaluate(() => {
        const w = window as any;
        w.__t1 = performance.now();
        w.__riverTest.setSyncStatus("disconnected");
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

      await setSyncStatus(page, "connecting");
      await expect(page.getByTestId(INDICATOR_TESTID)).toBeVisible();

      // 800ms, not less: if the hold clock were reset by the second busy
      // spell, the dots would hide ~shown_t + 800 + 1500, comfortably past the
      // bound below. A shorter wait leaves that mutation within ~50ms of it.
      await setSyncStatus(page, "disconnected");
      await page.waitForTimeout(800);
      await setSyncStatus(page, "connecting");
      await page.waitForTimeout(200);
      await setSyncStatus(page, "disconnected");

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
      // spell restarting a fresh 1.5s.
      expect(hidden!.t).toBeLessThanOrEqual(shown!.t + 1_500 + 500);
    });

    test("connecting shows ten dots at the bottom centre", async ({
      page,
    }) => {
      await page.goto("/");
      await waitForApp(page);
      await setSyncStatus(page, "connecting");

      const indicator = page.getByTestId(INDICATOR_TESTID);
      await expect(indicator).toBeVisible();
      await expect(indicator).toHaveAttribute("data-reason", "connecting");
      await expect(page.getByTestId(DOT_TESTID)).toHaveCount(10);

      const vp = page.viewportSize();
      expect(vp).toBeTruthy();
      const box = await indicator.boundingBox();
      expect(box).toBeTruthy();

      const bottomGap = vp!.height - (box!.y + box!.height);
      expect(bottomGap).toBeLessThanOrEqual(14);

      const centerX = box!.x + box!.width / 2;
      expect(Math.abs(centerX - vp!.width / 2)).toBeLessThanOrEqual(2);

      await expect(page.getByTestId(STATUS_TESTID)).toContainText(
        "Connecting"
      );
    });

    test("dots are phase-shifted on one wave", async ({ page }) => {
      await page.goto("/");
      await waitForApp(page);
      await setSyncStatus(page, "connecting");
      await expect(page.getByTestId(INDICATOR_TESTID)).toBeVisible();
      await expect(page.getByTestId(DOT_TESTID)).toHaveCount(10);

      const styles = await page
        .getByTestId(DOT_TESTID)
        .evaluateAll((els) =>
          els.map((el) => {
            const cs = getComputedStyle(el as HTMLElement);
            return { name: cs.animationName, delay: cs.animationDelay };
          })
        );

      expect(styles).toHaveLength(10);
      for (const s of styles) {
        expect(s.name).toBe("river-flow-wave");
      }

      const parseSeconds = (v: string) => {
        const n = parseFloat(v);
        return v.trim().endsWith("ms") ? n / 1000 : n;
      };
      const delays = styles.map((s) => parseSeconds(s.delay));

      // Strictly increasing across the row — if the `--i` wiring breaks and
      // every dot shares one delay, this fails.
      for (let i = 1; i < delays.length; i++) {
        expect(delays[i]).toBeGreaterThan(delays[i - 1]);
      }
    });

    test("does not intercept taps", async ({ page }) => {
      await page.goto("/");
      await waitForApp(page);
      await setSyncStatus(page, "connecting");

      const indicator = page.getByTestId(INDICATOR_TESTID);
      await expect(indicator).toBeVisible();

      const pointerEvents = await indicator.evaluate(
        (el) => getComputedStyle(el).pointerEvents
      );
      expect(pointerEvents).toBe("none");

      if (isMobile) {
        await page.getByTestId("hamburger-rooms-button").click();
      }
      // Self owns this example room, so it has a composer. Not simply the
      // first room item: in some rooms self is not a member, and the composer
      // is replaced by the "you're not a member" notice.
      await selectListedRoom(page, "Your Private Room");

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

    test("mobile rooms panel: no overlap with the connection pill", async ({
      page,
    }) => {
      test.skip(!isMobile, "mobile only");

      await page.goto("/");
      await waitForApp(page);
      await setSyncStatus(page, "connecting");

      const indicator = page.getByTestId(INDICATOR_TESTID);
      await expect(indicator).toBeVisible();

      await page.getByTestId("hamburger-rooms-button").click();
      const pill = page.locator(
        '[data-testid="connection-status-indicator"]:visible'
      );
      await expect(pill).toHaveCount(1);

      const indicatorBox = await indicator.boundingBox();
      const pillBox = await pill.boundingBox();
      expect(indicatorBox).toBeTruthy();
      expect(pillBox).toBeTruthy();
      expect(boxesIntersect(indicatorBox as Rect, pillBox as Rect)).toBe(
        false
      );
    });

    test("reduced motion swaps the wave for a shimmer", async ({ page }) => {
      await page.emulateMedia({ reducedMotion: "reduce" });
      await page.goto("/");
      await waitForApp(page);
      await setSyncStatus(page, "connecting");

      const indicator = page.getByTestId(INDICATOR_TESTID);
      await expect(indicator).toBeVisible();

      const name = await page
        .getByTestId(DOT_TESTID)
        .first()
        .evaluate((el) => getComputedStyle(el).animationName);
      expect(name).toBe("river-flow-shimmer");
    });

    test("disconnected in a no-sync build stays hidden, error stays hidden", async ({
      page,
    }) => {
      await page.goto("/");
      await waitForApp(page);

      await setSyncStatus(page, "connecting");
      await expect(page.getByTestId(INDICATOR_TESTID)).toBeVisible();

      await setSyncStatus(page, "disconnected");
      await expect(page.getByTestId(INDICATOR_TESTID)).toHaveCount(0, {
        timeout: 3_000,
      });

      await setSyncStatus(page, "error");
      // `Error` is idle, so nothing may appear. The wait gives a wrong
      // implementation time to show up rather than racing a fast check.
      await page.waitForTimeout(1_000);
      await expect(page.getByTestId(INDICATOR_TESTID)).toHaveCount(0);
    });

    test("the hold keeps the reason", async ({ page }) => {
      await page.goto("/");
      await waitForApp(page);

      await setSyncStatus(page, "connecting");
      const indicator = page.getByTestId(INDICATOR_TESTID);
      await expect(indicator).toBeVisible();
      await expect(indicator).toHaveAttribute("data-reason", "connecting");

      await setSyncStatus(page, "disconnected");
      // Still visible and still reporting "connecting" while held — not
      // blank, and not swapped to a different reason. The wait is what makes
      // this a check on the hold: without it the assertion can pass in the
      // few ms before a hold-less build unmounts the dots.
      await page.waitForTimeout(500);
      await expect(indicator).toHaveCount(1);
      await expect(indicator).toHaveAttribute("data-reason", "connecting");

      await expect(indicator).toHaveCount(0, { timeout: 3_000 });
    });

    test("connected with rooms still loading shows loading-rooms", async ({
      page,
    }) => {
      // PREMISE: this build's ROOMS_LOAD_STATE sits at its `Loading` default
      // forever — nothing in the example fixture advances it — so calling
      // ONLY `setSyncStatus("connected")`, with no `setRoomsLoadState` call,
      // is enough to observe `data-reason="loading-rooms"` (row 5 of the
      // `loading_reason` table). If the example fixture ever starts writing
      // ROOMS_LOAD_STATE on its own, this test would stop proving anything.
      await page.goto("/");
      await waitForApp(page);

      await setSyncStatus(page, "connected");
      const indicator = page.getByTestId(INDICATOR_TESTID);
      await expect(indicator).toBeVisible();
      await expect(indicator).toHaveAttribute("data-reason", "loading-rooms");

      await setLoadState(page, "loaded");
      await expect(indicator).toHaveCount(0, { timeout: 3_000 });
    });
  });
}
