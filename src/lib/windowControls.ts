import { ref } from "vue";
import type { Window } from "@tauri-apps/api/window";
import type { UnlistenFn } from "@tauri-apps/api/event";

export type TitlebarWindow = Pick<
  Window,
  | "minimize"
  | "toggleMaximize"
  | "close"
  | "startDragging"
  | "isMaximized"
  | "isFocused"
  | "onResized"
  | "onFocusChanged"
>;

export function createWindowControls(window: TitlebarWindow) {
  const maximized = ref(false);
  const focused = ref(true);
  const failed = ref(false);
  const listeners: UnlistenFn[] = [];
  let disposed = false;
  let stateRequest = 0;

  async function run(action: () => Promise<unknown>) {
    if (disposed) return;
    failed.value = false;
    try {
      await action();
    } catch (error) {
      if (!disposed) failed.value = true;
      console.error("Window titlebar action failed", error);
    }
  }
  async function syncMaximized() {
    if (disposed) return;
    const request = ++stateRequest;
    const value = await window.isMaximized();
    if (!disposed && request === stateRequest) maximized.value = value;
  }
  async function bind(registration: Promise<UnlistenFn>) {
    const unlisten = await registration;
    if (disposed) unlisten();
    else listeners.push(unlisten);
  }
  async function initialize() {
    await run(async () => {
      await Promise.all([
        bind(window.onResized(() => void run(syncMaximized))),
        bind(
          window.onFocusChanged(({ payload }) => {
            if (!disposed) focused.value = payload;
          }),
        ),
      ]);
      await syncMaximized();
      if (disposed) return;
      const value = await window.isFocused();
      if (!disposed) focused.value = value;
    });
  }
  function dispose() {
    disposed = true;
    for (const unlisten of listeners.splice(0)) unlisten();
  }
  const minimize = () => run(() => window.minimize());
  const close = () => run(() => window.close());
  const toggleMaximize = () =>
    run(async () => {
      await window.toggleMaximize();
      await syncMaximized();
    });
  function drag(event: Pick<MouseEvent, "buttons" | "detail">) {
    if (event.buttons !== 1) return;
    // Handle double-click here, rather than combining a native drag region with a second handler.
    return event.detail === 2 ? toggleMaximize() : run(() => window.startDragging());
  }
  return { maximized, focused, failed, initialize, dispose, minimize, toggleMaximize, close, drag };
}
