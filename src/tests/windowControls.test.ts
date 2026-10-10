/// <reference types="node" />
import { readFileSync } from "node:fs";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createWindowControls, type TitlebarWindow } from "../lib/windowControls";

function fixture() {
  let resize: (() => void) | undefined;
  let focus: ((event: { payload: boolean }) => void) | undefined;
  const stopResize = vi.fn();
  const stopFocus = vi.fn();
  const window = {
    minimize: vi.fn(async () => {}),
    toggleMaximize: vi.fn(async () => {}),
    close: vi.fn(async () => {}),
    startDragging: vi.fn(async () => {}),
    isMaximized: vi.fn(async () => false),
    isFocused: vi.fn(async () => true),
    onResized: vi.fn(async (handler: () => void): Promise<() => void> => {
      resize = handler;
      return stopResize;
    }),
    onFocusChanged: vi.fn(
      async (handler: (event: { payload: boolean }) => void): Promise<() => void> => {
        focus = handler;
        return stopFocus;
      },
    ),
  };
  const controls = createWindowControls(window as unknown as TitlebarWindow);
  return {
    window,
    controls,
    stopResize,
    stopFocus,
    resize: () => resize?.(),
    focus: (value: boolean) => focus?.({ payload: value }),
  };
}

afterEach(() => vi.restoreAllMocks());

describe("custom titlebar window controls", () => {
  it("reads the initial maximized and focus state", async () => {
    const { window, controls } = fixture();
    window.isMaximized.mockResolvedValue(true);
    window.isFocused.mockResolvedValue(false);
    await controls.initialize();
    expect(controls.maximized.value).toBe(true);
    expect(controls.focused.value).toBe(false);
    expect(controls.failed.value).toBe(false);
    controls.dispose();
  });

  it("minimizes and closes through the native window API", async () => {
    const { window, controls } = fixture();
    await controls.minimize();
    await controls.close();
    expect(window.minimize).toHaveBeenCalledOnce();
    expect(window.close).toHaveBeenCalledOnce();
    expect(window.startDragging).not.toHaveBeenCalled();
  });

  it("updates the maximize/restore state after toggling", async () => {
    const { window, controls } = fixture();
    window.isMaximized.mockResolvedValueOnce(true).mockResolvedValueOnce(false);
    await controls.toggleMaximize();
    expect(controls.maximized.value).toBe(true);
    await controls.toggleMaximize();
    expect(controls.maximized.value).toBe(false);
    expect(window.toggleMaximize).toHaveBeenCalledTimes(2);
  });

  it("drags on a primary click and toggles once on a double-click", async () => {
    const { window, controls } = fixture();
    await controls.drag({ buttons: 1, detail: 1 });
    expect(window.startDragging).toHaveBeenCalledOnce();
    await controls.drag({ buttons: 1, detail: 2 });
    expect(window.toggleMaximize).toHaveBeenCalledOnce();
    expect(window.startDragging).toHaveBeenCalledOnce();
  });

  it("ignores right, middle and combined-button clicks", async () => {
    const { window, controls } = fixture();
    for (const buttons of [0, 2, 4, 3]) await controls.drag({ buttons, detail: 2 });
    expect(window.startDragging).not.toHaveBeenCalled();
    expect(window.toggleMaximize).not.toHaveBeenCalled();
  });

  it("synchronizes externally changed maximize and focus states", async () => {
    const { window, controls, resize, focus } = fixture();
    await controls.initialize();
    window.isMaximized.mockResolvedValue(true);
    resize();
    await vi.waitFor(() => expect(controls.maximized.value).toBe(true));
    focus(false);
    expect(controls.focused.value).toBe(false);
    focus(true);
    expect(controls.focused.value).toBe(true);
    controls.dispose();
  });

  it("does not let older resize queries overwrite newer state", async () => {
    const { window, controls, resize } = fixture();
    await controls.initialize();
    let resolveOld!: (value: boolean) => void;
    window.isMaximized.mockImplementationOnce(
      () =>
        new Promise<boolean>((resolve) => {
          resolveOld = resolve;
        }),
    );
    resize();
    window.isMaximized.mockResolvedValue(true);
    resize();
    await vi.waitFor(() => expect(controls.maximized.value).toBe(true));
    resolveOld(false);
    await Promise.resolve();
    expect(controls.maximized.value).toBe(true);
    controls.dispose();
  });

  it("removes listeners exactly once and ignores actions after disposal", async () => {
    const { window, controls, stopResize, stopFocus, focus } = fixture();
    await controls.initialize();
    controls.dispose();
    controls.dispose();
    expect(stopResize).toHaveBeenCalledOnce();
    expect(stopFocus).toHaveBeenCalledOnce();
    focus(false);
    expect(controls.focused.value).toBe(true);
    await controls.minimize();
    expect(window.minimize).not.toHaveBeenCalled();
  });

  it("cleans up listeners registered after the component is unmounted", async () => {
    const { window, controls, stopResize, stopFocus } = fixture();
    let resolveRegistration!: (stop: () => void) => void;
    window.onResized.mockImplementationOnce(
      () =>
        new Promise<() => void>((resolve) => {
          resolveRegistration = resolve;
        }),
    );
    const initializing = controls.initialize();
    controls.dispose();
    resolveRegistration(stopResize);
    await initializing;
    expect(stopResize).toHaveBeenCalledOnce();
    expect(stopFocus).toHaveBeenCalledOnce();
    expect(window.isMaximized).not.toHaveBeenCalled();
  });

  it("handles native API failures and recovers on the next action", async () => {
    const { window, controls } = fixture();
    vi.spyOn(console, "error").mockImplementation(() => {});
    window.minimize.mockRejectedValueOnce(new Error("permission denied"));
    await expect(controls.minimize()).resolves.toBeUndefined();
    expect(controls.failed.value).toBe(true);
    await controls.minimize();
    expect(controls.failed.value).toBe(false);
  });

  it("handles listener registration failures without rejecting initialization", async () => {
    const { window, controls, stopFocus } = fixture();
    vi.spyOn(console, "error").mockImplementation(() => {});
    window.onResized.mockRejectedValueOnce(new Error("unavailable"));
    await expect(controls.initialize()).resolves.toBeUndefined();
    expect(controls.failed.value).toBe(true);
    controls.dispose();
    expect(stopFocus).toHaveBeenCalledOnce();
  });
});

const read = (path: string) =>
  readFileSync(new URL(path, new URL("../../", import.meta.url)), "utf8");
describe("custom titlebar desktop integration", () => {
  it("keeps the titlebar free of application branding", () => {
    const titlebar = read("src/components/Titlebar.vue");
    expect(titlebar).not.toContain("<img");
    expect(titlebar).not.toContain("settings.title");
    expect(titlebar).not.toContain("titlebar-title");
  });

  it("disables native decorations and grants only the required extra controls", () => {
    const config = JSON.parse(read("src-tauri/tauri.conf.json"));
    const capability = JSON.parse(read("src-tauri/capabilities/default.json"));
    expect(config.app.windows[0].decorations).toBe(false);
    expect(capability.windows).toContain("main");
    for (const permission of ["minimize", "toggle-maximize", "close", "start-dragging"])
      expect(capability.permissions).toContain("core:window:allow-" + permission);
  });

  it("shows window controls only in the desktop and keeps buttons outside the drag region", () => {
    expect(read("src/App.vue")).toContain('<Titlebar v-if="desktop"');
    const titlebar = read("src/components/Titlebar.vue");
    expect(titlebar).not.toContain("data-tauri-drag-region");
    const region = titlebar.split('<div class="titlebar-drag-region"')[1]!.split("</div>")[0]!;
    expect(region).toContain('@mousedown="controls.drag"');
    expect(region).not.toContain("<button");
    expect(titlebar).toContain("onUnmounted(controls.dispose)");
  });
});
