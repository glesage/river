import { test, expect, Page } from "@playwright/test";
import { waitForApp, openRoomWithComposer } from "./example-room";

// The minimum tap size from #605 was reverted in #606 because it made every
// reaction row taller. Touch visibility is covered in message-reply-button.spec.ts.

const PLUS = '[data-testid="add-reaction-button"]';
const REPLY = '[data-testid="message-reply-button"]';
const CHIP = '[data-testid="reaction-chip"]';

function withoutReactions(page: Page) {
  return page.locator(`[id^="msg-"]:not(:has(${CHIP}))`).first();
}

test("on a touch pointer the smiley and action buttons have no minimum tap size", async ({
  page,
}) => {
  await page.goto("/");
  await waitForApp(page);
  const coarse = await page.evaluate(
    () => window.matchMedia("(hover: none), (any-pointer: coarse)").matches
  );
  test.skip(!coarse, "touch only: the reverted minimum size was a touch rule");

  const narrow = (page.viewportSize()?.width ?? 1280) < 768;
  await openRoomWithComposer(page, narrow);

  const row = withoutReactions(page);
  await row.scrollIntoViewIfNeeded();

  for (const sel of [PLUS, REPLY]) {
    const box = await row.locator(sel).boundingBox();
    expect(box, `${sel} is rendered`).not.toBeNull();
    expect(
      Math.min(box!.width, box!.height),
      `${sel} has a minimum tap size again (reverted in #606)`
    ).toBeLessThan(44);
  }
});
