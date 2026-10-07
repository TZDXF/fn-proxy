import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { nextTick } from "vue";
import { useWorkspace } from "../lib/workspace";
import type { ConnectionInfo, Profile, DiscoveredService } from "../lib/types";

const mocks = vi.hoisted(() => ({
  desktop: false,
  invoke: vi.fn(),
  mounted: [] as (() => Promise<void>)[],
  unmounted: [] as (() => void)[],
}));
vi.mock("vue", async (original) => ({
  ...(await original<typeof import("vue")>()),
  onMounted: (callback: () => Promise<void>) => mocks.mounted.push(callback),
  onUnmounted: (callback: () => void) => mocks.unmounted.push(callback),
}));
vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => mocks.desktop, invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => vi.fn()) }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
const profile = (id: string, fnId = id): Profile => ({
  id,
  fnId,
  username: "admin",
  remember: true,
  autoConnect: true,
  services: [],
});
const service = (nasPort = 8084): DiscoveredService => ({
  id: "service",
  name: "API",
  nasPort,
  upstream: "https://api.my-nas.fnos.net/",
  fnDomain: "api",
  source: "fixture",
});
const info = (fnId: string): ConnectionInfo => ({
  connected: true,
  fnId,
  username: "admin",
  relay: "remote",
  authMode: "fixture",
  message: "connected",
});
beforeEach(() => {
  mocks.desktop = false;
  mocks.invoke.mockReset();
  mocks.mounted.length = 0;
  mocks.unmounted.length = 0;
});
afterEach(() => {
  mocks.unmounted.forEach((callback) => callback());
});

describe("multiple connection workspaces", () => {
  it("switches profiles, inventories, probe results and running proxies without disconnecting", () => {
    const w = useWorkspace();
    const first = w.selectedConnectionId.value;
    w.profile.value.fnId = "my-nas";
    w.password.value = "draft";
    w.connection.value = info("my-nas");
    w.proxy.value = { running: true, listeners: [], requests: 7 };
    w.probes.value.api = { status: 200, reachable: true, message: "ok" };
    w.addConnection();
    expect(w.selectedConnectionId.value).not.toBe(first);
    expect(w.profile.value.fnId).toBe("");
    expect(w.proxy.value.running).toBe(false);
    expect(w.password.value).toBe("");
    expect(w.probes.value).toEqual({});
    w.selectedConnectionId.value = first;
    expect(w.connection.value.connected).toBe(true);
    expect(w.proxy.value.requests).toBe(7);
    expect(w.password.value).toBe("draft");
    expect(w.probes.value.api!.reachable).toBe(true);
    expect(mocks.invoke).not.toHaveBeenCalled();
  });
  it("uses service ports and avoids mappings from other connections", () => {
    const w = useWorkspace();
    w.profile.value.fnId = "my-nas";
    w.addDiscovered(service());
    expect(w.profile.value.services[0]!.localPort).toBe(8084);
    w.addConnection();
    w.profile.value.fnId = "my-nas";
    w.addDiscovered(service());
    expect(w.profile.value.services[0]!.localPort).toBe(8085);
  });
  it("updates default manual ports but preserves user overrides and existing routes", async () => {
    const w = useWorkspace();
    w.profile.value.fnId = "my-nas";
    w.showEditor();
    expect(w.editor.localPort).toBe(8084);
    w.editor.nasPort = 80;
    await nextTick();
    expect(w.editor.localPort).toBe(80);
    w.editor.localPort = 9000;
    w.editor.nasPort = 443;
    await nextTick();
    expect(w.editor.localPort).toBe(9000);
    w.showEditor({
      id: "old",
      name: "API",
      nasPort: 8084,
      localPort: 18084,
      upstream: service().upstream,
      enabled: true,
    });
    w.editor.nasPort = 8085;
    await nextTick();
    expect(w.editor.localPort).toBe(18084);
  });
  it("rejects manually reusing another connection's local port", () => {
    const w = useWorkspace();
    w.profile.value.fnId = "my-nas";
    w.addDiscovered(service());
    w.addConnection();
    w.profile.value.fnId = "my-nas";
    w.showEditor();
    Object.assign(w.editor, { name: "API", localPort: 8084, upstream: service().upstream });
    w.commitEditor();
    expect(w.profile.value.services).toHaveLength(0);
    expect(w.notice.value?.message).toContain("端口已被");
  });
  it("loads all saved profiles and routes IPC to the selected connection", async () => {
    mocks.desktop = true;
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "get_bootstrap")
        return {
          profiles: [
            { profile: profile("first"), hasSavedPassword: true },
            { profile: profile("second"), hasSavedPassword: false },
          ],
        };
      if (command === "get_snapshot")
        return {
          connections: [
            {
              id: "first",
              connection: info("first"),
              proxy: { running: true, listeners: [], requests: 5 },
            },
            {
              id: "second",
              connection: info("second"),
              proxy: { running: false, listeners: [], requests: 0 },
            },
          ],
        };
      if (command === "get_logs") return [];
      if (command === "connect_nas") return info("second");
      if (command === "start_proxy") return { running: true, listeners: [], requests: 0 };
      return undefined;
    });
    const w = useWorkspace();
    await mocks.mounted[0]!();
    expect(w.connections).toHaveLength(2);
    expect(w.hasSavedPassword.value).toBe(true);
    w.selectedConnectionId.value = "second";
    expect(w.hasSavedPassword.value).toBe(false);
    w.password.value = "test-password";
    await w.connect();
    expect(mocks.invoke).toHaveBeenCalledWith(
      "connect_nas",
      expect.objectContaining({ connectionId: "second" }),
    );
    expect(w.password.value).toBe("");
    await w.save();
    expect(mocks.invoke).toHaveBeenCalledWith(
      "save_login",
      expect.objectContaining({
        connectionId: "second",
        profile: expect.objectContaining({ id: "second" }),
      }),
    );
    await w.toggleProxy();
    expect(mocks.invoke).toHaveBeenCalledWith(
      "start_proxy",
      expect.objectContaining({ connectionId: "second" }),
    );
    await w.refreshToken();
    expect(mocks.invoke).toHaveBeenCalledWith("refresh_session", { connectionId: "second" });
    await w.disconnect();
    expect(mocks.invoke).toHaveBeenCalledWith("disconnect_nas", { connectionId: "second" });
    w.selectedConnectionId.value = "first";
    expect(w.proxy.value.running).toBe(true);
  });
  it("deletes only the selected connection and leaves the other one running", async () => {
    mocks.desktop = true;
    mocks.invoke.mockResolvedValue(undefined);
    const w = useWorkspace();
    const first = w.selectedConnectionId.value;
    w.proxy.value = { running: true, listeners: [], requests: 2 };
    w.addConnection();
    const second = w.selectedConnectionId.value;
    await w.removeConnection();
    expect(mocks.invoke).toHaveBeenCalledWith("remove_connection", { connectionId: second });
    expect(w.connections).toHaveLength(1);
    expect(w.selectedConnectionId.value).toBe(first);
    expect(w.proxy.value.running).toBe(true);
  });
});

describe("saved connections and incremental service mappings", () => {
  it("starts on the home page and keeps unsaved drafts out of saved lists", () => {
    const w = useWorkspace();
    expect(w.section.value).toBe("overview");
    expect(w.savedConnections.value).toHaveLength(0);
    w.addConnection();
    expect(w.savedConnections.value).toHaveLength(0);
  });
  it("adds a running mapping without stopping or restarting existing proxies", async () => {
    mocks.desktop = true;
    const w = useWorkspace();
    w.profile.value.fnId = "my-nas";
    w.connection.value = info("my-nas");
    const old = {
      id: "old",
      name: "Existing",
      nasPort: 8080,
      localPort: 18080,
      upstream: "https://old.my-nas.fnos.net/",
      enabled: true,
    };
    w.profile.value.services = [old];
    const oldListener = {
      name: old.name,
      nasPort: old.nasPort,
      localUrl: "http://127.0.0.1:18080/",
      upstream: old.upstream,
    };
    w.proxy.value = { running: true, listeners: [oldListener], requests: 42 };
    mocks.invoke.mockImplementation(async (command: string, args: { services: (typeof old)[] }) => {
      if (command === "update_services")
        return {
          running: true,
          requests: 42,
          listeners: args.services.map((s) => ({
            name: s.name,
            nasPort: s.nasPort,
            localUrl: `http://127.0.0.1:${s.localPort}/`,
            upstream: s.upstream,
          })),
        };
      throw new Error(`Unexpected command: ${command}`);
    });
    w.showEditor();
    Object.assign(w.editor, { name: "Added", upstream: service().upstream });
    await w.commitEditor();
    expect(w.profile.value.services).toHaveLength(2);
    expect(w.profile.value.services[0]).toEqual(old);
    expect(w.proxy.value.running).toBe(true);
    expect(w.proxy.value.requests).toBe(42);
    expect(w.proxy.value.listeners).toContainEqual(oldListener);
    expect(w.editing.value).toBe(false);
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    expect(mocks.invoke).toHaveBeenCalledWith(
      "update_services",
      expect.objectContaining({
        connectionId: w.selectedConnectionId.value,
        services: expect.arrayContaining([old]),
      }),
    );
  });
  it("keeps the dialog open and original configuration on listener or storage failure", async () => {
    mocks.desktop = true;
    mocks.invoke.mockRejectedValue(new Error("无法监听本地端口 8084"));
    const w = useWorkspace();
    w.profile.value.fnId = "my-nas";
    w.proxy.value = { running: true, listeners: [], requests: 9 };
    w.showEditor();
    Object.assign(w.editor, { name: "Added", upstream: service().upstream });
    await w.commitEditor();
    expect(w.profile.value.services).toHaveLength(0);
    expect(w.proxy.value).toEqual({ running: true, listeners: [], requests: 9 });
    expect(w.editing.value).toBe(true);
    expect(w.notice.value?.error).toBe(true);
  });
  it("persists mappings for a saved, disconnected connection", async () => {
    mocks.desktop = true;
    const w = useWorkspace();
    w.profile.value.fnId = "my-nas";
    mocks.invoke.mockResolvedValue({ running: false, listeners: [], requests: 0 });
    w.showEditor();
    Object.assign(w.editor, { name: "Added", upstream: service().upstream });
    await w.commitEditor();
    expect(w.profile.value.services).toHaveLength(1);
    expect(mocks.invoke).toHaveBeenCalledWith("update_services", expect.anything());
    const route = w.profile.value.services[0]!;
    await w.setServiceEnabled(route, false);
    expect(w.profile.value.services[0]?.enabled).toBe(false);
    await w.remove(w.profile.value.services[0]!);
    expect(w.profile.value.services).toHaveLength(0);
  });
  it("allows deleting the final connection and leaves a clean unsaved workspace", async () => {
    const w = useWorkspace();
    const id = w.profile.value.id;
    w.profile.value.fnId = "my-nas";
    await w.removeConnection();
    expect(w.profile.value.id).not.toBe(id);
    expect(w.profile.value.fnId).toBe("");
    expect(w.savedConnections.value).toHaveLength(0);
  });
  it("does not allow editing, removing or disabling an existing running mapping", async () => {
    mocks.desktop = true;
    const w = useWorkspace();
    const route = {
      id: "old",
      name: "Existing",
      nasPort: 8080,
      localPort: 18080,
      upstream: "https://old.my-nas.fnos.net/",
      enabled: true,
    };
    w.profile.value.fnId = "my-nas";
    w.profile.value.services = [route];
    w.proxy.value = { running: true, listeners: [], requests: 4 };
    w.showEditor(route);
    await w.commitEditor();
    await w.remove(route);
    await w.setServiceEnabled(route, false);
    expect(w.profile.value.services).toEqual([route]);
    expect(mocks.invoke).not.toHaveBeenCalled();
  });
});
