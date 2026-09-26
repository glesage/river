import { test, expect, Page } from "@playwright/test";
import { waitForApp } from "./example-room";
import { openInviteViaDmPicker } from "./invite-picker";
import { callRiverTest } from "./river-test";

// Test hooks expose states unavailable in no-sync builds: an unsigned room
// awaiting its first GET, and an invite send held in flight. Without
// holdInviteSend, no-sync sends finish too quickly to observe. The wave dots of
// the rooms loading/migrating states are checked in rooms-loading-state.spec.ts.

const SPINNER = ".animate-spin";

async function expectSmallDots(page: Page, scope: ReturnType<Page["locator"]>, testid: string) {
  const dots = scope.getByTestId(testid);
  await expect(dots).toBeVisible();
  const dot = dots.locator(".pill-activity-dot");
  await expect(dot).toHaveCount(5);
  // The container shares the pill's layout but must never collapse like it.
  const layout = await dots.evaluate((el) => {
    const style = getComputedStyle(el);
    return {
      display: style.display,
      alignItems: style.alignItems,
      gap: style.gap,
      height: style.height,
      flexShrink: style.flexShrink,
    };
  });
  // Every host is a flex item, which blockifies inline-flex to flex.
  expect(layout.display).toMatch(/^(inline-)?flex$/);
  expect(layout).toMatchObject({
    alignItems: "center",
    gap: "2px",
    height: "8px",
    flexShrink: "0",
  });
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

    test("a room waiting for its first sync shows small dots in its row and banner", async ({
      page,
    }) => {
      const ROOM = "Your Private Room";
      await callRiverTest(page, "awaitRoomSync", ROOM);
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

      await callRiverTest(page, "holdInviteSend");
      const footer = page.getByText("Sending invite…");
      await expect(footer).toBeVisible();
      await expectSmallDots(page, page.locator("body"), "invite-sending-dots");
      await expect(page.locator(SPINNER)).toHaveCount(0);
    });
  });
}
