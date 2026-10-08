<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { isTauri } from "@tauri-apps/api/core";
import Titlebar from "./components/Titlebar.vue";
import { useI18n } from "vue-i18n";
import SettingsPage from "./components/SettingsPage.vue";
import { Label, TabsRoot, TabsList, TabsTrigger, TabsContent } from "reka-ui";
import Icon from "./components/Icon.vue";
import UiButton from "./components/ui/UiButton.vue";
import UiInput from "./components/ui/UiInput.vue";
import UiSwitch from "./components/ui/UiSwitch.vue";
import UiSelect from "./components/ui/UiSelect.vue";
import UiDialog from "./components/ui/UiDialog.vue";
import {
  dockerPortStatus,
  dockerPortService,
  suggestedLocalPort,
  type Profile,
  type DiscoveredService,
  type ServiceRoute,
} from "./lib/types";
import { useWorkspace } from "./lib/workspace";
import { localize } from "./lib/text";

const { t, locale } = useI18n({ useScope: "global" });
const desktop = isTauri();
const w = useWorkspace();
const {
  profile,
  savedConnections,
  selectedConnectionId,
  password,
  otp,
  showPassword,
  hasSavedPassword,
  connecting,
  section,
  busy,
  notice,
  connection,
  proxy,
  discovered,
  inventory,
  probes,
  editor,
  editing,
  logs,
  allowLanAccess,
  settingsReady,
  anyProxyRunning,
} = w;
const nav = computed(() => [
  { id: "overview", name: t("nav.overview"), icon: "grid" },
  { id: "connections", name: t("nav.connections"), icon: "link" },
  { id: "services", name: t("nav.services"), icon: "server" },
  { id: "logs", name: t("nav.logs"), icon: "terminal" },
  { id: "settings", name: t("nav.settings"), icon: "settings" },
]);
const title = computed(
  () => nav.value.find((item) => item.id === section.value)?.name ?? t("nav.overview"),
);
const runningCount = computed(() => savedConnections.value.filter((c) => c.proxy.running).length);
const serviceCount = computed(() =>
  savedConnections.value.reduce((total, c) => total + c.profile.services.length, 0),
);
const requests = computed(() =>
  savedConnections.value.reduce((total, c) => total + c.proxy.requests, 0),
);
const connectionOptions = computed(() =>
  savedConnections.value.map((c) => ({
    value: c.profile.id,
    label: `${c.profile.fnId} · ${c.profile.username}`,
  })),
);
const connectionDialog = ref(false);
const connectionBackup = ref<Profile | null>(null);
const serviceTab = ref("manual");
watch(serviceTab, async (tab) => {
  if (tab === "discovery" && editing.value) await w.discover();
});
const deleteTarget = ref<{ kind: "connection" | "service"; id: string; name: string } | null>(null);
const time = (n: number) => new Date(n).toLocaleString(locale.value, { hour12: false });
const cloneProfile = (p: Profile): Profile => ({
  ...p,
  services: p.services.map((s) => ({ ...s })),
});
function newConnection() {
  if (busy.value) return;
  notice.value = null;
  w.addConnection();
  connectionBackup.value = null;
  connectionDialog.value = true;
}
function editConnection(id: string) {
  if (busy.value) return;
  selectedConnectionId.value = id;
  connectionBackup.value = cloneProfile(profile.value);
  connectionDialog.value = true;
}
watch(connectionDialog, async (open) => {
  if (!open && connectionBackup.value) {
    Object.assign(profile.value, connectionBackup.value);
    connectionBackup.value = null;
  }
  if (!open) {
    password.value = "";
    otp.value = "";
    const target = w.connections.find((c) => c.profile.id === selectedConnectionId.value);
    if (target && !target.saved) {
      notice.value = null;
      await w.removeConnection(true);
    }
  }
});
async function saveConnection() {
  if (await w.save()) {
    connectionBackup.value = null;
    connectionDialog.value = false;
  }
}
function selectConnection(id: string) {
  if (!busy.value) selectedConnectionId.value = id;
}
async function toggleConnectionProxy(id: string) {
  if (busy.value) return;
  const target = w.connections.find((c) => c.profile.id === id);
  if (!target || target.connecting) return;
  selectConnection(id);
  const originSection = section.value;
  if (!target.proxy.running && !w.formMatchesSession.value && !(await w.connect(id))) {
    if (
      selectedConnectionId.value === id &&
      section.value === originSection &&
      !connectionDialog.value
    )
      editConnection(id);
    return;
  }
  await w.toggleProxy(id);
}
function showServices(id: string) {
  selectConnection(id);
  section.value = "services";
}
async function newService(route?: ServiceRoute) {
  if (busy.value || connecting.value) return;
  const connectionId = selectedConnectionId.value;
  notice.value = null;
  if (!w.formMatchesSession.value && !(await w.connect(connectionId))) return;
  if (selectedConnectionId.value !== connectionId) return;
  serviceTab.value = "manual";
  w.showEditor(route);
}
function chooseService(service: DiscoveredService) {
  w.showEditor({
    id: "",
    name: service.name,
    nasPort: service.nasPort,
    upstream: service.upstream,
    enabled: true,
    localPort: suggestedLocalPort(
      service.nasPort,
      w.connections.flatMap((c) => c.profile.services.map((s) => s.localPort)),
    ),
  });
  serviceTab.value = "manual";
}
async function confirmDelete() {
  const target = deleteTarget.value;
  if (!target) return;
  if (target.kind === "connection") {
    selectConnection(target.id);
    await w.removeConnection();
    if (!w.savedConnections.value.some((c) => c.profile.id === target.id))
      deleteTarget.value = null;
  } else {
    const route = profile.value.services.find((s) => s.id === target.id);
    if (route) await w.remove(route);
    if (!profile.value.services.some((s) => s.id === target.id)) deleteTarget.value = null;
  }
}
</script>

<template>
  <Titlebar v-if="desktop" />
  <TabsRoot
    v-model="section"
    orientation="vertical"
    class="app-shell"
    :class="{ 'app-shell--desktop': desktop }"
  >
    <aside class="sidebar">
      <div class="brand">
        <img class="brand-mark" src="/app-icon.svg" alt="" width="37" height="37" />
        <strong>FN Proxy<span>.</span></strong>
      </div>
      <TabsList class="navigation" :aria-label="t('nav.main')">
        <TabsTrigger
          v-for="item in nav"
          :key="item.id"
          :value="item.id"
          :aria-label="item.name"
          class="nav-item"
          :disabled="!!busy"
        >
          <Icon :name="item.icon" /><span>{{ item.name }}</span
          ><span v-if="item.id === 'services' && serviceCount" class="nav-count">{{
            serviceCount
          }}</span>
        </TabsTrigger>
      </TabsList>
      <div class="sidebar-footer">
        <span class="status-dot" :class="{ online: runningCount > 0 }" /><span>{{
          runningCount ? t("status.runningProxies", { count: runningCount }) : t("status.proxyOff")
        }}</span
        ><span class="version">v0.1.0</span>
      </div>
    </aside>
    <div class="main-shell">
      <main>
        <div class="page-heading">
          <h1>{{ title }}</h1>
          <UiButton
            v-if="section === 'connections'"
            variant="primary"
            :disabled="!!busy"
            @click="newConnection"
            ><Icon name="plus" />{{ t("common.addConnection") }}</UiButton
          ><UiButton
            v-if="section === 'services'"
            variant="primary"
            :disabled="
              !!busy || !savedConnections.some((c) => c.profile.id === selectedConnectionId)
            "
            @click="newService()"
            ><Icon name="plus" />{{ t("common.addService") }}</UiButton
          >
        </div>
        <TabsContent value="overview" class="page-content">
          <div class="stats-grid">
            <div class="stat-card">
              <span class="stat-icon"><Icon name="link" :size="21" /></span
              ><span>{{ t("overview.savedConnections") }}</span
              ><strong
                >{{ savedConnections.length
                }}<small v-if="locale === 'zh-CN'">{{ t("overview.countUnit") }}</small></strong
              ><span class="stat-detail">{{
                t("status.runningProxies", { count: runningCount })
              }}</span>
            </div>
            <div class="stat-card">
              <span class="stat-icon"><Icon name="server" :size="21" /></span
              ><span>{{ t("overview.services") }}</span
              ><strong
                >{{ serviceCount
                }}<small v-if="locale === 'zh-CN'">{{ t("overview.countUnit") }}</small></strong
              ><span class="stat-detail">{{
                t("overview.listeners", {
                  count: savedConnections.reduce((n, c) => n + c.proxy.listeners.length, 0),
                })
              }}</span>
            </div>
            <div class="stat-card">
              <span class="stat-icon"><Icon name="power" :size="21" /></span
              ><span>{{ t("overview.runningProxies") }}</span
              ><strong
                >{{ runningCount
                }}<small v-if="locale === 'zh-CN'">{{ t("overview.countUnit") }}</small></strong
              ><span class="stat-detail">{{
                t("overview.requests", { count: requests.toLocaleString(locale) })
              }}</span>
            </div>
          </div>
          <section class="panel">
            <div class="panel-heading">
              <h2>{{ t("overview.connectionSummary") }}</h2>
              <UiButton :disabled="!!busy" @click="newConnection"
                ><Icon name="plus" />{{ t("common.addConnection") }}</UiButton
              >
            </div>
            <div v-if="!savedConnections.length" class="empty-state">
              <Icon name="link" :size="36" />
              <h3>{{ t("overview.noConnections") }}</h3>
              <UiButton variant="primary" @click="newConnection">{{
                t("common.addConnection")
              }}</UiButton>
            </div>
            <div v-for="c in savedConnections" :key="c.profile.id" class="overview-row">
              <div class="connection-avatar"><Icon name="server" :size="22" /></div>
              <div class="row-identity">
                <strong>{{ c.profile.fnId }}</strong
                ><span>{{ c.profile.username }}</span>
              </div>
              <span class="badge" :class="{ success: c.proxy.running }"
                ><span class="status-dot" :class="{ online: c.proxy.running }" />{{
                  c.proxy.running ? t("status.proxyRunning") : t("status.proxyStopped")
                }}</span
              ><UiButton variant="ghost" :disabled="!!busy" @click="showServices(c.profile.id)"
                >{{ t("overview.serviceCount", { count: c.profile.services.length })
                }}<Icon name="arrow" :size="15" /></UiButton
              ><UiButton
                :variant="c.proxy.running ? 'secondary' : 'primary'"
                :disabled="
                  !!busy || (!c.proxy.running && !c.profile.services.some((s) => s.enabled))
                "
                @click="toggleConnectionProxy(c.profile.id)"
                ><Icon name="power" :size="16" />{{
                  c.proxy.running ? t("actions.stopProxy") : t("actions.startProxy")
                }}</UiButton
              >
            </div>
          </section>
        </TabsContent>
        <TabsContent value="connections" class="page-content">
          <section class="panel">
            <div class="panel-heading">
              <h2>
                {{ t("overview.savedConnections") }}
                <span class="count">{{ savedConnections.length }}</span>
              </h2>
            </div>
            <div v-if="!savedConnections.length" class="empty-state">
              <Icon name="link" :size="36" />
              <h3>{{ t("overview.noConnections") }}</h3>
              <UiButton variant="primary" @click="newConnection">{{
                t("common.addConnection")
              }}</UiButton>
            </div>
            <div v-else class="table-scroll">
              <table>
                <thead>
                  <tr>
                    <th>FN ID</th>
                    <th>{{ t("connection.account") }}</th>
                    <th>{{ t("connection.services") }}</th>
                    <th>{{ t("connection.proxy") }}</th>
                    <th class="align-right">{{ t("actions.operations") }}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-for="c in savedConnections" :key="c.profile.id">
                    <td>
                      <strong>{{ c.profile.fnId }}</strong>
                    </td>
                    <td>{{ c.profile.username }}</td>
                    <td>
                      <UiButton
                        variant="ghost"
                        :disabled="!!busy"
                        @click="showServices(c.profile.id)"
                        >{{ c.profile.services.length }}</UiButton
                      >
                    </td>
                    <td>
                      <span :class="['badge', { success: c.proxy.running }]">{{
                        c.proxy.running ? t("common.running") : t("common.stopped")
                      }}</span>
                    </td>
                    <td>
                      <div class="row-actions">
                        <UiButton
                          variant="ghost"
                          :disabled="
                            !!busy ||
                            c.connecting ||
                            (!c.proxy.running && !c.profile.services.some((s) => s.enabled))
                          "
                          :aria-busy="c.connecting"
                          :aria-label="
                            t(
                              c.proxy.running
                                ? 'actions.stopConnectionProxy'
                                : 'actions.startConnectionProxy',
                              {
                                name: c.profile.fnId,
                              },
                            )
                          "
                          @click="toggleConnectionProxy(c.profile.id)"
                          ><Icon name="power" />{{
                            c.proxy.running
                              ? t("actions.stopProxy")
                              : c.connecting
                                ? t("actions.startingProxy")
                                : t("actions.startProxy")
                          }}</UiButton
                        ><UiButton
                          variant="ghost"
                          :disabled="!!busy"
                          :aria-label="t('actions.editConnection', { name: c.profile.fnId })"
                          @click="editConnection(c.profile.id)"
                          ><Icon name="edit" />{{ t("common.edit") }}</UiButton
                        ><UiButton
                          variant="ghost"
                          :disabled="!!busy"
                          :aria-label="t('actions.deleteConnection', { name: c.profile.fnId })"
                          @click="
                            deleteTarget = {
                              kind: 'connection',
                              id: c.profile.id,
                              name: c.profile.fnId,
                            }
                          "
                          ><Icon name="trash"
                        /></UiButton>
                      </div>
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>
          </section>
        </TabsContent>
        <TabsContent value="services" class="page-content">
          <div v-if="savedConnections.length" class="service-toolbar">
            <UiSelect
              v-model="selectedConnectionId"
              :options="connectionOptions"
              :disabled="!!busy"
              :label="t('connection.current')"
            /><span :class="['badge', { success: proxy.running }]">{{
              proxy.running ? t("status.proxyRunning") : t("status.proxyStopped")
            }}</span
            ><UiButton
              :disabled="!!busy || (!proxy.running && !profile.services.some((s) => s.enabled))"
              @click="toggleConnectionProxy(selectedConnectionId)"
              ><Icon name="power" />{{
                proxy.running ? t("actions.stopProxy") : t("actions.startProxy")
              }}</UiButton
            >
          </div>
          <section class="panel">
            <div class="panel-heading">
              <h2>
                {{ t("service.added") }} <span class="count">{{ profile.services.length }}</span>
              </h2>
              <span class="muted">{{ profile.fnId }}</span>
            </div>
            <div v-if="!profile.services.length" class="empty-state">
              <Icon name="server" :size="36" />
              <h3>{{ t("service.empty") }}</h3>
              <UiButton
                v-if="savedConnections.length"
                variant="primary"
                :disabled="!!busy"
                @click="newService()"
                >{{ t("common.addService") }}</UiButton
              ><UiButton v-else variant="primary" @click="newConnection">{{
                t("common.addConnection")
              }}</UiButton>
            </div>
            <div v-else class="table-scroll">
              <table>
                <thead>
                  <tr>
                    <th>{{ t("service.name") }}</th>
                    <th>{{ t("service.upstream") }}</th>
                    <th>{{ t("service.local") }}</th>
                    <th>{{ t("service.status") }}</th>
                    <th>{{ t("service.enabled") }}</th>
                    <th class="align-right">{{ t("actions.operations") }}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-for="route in profile.services" :key="route.id">
                    <td>
                      <strong>{{ route.name }}</strong
                      ><span class="cell-sub">NAS :{{ route.nasPort }}</span>
                    </td>
                    <td>
                      <span class="url-text" :title="route.upstream">{{ route.upstream }}</span>
                    </td>
                    <td>
                      <div class="address-cell">
                        <code>{{ w.localUrl(route.localPort) }}</code
                        ><UiButton
                          variant="ghost"
                          :aria-label="t('actions.copyLocal', { name: route.name })"
                          @click="w.copy(w.localUrl(route.localPort))"
                          ><Icon name="copy" :size="15"
                        /></UiButton>
                      </div>
                    </td>
                    <td>
                      <span
                        v-if="
                          proxy.listeners.some((l) => l.localUrl === w.localUrl(route.localPort))
                        "
                        class="badge success"
                        >{{ t("status.listening") }}</span
                      ><span
                        v-else-if="probes[route.id]"
                        :class="['badge', probes[route.id]?.reachable ? 'success' : 'error']"
                        >HTTP {{ probes[route.id]?.status }}</span
                      ><span v-else class="badge">{{
                        route.enabled ? t("status.pending") : t("status.disabled")
                      }}</span>
                    </td>
                    <td>
                      <UiSwitch
                        :model-value="route.enabled"
                        :disabled="!!busy"
                        :label="t('actions.enable', { name: route.name })"
                        @update:model-value="w.setServiceEnabled(route, $event)"
                      />
                    </td>
                    <td>
                      <div class="row-actions">
                        <UiButton
                          variant="ghost"
                          :disabled="!!busy || !connection.connected"
                          :aria-label="t('actions.test', { name: route.name })"
                          @click="w.probe(route)"
                          ><Icon name="refresh" :size="16" /></UiButton
                        ><UiButton
                          variant="ghost"
                          :disabled="
                            !!busy ||
                            !proxy.listeners.some((l) => l.localUrl === w.localUrl(route.localPort))
                          "
                          :aria-label="t('actions.open', { name: route.name })"
                          @click="w.open(route.localPort)"
                          ><Icon name="external" :size="16" /></UiButton
                        ><UiButton
                          variant="ghost"
                          :disabled="!!busy"
                          :aria-label="t('actions.edit', { name: route.name })"
                          @click="newService(route)"
                          ><Icon name="edit" :size="16" /></UiButton
                        ><UiButton
                          variant="ghost"
                          :disabled="!!busy"
                          :aria-label="t('actions.delete', { name: route.name })"
                          @click="
                            deleteTarget = { kind: 'service', id: route.id, name: route.name }
                          "
                          ><Icon name="trash" :size="16"
                        /></UiButton>
                      </div>
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>
          </section>
        </TabsContent>
        <TabsContent value="logs" class="page-content"
          ><section class="panel">
            <div class="panel-heading">
              <h2>
                {{ t("nav.logs") }} <span class="count">{{ logs.length }}</span>
              </h2>
              <UiButton :disabled="!logs.length" @click="logs = []">{{
                t("common.clear")
              }}</UiButton>
            </div>
            <div v-if="!logs.length" class="empty-state">
              <Icon name="terminal" :size="36" />
              <h3>{{ t("logs.empty") }}</h3>
            </div>
            <div v-else class="log-list">
              <div
                v-for="(entry, index) in [...logs].reverse()"
                :key="`${entry.time}-${index}`"
                class="log-row"
              >
                <time>{{ time(entry.time) }}</time
                ><span :class="['log-level', entry.level]">{{
                  {
                    info: t("logs.info"),
                    success: t("logs.success"),
                    warn: t("logs.warn"),
                    error: t("logs.error"),
                  }[entry.level]
                }}</span
                ><span>{{ `[${entry.label}] ` }}{{ localize(entry.message) }}</span>
              </div>
            </div>
          </section></TabsContent
        >
        <TabsContent value="settings" class="page-content">
          <SettingsPage
            :allow-lan-access="allowLanAccess"
            :network-disabled="!desktop || !settingsReady || Boolean(busy) || anyProxyRunning"
            :any-proxy-running="anyProxyRunning"
            @update:allow-lan-access="w.setAllowLanAccess"
          />
        </TabsContent>
      </main>
    </div>
  </TabsRoot>

  <UiDialog
    v-model="connectionDialog"
    :title="connectionBackup ? t('connection.edit') : t('common.addConnection')"
    :busy="!!busy"
    :notice="notice"
  >
    <form @submit.prevent="saveConnection">
      <div class="form-grid">
        <div class="field">
          <Label for="fn-id">FN ID</Label
          ><UiInput
            id="fn-id"
            v-model="profile.fnId"
            placeholder="my-nas"
            autocomplete="off"
            required
            :disabled="!!busy || proxy.running"
          />
        </div>
        <div class="field">
          <Label for="username">{{ t("connection.username") }}</Label
          ><UiInput
            id="username"
            v-model="profile.username"
            autocomplete="username"
            required
            :disabled="!!busy || proxy.running"
          />
        </div>
      </div>
      <div class="field">
        <Label for="password">{{ t("connection.password") }}</Label>
        <div class="password-input">
          <UiInput
            id="password"
            v-model="password"
            :type="showPassword ? 'text' : 'password'"
            :placeholder="hasSavedPassword ? t('connection.savedPassword') : ''"
            autocomplete="current-password"
            :disabled="!!busy || proxy.running"
          /><UiButton
            variant="ghost"
            :aria-label="showPassword ? t('connection.hidePassword') : t('connection.showPassword')"
            @click="showPassword = !showPassword"
            ><Icon name="eye"
          /></UiButton>
        </div>
      </div>
      <div class="field">
        <Label for="otp">{{ t("connection.otp") }}</Label
        ><UiInput
          id="otp"
          v-model="otp"
          inputmode="numeric"
          autocomplete="one-time-code"
          :disabled="!!busy || proxy.running"
        />
      </div>
      <div class="setting-row">
        <Label for="remember">{{ t("connection.remember") }}</Label
        ><UiSwitch
          id="remember"
          v-model="profile.remember"
          :disabled="!!busy"
          :label="t('connection.remember')"
          @update:model-value="!$event && (profile.autoConnect = false)"
        />
      </div>
      <div class="dialog-footer">
        <UiButton
          :disabled="!!busy || connecting || proxy.running"
          :aria-busy="connecting"
          @click="w.connect()"
          >{{ connecting ? t("connection.connecting") : t("connection.test") }}</UiButton
        ><UiButton variant="primary" type="submit" :disabled="!!busy">{{
          busy === "save" ? t("common.saving") : t("connection.save")
        }}</UiButton>
      </div>
    </form>
  </UiDialog>

  <UiDialog
    v-model="editing"
    :title="editor.id ? t('service.editMapping') : t('service.newMapping')"
    :busy="!!busy"
    :notice="notice"
    wide
  >
    <TabsRoot v-model="serviceTab"
      ><TabsList v-if="!editor.id" class="dialog-tabs" :aria-label="t('service.addMethod')"
        ><TabsTrigger value="manual">{{ t("service.manual") }}</TabsTrigger
        ><TabsTrigger value="discovery">{{ t("service.discovery") }}</TabsTrigger></TabsList
      >
      <TabsContent value="manual"
        ><form @submit.prevent="w.commitEditor()">
          <div class="field">
            <Label for="service-name">{{ t("service.name") }}</Label
            ><UiInput id="service-name" v-model="editor.name" required :disabled="!!busy" />
          </div>
          <div class="field">
            <Label for="upstream">{{ t("service.upstream") }}</Label
            ><UiInput
              id="upstream"
              v-model="editor.upstream"
              type="url"
              :placeholder="`https://service.${profile.fnId || 'my-nas'}.fnos.net/`"
              required
              :disabled="!!busy"
            />
          </div>
          <div class="form-grid">
            <div class="field">
              <Label for="nas-port">{{ t("service.nasPort") }}</Label
              ><UiInput
                id="nas-port"
                v-model="editor.nasPort"
                type="number"
                min="1"
                max="65535"
                required
                :disabled="!!busy"
              />
              <span class="field-hint">{{ t("service.portBindingHint") }}</span>
            </div>
            <div class="field">
              <Label for="local-port">{{ t("service.localPort") }}</Label
              ><UiInput
                id="local-port"
                v-model="editor.localPort"
                type="number"
                min="1"
                max="65535"
                required
                :disabled="!!busy"
              />
            </div>
          </div>
          <div class="setting-row">
            <Label for="service-enabled">{{ t("service.enable") }}</Label
            ><UiSwitch
              id="service-enabled"
              v-model="editor.enabled"
              :disabled="!!busy"
              :label="t('service.enable')"
            />
          </div>
          <div class="dialog-footer">
            <UiButton :disabled="!!busy" @click="editing = false">{{ t("common.cancel") }}</UiButton
            ><UiButton type="submit" variant="primary" :disabled="!!busy">{{
              busy ? t("common.saving") : t("service.save")
            }}</UiButton>
          </div>
        </form></TabsContent
      >
      <TabsContent value="discovery"
        ><div class="discovery-toolbar">
          <strong>{{ profile.fnId }}</strong
          ><UiButton :disabled="!!busy || connecting" @click="w.discover()"
            ><Icon name="refresh" />{{ busy ? t("common.reading") : t("service.read") }}</UiButton
          >
        </div>
        <div v-if="!inventory" class="empty-state compact">
          <Icon name="server" :size="30" />
          <h3>{{ connecting ? t("connection.connecting") : t("service.notRead") }}</h3>
        </div>
        <template v-else
          ><div class="source-status">
            <span
              v-for="source in inventory.sources"
              :key="source.id"
              :class="['badge', source.status === 'ok' ? 'success' : 'error']"
              :title="localize(source.message)"
              >{{ localize(source.name) }} ·
              {{ source.status === "ok" ? (source.count ?? 0) : t("common.unavailable") }}</span
            >
          </div>
          <div class="discovered-list">
            <div v-for="service in discovered" :key="service.id" class="discovered-row">
              <div>
                <strong>{{ service.name }}</strong
                ><span class="cell-sub">:{{ service.nasPort }} · {{ service.upstream }}</span>
              </div>
              <UiButton
                :disabled="profile.services.some((s) => s.nasPort === service.nasPort)"
                @click="chooseService(service)"
                >{{
                  profile.services.some((s) => s.nasPort === service.nasPort)
                    ? t("common.added")
                    : t("common.select")
                }}</UiButton
              >
            </div>
            <div v-if="!discovered.length" class="empty-state compact">
              <h3>{{ t("service.noAvailable") }}</h3>
            </div>
          </div>
          <template v-if="inventory.docker"
            ><h3 class="subsection-title">{{ t("service.dockerPorts") }}</h3>
            <div class="discovered-list">
              <div v-for="row in inventory.docker.rows" :key="row.id" class="discovered-row">
                <div>
                  <strong>{{ row.containerName }}</strong
                  ><span class="cell-sub"
                    >{{ row.nasPort ?? "—" }} → {{ row.containerPort ?? "—" }}/{{ row.protocol }} ·
                    {{ dockerPortStatus(row.status) }}</span
                  >
                </div>
                <UiButton
                  :disabled="
                    row.status !== 'mapped' ||
                    profile.services.some((s) => s.nasPort === row.nasPort)
                  "
                  @click="chooseService(dockerPortService(row))"
                  >{{
                    profile.services.some((s) => s.nasPort === row.nasPort)
                      ? t("common.added")
                      : t("common.select")
                  }}</UiButton
                >
              </div>
            </div></template
          ></template
        ></TabsContent
      >
    </TabsRoot>
  </UiDialog>
  <UiDialog
    :model-value="!!deleteTarget"
    :title="t('dialog.confirmDelete')"
    :busy="!!busy"
    :notice="notice"
    @update:model-value="!$event && (deleteTarget = null)"
    ><div class="delete-name">{{ deleteTarget?.name }}</div>
    <div class="dialog-footer">
      <UiButton :disabled="!!busy" @click="deleteTarget = null">{{ t("common.cancel") }}</UiButton
      ><UiButton variant="danger" :disabled="!!busy" @click="confirmDelete">{{
        t("common.delete")
      }}</UiButton>
    </div></UiDialog
  >
  <div
    v-if="notice && !connectionDialog && !editing && !deleteTarget"
    :class="['toast', { error: notice.error }]"
    :role="notice.error ? 'alert' : 'status'"
  >
    <Icon :name="notice.error ? 'info' : 'check'" /><span>{{ notice.message }}</span
    ><UiButton variant="ghost" :aria-label="t('common.closeNotice')" @click="notice = null"
      ><Icon name="close" :size="16"
    /></UiButton>
  </div>
</template>
