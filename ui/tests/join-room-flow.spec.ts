import { test, expect, Page } from "@playwright/test";
import { waitForApp } from "./example-room";

// Joining a room from an invitation. Once the user clicks Accept, the modal
// closes and the only join UI is the big loading dots ("Joining room…")
// through both of its progress states (PendingSubscription, Subscribing),
// ended by a toast: "Joined {room}" on success, or an error toast with Retry.
// The old in-modal "Preparing to subscribe to room..." / "Subscribing to
// room..." screens are gone.
//
// PREMISES (see `example_data.rs::install_test_hooks`):
//   - A no-sync build is `Disconnected` for good, and the big dots need
//     `Connected`, so every test starts with `setSyncStatus("connected")`.
//   - `presentTestInvitation()` opens the invitation modal for a room the
//     example data doesn't have. With no synchronizer, an accepted join stays
//     pending until `finishTestJoin()` or `failTestJoin()` ends it.

const INDICATOR = "network-activity-indicator";

async function hook(page: Page, name: string, arg?: string) {
  await page.evaluate(
    ({ name, arg }) => {
      (window as any).__riverTest[name](arg);
    },
    { name, arg }
  );
}

/**
 * Re-render `App`, whose body re-runs the invitation openers and the recovery
 * path. Switching the mobile panel writes `MOBILE_VIEW`, which `App` reads; a
 * desktop room click alone does not (the view is already the chat), so the
 * desktop run narrows the window to reach the hamburger, then restores it.
 */
async function rerenderApp(page: Page, isMobile: boolean) {
  const viewport = page.viewportSize()!;
  if (!isMobile) {
    await page.setViewportSize({ width: 390, height: 844 });
  }
  await page.getByTestId("hamburger-rooms-button").click();
  await page
    .getByTestId("room-list")
    .getByRole("button", { name: "Public Discussion Room" })
    .click();
  if (!isMobile) {
    await page.setViewportSize(viewport);
  }
}

/** Present the test invitation and accept it with the default nickname. */
async function acceptTestInvitation(page: Page) {
  await hook(page, "setSyncStatus", "connected");
  await hook(page, "presentTestInvitation");
  const modal = page.getByTestId("receive-invitation-modal");
  await expect(modal).toBeVisible({ timeout: 5_000 });
  await page.getByTestId("receive-invitation-accept-button").click();
  await expect(modal).toHaveCount(0);
}

for (const { label, viewport, isMobile } of [
  { label: "mobile", viewport: { width: 390, height: 844 }, isMobile: true },
  { label: "desktop", viewport: { width: 1280, height: 800 }, isMobile: false },
]) {
  test.describe(`Joining a room (${label})`, () => {
    test.use({ viewport });

    test.beforeEach(async ({ page }) => {
      await page.goto("/");
      await waitForApp(page);
    });

    test("Accept closes the modal and the big dots show the join", async ({ page }) => {
      await acceptTestInvitation(page);

      const dots = page.getByTestId(INDICATOR);
      await expect(dots).toBeVisible();
      await expect(dots).toHaveAttribute("data-reason", "joining-room");
      await expect(page.getByTestId("network-activity-status")).toHaveText("Joining room…");
      await expect(page.getByText("Preparing to subscribe")).toHaveCount(0);
      await expect(page.getByText("Subscribing to room")).toHaveCount(0);
    });

    test("the modal does not come back while the join runs", async ({ page }) => {
      await acceptTestInvitation(page);
      await expect(page.getByTestId(INDICATOR)).toBeVisible();

      await rerenderApp(page, isMobile);
      await page.waitForTimeout(1_000);
      await expect(page.getByTestId("receive-invitation-modal")).toHaveCount(0);
    });

    test("a finished join says so in a toast", async ({ page }) => {
      await acceptTestInvitation(page);
      await expect(page.getByTestId(INDICATOR)).toBeVisible();

      await hook(page, "finishTestJoin");
      const toast = page.getByTestId("toast");
      await expect(toast).toHaveText(/Room joined/);
      await expect(toast).toHaveAttribute("data-kind", "info");
      await expect(page.getByTestId(INDICATOR)).toHaveCount(0, { timeout: 3_000 });
    });

    test("a failed join shows an error toast whose Retry restarts it", async ({ page }) => {
      await acceptTestInvitation(page);
      await expect(page.getByTestId(INDICATOR)).toBeVisible();

      await hook(page, "failTestJoin");
      const toast = page.getByTestId("toast");
      await expect(toast).toHaveAttribute("data-kind", "error");
      await expect(toast).toContainText("Couldn't join the room: room is at capacity");
      await expect(page.getByTestId(INDICATOR), "a failed join is not running").toHaveCount(0, {
        timeout: 3_000,
      });
      // Still up after a normal toast would have gone.
      await page.waitForTimeout(5_500);
      await expect(toast).toBeVisible();

      await page.getByTestId("toast-action").click();
      await expect(toast).toHaveCount(0);
      await expect(page.getByTestId(INDICATOR)).toHaveAttribute("data-reason", "joining-room");
      await expect(page.getByTestId("receive-invitation-modal")).toHaveCount(0);
    });

    test("after a failure, nothing reopens or restarts the join on its own", async ({ page }) => {
      await acceptTestInvitation(page);
      await hook(page, "failTestJoin");
      await expect(page.getByTestId("toast")).toHaveAttribute("data-kind", "error");
      await expect(page.getByTestId(INDICATOR)).toHaveCount(0, { timeout: 3_000 });

      // A failed join is left for the user's Retry: no modal, and no silent
      // re-accept bringing the dots back.
      await rerenderApp(page, isMobile);
      await page.waitForTimeout(1_500);
      await expect(page.getByTestId("receive-invitation-modal")).toHaveCount(0);
      await expect(page.getByTestId(INDICATOR)).toHaveCount(0);
      await expect(page.getByTestId("toast")).toHaveAttribute("data-kind", "error");
    });

    test("after a failure, asking for the invitation again reopens it", async ({ page }) => {
      await acceptTestInvitation(page);
      await hook(page, "failTestJoin");
      await expect(page.getByTestId("toast")).toHaveAttribute("data-kind", "error");

      // An explicit request (a DM card's Accept, a link click) may reopen a
      // FAILED join's invitation, with the normal options to accept again.
      await hook(page, "presentTestInvitation");
      await expect(page.getByTestId("receive-invitation-modal")).toBeVisible();
      await expect(page.getByTestId("receive-invitation-accept-button")).toBeVisible();
    });

    test("mobile: accepting from the rooms list switches to the chat", async ({ page }) => {
      test.skip(!isMobile, "mobile only: desktop shows every panel at once");

      await page.getByTestId("hamburger-rooms-button").click();
      await expect(page.getByTestId("room-list")).toBeVisible();

      await acceptTestInvitation(page);
      await expect(page.getByTestId("room-list")).toBeHidden();
      await expect(page.getByTestId(INDICATOR)).toBeVisible();
    });
  });
}
