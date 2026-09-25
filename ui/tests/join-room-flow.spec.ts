import { test, expect, Page } from "@playwright/test";
import { waitForApp } from "./example-room";
import { callRiverTest } from "./river-test";

// No-sync needs an explicit Connected status for the loading indicator.
// The test invitation targets a room absent from example data; without a
// synchronizer, its join stays pending until finishTestJoin / failTestJoin.

const INDICATOR = "network-activity-indicator";

// App reads MOBILE_VIEW, so switching panels re-runs invitation recovery.
// A desktop room click alone does not; narrow the viewport to reach the hamburger.
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


async function acceptTestInvitation(page: Page) {
  await callRiverTest(page, "setSyncStatus", "connected");
  await callRiverTest(page, "presentTestInvitation");
  const modal = page.getByTestId("receive-invitation-modal");
  await expect(modal).toBeVisible({ timeout: 5_000 });
  await page.getByTestId("receive-invitation-accept-button").click();
  await expect(modal).toHaveCount(0);
}

test.describe("Joining a room", () => {
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

  test("a finished join says so in a toast", async ({ page }) => {
    await acceptTestInvitation(page);
    await expect(page.getByTestId(INDICATOR)).toBeVisible();

    await callRiverTest(page, "finishTestJoin");
    const toast = page.getByTestId("toast");
    await expect(toast).toHaveText(/Room joined/);
    await expect(toast).toHaveAttribute("data-kind", "info");
    await expect(page.getByTestId(INDICATOR)).toHaveCount(0, { timeout: 3_000 });
  });

  test("a failed join shows an error toast whose Retry restarts it", async ({ page }) => {
    await acceptTestInvitation(page);
    await expect(page.getByTestId(INDICATOR)).toBeVisible();

    await callRiverTest(page, "failTestJoin");
    const toast = page.getByTestId("toast");
    await expect(toast).toHaveAttribute("data-kind", "error");
    await expect(toast).toContainText("Couldn't join the room: room is at capacity");
    await expect(page.getByTestId(INDICATOR), "a failed join is not running").toHaveCount(0, {
      timeout: 3_000,
    });


    await page.getByTestId("toast-action").click();
    await expect(toast).toHaveCount(0);
    await expect(page.getByTestId(INDICATOR)).toHaveAttribute("data-reason", "joining-room");
    await expect(page.getByTestId("receive-invitation-modal")).toHaveCount(0);
  });

  test("after a failure, asking for the invitation again reopens it", async ({ page }) => {
    await acceptTestInvitation(page);
    await callRiverTest(page, "failTestJoin");
    await expect(page.getByTestId("toast")).toHaveAttribute("data-kind", "error");


    await callRiverTest(page, "presentTestInvitation");
    await expect(page.getByTestId("receive-invitation-modal")).toBeVisible();
    await expect(page.getByTestId("receive-invitation-accept-button")).toBeVisible();
  });
});

for (const { label, viewport, isMobile } of [
  { label: "mobile", viewport: { width: 390, height: 844 }, isMobile: true },
  { label: "desktop", viewport: { width: 1280, height: 800 }, isMobile: false },
]) {
  test.describe(`Joining a room, navigation (${label})`, () => {
    test.use({ viewport });

    test.beforeEach(async ({ page }) => {
      await page.goto("/");
      await waitForApp(page);
    });

    test("the modal does not come back while the join runs", async ({ page }) => {
      await acceptTestInvitation(page);
      await expect(page.getByTestId(INDICATOR)).toBeVisible();

      await rerenderApp(page, isMobile);
      await page.waitForTimeout(1_000);
      await expect(page.getByTestId("receive-invitation-modal")).toHaveCount(0);
    });

    test("after a failure, nothing reopens or restarts the join on its own", async ({ page }) => {
      await acceptTestInvitation(page);
      await callRiverTest(page, "failTestJoin");
      await expect(page.getByTestId("toast")).toHaveAttribute("data-kind", "error");
      await expect(page.getByTestId(INDICATOR)).toHaveCount(0, { timeout: 3_000 });


      await rerenderApp(page, isMobile);
      await page.waitForTimeout(1_500);
      await expect(page.getByTestId("receive-invitation-modal")).toHaveCount(0);
      await expect(page.getByTestId(INDICATOR)).toHaveCount(0);
      await expect(page.getByTestId("toast")).toHaveAttribute("data-kind", "error");
    });
  });
}

test.describe("Joining a room (mobile-only navigation)", () => {
  test.use({ viewport: { width: 390, height: 844 } });

  test.beforeEach(async ({ page }) => {
    await page.goto("/");
    await waitForApp(page);
  });

  test("mobile: accepting from the rooms list switches to the chat", async ({ page }) => {
    await page.getByTestId("hamburger-rooms-button").click();
    await expect(page.getByTestId("room-list")).toBeVisible();

    await acceptTestInvitation(page);
    await expect(page.getByTestId("room-list")).toBeHidden();
    await expect(page.getByTestId(INDICATOR)).toBeVisible();
  });
});
