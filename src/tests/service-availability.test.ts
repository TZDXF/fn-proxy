import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { effectScope, nextTick, reactive, type EffectScope } from "vue";
import { remoteStatus, useServiceAvailability } from "../lib/serviceAvailability";
import type { RouteProbe, ServiceRoute } from "../lib/types";

const mocks = vi.hoisted(() => ({
  desktop: true,
  mounted: [] as (() => void)[],
  unmounted: [] as (() => void)[],
}));
vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => mocks.desktop, invoke: vi.fn() }));
vi.mock("vue", async (original) => ({
  ...(await original<typeof import("vue")>()),
  onMounted: (callback: () => void) => mocks.mounted.push(callback),
  onUnmounted: (callback: () => void) => mocks.unmounted.push(callback),
}));
const scopes: EffectScope[] = [];
const route = (id = "api"): ServiceRoute => ({
  id,
  name: id,
  nasPort: 2017,
  localPort: 2017,
  upstream: `https://${id}.my-nas.fnos.net/`,
  enabled: true,
});
const ok: RouteProbe = {
  status: 200,
  reachable: true,
  message: { code: "workspace.probeReachable" },
};
const unavailable: RouteProbe = {
  status: 0,
  reachable: false,
  message: { code: "workspace.probeUnavailable" },
};
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => (resolve = done));
  return { promise, resolve };
}
function setup(routes = [route()], connected = true, fetch = vi.fn(async () => ok)) {
  const source = reactive({ connectionId: "a", connected, revision: 0, routes });
  const scope = effectScope();
  scopes.push(scope);
  const health = scope.run(() => useServiceAvailability(() => ({ ...source }), fetch))!;
  return { source, health, fetch };
}
async function settle() {
  await nextTick();
  await Promise.resolve();
  await Promise.resolve();
  await nextTick();
}
beforeEach(() => {
  vi.useFakeTimers();
  mocks.desktop = true;
  mocks.mounted.length = 0;
  mocks.unmounted.length = 0;
});
afterEach(() => {
  mocks.unmounted.forEach((callback) => callback());
  scopes.splice(0).forEach((scope) => scope.stop());
  vi.useRealTimers();
});

describe("remote address availability", () => {
  it("deduplicates discovery and saved addresses without depending on local listeners", async () => {
    const { fetch, health } = setup([route(), { ...route(), id: "discovered-api" }]);
    expect(fetch).toHaveBeenCalledExactlyOnceWith("a", route());
    await settle();
    expect(health.checks[route().upstream]?.result).toEqual(ok);
  });

  it("checks newly read or edited addresses and removes the old URL's status", async () => {
    const { source, health, fetch } = setup();
    await settle();
    source.routes = [route("new")];
    await settle();
    expect(health.checks[route().upstream]).toBeUndefined();
    expect(health.checks[route("new").upstream]?.result).toEqual(ok);
    expect(fetch).toHaveBeenCalledTimes(2);
  });

  it("limits concurrency to two and drops queued removed addresses", async () => {
    const first = deferred<RouteProbe>();
    const second = deferred<RouteProbe>();
    const fetch = vi
      .fn(async () => ok)
      .mockImplementationOnce(() => first.promise)
      .mockImplementationOnce(() => second.promise);
    const { source, health } = setup([route("one"), route("two"), route("three")], true, fetch);
    expect(fetch).toHaveBeenCalledTimes(2);
    source.routes.pop();
    await settle();
    first.resolve(unavailable);
    second.resolve(ok);
    await settle();
    expect(fetch).toHaveBeenCalledTimes(2);
    expect(health.checking.value).toBe(false);
    expect(health.checks[route("one").upstream]?.result).toEqual(unavailable);
  });

  it("refreshes every minute and manual checks can clear a previous failure", async () => {
    const fetch = vi.fn(async () => ok).mockResolvedValueOnce(unavailable);
    const { health } = setup([route()], true, fetch);
    mocks.mounted.forEach((callback) => callback());
    await settle();
    expect(remoteStatus(health.checks[route().upstream]).label).toBe("unavailable");
    await vi.advanceTimersByTimeAsync(59_999);
    expect(fetch).toHaveBeenCalledOnce();
    await vi.advanceTimersByTimeAsync(1);
    expect(fetch).toHaveBeenCalledTimes(2);
    expect(remoteStatus(health.checks[route().upstream]).label).toBe("available");
    health.recheck();
    await settle();
    expect(fetch).toHaveBeenCalledTimes(3);
  });

  it("does not probe browser previews or disconnected NAS sessions", async () => {
    mocks.desktop = false;
    const preview = setup();
    mocks.desktop = true;
    const disconnected = setup([route()], false);
    await settle();
    expect(preview.fetch).not.toHaveBeenCalled();
    expect(disconnected.fetch).not.toHaveBeenCalled();
    disconnected.source.connected = true;
    await settle();
    expect(disconnected.fetch).toHaveBeenCalledOnce();
  });

  it("keeps the one-minute refresh cadence when a response arrives after the initial check", async () => {
    const delayed = deferred<RouteProbe>();
    const fetch = vi.fn(async () => ok).mockImplementationOnce(() => delayed.promise);
    setup([route()], true, fetch);
    mocks.mounted.forEach((callback) => callback());
    await vi.advanceTimersByTimeAsync(1000);
    delayed.resolve(unavailable);
    await settle();
    await vi.advanceTimersByTimeAsync(59_000);
    expect(fetch).toHaveBeenCalledTimes(2);
  });

  it.each(["connection", "session", "url", "disconnect"])(
    "ignores late results after a change to the %s",
    async (change) => {
      const old = deferred<RouteProbe>();
      const fetch = vi.fn(async () => ok).mockImplementationOnce(() => old.promise);
      const { source, health } = setup([route()], true, fetch);
      if (change === "connection") source.connectionId = "b";
      if (change === "session") source.revision += 1;
      if (change === "url") source.routes = [route("new")];
      if (change === "disconnect") source.connected = false;
      await settle();
      old.resolve(unavailable);
      await settle();
      if (change === "disconnect") expect(Object.keys(health.checks)).toHaveLength(0);
      else expect(health.checks[source.routes[0]!.upstream]?.result).toEqual(ok);
    },
  );

  it("does not mark a remote URL invalid when IPC or the session check fails", async () => {
    const fetch = vi.fn(async (): Promise<RouteProbe> => {
      throw new Error("fixture IPC failure");
    });
    const { health } = setup([route()], true, fetch);
    await settle();
    expect(remoteStatus(health.checks[route().upstream])).toEqual({ label: "unknown", tone: "" });
  });

  it("clears the timer, pending queue and late results when the view closes", async () => {
    const old = deferred<RouteProbe>();
    const fetch = vi.fn(() => old.promise);
    const { health } = setup([route("one"), route("two"), route("three")], true, fetch);
    mocks.mounted.forEach((callback) => callback());
    mocks.unmounted.forEach((callback) => callback());
    old.resolve(ok);
    await settle();
    await vi.advanceTimersByTimeAsync(120_000);
    expect(fetch).toHaveBeenCalledTimes(2);
    expect(Object.values(health.checks).every((check) => !check.result)).toBe(true);
  });

  it.each([
    [401, "unauthorized", "warn"],
    [403, "forbidden", "warn"],
    [404, "notFound", "error"],
    [410, "notFound", "error"],
    [502, "failed", "error"],
  ])("distinguishes HTTP %i from a connection failure", (status, label, tone) => {
    expect(
      remoteStatus({
        checking: false,
        result: { status: status as number, reachable: false, message: { code: "fixture" } },
      }),
    ).toEqual({ label, tone });
  });
});
