<script setup lang="ts">
import { computed, ref, watch } from "vue";
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

const w = useWorkspace();
const {
  profile,
  savedConnections,
  selectedConnectionId,
  password,
  otp,
  showPassword,
  hasSavedPassword,
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
} = w;
const nav = [
  { id: "overview", name: "首页", icon: "grid" },
  { id: "connections", name: "连接", icon: "link" },
  { id: "services", name: "服务映射", icon: "server" },
  { id: "logs", name: "运行日志", icon: "terminal" },
];
const title = computed(() => nav.find((item) => item.id === section.value)?.name ?? "首页");
const connectedCount = computed(
  () => savedConnections.value.filter((c) => c.connection.connected).length,
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
const deleteTarget = ref<{ kind: "connection" | "service"; id: string; name: string } | null>(null);
const time = (n: number) => new Date(n).toLocaleString("zh-CN", { hour12: false });
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
  if (!w.formMatchesSession.value && !(await w.connect())) return;
  if (await w.save()) {
    connectionBackup.value = null;
    connectionDialog.value = false;
  }
}
function selectConnection(id: string) {
  if (!busy.value) selectedConnectionId.value = id;
}
async function toggleConnectionProxy(id: string) {
  selectConnection(id);
  if (!proxy.value.running && !w.formMatchesSession.value && !(await w.connect())) {
    editConnection(id);
    return;
  }
  await w.toggleProxy();
}
function showServices(id: string) {
  selectConnection(id);
  section.value = "services";
}
function newService(route?: ServiceRoute) {
  notice.value = null;
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
  <TabsRoot v-model="section" orientation="vertical" class="app-shell">
    <aside class="sidebar">
      <div class="brand">
        <div class="brand-mark"><Icon name="link" :size="25" /></div>
        <strong>FN Proxy<span>.</span></strong>
      </div>
      <TabsList class="navigation" aria-label="主导航">
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
        <span class="status-dot" :class="{ online: connectedCount > 0 }" /><span>{{
          connectedCount ? `${connectedCount} 个连接在线` : "未连接"
        }}</span
        ><span class="version">v0.1.0</span>
      </div>
    </aside>
    <div class="main-shell">
      <header class="topbar">
        <div>
          工作空间 <span class="separator">/</span> <strong>{{ title }}</strong>
        </div>
        <span class="runtime-status"
          ><span class="status-dot" :class="{ online: runningCount > 0 }" />{{
            runningCount ? `${runningCount} 个代理运行中` : "代理未开启"
          }}</span
        >
      </header>
      <main>
        <div class="page-heading">
          <h1>{{ title }}</h1>
          <UiButton
            v-if="section === 'connections'"
            variant="primary"
            :disabled="!!busy"
            @click="newConnection"
            ><Icon name="plus" />新增连接</UiButton
          ><UiButton
            v-if="section === 'services'"
            variant="primary"
            :disabled="
              !!busy || !savedConnections.some((c) => c.profile.id === selectedConnectionId)
            "
            @click="newService()"
            ><Icon name="plus" />新增服务</UiButton
          >
        </div>
        <TabsContent value="overview" class="page-content">
          <div class="stats-grid">
            <div class="stat-card">
              <span class="stat-icon"><Icon name="link" :size="21" /></span><span>已保存连接</span
              ><strong>{{ savedConnections.length }}<small>个</small></strong
              ><span class="stat-detail">{{ connectedCount }} 个在线</span>
            </div>
            <div class="stat-card">
              <span class="stat-icon"><Icon name="server" :size="21" /></span><span>服务数量</span
              ><strong>{{ serviceCount }}<small>个</small></strong
              ><span class="stat-detail"
                >{{
                  savedConnections.reduce((n, c) => n + c.proxy.listeners.length, 0)
                }}
                个监听中</span
              >
            </div>
            <div class="stat-card">
              <span class="stat-icon"><Icon name="power" :size="21" /></span><span>运行中代理</span
              ><strong>{{ runningCount }}<small>个</small></strong
              ><span class="stat-detail">{{ requests.toLocaleString() }} 次请求</span>
            </div>
          </div>
          <section class="panel">
            <div class="panel-heading">
              <h2>连接总览</h2>
              <UiButton :disabled="!!busy" @click="newConnection"
                ><Icon name="plus" />新增连接</UiButton
              >
            </div>
            <div v-if="!savedConnections.length" class="empty-state">
              <Icon name="link" :size="36" />
              <h3>暂无连接</h3>
              <UiButton variant="primary" @click="newConnection">新增连接</UiButton>
            </div>
            <div v-for="c in savedConnections" :key="c.profile.id" class="overview-row">
              <div class="connection-avatar"><Icon name="server" :size="22" /></div>
              <div class="row-identity">
                <strong>{{ c.profile.fnId }}</strong
                ><span>{{ c.profile.username }}</span>
              </div>
              <span class="badge" :class="{ success: c.connection.connected }"
                ><span class="status-dot" :class="{ online: c.connection.connected }" />{{
                  c.connection.connected ? "已连接" : "未连接"
                }}</span
              ><UiButton variant="ghost" :disabled="!!busy" @click="showServices(c.profile.id)"
                >{{ c.profile.services.length }} 个服务<Icon name="arrow" :size="15" /></UiButton
              ><UiButton
                :variant="c.proxy.running ? 'secondary' : 'primary'"
                :disabled="
                  !!busy || (!c.proxy.running && !c.profile.services.some((s) => s.enabled))
                "
                @click="toggleConnectionProxy(c.profile.id)"
                ><Icon name="power" :size="16" />{{
                  c.proxy.running ? "停止代理" : "开启代理"
                }}</UiButton
              >
            </div>
          </section>
        </TabsContent>
        <TabsContent value="connections" class="page-content">
          <section class="panel">
            <div class="panel-heading">
              <h2>
                已保存连接 <span class="count">{{ savedConnections.length }}</span>
              </h2>
            </div>
            <div v-if="!savedConnections.length" class="empty-state">
              <Icon name="link" :size="36" />
              <h3>暂无连接</h3>
              <UiButton variant="primary" @click="newConnection">新增连接</UiButton>
            </div>
            <div v-else class="table-scroll">
              <table>
                <thead>
                  <tr>
                    <th>FN ID</th>
                    <th>账号</th>
                    <th>连接状态</th>
                    <th>服务</th>
                    <th>代理</th>
                    <th class="align-right">操作</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-for="c in savedConnections" :key="c.profile.id">
                    <td>
                      <strong>{{ c.profile.fnId }}</strong>
                    </td>
                    <td>{{ c.profile.username }}</td>
                    <td>
                      <span class="badge" :class="{ success: c.connection.connected }"
                        ><span class="status-dot" :class="{ online: c.connection.connected }" />{{
                          c.connection.connected ? "已连接" : "未连接"
                        }}</span
                      >
                    </td>
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
                        c.proxy.running ? "运行中" : "已停止"
                      }}</span>
                    </td>
                    <td>
                      <div class="row-actions">
                        <UiButton
                          variant="ghost"
                          :disabled="!!busy"
                          @click="editConnection(c.profile.id)"
                          >{{ c.connection.connected ? "编辑" : "连接" }}</UiButton
                        ><UiButton
                          v-if="c.connection.connected"
                          variant="ghost"
                          :disabled="!!busy"
                          @click="
                            selectConnection(c.profile.id);
                            w.disconnect();
                          "
                          >断开</UiButton
                        ><UiButton
                          variant="ghost"
                          :disabled="!!busy"
                          :aria-label="`删除连接 ${c.profile.fnId}`"
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
              label="当前连接"
            /><span :class="['badge', { success: proxy.running }]">{{
              proxy.running ? "代理运行中" : "代理已停止"
            }}</span
            ><UiButton
              :disabled="!!busy || (!proxy.running && !profile.services.some((s) => s.enabled))"
              @click="toggleConnectionProxy(selectedConnectionId)"
              ><Icon name="power" />{{ proxy.running ? "停止代理" : "开启代理" }}</UiButton
            >
          </div>
          <section class="panel">
            <div class="panel-heading">
              <h2>
                已添加服务 <span class="count">{{ profile.services.length }}</span>
              </h2>
              <span class="muted">{{ profile.fnId }}</span>
            </div>
            <div v-if="!profile.services.length" class="empty-state">
              <Icon name="server" :size="36" />
              <h3>暂无服务映射</h3>
              <UiButton
                v-if="savedConnections.length"
                variant="primary"
                :disabled="!!busy"
                @click="newService()"
                >新增服务</UiButton
              ><UiButton v-else variant="primary" @click="newConnection">新增连接</UiButton>
            </div>
            <div v-else class="table-scroll">
              <table>
                <thead>
                  <tr>
                    <th>服务名称</th>
                    <th>远程地址</th>
                    <th>本地地址</th>
                    <th>状态</th>
                    <th>启用</th>
                    <th class="align-right">操作</th>
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
                          :aria-label="`复制 ${route.name} 本地地址`"
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
                        >监听中</span
                      ><span
                        v-else-if="probes[route.id]"
                        :class="['badge', probes[route.id]?.reachable ? 'success' : 'error']"
                        >HTTP {{ probes[route.id]?.status }}</span
                      ><span v-else class="badge">{{ route.enabled ? "待启动" : "未启用" }}</span>
                    </td>
                    <td>
                      <UiSwitch
                        :model-value="route.enabled"
                        :disabled="!!busy || proxy.running"
                        :label="`启用 ${route.name}`"
                        @update:model-value="w.setServiceEnabled(route, $event)"
                      />
                    </td>
                    <td>
                      <div class="row-actions">
                        <UiButton
                          variant="ghost"
                          :disabled="!!busy || !connection.connected"
                          :aria-label="`测试 ${route.name}`"
                          @click="w.probe(route)"
                          ><Icon name="refresh" :size="16" /></UiButton
                        ><UiButton
                          variant="ghost"
                          :disabled="
                            !!busy ||
                            !proxy.listeners.some((l) => l.localUrl === w.localUrl(route.localPort))
                          "
                          :aria-label="`打开 ${route.name}`"
                          @click="w.open(route.localPort)"
                          ><Icon name="external" :size="16" /></UiButton
                        ><UiButton
                          variant="ghost"
                          :disabled="!!busy || proxy.running"
                          :aria-label="`编辑 ${route.name}`"
                          @click="newService(route)"
                          ><Icon name="edit" :size="16" /></UiButton
                        ><UiButton
                          variant="ghost"
                          :disabled="!!busy || proxy.running"
                          :aria-label="`删除 ${route.name}`"
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
                运行日志 <span class="count">{{ logs.length }}</span>
              </h2>
              <UiButton :disabled="!logs.length" @click="logs = []">清空</UiButton>
            </div>
            <div v-if="!logs.length" class="empty-state">
              <Icon name="terminal" :size="36" />
              <h3>暂无日志</h3>
            </div>
            <div v-else class="log-list">
              <div
                v-for="(entry, index) in [...logs].reverse()"
                :key="`${entry.time}-${index}`"
                class="log-row"
              >
                <time>{{ time(entry.time) }}</time
                ><span :class="['log-level', entry.level]">{{
                  { info: "信息", success: "成功", warn: "警告", error: "错误" }[entry.level]
                }}</span
                ><span>{{ entry.message }}</span>
              </div>
            </div>
          </section></TabsContent
        >
      </main>
    </div>
  </TabsRoot>

  <UiDialog
    v-model="connectionDialog"
    :title="connectionBackup ? '编辑连接' : '新增连接'"
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
          <Label for="username">用户名</Label
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
        <Label for="password">密码</Label>
        <div class="password-input">
          <UiInput
            id="password"
            v-model="password"
            :type="showPassword ? 'text' : 'password'"
            :placeholder="hasSavedPassword ? '已保存密码' : ''"
            autocomplete="current-password"
            :disabled="!!busy || proxy.running"
          /><UiButton
            variant="ghost"
            :aria-label="showPassword ? '隐藏密码' : '显示密码'"
            @click="showPassword = !showPassword"
            ><Icon name="eye"
          /></UiButton>
        </div>
      </div>
      <div class="field">
        <Label for="otp">两步验证码</Label
        ><UiInput
          id="otp"
          v-model="otp"
          inputmode="numeric"
          autocomplete="one-time-code"
          :disabled="!!busy || proxy.running"
        />
      </div>
      <div class="setting-row">
        <Label for="remember">保存密码</Label
        ><UiSwitch
          id="remember"
          v-model="profile.remember"
          :disabled="!!busy"
          label="保存密码"
          @update:model-value="!$event && (profile.autoConnect = false)"
        />
      </div>
      <div class="dialog-footer">
        <span v-if="connection.connected" class="badge success">已连接</span
        ><UiButton :disabled="!!busy || proxy.running" @click="w.connect()">{{
          busy === "正在登录" ? "连接中…" : "测试连接"
        }}</UiButton
        ><UiButton variant="primary" type="submit" :disabled="!!busy">{{
          busy === "正在保存" ? "保存中…" : "保存连接"
        }}</UiButton>
      </div>
    </form>
  </UiDialog>

  <UiDialog
    v-model="editing"
    :title="editor.id ? '编辑服务映射' : '新增服务映射'"
    :busy="!!busy"
    :notice="notice"
    wide
  >
    <TabsRoot v-model="serviceTab"
      ><TabsList v-if="!editor.id" class="dialog-tabs" aria-label="添加服务方式"
        ><TabsTrigger value="manual">手动添加</TabsTrigger
        ><TabsTrigger value="discovery">服务发现</TabsTrigger></TabsList
      >
      <TabsContent value="manual"
        ><form @submit.prevent="w.commitEditor()">
          <div class="field">
            <Label for="service-name">服务名称</Label
            ><UiInput id="service-name" v-model="editor.name" required :disabled="!!busy" />
          </div>
          <div class="field">
            <Label for="upstream">远程地址</Label
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
              <Label for="nas-port">NAS 端口</Label
              ><UiInput
                id="nas-port"
                v-model="editor.nasPort"
                type="number"
                min="1"
                max="65535"
                required
                :disabled="!!busy"
              />
              <span class="field-hint"
                >固定使用 NAS 端口匹配服务；远程域名变化时自动更新，本地端口不变。Docker
                请填写宿主机发布端口。</span
              >
            </div>
            <div class="field">
              <Label for="local-port">本地端口</Label
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
            <Label for="service-enabled">启用服务</Label
            ><UiSwitch
              id="service-enabled"
              v-model="editor.enabled"
              :disabled="!!busy"
              label="启用服务"
            />
          </div>
          <div class="dialog-footer">
            <UiButton :disabled="!!busy" @click="editing = false">取消</UiButton
            ><UiButton type="submit" variant="primary" :disabled="!!busy">{{
              busy ? "保存中…" : "保存服务"
            }}</UiButton>
          </div>
        </form></TabsContent
      >
      <TabsContent value="discovery"
        ><div class="discovery-toolbar">
          <strong>{{ profile.fnId }}</strong
          ><UiButton :disabled="!!busy || !connection.connected" @click="w.discover()"
            ><Icon name="refresh" />{{ busy ? "读取中…" : "读取服务" }}</UiButton
          >
        </div>
        <div v-if="!inventory" class="empty-state compact">
          <Icon name="server" :size="30" />
          <h3>{{ connection.connected ? "尚未读取服务" : "未连接 NAS" }}</h3>
        </div>
        <template v-else
          ><div class="source-status">
            <span
              v-for="source in inventory.sources"
              :key="source.id"
              :class="['badge', source.status === 'ok' ? 'success' : 'error']"
              :title="source.message"
              >{{ source.name }} ·
              {{ source.status === "ok" ? (source.count ?? 0) : "不可用" }}</span
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
                  profile.services.some((s) => s.nasPort === service.nasPort) ? "已添加" : "选择"
                }}</UiButton
              >
            </div>
            <div v-if="!discovered.length" class="empty-state compact"><h3>暂无可添加服务</h3></div>
          </div>
          <template v-if="inventory.docker"
            ><h3 class="subsection-title">Docker 端口</h3>
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
                    profile.services.some((s) => s.nasPort === row.nasPort) ? "已添加" : "选择"
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
    title="确认删除"
    :busy="!!busy"
    :notice="notice"
    @update:model-value="!$event && (deleteTarget = null)"
    ><div class="delete-name">{{ deleteTarget?.name }}</div>
    <div class="dialog-footer">
      <UiButton :disabled="!!busy" @click="deleteTarget = null">取消</UiButton
      ><UiButton variant="danger" :disabled="!!busy" @click="confirmDelete">删除</UiButton>
    </div></UiDialog
  >
  <div
    v-if="notice && !connectionDialog && !editing && !deleteTarget"
    :class="['toast', { error: notice.error }]"
    :role="notice.error ? 'alert' : 'status'"
  >
    <Icon :name="notice.error ? 'info' : 'check'" /><span>{{ notice.message }}</span
    ><UiButton variant="ghost" aria-label="关闭通知" @click="notice = null"
      ><Icon name="close" :size="16"
    /></UiButton>
  </div>
</template>
