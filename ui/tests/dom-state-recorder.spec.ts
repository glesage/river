import { test, expect, Page } from "@playwright/test";
import { recordDomState, DomStateRecorder } from "./dom-state-recorder";

// Runs against plain markup, not the app. Each mutation gets its own browser
// task: an observer cannot see a state created and undone in the same one.

// Let the observer's callback run before the next step reads or mutates.
async function flush(page: Page) {
  await page.evaluate(() => new Promise((resolve) => setTimeout(resolve, 0)));
}

async function mutate(page: Page, fn: () => void) {
  await page.evaluate(fn);
  await flush(page);
}

async function states(recorder: DomStateRecorder) {
  return (await recorder.read()).map((e) => e.present);
}

test.describe("DOM state recorder", () => {
  test("records the initial state, then each change in order", async ({ page }) => {
    await page.setContent('<div id="root"></div>');
    const recorder = await recordDomState(page, { selector: '[data-testid="target"]' });
    try {
      await mutate(page, () => {
        const el = document.createElement("span");
        el.dataset.testid = "target";
        document.getElementById("root")!.append(el);
      });
      await mutate(page, () => {
        const other = document.createElement("p");
        document.getElementById("root")!.append(other);
        other.setAttribute("title", "unrelated");
      });
      await mutate(page, () => document.querySelector('[data-testid="target"]')!.remove());
      await mutate(page, () => document.getElementById("root")!.append(document.createElement("b")));

      const log = await recorder.read();
      expect(log.map((e) => e.present)).toEqual([false, true, false]);
      expect(log[1].t).toBeGreaterThanOrEqual(log[0].t);
      expect(log[2].t).toBeGreaterThanOrEqual(log[1].t);
    } finally {
      await recorder.dispose();
    }
  });

  test("counts only the active match in a laid-out placement", async ({ page }) => {
    await page.setContent(`
      <div data-testid="pill" id="hidden-pill" style="display: none">
        <span data-testid="dots" data-active="false"></span>
      </div>
      <div data-testid="pill" id="shown-pill">
        <span data-testid="dots" data-active="false"></span>
      </div>
    `);
    const recorder = await recordDomState(page, {
      selector: '[data-testid="dots"][data-active="true"]',
      visibleWithin: '[data-testid="pill"]',
      attributes: ["data-active"],
    });
    const setActive = async (pill: string, active: boolean) => {
      await page.evaluate(
        ({ pill, active }) =>
          document
            .querySelector(`#${pill} [data-testid="dots"]`)!
            .setAttribute("data-active", String(active)),
        { pill, active }
      );
      await flush(page);
    };
    try {
      await setActive("hidden-pill", true);
      await setActive("shown-pill", true);
      await mutate(page, () =>
        document.querySelector('#shown-pill [data-testid="dots"]')!.setAttribute("title", "x")
      );
      await setActive("shown-pill", false);

      expect(await states(recorder)).toEqual([false, true, false]);
    } finally {
      await recorder.dispose();
    }
  });

  test("recorders are independent, and disposal is repeatable", async ({ page }) => {
    await page.setContent('<div id="root"></div>');
    const a = await recordDomState(page, { selector: ".a" });
    const b = await recordDomState(page, { selector: ".b" });
    try {
      await mutate(page, () => {
        const el = document.createElement("i");
        el.className = "a";
        document.getElementById("root")!.append(el);
      });
      expect(await states(a)).toEqual([false, true]);
      expect(await states(b)).toEqual([false]);

      await a.dispose();
      await a.dispose();

      await mutate(page, () => {
        const el = document.createElement("i");
        el.className = "b";
        document.getElementById("root")!.append(el);
      });
      expect(await states(b)).toEqual([false, true]);

      const globals = await page.evaluate(() =>
        Object.keys(window).filter((k) => k.startsWith("__"))
      );
      expect(globals).toEqual([]);
    } finally {
      await a.dispose();
      await b.dispose();
    }
  });
});
