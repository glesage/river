import { test, expect, Page } from "@playwright/test";
import { waitForApp } from "./example-room";
import { openInviteViaDmPicker } from "./invite-picker";

// Every spinner in the UI is now loading dots (docs/plans/spinners-to-dots.md):
// wave dots (`.river-flow-dot`, ten per row) wherever the rooms rail or the
// no-room screen waits on the rooms, and the connection pill's small dots
// (`.pill-activity-dot`, five per row, accent blue) inline elsewhere — a
// room row and its conversation banner while that room awaits its first
// sync, and the invite-via-DM picker's footer while a send is in flight.
// No `.animate-spin` should remain anywhere on the page in any of these
// states.
//
// PREMISES (see `example_data.rs::install_test_hooks`):
//   - `setRoomsLoadState(state)` drives the rooms rail / no-room screen's
//     Loading and Migrating states, otherwise unreachable in a no-sync
//     browser build (freenet/river#509).
//   - `awaitRoomSync(roomName)` gives the named example room an unsigned
//     default state (keeping its name), so
//     `RoomData::is_awaiting_initial_sync()` is true — the same shape an
//     imported room has before its first GET.
//   - `holdInviteSend()` holds the invite-via-DM picker's
//     `INVITE_VIA_DM_PICKER_INFLIGHT` on, since a no-sync send finishes too
//     fast to observe the "Sending invite…" state otherwise.

const SPINNER = ".animate-spin";

async function hook(page: Page, name: string, arg?: string) {
  await page.evaluate(({ name, arg }) => (window as any).__riverTest[name](arg), { name, arg });
}

/** `testid` holds ten wave dots, and no spinner is left anywhere on the page. */
async function expectWaveDots(page: Page, testid: string) {
  const dots = page.getByTestId(testid);
  await expect(dots).toBeVisible();
  await expect(dots.locator(".river-flow-dot")).toHaveCount(10);
  await expect(page.locator(SPINNER)).toHaveCount(0);
}

/** `testid`, scoped to `scope`, holds the pill's five small dots, in accent blue, animating. */
async function expectSmallDots(page: Page, scope: ReturnType<Page["locator"]>, testid: string) {
  const dots = scope.getByTestId(testid);
  await expect(dots).toBeVisible();
  const dot = dots.locator(".pill-activity-dot");
  await expect(dot).toHaveCount(5);
  const { color, accent, playing } = await dot.first().evaluate((el) => {
    const probe = document.createElement("span");
    probe.className = "text-accent";
    document.body.append(probe);
    const accent = getComputedStyle(probe).color;
    probe.remove();
    const style = getComputedStyle(el);
    return {
      color: style.backgroundColor,
      accent,
      playing: style.animationPlayState,
    };
  });
  expect(color, "small dots are accent blue").toBe(accent);
  // Outside the pill, the pill's "paused while closed" rule must not apply.
  expect(playing).toBe("running");
}

for (const { label, viewport, isMobile } of [
  { label: "mobile", viewport: { width: 390, height: 844 }, isMobile: true },
  { label: "desktop", viewport: { width: 1280, height: 800 }, isMobile: false },
]) {
  test.describe(`Loading dots (${label})`, () => {
    test.use({ viewport });

    test.beforeEach(async ({ page }) => {
      await page.goto("/");
      await waitForApp(page);
    });

    for (const state of ["loading", "migrating"]) {
      test(`${state} rooms show wave dots, not a spinner`, async ({ page }) => {
        await hook(page, "setRoomsLoadState", state);
        await expectWaveDots(page, `conversation-rooms-${state}-dots`);
        if (!isMobile) {
          // Below 768px the rail is display:none.
          await expectWaveDots(page, `room-list-${state}-dots`);
        }
      });
    }

    test("a room waiting for its first sync shows small dots in its row and banner", async ({
      page,
    }) => {
      const ROOM = "Your Private Room";
      await hook(page, "awaitRoomSync", ROOM);
      if (isMobile) {
        await page.getByTestId("hamburger-rooms-button").click();
      }
      const roomList = page.getByTestId("room-list");
      const row = roomList.getByRole("button", { name: /Your Private Room/ });
      await expectSmallDots(page, roomList, "room-sync-dots");

      await row.click();
      const banner = page.getByTestId("room-sync-banner");
      await expect(banner).toContainText("Syncing room state from the network");
      await expectSmallDots(page, banner, "room-sync-banner-dots");
      await expect(page.locator(SPINNER)).toHaveCount(0);
    });

    test("the invite-via-DM picker shows small dots while it sends", async ({ page }) => {
      test.skip(isMobile, "the picker flow is covered on desktop");

      await openInviteViaDmPicker(page);

      await hook(page, "holdInviteSend");
      const footer = page.getByText("Sending invite…");
      await expect(footer).toBeVisible();
      await expectSmallDots(page, page.locator("body"), "invite-sending-dots");
      await expect(page.locator(SPINNER)).toHaveCount(0);
    });
  });
}
