import { test, expect, Page } from "@playwright/test";
import { waitForApp, openRoomWithComposer } from "./example-room";


const TOAST = "toast";

async function hook(page: Page, name: string, ...args: unknown[]) {
  await page.evaluate(
    ({ name, args }) => {
      (window as any).__riverTest[name](...args);
    },
    { name, args }
  );
}


async function toastIsOnTop(page: Page): Promise<boolean> {
  return page.getByTestId(TOAST).evaluate((toast) => {
    const r = toast.getBoundingClientRect();
    const hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
    return hit !== null && toast.contains(hit);
  });
}

test.describe("Toast", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/");
    await waitForApp(page);
  });

  test("sits above an open modal", async ({ page }) => {
    await hook(page, "presentTestInvitation");
    await expect(page.getByTestId("receive-invitation-modal")).toBeVisible();

    await hook(page, "showToast", "Joined Your Private Room");
    await expect(page.getByTestId(TOAST)).toBeVisible();
    await page.waitForTimeout(300);
    expect(await toastIsOnTop(page), "the modal must not paint over it").toBe(true);
  });

  test("disappears on its own after about 5 seconds", async ({ page }) => {
    await hook(page, "showToast", "Room joined");
    const toast = page.getByTestId(TOAST);
    await expect(toast).toBeVisible();
    const shownAt = Date.now();

    await page.waitForTimeout(4_000);
    await expect(toast, "still up well inside its 5s").toBeVisible();

    await expect(toast).toHaveCount(0, { timeout: 4_000 });
    const elapsed = Date.now() - shownAt;
    expect(elapsed).toBeGreaterThanOrEqual(4_500);
    expect(elapsed).toBeLessThan(7_000);
    await expect(page.getByTestId("toast-status")).toHaveText("");
  });

  test("the close button dismisses it", async ({ page }) => {
    await hook(page, "showToast", "Room joined");
    await expect(page.getByTestId(TOAST)).toBeVisible();
    await page.getByTestId("toast-dismiss").click();
    await expect(page.getByTestId(TOAST)).toHaveCount(0);
  });

  test("its action runs, then the toast closes", async ({ page }) => {
    await hook(page, "showToast", "Archived conversation with Alice", true);
    const action = page.getByTestId("toast-action");
    await expect(action).toHaveText("Do it");
    await action.click();

    await expect(page.getByTestId(TOAST)).toHaveCount(0);
    expect(await page.evaluate(() => (window as any).__riverTestToastActions)).toBe(1);
  });

  test("a newer toast replaces the older one", async ({ page }) => {
    await hook(page, "showToast", "First");
    await expect(page.getByTestId(TOAST)).toHaveText(/First/);
    await hook(page, "showToast", "Second");

    await expect(page.getByTestId(TOAST)).toHaveCount(1);
    await expect(page.getByTestId(TOAST)).toHaveText(/Second/);
  });

  // Test persistence here rather than repeating the long wait in each join test.
  test("an error toast stays until it is closed", async ({ page }) => {
    await hook(page, "showErrorToast", "Couldn't join the room: test");
    const toast = page.getByTestId(TOAST);
    await expect(toast).toHaveAttribute("data-kind", "error");

    await page.waitForTimeout(6_000);
    await expect(toast, "an error must not time out").toBeVisible();

    await page.getByTestId("toast-dismiss").click();
    await expect(toast).toHaveCount(0);
  });

  test("reduced motion only fades it in", async ({ page }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    await hook(page, "showToast", "Room joined");
    const toast = page.getByTestId(TOAST);
    await expect(toast).toBeVisible();
    expect(await toast.evaluate((el) => getComputedStyle(el).animationName)).toBe(
      "river-toast-fade-in"
    );
  });
});

for (const { label, viewport, isMobile } of [
  { label: "mobile", viewport: { width: 390, height: 844 }, isMobile: true },
  { label: "desktop", viewport: { width: 1280, height: 800 }, isMobile: false },
]) {
  test.describe(`Toast placement (${label})`, () => {
    test.use({ viewport });

    test.beforeEach(async ({ page }) => {
      await page.goto("/");
      await waitForApp(page);
    });

    test("shows centred at the top of the viewport", async ({ page }) => {
      await hook(page, "showToast", "Archived conversation with Alice");

      const toast = page.getByTestId(TOAST);
      await expect(toast).toBeVisible();
      await expect(toast).toHaveText(/Archived conversation with Alice/);
      await expect(toast).toHaveAttribute("data-kind", "info");
      await expect(page.getByTestId("toast-status")).toHaveText(
        "Archived conversation with Alice"
      );

      // Let the 150ms slide-in settle before measuring.
      await page.waitForTimeout(300);
      const box = (await toast.boundingBox())!;
      expect(box.y, "the toast's top sits near the viewport top").toBeGreaterThanOrEqual(0);
      expect(box.y).toBeLessThanOrEqual(24);
      const centre = box.x + box.width / 2;
      expect(Math.abs(centre - viewport.width / 2)).toBeLessThan(2);
    });

    test("sits above the room header and never overlaps the composer", async ({
      page,
    }) => {
      await openRoomWithComposer(page, isMobile);
      await hook(page, "showToast", "Invitation to \"Your Private Room\" sent");
      await expect(page.getByTestId(TOAST)).toBeVisible();
      await page.waitForTimeout(300);

      expect(await toastIsOnTop(page), "the header must not paint over it").toBe(true);

      const toast = (await page.getByTestId(TOAST).boundingBox())!;
      const composer = (await page.getByTestId("message-composer").boundingBox())!;
      expect(
        toast.y + toast.height <= composer.y,
        "the toast must stay clear of the composer"
      ).toBe(true);
    });
  });
}

test.describe("Toast (320px)", () => {
  test.use({ viewport: { width: 320, height: 640 } });

  test("a long message wraps inside the screen", async ({ page }) => {
    await page.goto("/");
    await waitForApp(page);
    await hook(
      page,
      "showErrorToast",
      "Couldn't join the room: room is at capacity (50/50 members); invitation cannot complete until an existing member leaves or is removed"
    );
    const toast = page.getByTestId(TOAST);
    await expect(toast).toBeVisible();
    await page.waitForTimeout(300);

    const box = (await toast.boundingBox())!;
    expect(box.x, "16px gutter on the left").toBeGreaterThanOrEqual(15);
    expect(box.x + box.width, "16px gutter on the right").toBeLessThanOrEqual(305);

    const dismiss = (await page.getByTestId("toast-dismiss").boundingBox())!;
    expect(dismiss.x + dismiss.width).toBeLessThanOrEqual(320);
  });
});
