import { expect, Page } from "@playwright/test";

// Team Chat Room includes self and other members, so the share-invite entry
// point must exist. Missing fixture members should fail, not skip.

// member_display_parts marks only self with ⭐, regardless of ownership.
export function isSelfRowText(text: string): boolean {
  return text.includes("⭐");
}


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


export async function openInviteViaDmPicker(page: Page): Promise<void> {
  await openMemberInfoForFirstNonSelf(page);
  await openShareInvitePicker(page);
}
