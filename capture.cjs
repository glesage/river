// Usage: node capture.cjs <baseURL> <outDir>
// Same scenes on any build that has the example-data test hooks.
const { chromium } = require("/Users/glesage/Desktop/Repos/river/ui/tests/node_modules/@playwright/test");
const fs = require("fs");
const path = require("path");

const [base, out] = process.argv.slice(2);
fs.mkdirSync(out, { recursive: true });

const FIXED = new Date("2026-11-17T10:00:00Z");

async function open(browser, width, height) {
  const ctx = await browser.newContext({
    viewport: { width, height },
    deviceScaleFactor: 2,
    timezoneId: "UTC",
    locale: "en-US",
    colorScheme: "light",
    hasTouch: width < 768,
  });
  const page = await ctx.newPage();
  await page.clock.setFixedTime(FIXED);
  await page.goto(base);
  await page.waitForSelector(".app-root", { timeout: 30000 });
  await page.waitForFunction(() => {
    const links = Array.from(document.querySelectorAll('link[rel="stylesheet"]'));
    return links.length > 0 && links.every((l) => l.sheet !== null);
  });
  return { ctx, page };
}

const hook = (page, name, ...args) => page.evaluate(([n, a]) => window.__riverTest[n](...a), [name, args]);
const frames = (page) => page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
const settle = async (page) => { await frames(page); await page.waitForTimeout(400); };

async function selectRoom(page, name) {
  const listed = page.getByTestId("room-list").getByRole("button", { name });
  if (!(await listed.isVisible())) await page.getByTestId("hamburger-rooms-button").filter({ visible: true }).click();
  await page.getByTestId("room-list").getByRole("button", { name }).click();
  await page.getByTestId("message-composer").waitFor({ state: "visible" });
  // The avatar palette is random per device; fix it so both builds match.
  await hook(page, "setAvatarSalt", 7);
  await settle(page);
}

function row(page, text) {
  return page.getByTestId("conversation-history").getByTestId("message-row").filter({ hasText: text }).last();
}
const msgRow = (r) => r.locator('xpath=ancestor-or-self::*[starts-with(@id,"msg-")][1]');
const chip = (r, emoji) => msgRow(r).locator('[data-testid="reaction-chip"]').filter({ hasText: emoji });

/// The message's whole row, padded, plus any reactor list that hangs off it.
async function shoot(page, target, name, extra = null) {
  const box = await msgRow(target).boundingBox();
  let x0 = box.x, y0 = box.y, x1 = box.x + box.width, y1 = box.y + box.height;
  if (extra && (await extra.count())) {
    const e = await extra.first().boundingBox();
    x0 = Math.min(x0, e.x); y0 = Math.min(y0, e.y); x1 = Math.max(x1, e.x + e.width); y1 = Math.max(y1, e.y + e.height);
  }
  const vp = page.viewportSize();
  const pad = 12;
  const clip = { x: Math.max(0, x0 - pad), y: Math.max(0, y0 - pad) };
  clip.width = Math.min(vp.width, x1 + pad) - clip.x;
  clip.height = Math.min(vp.height, y1 + pad) - clip.y;
  await page.screenshot({ path: path.join(out, `${name}.png`), clip });
  await page.screenshot({ path: path.join(out, `${name}-full.png`) });
}

/// A received message with three member reactions, in this order: 🦀, 👍, 🎉.
/// Alphabetical order (by code point) is 🎉 👍 🦀.
async function targetWithReactions(page) {
  await hook(page, "appendMessage", "Lunch at noon? I'll book the usual place.");
  const r = row(page, "Lunch at noon?");
  await r.waitFor({ state: "visible" });
  // One minute apart: reactions signed at one instant tie, and ties sort by emoji.
  for (const [i, e] of ["🦀", "👍", "🎉"].entries()) {
    await page.clock.setFixedTime(new Date(FIXED.getTime() + (i + 1) * 60_000));
    await hook(page, "memberReactsToNewest", e);
    await settle(page);
  }
  await chip(r, "🎉").waitFor({ state: "visible" });
  await settle(page);
  return r;
}

(async () => {
  const browser = await chromium.launch();
  const order = {};
  try {
    // 1. Standing order on desktop.
    {
      const { ctx, page } = await open(browser, 1280, 800);
      await selectRoom(page, "Your Private Room");
      const r = await targetWithReactions(page);
      order.desktop = await msgRow(r).locator('[data-testid="reaction-chip"]').evaluateAll((els) => els.map((e) => e.textContent.trim()));
      await page.mouse.move(5, 5);
      await settle(page);
      await shoot(page, r, "01-order-desktop");

      // 2. The reader reacts with 🦀: counts and selected tint.
      await chip(r, "🦀").click();
      await settle(page);
      await page.mouse.move(5, 5);
      await settle(page);
      await shoot(page, r, "02-own-reaction-desktop");

      // 3. Keyboard focus on a chip.
      await page.keyboard.press("Shift");
      await chip(r, "👍").focus();
      await settle(page);
      await shoot(page, r, "03-keyboard-focus-desktop");

      // 4. Shift+Enter opens the reactor list (unchanged from main).
      await chip(r, "🦀").focus();
      await page.keyboard.press("Shift+Enter");
      await page.getByTestId("reaction-owners").waitFor({ state: "visible" });
      await settle(page);
      await shoot(page, r, "04-shift-enter-list-desktop", page.getByTestId("reaction-owners"));
      await page.keyboard.press("Escape");
      await settle(page);

      // 5. Hover title (unchanged from main), read from the DOM.
      order.title = await chip(r, "🦀").getAttribute("title");
      order.semantics = await chip(r, "🦀").evaluate((el) => ({
        tag: el.tagName.toLowerCase(),
        role: el.getAttribute("role"),
        tabindex: el.getAttribute("tabindex"),
        pressed: el.getAttribute("aria-pressed"),
      }));
      await ctx.close();
    }
    // 6. Phone width.
    {
      const { ctx, page } = await open(browser, 390, 844);
      await selectRoom(page, "Your Private Room");
      const r = await targetWithReactions(page);
      order.mobile = await msgRow(r).locator('[data-testid="reaction-chip"]').evaluateAll((els) => els.map((e) => e.textContent.trim()));
      await chip(r, "🦀").tap();
      await settle(page);
      await shoot(page, r, "05-order-mobile");
      await ctx.close();
    }
  } finally {
    await browser.close();
  }
  fs.writeFileSync(path.join(out, "facts.json"), JSON.stringify(order, null, 2));
  console.log(JSON.stringify(order));
})().catch((e) => { console.error(e); process.exit(1); });
