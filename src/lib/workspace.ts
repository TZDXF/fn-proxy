import { computed, onMounted, onUnmounted, reactive, ref } from "vue";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
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
  type ServiceRoute,
  type LogEntry,
  type RouteProbe,
} from "./types";

export function useWorkspace() {
  const desktop = isTauri();
  const profile = reactive<Profile>({
    fnId: "",
    username: "",
    remember: false,
    autoConnect: false,
    services: [],
  });
  const password = ref("");
  const otp = ref("");
  const showPassword = ref(false);
  const savedIdentity = ref("");
  const hasSavedPassword = computed(
    () =>
      savedIdentity.value === `${profile.fnId.trim().toLowerCase()}\n${profile.username.trim()}`,
  );
  const section = ref<"overview" | "services" | "protocol" | "logs">("overview");
  const busy = ref("");
  const notice = ref<{ message: string; error: boolean } | null>(null);
  const inventory = ref<ServiceInventory | null>(null);
  const discovered = computed(() => inventory.value?.services ?? []);
  const logs = ref<LogEntry[]>([]);
  const connection = ref<ConnectionInfo>({
    connected: false,
    fnId: "",
    username: "",
    relay: "",
    authMode: "",
    message: "尚未连接 NAS",
  });
  const proxy = ref<ProxyStatus>({ running: false, listeners: [], requests: 0 });
  const probes = reactive<Record<string, RouteProbe>>({});
  const editor = reactive<ServiceRoute>({
    id: "",
    name: "",
    nasPort: 8084,
    localPort: 18084,
    upstream: "",
    enabled: true,
  });
  const editing = ref(false);
  const enabledServices = computed(() => profile.services.filter((s) => s.enabled));
  const formMatchesSession = computed(
    () =>
      connection.value.connected &&
      profile.fnId.trim().toLowerCase() === connection.value.fnId &&
      profile.username.trim() === connection.value.username,
  );
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
      notify("当前是界面预览。请运行 npm run desktop:dev，在桌面应用中登录和启用代理。", true);
      return;
    }
    if (busy.value) return;
    busy.value = label;
    try {
      return await action();
    } catch (error) {
      notify(String(error instanceof Error ? error.message : error), true);
      return;
    } finally {
      busy.value = "";
    }
  }
  async function refresh() {
    if (!desktop) return;
    const snapshot = await invoke<AppSnapshot>("get_snapshot");
    connection.value = snapshot.connection;
    proxy.value = snapshot.proxy;
  }
  async function connect() {
    await run("正在登录", async () => {
      profile.fnId = normalizeFnId(profile.fnId);
      if (!profile.username.trim()) throw new Error("请输入 NAS 用户名");
      if (!password.value && !hasSavedPassword.value) throw new Error("请输入 NAS 密码");
      inventory.value = null;
      connection.value = await invoke<ConnectionInfo>("connect_nas", {
        input: {
          fnId: profile.fnId,
          username: profile.username.trim(),
          password: password.value || null,
          otp: otp.value || null,
          remember: profile.remember,
        },
      });
      password.value = "";
      otp.value = "";
      notify("自动登录成功。下一步读取服务映射，或手动添加服务。");
      await refresh();
    });
  }
  async function save() {
    await run("正在保存", async () => {
      if (!formMatchesSession.value) throw new Error("请先使用当前 FN ID 和账号测试连接成功");
      await invoke("save_login", {
        profile: { ...profile, services: profile.services.map((s) => ({ ...s })) },
      });
      savedIdentity.value = profile.remember
        ? `${profile.fnId.trim().toLowerCase()}\n${profile.username.trim()}`
        : "";
      notify(
        profile.remember
          ? "登录已保存，密码存放在 Windows 凭据管理器中。"
          : "配置已保存，不持久化密码。",
      );
    });
  }
  async function forget() {
    await run("删除已保存凭据", async () => {
      await invoke("forget_login");
      savedIdentity.value = "";
      profile.remember = false;
      profile.autoConnect = false;
      notify("已删除已保存密码，当前连接不会因此立即撤销。");
    });
  }
  async function disconnect() {
    await run("正在断开", async () => {
      await invoke("disconnect_nas");
      inventory.value = null;
      await refresh();
      notify("连接和本地代理已停止。");
    });
  }
  async function discover() {
    await run("读取完整入口清单", async () => {
      inventory.value = null;
      inventory.value = await invoke<ServiceInventory>("get_service_inventory");
      notify(
        `当前账号可见 ${inventory.value.totalEntries} 个入口，${discovered.value.length} 个独立端口服务，${inventory.value.unmappedEntries} 个入口无法建立端口映射。`,
      );
      section.value = "services";
    });
  }
  function addDiscovered(service: DiscoveredService) {
    try {
      if (proxy.value.running) throw new Error("请先停止代理再修改映射");
      if (profile.services.some((s) => s.upstream === service.upstream))
        throw new Error("此服务已添加");
      const route: ServiceRoute = {
        id: crypto.randomUUID(),
        name: service.name,
        nasPort: service.nasPort,
        localPort: suggestedLocalPort(
          service.nasPort,
          profile.services.map((s) => s.localPort),
        ),
        upstream: service.upstream,
        enabled: true,
      };
      validateService(route, profile.fnId);
      profile.services.push(route);
      notify("映射已添加。点击保存登录可一并保存服务配置。");
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
        localPort: suggestedLocalPort(
          8084,
          profile.services.map((s) => s.localPort),
        ),
        upstream: "",
        enabled: true,
      },
    );
    editing.value = true;
  }
  function commitEditor() {
    try {
      if (proxy.value.running) throw new Error("请先停止代理再修改映射");
      validateService(editor, profile.fnId);
      if (profile.services.some((s) => s.id !== editor.id && s.localPort === editor.localPort))
        throw new Error("本地端口已被另一个映射使用");
      const index = profile.services.findIndex((s) => s.id === editor.id);
      const route = {
        ...editor,
        id: editor.id || crypto.randomUUID(),
        name: editor.name.trim(),
        upstream: new URL(editor.upstream).origin + "/",
      };
      if (index < 0) profile.services.push(route);
      else profile.services[index] = route;
      editing.value = false;
    } catch (error) {
      notify(String(error instanceof Error ? error.message : error), true);
    }
  }
  function remove(route: ServiceRoute) {
    if (proxy.value.running) {
      notify("请先停止代理再删除服务", true);
      return;
    }
    profile.services = profile.services.filter((s) => s.id !== route.id);
  }
  async function probe(route: ServiceRoute) {
    await run("测试服务", async () => {
      probes[route.id] = await invoke<RouteProbe>("probe_service", { route });
      notify(
        `${route.name} · HTTP ${probes[route.id]!.status} · ${probes[route.id]!.message}`,
        !probes[route.id]!.reachable,
      );
    });
  }
  async function toggleProxy() {
    await run(proxy.value.running ? "停止代理" : "启动代理", async () => {
      if (proxy.value.running) {
        await invoke("stop_proxy");
        await refresh();
        notify("本地代理已停止。");
        return;
      }
      if (!formMatchesSession.value) throw new Error("请先测试当前账号的连接");
      proxy.value = await invoke<ProxyStatus>("start_proxy", {
        services: profile.services.map((s) => ({ ...s })),
      });
      notify("代理已启动。本机浏览器和 API 客户端可使用下方本地地址。");
    });
  }
  async function refreshToken() {
    await run("更新服务凭据", async () => {
      await invoke("refresh_session");
      notify("服务访问凭据已更新，不需要复制 Cookie。");
    });
  }
  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      notify("已复制到剪贴板");
    } catch {
      notify("复制不可用，请手动选择并复制地址", true);
    }
  }
  async function open(port: number) {
    await run("打开服务", async () => {
      await openUrl(localUrl(port));
    });
  }
  onMounted(async () => {
    if (!desktop) return;
    try {
      const bootstrap = await invoke<{ profile: Profile; hasSavedPassword: boolean }>(
        "get_bootstrap",
      );
      Object.assign(profile, bootstrap.profile);
      savedIdentity.value = bootstrap.hasSavedPassword
        ? `${profile.fnId.trim().toLowerCase()}\n${profile.username.trim()}`
        : "";
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
      notify("读取桌面后端状态失败，请查看应用启动日志", true);
    }
  });
  onUnmounted(() => {
    if (timer) clearInterval(timer);
    if (noticeTimer) clearTimeout(noticeTimer);
    unlisten?.();
    password.value = "";
    otp.value = "";
  });
  return {
    desktop,
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
    probe,
    toggleProxy,
    refreshToken,
    copy,
    open,
    addDiscovered,
    localUrl,
  };
}
