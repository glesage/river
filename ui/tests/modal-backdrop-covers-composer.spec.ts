import { test, expect, Page } from "@playwright/test";
import { waitForApp } from "./example-room";

// The composer needs z-50 above its own click-catchers; z-40 modal backdrops
// previously left it undimmed. elementFromPoint only finds the modal's
// transparent wrapper, so use elementsFromPoint to inspect the paint order.

const ROOM_NAME = "Public Discussion Room";

async function selectRoom(page: Page) {
  const roomBtn = page.getByRole("button", { name: ROOM_NAME });
  await expect(roomBtn).toBeVisible({ timeout: 5_000 });
  await roomBtn.click();
  await expect(page.getByRole("heading", { name: ROOM_NAME })).toBeVisible({
    timeout: 5_000,
  });
  await expect(page.getByTestId("message-composer")).toBeVisible();
}

// Sample near the composer's edge to avoid the modal card.
async function backdropIsAboveComposer(page: Page, backdropTestId: string) {
  return page.evaluate((backdropTestId) => {
    const composer = document.querySelector(
      '[data-testid="message-composer"]',
    ) as HTMLElement;
    const backdrop = document.querySelector(
      `[data-testid="${backdropTestId}"]`,
    ) as HTMLElement;
    const r = composer.getBoundingClientRect();
    const stack = document.elementsFromPoint(r.left + 8, r.top + r.height / 2);
    const backdropAt = stack.indexOf(backdrop);
    const composerAt = stack.findIndex((el) => composer.contains(el));
    return { backdropAt, composerAt };
  }, backdropTestId);
}

test.describe("Modal backdrop covers the message composer", () => {
  test.use({ viewport: { width: 1280, height: 800 } });

  test.beforeEach(async ({ page }) => {
    await page.goto("/");
    await waitForApp(page);
    await selectRoom(page);
  });

  test("invite-member modal", async ({ page }) => {
    await page.getByTestId("invite-member-button").click();
    await expect(page.getByTestId("invite-member-modal")).toBeVisible();

    const { backdropAt, composerAt } = await backdropIsAboveComposer(
      page,
      "invite-member-backdrop",
    );
    expect(backdropAt).toBeGreaterThanOrEqual(0);
    expect(composerAt).toBeGreaterThanOrEqual(0);
    expect(backdropAt).toBeLessThan(composerAt);
  });

  test("create-room modal", async ({ page }) => {
    await page.getByTestId("create-room-button").click();
    await expect(page.getByTestId("create-room-modal")).toBeVisible();

    const { backdropAt, composerAt } = await backdropIsAboveComposer(
      page,
      "create-room-backdrop",
    );
    expect(backdropAt).toBeGreaterThanOrEqual(0);
    expect(composerAt).toBeGreaterThanOrEqual(0);
    expect(backdropAt).toBeLessThan(composerAt);
  });
});
