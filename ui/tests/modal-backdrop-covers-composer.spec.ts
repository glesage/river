import { test, expect, Page } from "@playwright/test";
import { waitForApp } from "./example-room";

// A modal's dimming backdrop must paint over the message composer.
//
// The composer bar is `relative z-50` (it has to sit above its own z-40
// emoji / @mention click-catchers). The invite-member and create-room modals
// used to put their backdrop at z-40, so it dimmed the whole app EXCEPT the
// composer, which stayed bright in front of the overlay.
//
// `elementFromPoint` cannot see this: the modal's transparent full-viewport
// centering wrapper (z-50) sits above both and would be returned either way.
// `elementsFromPoint` returns the whole stack in paint order, top first, so
// the backdrop has to come before every composer element at a point inside
// the composer.

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

/** Paint order at a point inside the composer, clear of the modal card. */
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
