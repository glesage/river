import { test, expect, Page } from "@playwright/test";
import { waitForApp } from "./example-room";

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

// Whether a member row's display text marks it as the local user (mirrors
// `invite-via-dm-picker.spec.ts::isSelfRowText`): `member_display_parts`
// (members.rs) gives every self row — and only the self row — a ⭐ badge.
function isSelfRowText(text: string): boolean {
  return text.includes("⭐");
}

/**
 * Select a room that lists the local user as a Member, open the member-info
 * modal for another member, and click "Share an invite via DM…", the way
 * `invite-via-dm-picker.spec.ts::openMemberInfo` /
 * `openPickerAndReadTitle` do. Returns whether the picker opened; when it
 * didn't (no non-self member row, or no "Share an invite via DM" entry
 * point — observer-only example data), the caller should skip.
 */
async function openInviteViaDmPicker(page: Page): Promise<boolean> {
  // Example-data's "Team Chat Room" lists the local user as a Member.
  await page.getByText("Team Chat Room").first().click();

  // The member list renders after the room hydrates; wait for at least one
  // member row before iterating, so we don't race the first paint.
  await page
    .locator('button[title^="Member ID"]')
    .first()
    .waitFor({ state: "visible", timeout: 5_000 })
    .catch(() => undefined);

  const memberButtons = page.locator('button[title^="Member ID"]');
  const count = await memberButtons.count();
  let openedMemberInfo = false;
  for (let i = 0; i < count; i++) {
    const text = (await memberButtons.nth(i).textContent()) || "";
    if (!isSelfRowText(text)) {
      await memberButtons.nth(i).click();
      openedMemberInfo = true;
      break;
    }
  }
  if (!openedMemberInfo) {
    return false;
  }

  const shareInvite = page.getByRole("button", { name: /share an invite/i }).first();
  await shareInvite.waitFor({ state: "visible", timeout: 5_000 }).catch(() => undefined);
  if (!(await shareInvite.isVisible().catch(() => false))) {
    return false;
  }
  await shareInvite.click();

  const header = page.getByRole("heading", { name: /invite .+ to another room/i });
  await expect(header).toBeVisible({ timeout: 5_000 });
  return true;
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

      const opened = await openInviteViaDmPicker(page);
      if (!opened) {
        test.skip(true, "no 'Share an invite via DM' entry point — example data may be observer-only");
        return;
      }

      await hook(page, "holdInviteSend");
      const footer = page.getByText("Sending invite…");
      await expect(footer).toBeVisible();
      await expectSmallDots(page, page.locator("body"), "invite-sending-dots");
      await expect(page.locator(SPINNER)).toHaveCount(0);
    });
  });
}
