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
  source: { code: "fixture" },
});
const info = (fnId: string): ConnectionInfo => ({
  connected: true,
  fnId,
  username: "admin",
  relay: "remote",
  authMode: "fixture",
  message: { code: "connected" },
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
    w.probes.value.api = { status: 200, reachable: true, message: { code: "ok" } };
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

describe("non-blocking background connections", () => {
  function deferred<T>() {
    let resolve!: (value: T) => void;
    let reject!: (error: Error) => void;
    const promise = new Promise<T>((yes, no) => {
      resolve = yes;
      reject = no;
    });
    return { promise, resolve, reject };
  }

  function setup() {
    mocks.desktop = true;
    const w = useWorkspace();
    Object.assign(w.profile.value, { fnId: "nas-a", username: "admin" });
    w.password.value = "first-password";
    const first = w.connections[0]!;
    w.addConnection();
    Object.assign(w.profile.value, { fnId: "nas-b", username: "admin" });
    w.password.value = "second-password";
    const second = w.connections[1]!;
    w.selectedConnectionId.value = first.profile.id;
    return { w, first, second };
  }

  it("does not hold the global busy lock and saves another connection while login is pending", async () => {
    const { w, first, second } = setup();
    const request = deferred<ConnectionInfo>();
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "connect_nas") return request.promise;
      if (command === "get_snapshot") return { connections: [] };
      if (command === "save_login") return false;
    });
    const connecting = w.connect();
    expect(w.busy.value).toBe("");
    expect(first.connecting).toBe(true);
    expect(w.connecting.value).toBe(true);
    w.selectedConnectionId.value = second.profile.id;
    expect(w.connecting.value).toBe(false);
    expect(await w.save()).toBe(true);
    expect(w.savedConnections.value[0]!.profile.id).toBe(second.profile.id);
    w.password.value = "new-second-password";
    w.addConnection();
    const draftId = w.selectedConnectionId.value;
    request.resolve(info("nas-a"));
    expect(await connecting).toBe(true);
    expect(first.connection.fnId).toBe("nas-a");
    expect(first.password).toBe("");
    expect(first.connecting).toBe(false);
    expect(second.connection.connected).toBe(false);
    expect(second.password).toBe("new-second-password");
    expect(w.selectedConnectionId.value).toBe(draftId);
    expect(w.connections).toHaveLength(3);
  });

  it("connects different rows concurrently and applies out-of-order results to the correct row", async () => {
    const { w, first, second } = setup();
    const a = deferred<ConnectionInfo>();
    const b = deferred<ConnectionInfo>();
    mocks.invoke.mockImplementation(async (command: string, args?: { connectionId: string }) => {
      if (command === "connect_nas")
        return args?.connectionId === first.profile.id ? a.promise : b.promise;
      if (command === "get_snapshot") return { connections: [] };
    });
    const connectingA = w.connect(first.profile.id);
    w.selectedConnectionId.value = second.profile.id;
    const connectingB = w.connect();
    expect(first.connecting).toBe(true);
    expect(second.connecting).toBe(true);
    expect(w.busy.value).toBe("");
    b.resolve(info("nas-b"));
    await connectingB;
    expect(second.connection.fnId).toBe("nas-b");
    expect(first.connection.connected).toBe(false);
    expect(first.connecting).toBe(true);
    a.resolve(info("nas-a"));
    await connectingA;
    expect(first.connection.fnId).toBe("nas-a");
    expect(second.connection.fnId).toBe("nas-b");
    expect(w.selectedConnectionId.value).toBe(second.profile.id);
    expect(first.connecting).toBe(false);
    expect(second.connecting).toBe(false);
  });

  it("ignores duplicate requests only for the same row", async () => {
    const { w, first } = setup();
    const request = deferred<ConnectionInfo>();
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "connect_nas") return request.promise;
      if (command === "get_snapshot") return { connections: [] };
    });
    const connecting = w.connect();
    expect(await w.connect()).toBeUndefined();
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    expect(first.connecting).toBe(true);
    expect(w.busy.value).toBe("");
    request.resolve(info("nas-a"));
    await connecting;
  });

  it("does not erase new form input entered during a pending request", async () => {
    const { w, first } = setup();
    const request = deferred<ConnectionInfo>();
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "connect_nas") return request.promise;
      if (command === "get_snapshot") return { connections: [] };
    });
    w.otp.value = "old-otp";
    const connecting = w.connect();
    w.profile.value.username = "edited-admin";
    w.password.value = "edited-password";
    w.otp.value = "new-otp";
    request.resolve(info("nas-a"));
    await connecting;
    expect(first.profile.username).toBe("edited-admin");
    expect(first.password).toBe("edited-password");
    expect(first.otp).toBe("new-otp");
    expect(first.connection.username).toBe("admin");
  });

  it("releases the row indicator on failure and leaves the newly selected connection alone", async () => {
    const { w, first, second } = setup();
    const request = deferred<ConnectionInfo>();
    mocks.invoke.mockReturnValue(request.promise);
    const connecting = w.connect();
    w.selectedConnectionId.value = second.profile.id;
    request.reject(new Error("login failed"));
    expect(await connecting).toBeUndefined();
    expect(first.connecting).toBe(false);
    expect(first.password).toBe("first-password");
    expect(second.password).toBe("second-password");
    expect(w.selectedConnectionId.value).toBe(second.profile.id);
    expect(w.busy.value).toBe("");
    expect(w.notice.value).toEqual({ message: "login failed", error: true });
  });

  it("does not apply a late result to a deleted row or its replacement", async () => {
    const { w, first, second } = setup();
    const request = deferred<ConnectionInfo>();
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "connect_nas") return request.promise;
    });
    const connecting = w.connect();
    await w.removeConnection();
    expect(w.selectedConnectionId.value).toBe(second.profile.id);
    request.resolve(info("nas-a"));
    expect(await connecting).toBeUndefined();
    expect(first.connection.connected).toBe(false);
    expect(second.connection.connected).toBe(false);
    expect(second.password).toBe("second-password");
    expect(mocks.invoke).not.toHaveBeenCalledWith("get_snapshot");
  });

  it("closes a connecting draft without locking the page while backend cleanup waits", async () => {
    const { w, first, second } = setup();
    const request = deferred<ConnectionInfo>();
    const cleanup = deferred<void>();
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "connect_nas") return request.promise;
      if (command === "remove_connection") return cleanup.promise;
    });
    const connecting = w.connect();
    const deleting = w.removeConnection(true);
    expect(w.connections.some((c) => c.profile.id === first.profile.id)).toBe(false);
    expect(w.selectedConnectionId.value).toBe(second.profile.id);
    expect(w.busy.value).toBe("");
    w.addConnection();
    const newId = w.selectedConnectionId.value;
    request.resolve(info("nas-a"));
    expect(await connecting).toBeUndefined();
    cleanup.resolve();
    await deleting;
    expect(w.selectedConnectionId.value).toBe(newId);
    expect(w.busy.value).toBe("");
    expect(w.notice.value).toBeNull();
  });

  it("does not resurrect state or issue notifications after unmount", async () => {
    const { w, first } = setup();
    const request = deferred<ConnectionInfo>();
    mocks.invoke.mockReturnValue(request.promise);
    const connecting = w.connect();
    mocks.unmounted[0]!();
    request.resolve(info("nas-a"));
    expect(await connecting).toBeUndefined();
    expect(first.connection.connected).toBe(false);
    expect(first.connecting).toBe(false);
    expect(w.notice.value).toBeNull();
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
  });

  it("starts the proxy for the explicitly requested connection after selection changes", async () => {
    const { w, first, second } = setup();
    first.connection = info("nas-a");
    w.selectedConnectionId.value = second.profile.id;
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "start_proxy") return { running: true, listeners: [], requests: 0 };
      if (command === "get_snapshot") return { connections: [] };
    });
    await w.toggleProxy(first.profile.id);
    expect(mocks.invoke).toHaveBeenCalledWith("start_proxy", {
      connectionId: first.profile.id,
      services: [],
    });
    expect(first.proxy.running).toBe(true);
    expect(second.proxy.running).toBe(false);
    expect(w.selectedConnectionId.value).toBe(second.profile.id);
  });
});

describe("saving connection configuration without connecting", () => {
  it("saves a new disconnected connection without a password or login IPC", async () => {
    mocks.desktop = true;
    mocks.invoke.mockResolvedValue(false);
    const w = useWorkspace();
    w.profile.value.fnId = "  My-NAS  ";
    w.profile.value.username = " admin ";
    expect(await w.save()).toBe(true);
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    expect(mocks.invoke).toHaveBeenCalledWith("save_login", {
      connectionId: w.selectedConnectionId.value,
      profile: expect.objectContaining({ fnId: "my-nas", username: "admin", remember: false }),
      password: null,
    });
    expect(w.savedConnections.value).toHaveLength(1);
    expect(w.connection.value.connected).toBe(false);
    expect(w.hasSavedPassword.value).toBe(false);
  });

  it("sends a remembered password separately and clears transient secrets after saving", async () => {
    mocks.desktop = true;
    mocks.invoke.mockResolvedValue(true);
    const w = useWorkspace();
    Object.assign(w.profile.value, { fnId: "my-nas", username: "admin", remember: true });
    w.password.value = "new-password";
    w.otp.value = "123456";
    expect(await w.save()).toBe(true);
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    const payload = mocks.invoke.mock.calls[0]![1];
    expect(payload.password).toBe("new-password");
    expect(payload.profile).not.toHaveProperty("password");
    expect(payload).not.toHaveProperty("otp");
    expect(w.hasSavedPassword.value).toBe(true);
    expect(w.password.value).toBe("");
    expect(w.otp.value).toBe("");
    expect(w.connection.value.connected).toBe(false);
  });

  it("preserves edits and secrets after a failed save without attempting a connection", async () => {
    mocks.desktop = true;
    mocks.invoke.mockRejectedValue(new Error("save failed"));
    const w = useWorkspace();
    Object.assign(w.profile.value, { fnId: "my-nas", username: "admin", remember: true });
    w.password.value = "draft-password";
    expect(await w.save()).toBeUndefined();
    expect(w.password.value).toBe("draft-password");
    expect(w.savedConnections.value).toHaveLength(0);
    expect(w.hasSavedPassword.value).toBe(false);
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    expect(w.notice.value).toEqual({ message: "save failed", error: true });
  });

  it("validates the account before submitting configuration", async () => {
    mocks.desktop = true;
    const w = useWorkspace();
    w.profile.value.fnId = "my-nas";
    w.profile.value.username = "  ";
    expect(await w.save()).toBeUndefined();
    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(w.notice.value?.error).toBe(true);
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
  it.each([
    { name: "Renamed" },
    { nasPort: 9090 },
    { localPort: 19090 },
    { upstream: "https://new.my-nas.fnos.net/" },
    { enabled: false },
  ])("applies edits to an existing running mapping: %j", async (changes) => {
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
    mocks.invoke.mockImplementation(async (_command, { services }) => ({
      running: services.some((s: typeof route) => s.enabled),
      listeners: services
        .filter((s: typeof route) => s.enabled)
        .map((s: typeof route) => ({
          name: s.name,
          nasPort: s.nasPort,
          localUrl: `http://127.0.0.1:${s.localPort}/`,
          upstream: s.upstream,
        })),
      requests: 4,
    }));
    w.showEditor(route);
    Object.assign(w.editor, changes);
    await w.commitEditor();
    expect(w.profile.value.services).toEqual([{ ...route, ...changes }]);
    expect(w.proxy.value.running).toBe(changes.enabled !== false);
    expect(w.proxy.value.requests).toBe(4);
    expect(w.editing.value).toBe(false);
    expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith("update_services", {
      connectionId: w.selectedConnectionId.value,
      services: [{ ...route, ...changes }],
      editedServiceId: route.id,
    });
  });
  it.each(["remove", "disable"])(
    "allows %s while running and stops the final listener",
    async (action) => {
      mocks.desktop = true;
      const w = useWorkspace();
      const route = { ...service(), localPort: 8084, enabled: true };
      w.profile.value.fnId = "my-nas";
      w.profile.value.services = [route];
      w.proxy.value = { running: true, listeners: [], requests: 4 };
      mocks.invoke.mockResolvedValue({ running: false, listeners: [], requests: 4 });
      if (action === "remove") await w.remove(route);
      else await w.setServiceEnabled(route, false);
      const services = action === "remove" ? [] : [{ ...route, enabled: false }];
      expect(w.profile.value.services).toEqual(services);
      expect(w.proxy.value).toEqual({ running: false, listeners: [], requests: 4 });
      expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith("update_services", {
        connectionId: w.selectedConnectionId.value,
        services,
      });
    },
  );
  it.each(["edit", "remove", "disable"])(
    "preserves an existing running mapping when %s fails",
    async (action) => {
      mocks.desktop = true;
      const w = useWorkspace();
      const route = { ...service(), localPort: 8084, enabled: true };
      const status = {
        running: true,
        listeners: [
          {
            name: route.name,
            nasPort: route.nasPort,
            localUrl: "http://127.0.0.1:8084/",
            upstream: route.upstream,
          },
        ],
        requests: 4,
      };
      w.profile.value.fnId = "my-nas";
      w.profile.value.services = [route];
      w.proxy.value = status;
      mocks.invoke.mockRejectedValue(new Error("保存失败"));
      if (action === "edit") {
        w.showEditor(route);
        w.editor.localPort = 18084;
        await w.commitEditor();
        expect(w.editing.value).toBe(true);
      } else if (action === "remove") await w.remove(route);
      else await w.setServiceEnabled(route, false);
      expect(w.profile.value.services).toEqual([route]);
      expect(w.proxy.value).toEqual(status);
      expect(w.notice.value?.error).toBe(true);
    },
  );
});

describe("LAN access settings", () => {
  it("defaults to disabled and blocks changes before backend state is ready", async () => {
    mocks.desktop = true;
    const w = useWorkspace();
    expect(w.allowLanAccess.value).toBe(false);
    expect(w.settingsReady.value).toBe(false);
    await w.setAllowLanAccess(true);
    expect(mocks.invoke).not.toHaveBeenCalled();
  });
  it("restores the backend setting even when no connections have been saved", async () => {
    mocks.desktop = true;
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "get_bootstrap") return { allowLanAccess: true, profiles: [] };
      if (command === "get_snapshot") return { connections: [] };
      if (command === "get_logs") return [];
    });
    const w = useWorkspace();
    await mocks.mounted[0]!();
    expect(w.allowLanAccess.value).toBe(true);
    expect(w.settingsReady.value).toBe(true);
  });
  it("saves only through backend IPC and updates the switch after success", async () => {
    mocks.desktop = true;
    const w = useWorkspace();
    w.settingsReady.value = true;
    let complete!: () => void;
    mocks.invoke.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          complete = resolve;
        }),
    );
    const saved = w.setAllowLanAccess(true);
    expect(w.busy.value).toBe("lan-access");
    expect(w.allowLanAccess.value).toBe(false);
    expect(mocks.invoke).toHaveBeenCalledWith("set_allow_lan_access", { enabled: true });
    complete();
    await saved;
    expect(w.allowLanAccess.value).toBe(true);
    expect(w.busy.value).toBe("");
  });
  it("keeps the previous setting and reports a failed save", async () => {
    mocks.desktop = true;
    mocks.invoke.mockRejectedValue(new Error("save failed"));
    const w = useWorkspace();
    w.settingsReady.value = true;
    await w.setAllowLanAccess(true);
    expect(w.allowLanAccess.value).toBe(false);
    expect(w.notice.value).toEqual({ message: "save failed", error: true });
    expect(w.busy.value).toBe("");
  });
  it("blocks changes when a proxy on another connection is still running", async () => {
    mocks.desktop = true;
    const w = useWorkspace();
    w.settingsReady.value = true;
    w.proxy.value = { running: true, listeners: [], requests: 0 };
    w.addConnection();
    expect(w.proxy.value.running).toBe(false);
    expect(w.anyProxyRunning.value).toBe(true);
    await w.setAllowLanAccess(true);
    expect(w.allowLanAccess.value).toBe(false);
    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(w.notice.value?.message).toContain("停止所有连接");
  });
});

describe("fixed NAS port domain cache", () => {
  it("refreshes domains from snapshots without overwriting local settings or another NAS", async () => {
    vi.useFakeTimers();
    try {
      mocks.desktop = true;
      const route = {
        id: "fixed",
        name: "Custom",
        nasPort: 8084,
        localPort: 18084,
        upstream: "https://old.my-nas.fnos.net/",
        enabled: false,
      };
      let snapshotServices = [{ ...route }];
      let snapshotFnId = "my-nas";
      mocks.invoke.mockImplementation(async (command: string) => {
        if (command === "get_bootstrap")
          return {
            profiles: [
              {
                profile: {
                  ...profile("first", "my-nas"),
                  services: [route],
                },
                hasSavedPassword: true,
              },
            ],
          };
        if (command === "get_logs") return [];
        if (command === "get_snapshot")
          return {
            connections: [
              {
                id: "first",
                connection: info(snapshotFnId),
                services: snapshotServices,
                proxy: { running: false, listeners: [], requests: 0 },
              },
            ],
          };
      });
      const w = useWorkspace();
      await mocks.mounted[0]!();
      w.password.value = "unsaved password";
      w.profile.value.services[0]!.name = "Unsaved name";
      w.profile.value.services[0]!.localPort = 19000;
      snapshotServices = [{ ...route, upstream: "https://new.my-nas.fnos.net/", enabled: true }];
      await vi.advanceTimersByTimeAsync(2000);
      expect(w.profile.value.services[0]).toEqual({
        ...route,
        name: "Unsaved name",
        localPort: 19000,
        upstream: "https://new.my-nas.fnos.net/",
      });
      expect(w.password.value).toBe("unsaved password");
      // A response for a different fixed port must not replace this route's cache.
      snapshotServices = [{ ...route, nasPort: 9000, upstream: "https://wrong.my-nas.fnos.net/" }];
      await vi.advanceTimersByTimeAsync(2000);
      expect(w.profile.value.services[0]!.upstream).toBe("https://new.my-nas.fnos.net/");
      snapshotFnId = "other-nas";
      snapshotServices = [{ ...route, upstream: "https://new.other-nas.fnos.net/" }];
      await vi.advanceTimersByTimeAsync(2000);
      expect(w.profile.value.services[0]!.upstream).toBe("https://new.my-nas.fnos.net/");
    } finally {
      mocks.unmounted.forEach((callback) => callback());
      vi.useRealTimers();
    }
  });
  it("does not add a duplicate fixed NAS port when the discovered domain changes", async () => {
    const w = useWorkspace();
    w.profile.value.fnId = "my-nas";
    await w.addDiscovered(service());
    await w.addDiscovered({ ...service(), upstream: "https://changed.my-nas.fnos.net/" });
    expect(w.profile.value.services).toHaveLength(1);
    expect(w.profile.value.services[0]!.nasPort).toBe(8084);
    expect(w.notice.value?.error).toBe(true);
  });
  it("accepts the authoritative live domain returned while adding another mapping", async () => {
    mocks.desktop = true;
    const w = useWorkspace();
    w.profile.value.fnId = "my-nas";
    const existing = {
      id: "fixed",
      name: "API",
      nasPort: 8084,
      localPort: 18084,
      upstream: "https://old.my-nas.fnos.net/",
      enabled: true,
    };
    w.profile.value.services = [existing];
    w.proxy.value = { running: true, listeners: [], requests: 7 };
    mocks.invoke.mockResolvedValue({
      running: true,
      requests: 7,
      listeners: [
        {
          name: "API",
          nasPort: 8084,
          localUrl: "http://127.0.0.1:18084/",
          upstream: "https://new.my-nas.fnos.net/",
        },
      ],
    });
    w.showEditor();
    Object.assign(w.editor, {
      name: "Added",
      nasPort: 9000,
      localPort: 19000,
      upstream: "https://added.my-nas.fnos.net/",
    });
    await w.commitEditor();
    expect(w.profile.value.services[0]!.upstream).toBe("https://new.my-nas.fnos.net/");
    expect(w.profile.value.services[0]!.localPort).toBe(18084);
  });
});
