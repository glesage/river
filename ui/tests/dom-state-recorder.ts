import { Page } from "@playwright/test";

export type PresenceEvent = { t: number; present: boolean };

export type DomStateOptions = {
  /** A plain CSS selector: Playwright pseudo-classes such as `:visible` do not work here. */
  selector: string;
  /** When set, a match counts only if its closest such ancestor is laid out. */
  visibleWithin?: string;
  /** Attributes whose changes can flip the state, beyond insertion and removal. */
  attributes?: string[];
};

export type DomStateRecorder = {
  read(): Promise<PresenceEvent[]>;
  dispose(): Promise<void>;
};

/**
 * Record whether any element matches, as the initial state then each change,
 * timestamped in-page so Playwright round trips do not skew the timings.
 * Dispose it in `finally`.
 */
export async function recordDomState(
  page: Page,
  options: DomStateOptions
): Promise<DomStateRecorder> {
  const handle = await page.evaluateHandle(({ selector, visibleWithin, attributes }) => {
    const isPresent = () =>
      Array.from(document.querySelectorAll(selector)).some((el) => {
        if (!visibleWithin) return true;
        const placement = el.closest<HTMLElement>(visibleWithin);
        return placement !== null && placement.offsetParent !== null;
      });
    let last = isPresent();
    const log = [{ t: performance.now(), present: last }];
    const observer = new MutationObserver(() => {
      const now = isPresent();
      if (now !== last) {
        last = now;
        log.push({ t: performance.now(), present: now });
      }
    });
    observer.observe(document.body, {
      childList: true,
      subtree: true,
      ...(attributes ? { attributes: true, attributeFilter: attributes } : {}),
    });
    return { log, observer };
  }, options);

  let disposed = false;
  return {
    read: () => handle.evaluate(({ log }) => log),
    async dispose() {
      if (disposed) return;
      disposed = true;
      // A closed or navigated page has already dropped the observer.
      await handle.evaluate(({ observer }) => observer.disconnect()).catch(() => {});
      await handle.dispose();
    },
  };
}
