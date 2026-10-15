import { invoke, isTauri } from "@tauri-apps/api/core";
import { ref, watch } from "vue";
import { createServiceIconCache } from "./serviceIconCache";

function browserStorage() {
  try {
    return typeof window === "undefined" ? undefined : window.localStorage;
  } catch {
    return undefined;
  }
}

const sharedCache = createServiceIconCache(
  (request) => invoke<string | null>("get_service_icon", { ...request }),
  browserStorage(),
);

interface ServiceIconSource {
  connectionId: string;
  upstream: string;
  connected: boolean;
}

export function useServiceIcon(source: () => ServiceIconSource, cache = sharedCache) {
  const url = ref("");
  watch(
    source,
    async ({ connectionId, upstream, connected }, _, onCleanup) => {
      let current = true;
      onCleanup(() => {
        current = false;
      });
      url.value = "";
      if (!isTauri() || !connectionId || !upstream) return;
      const request = { connectionId, upstream };
      const cached = cache.read(request);
      if (cached !== undefined) {
        url.value = cached;
        return;
      }
      if (!connected) return;
      const icon = await cache.load(request);
      if (current) url.value = icon;
    },
    { immediate: true },
  );
  return url;
}
