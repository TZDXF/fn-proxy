import { describe, expect, it, vi } from "vitest";
import {
  createServiceIconCache,
  SERVICE_ICON_CACHE_KEY,
  type ServiceIconRequest,
} from "../lib/serviceIconCache";

const ICON = "data:image/png;base64,aWNvbg==";
const DAY = 24 * 60 * 60 * 1000;
const request = (connectionId = "a", upstream = "https://app.my-nas.fnos.net/") => ({
  connectionId,
  upstream,
});
function fixture(initial?: string) {
  let time = 1_000_000;
  const data = new Map<string, string>();
  if (initial !== undefined) data.set(SERVICE_ICON_CACHE_KEY, initial);
  const storage = {
    getItem: vi.fn((key: string) => data.get(key) ?? null),
    setItem: vi.fn((key: string, value: string) => {
      data.set(key, value);
    }),
  };
  const fetch = vi.fn(async (_request: ServiceIconRequest): Promise<string | null> => ICON);
  const now = () => time;
  const cache = createServiceIconCache(fetch, storage, now);
  return {
    cache,
    fetch,
    storage,
    now,
    data,
    advance: (ms: number) => {
      time += ms;
    },
  };
}

describe("service icon cache", () => {
  it("reuses successful images and persists only image data and cache metadata", async () => {
    const { cache, fetch, data, now } = fixture();
    expect(await cache.load(request())).toBe(ICON);
    expect(cache.read(request())).toBe(ICON);
    expect(await cache.load(request())).toBe(ICON);
    expect(fetch).toHaveBeenCalledOnce();
    expect(JSON.parse(data.get(SERVICE_ICON_CACHE_KEY)!)).toEqual([
      {
        key: JSON.stringify(["a", request().upstream]),
        url: ICON,
        expiresAt: now() + 7 * DAY,
      },
    ]);
  });

  it("restores images after an application restart without fetching them again", async () => {
    const { cache, fetch, storage, now } = fixture();
    await cache.load(request());
    const restarted = createServiceIconCache(fetch, storage, now);
    expect(restarted.read(request())).toBe(ICON);
    expect(await restarted.load(request())).toBe(ICON);
    expect(fetch).toHaveBeenCalledOnce();
  });

  it("refreshes images only after the seven-day cache lifetime expires", async () => {
    const { cache, fetch, advance } = fixture();
    await cache.load(request());
    advance(7 * DAY - 1);
    expect(cache.read(request())).toBe(ICON);
    advance(1);
    expect(cache.read(request())).toBeUndefined();
    await cache.load(request());
    expect(fetch).toHaveBeenCalledTimes(2);
  });

  it("accepts and restores SVG image data URLs", async () => {
    const { cache, fetch, storage, now } = fixture();
    const svg = "data:image/svg+xml;base64,PHN2Zy8+";
    fetch.mockResolvedValue(svg);
    expect(await cache.load(request())).toBe(svg);
    const restarted = createServiceIconCache(fetch, storage, now);
    expect(restarted.read(request())).toBe(svg);
    expect(await restarted.load(request())).toBe(svg);
    expect(fetch).toHaveBeenCalledOnce();
  });

  it("does not restore expired images", async () => {
    const { cache, fetch, advance, storage, now } = fixture();
    await cache.load(request());
    advance(7 * DAY);
    const restarted = createServiceIconCache(fetch, storage, now);
    expect(restarted.read(request())).toBeUndefined();
    await restarted.load(request());
    expect(fetch).toHaveBeenCalledTimes(2);
  });

  it.each(["missing", "rejected"])(
    "caches %s results for one minute without persisting them",
    async (mode) => {
      const { cache, fetch, advance, storage } = fixture();
      if (mode === "missing") fetch.mockResolvedValueOnce(null);
      else fetch.mockRejectedValueOnce(new Error("unavailable"));
      expect(await cache.load(request())).toBe("");
      expect(await cache.load(request())).toBe("");
      expect(fetch).toHaveBeenCalledOnce();
      expect(storage.setItem).not.toHaveBeenCalled();
      advance(60_000);
      expect(await cache.load(request())).toBe(ICON);
      expect(fetch).toHaveBeenCalledTimes(2);
    },
  );

  it("treats synchronous request errors as short-lived failures", async () => {
    const fetch = vi.fn(() => {
      throw new Error("IPC unavailable");
    });
    const cache = createServiceIconCache(fetch);
    expect(await cache.load(request())).toBe("");
    expect(await cache.load(request())).toBe("");
    expect(fetch).toHaveBeenCalledOnce();
  });

  it("does not mix cached images across connections or upstreams", async () => {
    const { cache, fetch } = fixture();
    await cache.load(request());
    await cache.load(request("b"));
    await cache.load(request("a", "https://other.my-nas.fnos.net/"));
    expect(fetch).toHaveBeenCalledTimes(3);
    await cache.load(request());
    expect(fetch).toHaveBeenCalledTimes(3);
  });

  it("limits entry count and evicts the least recently used image first", async () => {
    const { cache, fetch, data } = fixture();
    for (let i = 0; i < 128; i++) await cache.load(request(String(i)));
    expect(cache.read(request("0"))).toBe(ICON);
    await cache.load(request("128"));
    expect(cache.read(request("0"))).toBe(ICON);
    expect(cache.read(request("1"))).toBeUndefined();
    expect(JSON.parse(data.get(SERVICE_ICON_CACHE_KEY)!)).toHaveLength(128);
    expect(fetch).toHaveBeenCalledTimes(129);
  });

  it("bounds stored image data to two MiB", async () => {
    const { cache, fetch, data } = fixture();
    const large = "data:image/png;base64," + "A".repeat(350_000);
    fetch.mockResolvedValue(large);
    for (let i = 0; i < 4; i++) await cache.load(request(String(i)));
    const saved = data.get(SERVICE_ICON_CACHE_KEY)!;
    expect(saved.length * 2).toBeLessThanOrEqual(2 * 1024 * 1024);
    expect(cache.read(request("0"))).toBeUndefined();
    expect(cache.read(request("3"))).toBe(large);
  });

  it("keeps memory caching when persistent storage is disabled or full", async () => {
    const storage = {
      getItem: () => {
        throw new Error("disabled");
      },
      setItem: () => {
        throw new Error("quota");
      },
    };
    const fetch = vi.fn(async () => ICON);
    const cache = createServiceIconCache(fetch, storage);
    expect(await cache.load(request())).toBe(ICON);
    expect(await cache.load(request())).toBe(ICON);
    expect(fetch).toHaveBeenCalledOnce();
  });

  it.each(["broken JSON", "{}", "[null, 42]", '[{"key":"x","url":"https://evil.test/"}]'])(
    "ignores invalid persistent cache data: %s",
    async (raw) => {
      const { cache, fetch } = fixture(raw);
      expect(cache.read(request())).toBeUndefined();
      expect(await cache.load(request())).toBe(ICON);
      expect(fetch).toHaveBeenCalledOnce();
    },
  );

  it.each([
    "https://evil.test/icon.png",
    "data:text/html;base64,AAAA",
    "data:application/xml;base64,AAAA",
  ])("never persists an unsupported image response: %s", async (url) => {
    const { cache, fetch, storage } = fixture();
    fetch.mockResolvedValue(url);
    expect(await cache.load(request())).toBe("");
    expect(storage.setItem).not.toHaveBeenCalled();
  });
});
