import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSSRApp } from "vue";
import { renderToString } from "vue/server-renderer";
import App from "../App.vue";
import { i18n } from "../lib/i18n";
import { preferences } from "../lib/preferences";

const state = vi.hoisted(() => ({
  authenticated: false,
  running: false,
  connecting: false,
  enabledService: true,
}));
vi.mock("/app-icon.svg", () => ({ default: "/app-icon.svg" }));
vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => false, invoke: vi.fn() }));
vi.mock("../lib/workspace", async (original) => {
  const { useWorkspace } = await original<typeof import("../lib/workspace")>();
  return {
    useWorkspace: () => {
      const w = useWorkspace();
      const target = w.connections[0]!;
      target.saved = true;
      target.profile.fnId = "my-nas";
      target.profile.username = "admin";
      target.profile.services = [
        {
          id: "service",
          name: "API",
          nasPort: 8084,
          localPort: 8084,
          upstream: "https://api.my-nas.fnos.net/",
          enabled: state.enabledService,
        },
      ];
      target.connection.connected = state.authenticated;
      target.proxy.running = state.running;
      target.connecting = state.connecting;
      w.section.value = "connections";
      return w;
    },
  };
});
const originalLocale = preferences.locale.value;
beforeEach(() => {
  preferences.locale.value = "zh-CN";
  Object.assign(state, {
    authenticated: false,
    running: false,
    connecting: false,
    enabledService: true,
  });
});
afterEach(() => {
  preferences.locale.value = originalLocale;
});
async function renderList() {
  const html = await renderToString(createSSRApp(App).use(i18n));
  const table = html.match(/<table>([\s\S]*?)<\/table>/)?.[1];
  expect(table).toBeDefined();
  return table!;
}
function button(table: string, label: string) {
  return table.match(new RegExp(`<button[^>]*aria-label="${label}"[^>]*>`))?.[0];
}

describe("rendered connection list proxy controls", () => {
  it.each([false, true])(
    "offers proxy startup when authenticated=%s but stopped",
    async (authenticated) => {
      state.authenticated = authenticated;
      const table = await renderList();
      expect(button(table, "开启 my-nas 代理")).toBeDefined();
      expect(table).toContain("开启代理");
      expect(table).not.toContain("认证状态");
      expect(table).not.toContain("已认证");
      expect(table).not.toContain("未认证");
      expect(table).not.toContain("断开");
      expect(table).toContain("已停止");
      expect(table).not.toContain('aria-label="连接 my-nas"');
      expect(button(table, "开启 my-nas 代理")).not.toContain("disabled");
    },
  );

  it.each([false, true])(
    "only offers proxy stop when running, even when authenticated=%s",
    async (authenticated) => {
      state.authenticated = authenticated;
      state.running = true;
      const table = await renderList();
      expect(button(table, "停止 my-nas 代理")).toBeDefined();
      expect(table).toContain("停止代理");
      expect(table).toContain("运行中");
      expect(table).not.toContain("断开");
      expect(table).not.toContain("认证状态");
      expect(table).not.toContain("已认证");
      expect(table).not.toContain("未认证");
      expect(table).not.toContain("开启代理");
      expect(table).not.toContain('aria-label="连接 my-nas"');
    },
  );

  it("disables startup without enabled services but still allows stopping", async () => {
    state.enabledService = false;
    expect(button(await renderList(), "开启 my-nas 代理")).toContain("disabled");
    state.running = true;
    expect(button(await renderList(), "停止 my-nas 代理")).not.toContain("disabled");
  });

  it("shows startup progress and disables repeated startup during authentication", async () => {
    state.connecting = true;
    const table = await renderList();
    const start = button(table, "开启 my-nas 代理");
    expect(start).toContain("disabled");
    expect(start).toContain('aria-busy="true"');
    expect(table).toContain("开启中…");
    expect(table).not.toContain('aria-label="连接 my-nas"');
  });
});
