import { expect, Locator } from "@playwright/test";

/**
 * Under reduced motion a dot must still read as activity without travelling.
 * Sampled in real time over one animation cycle (plus a little): its opacity
 * takes more than one value, its position exactly one.
 */
export async function expectShimmerInPlace(dot: Locator, cycleMs: number) {
  const opacities = new Set<string>();
  const positions = new Set<string>();
  const start = Date.now();
  while (Date.now() - start < cycleMs + 200) {
    const sample = await dot.evaluate((el) => {
      const r = el.getBoundingClientRect();
      return { x: r.x, y: r.y, opacity: getComputedStyle(el).opacity };
    });
    opacities.add(sample.opacity);
    positions.add(`${sample.x.toFixed(2)},${sample.y.toFixed(2)}`);
    await dot.page().waitForTimeout(150);
  }
  expect(opacities.size, "opacity still moves").toBeGreaterThan(1);
  expect(positions.size, "the dot never travels").toBe(1);
}
