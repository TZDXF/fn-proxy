import { i18n } from "./i18n";
import { computed, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  dockerPortService,
  normalizeFnId,
  validateService,
  suggestedLocalPort,
  localUrl,
  type Profile,
  type AppSnapshot,
  type ConnectionInfo,
  type ProxyStatus,
  type DiscoveredService,
  type ServiceInventory,
  type DockerPortRow,
  type ServiceRoute,
  type LogEntry,
  type RouteProbe,
} from "./types";

export function useWorkspace() {
  const t = i18n.global.t;
  const desktop = isTauri();
  function createConnection(profile?: Profile, hasSavedPassword = false, saved = false) {
    const config: Profile = profile ?? {
      id: crypto.randomUUID(),
      fnId: "",
      username: "",
      remember: false,
      autoConnect: false,
      services: [],
    };
    return {
      profile: config,
      saved,
      password: "",
      otp: "",
      showPassword: false,
      savedIdentity: hasSavedPassword
        ? `${config.fnId.trim().toLowerCase()}\n${config.username.trim()}`
        : "",
      inventory: null as ServiceInventory | null,
      probes: {} as Record<string, RouteProbe>,
      connection: {
        connected: false,
        fnId: "",
        username: "",
        relay: "",
        authMode: "",
        message: t("connection.notConnected"),
      } as ConnectionInfo,
      proxy: { running: false, listeners: [], requests: 0 } as ProxyStatus,
    };
  }
  const connections = reactive([createConnection()]);
  const selectedConnectionId = ref(connections[0]!.profile.id);
  const current = computed(() =>
    connections.find((c) => c.profile.id === selectedConnectionId.value)!,
  );
  const profile = computed(() => current.value.profile);
  const password = computed({
    get: () => current.value.password,
    set: (value: string) => {
      current.value.password = value;
    },
  });
  const otp = computed({
    get: () => current.value.otp,
    set: (value: string) => {
      current.value.otp = value;
    },
  });
  const showPassword = computed({
    get: () => current.value.showPassword,
    set: (value: boolean) => {
      current.value.showPassword = value;
    },
  });
  const savedIdentity = computed({
    get: () => current.value.savedIdentity,
    set: (value: string) => {
      current.value.savedIdentity = value;
    },
  });
  const hasSavedPassword = computed(
    () =>
      savedIdentity.value ===
      `${profile.value.fnId.trim().toLowerCase()}\n${profile.value.username.trim()}`,
  );
  const section = ref<"overview" | "connections" | "services" | "logs" | "settings">("overview");
  const savedConnections = computed(() => connections.filter((c) => c.saved));
  const busy = ref("");
  const notice = ref<{ message: string; error: boolean } | null>(null);
  const inventory = computed({
    get: () => current.value.inventory,
    set: (value: ServiceInventory | null) => {
      current.value.inventory = value;
    },
  });
  const discovered = computed(() => inventory.value?.services ?? []);
  const logs = ref<LogEntry[]>([]);
  const connection = computed({
    get: () => current.value.connection,
    set: (value: ConnectionInfo) => {
      current.value.connection = value;
    },
  });
  const proxy = computed({
    get: () => current.value.proxy,
    set: (value: ProxyStatus) => {
      current.value.proxy = value;
    },
  });
  const probes = computed(() => current.value.probes);
  const editor = reactive<ServiceRoute>({
    id: "",
    name: "",
    nasPort: 8084,
    localPort: 8084,
    upstream: "",
    enabled: true,
  });
  const editing = ref(false);
  const enabledServices = computed(() => profile.value.services.filter((s) => s.enabled));
  const formMatchesSession = computed(
    () =>
      connection.value.connected &&
      profile.value.fnId.trim().toLowerCase() === connection.value.fnId &&
      profile.value.username.trim() === connection.value.username,
  );
  function occupiedPorts(excludeId = ""): number[] {
    return connections.flatMap((c) =>
      c.profile.services
        .filter((s) => c.profile.id !== selectedConnectionId.value || s.id !== excludeId)
        .map((s) => s.localPort),
    );
  }
  let suggestedEditorPort = editor.localPort;
  watch(
    () => editor.nasPort,
    (port) => {
      if (!editing.value || editor.id || editor.localPort !== suggestedEditorPort) return;
      if (!Number.isInteger(port) || port < 1 || port > 65535) return;
      suggestedEditorPort = suggestedLocalPort(port, occupiedPorts());
      editor.localPort = suggestedEditorPort;
    },
  );
  watch(selectedConnectionId, () => {
    editing.value = false;
  });
  function addConnection() {
    if (busy.value) return;
    const target = createConnection();
    connections.push(target);
    selectedConnectionId.value = target.profile.id;
    section.value = "connections";
  }
  async function removeConnection(silent = false) {
    if (busy.value) return;
    const id = selectedConnectionId.value;
    const removeLocal = () => {
      const index = connections.findIndex((c) => c.profile.id === id);
      connections[index]!.password = "";
      connections[index]!.otp = "";
      connections.splice(index, 1);
      if (!connections.length) connections.push(createConnection());
      selectedConnectionId.value = (connections.find((c) => c.saved) ?? connections[0]!).profile.id;
      if (!silent) notify(t("notice.deleted"));
    };
    if (!desktop) {
      removeLocal();
      return;
    }
    await run("delete-connection", async () => {
      await invoke("remove_connection", { connectionId: id });
      removeLocal();
    });
  }
  let timer: ReturnType<typeof setInterval> | undefined;
  let unlisten: UnlistenFn | undefined;
  let noticeTimer: ReturnType<typeof setTimeout> | undefined;
  function notify(message: string, error = false) {
    notice.value = { message, error };
    if (noticeTimer) clearTimeout(noticeTimer);
    noticeTimer = setTimeout(() => {
      notice.value = null;
    }, 8000);
  }
  async function run<T>(label: string, action: () => Promise<T>): Promise<T | undefined> {
    if (!desktop) {
      notify(t("notice.preview"), true);
      return;
    }
    if (busy.value) return;
    busy.value = label;
    try {
      return await action();
    } catch (error) {
      notify(String(error instanceof Error ? error.message : error), true);
      return undefined;
    } finally {
      busy.value = "";
    }
  }
  async function refresh() {
    if (!desktop) return;
    const snapshot = await invoke<AppSnapshot>("get_snapshot");
    for (const item of snapshot.connections) {
      const target = connections.find((c) => c.profile.id === item.id);
      if (target) {
        target.connection = item.connection;
        target.proxy = item.proxy;
        // Only accept domain cache changes for the same route and fixed NAS port.
        // Do not overwrite unsaved names, enabled flags, ports or connection form edits.
        for (const route of target.profile.services) {
          const synced = item.services?.find(
            (service) => service.id === route.id && service.nasPort === route.nasPort,
          );
          if (synced && target.profile.fnId === item.connection.fnId)
            route.upstream = synced.upstream;
        }
      }
    }
  }
  async function connect() {
    return await run("connect", async () => {
      profile.value.fnId = normalizeFnId(profile.value.fnId);
      profile.value.username = profile.value.username.trim();
      if (!profile.value.username) throw new Error(t("validation.username"));
      if (!password.value && !hasSavedPassword.value) throw new Error(t("validation.password"));
      inventory.value = null;
      connection.value = await invoke<ConnectionInfo>("connect_nas", {
        connectionId: selectedConnectionId.value,
        input: {
          fnId: profile.value.fnId,
          username: profile.value.username.trim(),
          password: password.value || null,
          otp: otp.value || null,
          remember: profile.value.remember,
        },
      });
      password.value = "";
      otp.value = "";
      notify(t("notice.connected"));
      await refresh();
      return true;
    });
  }
  async function save() {
    return await run("save", async () => {
      if (!formMatchesSession.value) throw new Error(t("validation.testIdentity"));
      await invoke("save_login", {
        connectionId: selectedConnectionId.value,
        profile: { ...profile.value, services: profile.value.services.map((s) => ({ ...s })) },
      });
      savedIdentity.value = profile.value.remember
        ? `${profile.value.fnId.trim().toLowerCase()}\n${profile.value.username.trim()}`
        : "";
      current.value.saved = true;
      notify(t("notice.saved"));
      return true;
    });
  }
  async function forget() {
    await run("forget", async () => {
      await invoke("forget_login", { connectionId: selectedConnectionId.value });
      savedIdentity.value = "";
      profile.value.remember = false;
      profile.value.autoConnect = false;
      notify(t("notice.credentialsRemoved"));
    });
  }
  async function disconnect() {
    await run("disconnect", async () => {
      await invoke("disconnect_nas", { connectionId: selectedConnectionId.value });
      inventory.value = null;
      await refresh();
      notify(t("notice.disconnected"));
    });
  }
  async function discover() {
    await run("discover", async () => {
      inventory.value = null;
      inventory.value = await invoke<ServiceInventory>("get_service_inventory", {
        connectionId: selectedConnectionId.value,
      });
      await refresh();
      notify(t("notice.discovered", { count: discovered.value.length }));
      section.value = "services";
    });
  }
  async function addDiscovered(service: DiscoveredService) {
    if (profile.value.services.some((s) => s.nasPort === service.nasPort)) {
      notify(t("notice.alreadyAdded"), true);
      return;
    }
    showEditor();
    Object.assign(editor, {
      name: service.name,
      nasPort: service.nasPort,
      localPort: suggestedLocalPort(service.nasPort, occupiedPorts()),
      upstream: service.upstream,
    });
    await commitEditor();
  }
  async function addDockerPort(row: DockerPortRow) {
    try {
      await addDiscovered(dockerPortService(row));
    } catch (error) {
      notify(String(error instanceof Error ? error.message : error), true);
    }
  }
  function showEditor(route?: ServiceRoute) {
    Object.assign(
      editor,
      route ?? {
        id: "",
        name: "",
        nasPort: 8084,
        localPort: suggestedLocalPort(8084, occupiedPorts()),
        upstream: "",
        enabled: true,
      },
    );
    suggestedEditorPort = editor.localPort;
    editing.value = true;
  }
  async function updateServices(services: ServiceRoute[]): Promise<boolean> {
    const target = current.value;
    if (!desktop) {
      target.profile.services = services;
      return true;
    }
    const result = await run("save-services", async () => {
      const status = await invoke<ProxyStatus>("update_services", {
        connectionId: target.profile.id,
        services: services.map((s) => ({ ...s })),
      });
      target.profile.services = services.map((route) => {
        const listener = status.listeners.find(
          (listener) =>
            listener.nasPort === route.nasPort && listener.localUrl === localUrl(route.localPort),
        );
        return listener ? { ...route, upstream: listener.upstream } : route;
      });
      target.proxy = status;
      notify(t("notice.servicesSaved"));
      return true;
    });
    return result === true;
  }
  async function commitEditor() {
    if (busy.value) return;
    try {
      if (proxy.value.running && editor.id) throw new Error(t("validation.stopBeforeEdit"));
      validateService(editor, profile.value.fnId);
      if (occupiedPorts(editor.id).includes(editor.localPort))
        throw new Error(t("validation.portOccupied"));
      const route = {
        ...editor,
        id: editor.id || crypto.randomUUID(),
        name: editor.name.trim(),
        upstream: new URL(editor.upstream).origin + "/",
      };
      const services = profile.value.services.map((s) => ({ ...s }));
      const index = services.findIndex((s) => s.id === editor.id);
      if (index < 0) services.push(route);
      else services[index] = route;
      if (await updateServices(services)) editing.value = false;
    } catch (error) {
      notify(String(error instanceof Error ? error.message : error), true);
    }
  }
  async function remove(route: ServiceRoute) {
    if (proxy.value.running) {
      notify(t("validation.stopBeforeDelete"), true);
      return;
    }
    await updateServices(profile.value.services.filter((s) => s.id !== route.id));
  }
  async function setServiceEnabled(route: ServiceRoute, enabled: boolean) {
    if (proxy.value.running) return;
    await updateServices(
      profile.value.services.map((s) => ({
        ...s,
        enabled: s.id === route.id ? enabled : s.enabled,
      })),
    );
  }
  async function probe(route: ServiceRoute) {
    await run("probe", async () => {
      probes.value[route.id] = await invoke<RouteProbe>("probe_service", {
        connectionId: selectedConnectionId.value,
        route,
      });
      notify(
        `${route.name} · HTTP ${probes.value[route.id]!.status} · ${probes.value[route.id]!.message}`,
        !probes.value[route.id]!.reachable,
      );
    });
  }
  async function toggleProxy() {
    await run(proxy.value.running ? "stop-proxy" : "start-proxy", async () => {
      if (proxy.value.running) {
        await invoke("stop_proxy", { connectionId: selectedConnectionId.value });
        await refresh();
        notify(t("notice.proxyStopped"));
        return;
      }
      if (!formMatchesSession.value) throw new Error(t("validation.testAccount"));
      proxy.value = await invoke<ProxyStatus>("start_proxy", {
        connectionId: selectedConnectionId.value,
        services: profile.value.services.map((s) => ({ ...s })),
      });
      await refresh();
      notify(t("notice.proxyStarted"));
    });
  }
  async function refreshToken() {
    await run("refresh-token", async () => {
      await invoke("refresh_session", { connectionId: selectedConnectionId.value });
      notify(t("notice.credentialsUpdated"));
    });
  }
  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      notify(t("notice.copied"));
    } catch {
      notify(t("notice.copyFailed"), true);
    }
  }
  async function open(port: number) {
    await run("open-service", async () => {
      await openUrl(localUrl(port));
    });
  }
  onMounted(async () => {
    if (!desktop) return;
    try {
      const bootstrap = await invoke<{
        profiles: { profile: Profile; hasSavedPassword: boolean }[];
      }>("get_bootstrap");
      if (bootstrap.profiles.length) {
        connections.splice(
          0,
          connections.length,
          ...bootstrap.profiles.map((item) =>
            createConnection(item.profile, item.hasSavedPassword, Boolean(item.profile.fnId)),
          ),
        );
        selectedConnectionId.value = connections[0]!.profile.id;
      }
      logs.value = await invoke<LogEntry[]>("get_logs");
      unlisten = await listen<LogEntry>("fn-proxy:log", ({ payload }) => {
        logs.value = [...logs.value.slice(-199), payload];
      });
      await refresh();
      timer = setInterval(() => {
        refresh().catch(() => {
          /* App teardown may close IPC before the UI timer. */
        });
      }, 2000);
    } catch {
      notify(t("notice.bootstrapFailed"), true);
    }
  });
  onUnmounted(() => {
    if (timer) clearInterval(timer);
    if (noticeTimer) clearTimeout(noticeTimer);
    unlisten?.();
    for (const target of connections) {
      target.password = "";
      target.otp = "";
    }
  });
  return {
    desktop,
    connections,
    savedConnections,
    selectedConnectionId,
    addConnection,
    removeConnection,
    profile,
    password,
    otp,
    showPassword,
    hasSavedPassword,
    section,
    busy,
    notice,
    discovered,
    inventory,
    logs,
    connection,
    proxy,
    probes,
    editor,
    editing,
    enabledServices,
    formMatchesSession,
    connect,
    save,
    forget,
    disconnect,
    discover,
    showEditor,
    commitEditor,
    remove,
    setServiceEnabled,
    probe,
    toggleProxy,
    refreshToken,
    copy,
    open,
    addDiscovered,
    addDockerPort,
    localUrl,
  };
}
