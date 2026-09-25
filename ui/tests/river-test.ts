import { Page } from "@playwright/test";

// The subset of `window.__riverTest` these specs use. `install_test_hooks` in
// ui/src/example_data.rs defines the hooks and is the source of truth.

export type SyncStatus = "connecting" | "connected" | "disconnected" | "error";
export type RoomsLoadState = "loading" | "migrating" | "failed" | "loaded";
// Anything else, or no argument, is treated as "sending".
export type UserActionKind = "sending" | "saving" | "creating-room";

export type RiverTestHooks = {
  setSyncStatus(status: SyncStatus): void;
  setRoomsLoadState(state: RoomsLoadState): void;
  beginUserAction(kind?: UserActionKind): void;
  endUserAction(): void;
  awaitRoomUpdate(): void;
  sendRoomUpdate(): void;
  answerRoomUpdate(): void;
  failRoomUpdate(): void;
  beginBackgroundRequest(): void;
  endBackgroundRequest(): void;
  showToast(message: string, withAction?: boolean): void;
  showErrorToast(message: string): void;
  presentTestInvitation(): void;
  finishTestJoin(): void;
  failTestJoin(): void;
  awaitRoomSync(roomName: string): void;
  holdInviteSend(): void;
};

export type RiverTestWindow = Window & { __riverTest: RiverTestHooks };

/**
 * Call one test hook in its own round trip. Code that times a hook, or runs
 * several in one task, must call `window.__riverTest` inside its own
 * `page.evaluate` instead.
 */
export async function callRiverTest<K extends keyof RiverTestHooks>(
  page: Page,
  name: K,
  ...args: Parameters<RiverTestHooks[K]>
): Promise<void> {
  await page.evaluate(
    ({ name, args }) => {
      const hooks = (window as { __riverTest?: Record<string, unknown> }).__riverTest;
      const hook = hooks?.[name];
      if (typeof hook !== "function") {
        throw new Error(`window.__riverTest.${name} is not available in this build`);
      }
      hook.apply(hooks, args);
    },
    { name: name as string, args: args as unknown[] }
  );
}
