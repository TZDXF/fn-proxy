import { computed, ref } from "vue";
import { getVersion } from "@tauri-apps/api/app";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { version as previewVersion } from "../../package.json";

export const RELEASES_URL = "https://github.com/TZDXF/fn-proxy/releases";
export interface UpdateInfo {
  currentVersion: string;
  latestVersion: string | null;
  updateAvailable: boolean;
  releaseUrl: string;
}
type UpdateState = "idle" | "checking" | "available" | "upToDate" | "noRelease" | "failed";
interface UpdateDependencies {
  desktop: boolean;
  getVersion: () => Promise<string>;
  check: () => Promise<UpdateInfo>;
  open: (url: string) => Promise<unknown>;
}
const defaultDependencies: UpdateDependencies = {
  desktop: isTauri(),
  getVersion,
  check: () => invoke<UpdateInfo>("check_for_updates"),
  open: async (url) => {
    if (isTauri()) await openUrl(url);
    else window.open(url, "_blank", "noopener,noreferrer");
  },
};

export function createUpdates(dependencies: UpdateDependencies = defaultDependencies) {
  const version = ref(dependencies.desktop ? "" : previewVersion);
  const versionLabel = computed(() => (version.value ? "v" + version.value : ""));
  const state = ref<UpdateState>("idle");
  const latest = ref<UpdateInfo | null>(null);
  const errorKey = ref("");
  const opening = ref(false);
  const openFailed = ref(false);
  const versionFailed = ref(false);
  const checking = computed(() => state.value === "checking");
  async function initialize() {
    if (!dependencies.desktop) return;
    try {
      version.value = await dependencies.getVersion();
      versionFailed.value = false;
    } catch {
      versionFailed.value = true;
    }
  }
  async function check() {
    if (!dependencies.desktop || checking.value) return;
    state.value = "checking";
    errorKey.value = "";
    latest.value = null;
    try {
      const result = await dependencies.check();
      version.value = result.currentVersion;
      versionFailed.value = false;
      latest.value = result;
      state.value =
        result.latestVersion === null
          ? "noRelease"
          : result.updateAvailable
            ? "available"
            : "upToDate";
    } catch (error) {
      state.value = "failed";
      errorKey.value =
        error === "rateLimited"
          ? "updates.rateLimited"
          : error === "invalidResponse"
            ? "updates.invalidResponse"
            : error === "unexpectedResponse"
              ? "updates.unexpectedResponse"
              : "updates.networkError";
    }
  }
  async function openRelease() {
    if (opening.value) return;
    opening.value = true;
    openFailed.value = false;
    try {
      const target = latest.value?.releaseUrl ?? RELEASES_URL;
      // Defence in depth: restrict opener even if the IPC payload is malformed.
      const url = new URL(target);
      if (
        url.origin !== "https://github.com" ||
        url.username ||
        url.password ||
        (url.pathname !== "/TZDXF/fn-proxy/releases" &&
          !url.pathname.startsWith("/TZDXF/fn-proxy/releases/tag/"))
      )
        throw new Error("Invalid release URL");
      await dependencies.open(url.href);
    } catch {
      openFailed.value = true;
    } finally {
      opening.value = false;
    }
  }
  return {
    version,
    versionLabel,
    state,
    latest,
    errorKey,
    opening,
    openFailed,
    versionFailed,
    checking,
    desktop: dependencies.desktop,
    initialize,
    check,
    openRelease,
  };
}
