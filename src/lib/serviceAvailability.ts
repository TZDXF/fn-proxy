import { invoke, isTauri } from "@tauri-apps/api/core";
import { computed, onMounted, onUnmounted, reactive, watch } from "vue";
import type { RouteProbe, ServiceRoute } from "./types";

export interface RemoteCheck {
  checking: boolean;
  checkedAt?: number;
  result?: RouteProbe;
}
export interface AvailabilitySource {
  connectionId: string;
  connected: boolean;
  revision: number;
  routes: ServiceRoute[];
}
const CHECK_INTERVAL = 60_000;

export function remoteStatus(check?: RemoteCheck) {
  const result = check?.result;
  if (!result) return { label: check?.checking ? "checking" : "unchecked", tone: "" };
  if (result.reachable) return { label: "available", tone: "success" };
  if (result.status === 401) return { label: "unauthorized", tone: "warn" };
  if (result.status === 403) return { label: "forbidden", tone: "warn" };
  if (result.status === 404 || result.status === 410) return { label: "notFound", tone: "error" };
  if (result.status === 0) {
    if (result.message.code === "workspace.probeTimeout")
      return { label: "timeout", tone: "error" };
    if (result.message.code === "workspace.probeUnavailable")
      return { label: "unavailable", tone: "error" };
    return { label: "unknown", tone: "" };
  }
  return { label: "failed", tone: "error" };
}

// Results belong to one connection session and one exact remote URL, never to a local listener.
export function useServiceAvailability(
  source: () => AvailabilitySource,
  fetchProbe = (connectionId: string, route: ServiceRoute) =>
    invoke<RouteProbe>("probe_service", { connectionId, route }),
) {
  const checks = reactive<Record<string, RemoteCheck>>({});
  let identity = "";
  let generation = 0;
  let active = 0;
  let disposed = false;
  let queue: { generation: number; connectionId: string; route: ServiceRoute }[] = [];
  let timer: ReturnType<typeof setInterval> | undefined;
  const hasRoute = (upstream: string) => source().routes.some((r) => r.upstream === upstream);

  function pump() {
    while (!disposed && active < 2 && queue.length) {
      const request = queue.shift()!;
      const { upstream } = request.route;
      if (request.generation !== generation || !source().connected || !hasRoute(upstream)) continue;
      active += 1;
      void (async () => {
        let result: RouteProbe;
        try {
          result = await fetchProbe(request.connectionId, request.route);
        } catch {
          // An IPC/session failure says nothing about whether the remote address still exists.
          result = { status: 0, reachable: false, message: { code: "workspace.probeUnknown" } };
        }
        if (
          !disposed &&
          request.generation === generation &&
          source().connected &&
          hasRoute(upstream)
        )
          checks[upstream] = { checking: false, checkedAt: Date.now(), result };
      })().finally(() => {
        active -= 1;
        pump();
      });
    }
  }
  function refresh(force = false) {
    if (disposed || !isTauri()) return;
    const current = source();
    const nextIdentity = JSON.stringify([
      current.connectionId,
      current.connected,
      current.revision,
    ]);
    if (identity !== nextIdentity) {
      identity = nextIdentity;
      generation += 1;
      queue = [];
      for (const key of Object.keys(checks)) delete checks[key];
    }
    for (const key of Object.keys(checks)) {
      if (!hasRoute(key)) delete checks[key];
    }
    if (!current.connected) return;
    for (const route of current.routes) {
      const previous = checks[route.upstream];
      if (
        previous?.checking ||
        (!force &&
          previous?.checkedAt !== undefined &&
          Date.now() - previous.checkedAt < CHECK_INTERVAL)
      )
        continue;
      checks[route.upstream] = { ...previous, checking: true };
      queue.push({ generation, connectionId: current.connectionId, route: { ...route } });
    }
    pump();
  }
  const stop = watch(
    () => {
      const current = source();
      return JSON.stringify([
        current.connectionId,
        current.connected,
        current.revision,
        current.routes.map((r) => r.upstream),
      ]);
    },
    () => refresh(),
    { immediate: true },
  );
  onMounted(() => {
    timer = setInterval(() => refresh(true), CHECK_INTERVAL);
  });
  onUnmounted(() => {
    disposed = true;
    generation += 1;
    queue = [];
    stop();
    if (timer) clearInterval(timer);
  });
  return {
    checks,
    checking: computed(() => Object.values(checks).some((check) => check.checking)),
    recheck: () => refresh(true),
  };
}
