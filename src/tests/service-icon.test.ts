import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { effectScope, nextTick, reactive, type EffectScope } from "vue";
import { useServiceIcon } from "../lib/serviceIcon";
import { createServiceIconCache } from "../lib/serviceIconCache";

const mocks = vi.hoisted(() => ({ desktop: true, invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({
  isTauri: () => mocks.desktop,
  invoke: mocks.invoke,
}));
const scopes: EffectScope[] = [];
let cache: ReturnType<typeof createServiceIconCache>;
function setup(connected = true) {
  const source = reactive({
    connectionId: "connection-a",
    upstream: "https://app.my-nas.fnos.net/",
    connected,
  });
  const scope = effectScope();
  scopes.push(scope);
  const url = scope.run(() => useServiceIcon(() => ({ ...source }), cache))!;
  return { source, url, scope };
}
async function settle() {
  await Promise.resolve();
  await Promise.resolve();
  await nextTick();
}
beforeEach(() => {
  mocks.desktop = true;
  mocks.invoke.mockReset();
  cache = createServiceIconCache((request) => mocks.invoke("get_service_icon", request));
});
afterEach(() => {
  scopes.splice(0).forEach((scope) => scope.stop());
});

describe("service website icons", () => {
  it("loads authenticated icons without starting or depending on a local proxy", async () => {
    mocks.invoke.mockResolvedValue("data:image/x-icon;base64,fixture");
    const { url, source } = setup();
    expect(url.value).toBe("");
    expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith("get_service_icon", {
      connectionId: source.connectionId,
      upstream: source.upstream,
    });
    await settle();
    expect(url.value).toBe("data:image/x-icon;base64,fixture");
  });

  it("does not request icons in browser preview or while disconnected", async () => {
    mocks.desktop = false;
    expect(setup().url.value).toBe("");
    mocks.desktop = true;
    const { source, url } = setup(false);
    expect(mocks.invoke).not.toHaveBeenCalled();
    mocks.invoke.mockResolvedValue("data:image/png;base64,fixture");
    source.connected = true;
    await settle();
    expect(url.value).toBe("data:image/png;base64,fixture");
  });

  it.each([null, new Error("optional icon request failed")])(
    "keeps the default icon on a missing icon or failed request: %s",
    async (result) => {
      if (result instanceof Error) mocks.invoke.mockRejectedValue(result);
      else mocks.invoke.mockResolvedValue(result);
      const { url } = setup();
      await settle();
      expect(url.value).toBe("");
    },
  );

  it("does not display late results from a previous connection or upstream", async () => {
    let resolveOld!: (value: string) => void;
    mocks.invoke.mockImplementationOnce(
      () =>
        new Promise<string>((resolve) => {
          resolveOld = resolve;
        }),
    );
    mocks.invoke.mockResolvedValue("data:image/png;base64,new");
    const { source, url } = setup();
    source.connectionId = "connection-b";
    source.upstream = "https://other.my-nas.fnos.net/";
    await settle();
    expect(url.value).toBe("data:image/png;base64,new");
    resolveOld("data:image/png;base64,old");
    await settle();
    expect(url.value).toBe("data:image/png;base64,new");
    expect(mocks.invoke).toHaveBeenLastCalledWith("get_service_icon", {
      connectionId: "connection-b",
      upstream: source.upstream,
    });
  });

  it("reuses cached icons while disconnected and after reconnect without refetching", async () => {
    mocks.invoke.mockResolvedValue("data:image/png;base64,old");
    const { source, url } = setup();
    await settle();
    expect(url.value).not.toBe("");
    source.connected = false;
    await settle();
    expect(url.value).toBe("data:image/png;base64,old");
    source.connected = true;
    await settle();
    expect(mocks.invoke).toHaveBeenCalledOnce();
    expect(url.value).toBe("data:image/png;base64,old");
  });

  it("shares in-flight requests and cached results between discovery and mapping components", async () => {
    let resolve!: (value: string) => void;
    mocks.invoke.mockImplementation(
      () =>
        new Promise<string>((done) => {
          resolve = done;
        }),
    );
    const first = setup();
    const second = setup();
    expect(mocks.invoke).toHaveBeenCalledOnce();
    resolve("data:image/png;base64,shared");
    await settle();
    expect(first.url.value).toBe("data:image/png;base64,shared");
    expect(second.url.value).toBe(first.url.value);
    first.scope.stop();
    second.scope.stop();
    expect(setup().url.value).toBe("data:image/png;base64,shared");
    expect(mocks.invoke).toHaveBeenCalledOnce();
  });

  it.each(["disconnect", "unmount"])("ignores pending results after %s", async (action) => {
    let resolve!: (value: string) => void;
    mocks.invoke.mockImplementation(
      () =>
        new Promise<string>((done) => {
          resolve = done;
        }),
    );
    const { source, url, scope } = setup();
    if (action === "disconnect") source.connected = false;
    else scope.stop();
    await settle();
    resolve("data:image/png;base64,stale");
    await settle();
    expect(url.value).toBe("");
  });
});
