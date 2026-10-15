export const SERVICE_ICON_CACHE_KEY = "fn-proxy.service-icons.v1";
const SUCCESS_TTL = 7 * 24 * 60 * 60 * 1000;
const FAILURE_TTL = 60 * 1000;
const MAX_ENTRIES = 128;
const MAX_BYTES = 2 * 1024 * 1024;

export interface ServiceIconRequest {
  connectionId: string;
  upstream: string;
}
interface CacheEntry {
  key: string;
  url: string;
  expiresAt: number;
}
type IconStorage = Pick<Storage, "getItem" | "setItem">;

function isIcon(value: unknown): value is string {
  return (
    typeof value === "string" &&
    /^data:image\/(?:x-icon|png|jpeg|gif|webp|svg\+xml);base64,[a-z0-9+/=]+$/i.test(value)
  );
}

function entryBytes(entry: CacheEntry) {
  // Include room for the serialized key, metadata and UTF-16 string storage.
  return (JSON.stringify(entry).length + 1) * 2;
}

export function createServiceIconCache(
  fetchIcon: (request: ServiceIconRequest) => Promise<string | null>,
  storage?: IconStorage,
  now: () => number = Date.now,
) {
  const entries = new Map<string, CacheEntry>();
  const pending = new Map<string, Promise<string>>();
  let bytes = 0;
  const keyOf = ({ connectionId, upstream }: ServiceIconRequest) =>
    JSON.stringify([connectionId, upstream]);

  function remove(key: string) {
    const entry = entries.get(key);
    if (entry) bytes -= entryBytes(entry);
    entries.delete(key);
  }
  function put(entry: CacheEntry) {
    remove(entry.key);
    if (entryBytes(entry) + 4 > MAX_BYTES) return;
    entries.set(entry.key, entry);
    bytes += entryBytes(entry);
    while (entries.size > MAX_ENTRIES || bytes + 4 > MAX_BYTES) {
      remove(entries.keys().next().value!);
    }
  }
  function persist() {
    try {
      // Persist image data only. No credentials or failure results are written to storage.
      const saved = [...entries.values()].filter((entry) => entry.url && entry.expiresAt > now());
      storage?.setItem(SERVICE_ICON_CACHE_KEY, JSON.stringify(saved));
    } catch {
      // Full/disabled storage must not prevent in-memory caching or displaying an icon.
    }
  }
  try {
    const raw = storage?.getItem(SERVICE_ICON_CACHE_KEY);
    const saved: unknown = raw && raw.length * 2 <= MAX_BYTES ? JSON.parse(raw) : null;
    if (Array.isArray(saved)) {
      for (const candidate of saved) {
        if (!candidate || typeof candidate !== "object") continue;
        const entry = candidate as Partial<CacheEntry>;
        if (
          typeof entry.key === "string" &&
          entry.key.length <= 4096 &&
          isIcon(entry.url) &&
          typeof entry.expiresAt === "number" &&
          Number.isFinite(entry.expiresAt) &&
          entry.expiresAt > now() &&
          entry.expiresAt <= now() + SUCCESS_TTL
        ) {
          put(entry as CacheEntry);
        }
      }
    }
  } catch {
    // Malformed/disabled storage is equivalent to an empty cache.
  }

  function read(request: ServiceIconRequest): string | undefined {
    const key = keyOf(request);
    const entry = entries.get(key);
    if (!entry) return undefined;
    if (entry.expiresAt <= now()) {
      remove(key);
      return undefined;
    }
    // Touch the entry so the least recently used icon is evicted first.
    entries.delete(key);
    entries.set(key, entry);
    return entry.url;
  }
  function load(request: ServiceIconRequest): Promise<string> {
    const cached = read(request);
    if (cached !== undefined) return Promise.resolve(cached);
    const key = keyOf(request);
    const existing = pending.get(key);
    if (existing) return existing;
    const complete = (value: unknown) => {
      const url = isIcon(value) ? value : "";
      put({ key, url, expiresAt: now() + (url ? SUCCESS_TTL : FAILURE_TTL) });
      pending.delete(key);
      if (url) persist();
      return url;
    };
    try {
      const task = fetchIcon({ ...request }).then(complete, () => complete(null));
      pending.set(key, task);
      return task;
    } catch {
      return Promise.resolve(complete(null));
    }
  }
  return { read, load };
}
