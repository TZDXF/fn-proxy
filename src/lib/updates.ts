import { computed, ref } from "vue";
import { getVersion } from "@tauri-apps/api/app";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { check as checkSignedUpdate, type DownloadEvent } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
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
  install?: (version: string, onEvent: (event: DownloadEvent) => void) => Promise<void>;
  restart?: () => Promise<void>;
  open: (url: string) => Promise<unknown>;
}
const defaultDependencies: UpdateDependencies = {
  desktop: isTauri(),
  getVersion,
  install: async (version, onEvent) => {
    const update = await checkSignedUpdate({ timeout: 20000 });
    if (!update) throw "missingInstaller";
    try {
      if (update.version !== version) throw "versionChanged";
      await update.downloadAndInstall(onEvent, { timeout: 600000 });
    } finally {
      await update.close();
    }
  },
  restart: relaunch,
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
  const downloading = ref(false);
  const installed = ref(false);
  const installing = ref(false);
  const downloaded = ref(0);
  const total = ref(0);
  const progress = computed(() =>
    total.value > 0 ? Math.min(100, Math.floor((downloaded.value / total.value) * 100)) : null,
  );
  const downloadError = ref("");
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
    if (!dependencies.desktop || checking.value || downloading.value || installed.value) return;
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
  async function download() {
    if (
      !dependencies.desktop ||
      downloading.value ||
      installed.value ||
      !latest.value?.updateAvailable
    )
      return;
    const target = latest.value.latestVersion;
    if (!target || !dependencies.install) return;
    downloading.value = true;
    downloadError.value = "";
    downloaded.value = 0;
    total.value = 0;
    installing.value = false;
    try {
      await dependencies.install(target, (event) => {
        if (event.event === "Started") total.value = event.data.contentLength ?? 0;
        else if (event.event === "Progress") downloaded.value += event.data.chunkLength;
        else if (event.event === "Finished") installing.value = true;
      });
      installed.value = true;
    } catch (error) {
      const known = ["missingInstaller", "versionChanged"];
      downloadError.value =
        "updates." + (typeof error === "string" && known.includes(error) ? error : "downloadError");
    } finally {
      downloading.value = false;
    }
  }
  async function restart() {
    if (!installed.value || !dependencies.restart) return;
    try {
      await dependencies.restart();
    } catch {
      downloadError.value = "updates.restartError";
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
    downloading,
    installed,
    installing,
    downloaded,
    total,
    progress,
    restart,
    downloadError,
    download,
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
