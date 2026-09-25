import { expect, Page } from "@playwright/test";

// Opening the invite-via-DM picker from "Team Chat Room", shared by
// loading-dots.spec.ts and invite-via-dm-picker.spec.ts.
//
// Example data always gives that room non-self members, and the local user
// is a member there, so the "Share an invite via DM" entry point always
// exists. A missing row or entry point is a regression: fail, don't skip.

/**
 * Whether a member row's display text marks it as the local user.
 * `member_display_parts` (members.rs) gives every self row, and only the self
 * row, a ⭐ badge, whether the user owns the room or is a plain member.
 */
export function isSelfRowText(text: string): boolean {
  return text.includes("⭐");
}

/** Select "Team Chat Room" and open the first non-self member's info modal. */
export async function openMemberInfoForFirstNonSelf(page: Page): Promise<void> {
  await page.getByText("Team Chat Room").first().click();

  const memberButtons = page.locator('button[title^="Member ID"]');
  await expect(memberButtons.first(), "Team Chat Room's member list rendered").toBeVisible({
    timeout: 5_000,
  });

  const count = await memberButtons.count();
  let found = false;
  for (let i = 0; i < count && !found; i++) {
    const text = (await memberButtons.nth(i).textContent()) || "";
    if (!isSelfRowText(text)) {
      await memberButtons.nth(i).click();
      found = true;
    }
  }
  expect(found, "Team Chat Room lists a member other than the local user").toBe(true);
}

/** From an open member-info modal, open the picker via "Share an invite via DM…". */
export async function openShareInvitePicker(page: Page): Promise<void> {
  const shareInvite = page.getByRole("button", { name: /share an invite/i }).first();
  await expect(shareInvite, "the 'Share an invite via DM' entry point").toBeVisible({
    timeout: 5_000,
  });
  await shareInvite.click();
  await expect(page.getByRole("heading", { name: /invite .+ to another room/i })).toBeVisible({
    timeout: 5_000,
  });
}

/** Open the picker for the first non-self member of "Team Chat Room". */
export async function openInviteViaDmPicker(page: Page): Promise<void> {
  await openMemberInfoForFirstNonSelf(page);
  await openShareInvitePicker(page);
}
