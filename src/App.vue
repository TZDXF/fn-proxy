<script setup lang="ts">
import { computed } from "vue";
import Icon from "./components/Icon.vue";
import { useWorkspace } from "./lib/workspace";
const w = useWorkspace();
const {
  profile,
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
  enabledServices,
  logs,
  formMatchesSession,
} = w;
const title = computed(
  () =>
    ({ overview: "连接与代理", services: "服务映射", protocol: "协议与安全", logs: "运行日志" })[
      section.value
    ],
);
const nav = [
  { id: "overview" as const, name: "连接与代理", icon: "grid" },
  { id: "services" as const, name: "服务映射", icon: "server" },
  { id: "protocol" as const, name: "协议与安全", icon: "shield" },
  { id: "logs" as const, name: "运行日志", icon: "terminal" },
];
const lastLogs = computed(() => logs.value.slice(-4).reverse());
const time = (n: number) => new Date(n).toLocaleTimeString("zh-CN", { hour12: false });
</script>

<template>
  <div class="app-shell">
    <aside class="sidebar">
      <div class="brand">
        <div class="brand-mark">
          <svg viewBox="0 0 32 32" fill="none" aria-hidden="true">
            <path
              d="M7 24V8h18M7 16h13M20 16v8"
              stroke="currentColor"
              stroke-width="4"
              stroke-linecap="round"
              stroke-linejoin="round"
            />
            <circle cx="25" cy="24" r="3" fill="currentColor" />
          </svg>
        </div>
        <div>
          <strong>FN Proxy<span class="brand-dot">.</span></strong
          ><span class="brand-sub">PRIVATE CONNECTION</span>
        </div>
      </div>
      <div class="nav-label">工作空间</div>
      <nav aria-label="主导航">
        <button
          v-for="item in nav"
          :key="item.id"
          :class="['nav-item', { active: section === item.id }]"
          @click="section = item.id"
        >
          <Icon :name="item.icon" /><span>{{ item.name }}</span
          ><span v-if="item.id === 'services' && profile.services.length" class="nav-count">{{
            profile.services.length
          }}</span>
        </button>
      </nav>
      <div class="sidebar-tip">
        <div class="tip-icon"><Icon name="shield" :size="22" /></div>
        <strong>你的凭据，留在本机</strong>
        <p>密码使用系统凭据管理器保存。服务 Token 不经过前端，不写入日志。</p>
        <button @click="section = 'protocol'">了解安全边界 <Icon name="arrow" :size="14" /></button>
      </div>
      <div class="sidebar-footer">
        <span class="status-dot" :class="{ online: connection.connected }" /><span>{{
          connection.connected ? "NAS 已连接" : "等待连接"
        }}</span
        ><span class="version">v0.1.0</span>
      </div>
    </aside>
    <div class="main-shell">
      <header class="topbar">
        <div class="breadcrumb">
          工作空间 <span>/</span> <strong>{{ title }}</strong>
        </div>
        <div class="topbar-right">
          <span class="beta">PHASE 02</span
          ><span class="platform"><span class="status-dot online" /> Windows · 本地运行</span>
        </div>
      </header>
      <main>
        <div v-if="!w.desktop" class="preview-banner">
          <Icon name="info" />当前为界面预览，登录与代理由桌面后端执行。请运行
          <code>npm run desktop:dev</code>。
        </div>
        <div class="page-heading">
          <div>
            <div class="eyebrow">FN CONNECT, WITHOUT THE FRICTION</div>
            <h1>{{ section === "overview" ? "让 NAS 服务，直达本机。" : title }}</h1>
            <p>
              {{
                section === "overview"
                  ? "一次安全登录，让浏览器、脚本与 API 客户端共享远程访问能力。"
                  : section === "services"
                    ? "把 NAS 端口与远程子域名配对，为每个服务分配一个本地入口。"
                    : section === "protocol"
                      ? "复用合法登录会话，不绕过认证，也不把 NAS 凭据交给其他应用。"
                      : "只记录连接状态和操作结果，不记录密码、Token 或业务数据。"
              }}
            </p>
          </div>
          <button
            v-if="connection.connected"
            class="button subtle"
            :disabled="!!busy"
            @click="w.refreshToken"
          >
            <Icon name="refresh" />更新服务凭据
          </button>
        </div>
        <template v-if="section === 'overview'">
          <div class="stats-grid">
            <div class="stat">
              <div class="stat-icon"><Icon name="link" /></div>
              <div>
                <span>NAS 连接</span
                ><strong
                  >{{ connection.connected ? "已认证" : "未连接"
                  }}<i :class="['mini-dot', { green: connection.connected }]"
                /></strong>
              </div>
            </div>
            <div class="stat">
              <div class="stat-icon"><Icon name="server" /></div>
              <div>
                <span>已配置服务</span
                ><strong>{{ profile.services.length }} <small>个服务入口</small></strong>
              </div>
            </div>
            <div class="stat">
              <div class="stat-icon"><Icon name="terminal" /></div>
              <div>
                <span>代理请求</span
                ><strong>{{ proxy.requests.toLocaleString() }} <small>次转发</small></strong>
              </div>
            </div>
          </div>
          <div class="workspace-grid">
            <section class="card connection-card">
              <div class="card-heading">
                <div class="heading-icon"><Icon name="globe" /></div>
                <div>
                  <h2>连接你的 NAS</h2>
                  <p>使用 FN ID 和 NAS 本地账号登录</p>
                </div>
                <span :class="['badge', connection.connected ? 'success' : 'neutral']">{{
                  connection.connected ? "已连接" : "待连接"
                }}</span>
              </div>
              <form @submit.prevent="w.connect">
                <div class="field">
                  <label for="fnid">FN ID <span>远程入口</span></label>
                  <div class="input-wrap">
                    <Icon name="link" /><input
                      id="fnid"
                      v-model="profile.fnId"
                      placeholder="例如 my-nas"
                      autocomplete="off"
                      :disabled="!!busy"
                    /><span class="input-suffix">.fnos.net</span>
                  </div>
                </div>
                <div class="field">
                  <label for="username">NAS 用户名</label
                  ><input
                    id="username"
                    v-model="profile.username"
                    placeholder="输入 NAS 本地账号"
                    autocomplete="username"
                    :disabled="!!busy"
                  />
                </div>
                <div class="field">
                  <label for="password"
                    >密码
                    <span v-if="hasSavedPassword" class="saved-hint"
                      ><Icon name="lock" :size="12" />已安全保存</span
                    ></label
                  >
                  <div class="input-wrap">
                    <input
                      id="password"
                      v-model="password"
                      :type="showPassword ? 'text' : 'password'"
                      :placeholder="
                        hasSavedPassword ? '留空使用系统中已保存的密码' : '输入 NAS 登录密码'
                      "
                      autocomplete="off"
                      :disabled="!!busy"
                    /><button
                      type="button"
                      class="input-button"
                      aria-label="切换密码可见性"
                      @click="showPassword = !showPassword"
                    >
                      <Icon name="eye" />
                    </button>
                  </div>
                </div>
                <details class="otp-details">
                  <summary>账号启用了二次验证？</summary>
                  <div class="field">
                    <label for="otp">六位动态验证码</label
                    ><input
                      id="otp"
                      v-model="otp"
                      placeholder="000000"
                      inputmode="numeric"
                      maxlength="6"
                      autocomplete="one-time-code"
                    />
                  </div>
                </details>
                <div class="options">
                  <label class="checkbox-label"
                    ><input
                      v-model="profile.remember"
                      type="checkbox"
                      @change="!profile.remember && (profile.autoConnect = false)"
                    /><span>将密码安全保存到系统凭据管理器</span></label
                  ><label class="checkbox-label secondary"
                    ><input
                      v-model="profile.autoConnect"
                      type="checkbox"
                      :disabled="!profile.remember"
                    /><span>启动应用时自动登录</span></label
                  >
                </div>
                <div class="form-actions">
                  <button type="submit" class="button primary" :disabled="!!busy || !w.desktop">
                    <span v-if="busy === '正在登录'" class="spinner" /><Icon v-else name="link" />{{
                      busy === "正在登录" ? "正在安全登录…" : "测试并登录"
                    }}</button
                  ><button
                    type="button"
                    class="button secondary-button"
                    :disabled="!!busy || !formMatchesSession"
                    @click="w.save"
                  >
                    <Icon name="lock" />保存登录
                  </button>
                </div>
              </form>
              <div class="connection-note">
                <Icon name="shield" :size="15" /><span
                  >加密握手 · 后端维护会话 · 前端不接收 Token</span
                >
              </div>
              <div v-if="connection.connected" class="connected-detail">
                <span class="status-dot online" /><span
                  >{{ connection.username }} @ {{ connection.fnId }}</span
                ><button :disabled="!!busy" @click="w.disconnect">断开连接</button>
              </div>
              <button
                v-if="hasSavedPassword"
                class="forget-link"
                :disabled="!!busy"
                @click="w.forget"
              >
                删除已保存的密码
              </button>
            </section>
            <div class="right-column">
              <section class="card proxy-card">
                <div class="card-heading">
                  <div class="heading-icon"><Icon name="power" /></div>
                  <div>
                    <h2>本地代理</h2>
                    <p>服务地址直接成为本机地址</p>
                  </div>
                  <span :class="['badge', proxy.running ? 'success' : 'neutral']">{{
                    proxy.running ? "运行中" : "已停止"
                  }}</span>
                </div>
                <div class="proxy-visual">
                  <div class="flow-node">
                    <div class="flow-icon"><Icon name="terminal" :size="22" /></div>
                    <strong>本机应用</strong><span>浏览器 / API</span>
                  </div>
                  <div class="flow-line"><span /><Icon name="arrow" :size="16" /></div>
                  <div class="flow-node">
                    <div class="flow-icon center"><Icon name="shield" :size="23" /></div>
                    <strong>FN Proxy</strong><span>自动附加凭据</span>
                  </div>
                  <div class="flow-line"><span /><Icon name="arrow" :size="16" /></div>
                  <div class="flow-node">
                    <div class="flow-icon"><Icon name="server" :size="22" /></div>
                    <strong>NAS 服务</strong><span>FN Connect</span>
                  </div>
                </div>
                <div class="proxy-meta">
                  <span><Icon name="lock" :size="14" />仅监听 127.0.0.1</span
                  ><span>HTTP · SSE · WebSocket</span>
                </div>
                <button
                  :class="['button proxy-button', proxy.running ? 'stop-button' : 'primary']"
                  :disabled="
                    !!busy ||
                    (!proxy.running && (!formMatchesSession || !enabledServices.length)) ||
                    !w.desktop
                  "
                  @click="w.toggleProxy"
                >
                  <Icon name="power" />{{
                    busy === "启动代理"
                      ? "正在启动…"
                      : busy === "停止代理"
                        ? "正在停止…"
                        : proxy.running
                          ? "停止代理"
                          : "启用代理"
                  }}
                </button>
                <p class="muted-help">
                  {{
                    !connection.connected
                      ? "先完成 NAS 登录，再添加服务映射。"
                      : !enabledServices.length
                        ? "添加至少一个服务映射，即可启用本地代理。"
                        : proxy.running
                          ? "关闭应用会停止本地代理；不在局域网暴露监听端口。"
                          : `已准备 ${enabledServices.length} 个服务，启动后通过本地端口访问。`
                  }}
                </p>
              </section>
              <section class="card quick-card">
                <div class="compact-heading">
                  <h2>从连接到访问，只需三步</h2>
                  <span class="caption">QUICK START</span>
                </div>
                <div class="quick-step">
                  <span :class="['step-number', { done: connection.connected }]">01</span>
                  <div>
                    <strong>使用 NAS 账号完成认证</strong>
                    <p>应用自动获取服务访问凭据，无需复制 Cookie。</p>
                  </div>
                </div>
                <div class="quick-step">
                  <span :class="['step-number', { done: profile.services.length }]">02</span>
                  <div>
                    <strong>添加服务端口映射</strong>
                    <p>从 NAS 读取映射，或填写已有服务远程地址。</p>
                  </div>
                  <button :disabled="!!busy" @click="section = 'services'">
                    <Icon name="arrow" :size="17" />
                  </button>
                </div>
                <div class="quick-step">
                  <span :class="['step-number', { done: proxy.running }]">03</span>
                  <div>
                    <strong>用 localhost 调用你的服务</strong>
                    <p>API 客户端不需要再次输入 NAS 账号密码。</p>
                  </div>
                </div>
              </section>
            </div>
          </div>
          <section class="card service-preview">
            <div class="compact-heading">
              <div>
                <h2>你的本地服务</h2>
                <p>每个服务使用独立的本地端口，保留原始 API 路径。</p>
              </div>
              <button class="text-button" @click="section = 'services'">
                管理映射 <Icon name="arrow" :size="16" />
              </button>
            </div>
            <div v-if="!profile.services.length" class="empty-inline">
              <div class="empty-icon"><Icon name="server" :size="24" /></div>
              <div>
                <strong>还没有服务映射</strong>
                <p>例如，把 NAS 的 8084 服务映射到本机 18084 端口。</p>
              </div>
              <button
                class="button secondary-button"
                @click="
                  section = 'services';
                  w.showEditor();
                "
              >
                <Icon name="plus" />添加第一个服务
              </button>
            </div>
            <div v-for="service in profile.services" :key="service.id" class="local-service">
              <span :class="['status-dot', { online: proxy.running && service.enabled }]" />
              <div>
                <strong>{{ service.name }}</strong
                ><span>NAS :{{ service.nasPort }}</span>
              </div>
              <code>{{ w.localUrl(service.localPort) }}</code
              ><button
                class="icon-button"
                aria-label="复制本地地址"
                @click="w.copy(w.localUrl(service.localPort))"
              >
                <Icon name="copy" /></button
              ><button
                class="icon-button"
                :disabled="!proxy.running || !service.enabled || !!busy"
                aria-label="打开本地服务"
                @click="w.open(service.localPort)"
              >
                <Icon name="external" />
              </button>
            </div>
          </section>
          <div class="recent-logs">
            <div class="compact-heading">
              <span>最近活动</span
              ><button class="text-button" @click="section = 'logs'">
                查看全部 <Icon name="arrow" :size="14" />
              </button>
            </div>
            <p v-if="!lastLogs.length" class="muted-help">
              等待你的第一次连接。日志不会包含认证凭据。
            </p>
            <div v-for="entry in lastLogs" :key="entry.time + entry.message" class="recent-log">
              <time>{{ time(entry.time) }}</time
              ><span :class="['log-dot', entry.level]" /><span>{{ entry.message }}</span>
            </div>
          </div>
        </template>
        <template v-else-if="section === 'services'">
          <section class="mapping-info">
            <Icon name="info" :size="20" />
            <div>
              <strong>端口不是从子域名“解码”出来的。</strong>
              <p>
                NAS 入口列表的 <code>uri.port</code> 与
                <code>uri.fnDomain</code> 直接给出映射；应用读取它们，不进行端口扫描或猜测。
              </p>
            </div>
          </section>
          <section class="card">
            <div class="compact-heading">
              <div>
                <h2>
                  服务映射 <span class="count-pill">{{ profile.services.length }}</span>
                </h2>
                <p>修改映射前请停止代理。添加后可点击保存登录保存配置。</p>
              </div>
              <div class="button-group">
                <button
                  class="button secondary-button"
                  :disabled="!!busy || !connection.connected"
                  @click="w.discover"
                >
                  <Icon name="refresh" />从 NAS 读取</button
                ><button
                  class="button primary"
                  :disabled="!!busy || proxy.running"
                  @click="w.showEditor()"
                >
                  <Icon name="plus" />添加服务
                </button>
              </div>
            </div>
            <div v-if="!profile.services.length" class="empty-state">
              <Icon name="server" :size="40" />
              <h3>为 NAS 服务创建一个本地入口</h3>
              <p>读取 NAS 的入口列表，或者添加你已能通过 FN Connect 访问的服务地址。</p>
            </div>
            <div v-for="service in profile.services" :key="service.id" class="mapping-row">
              <label class="mapping-toggle"
                ><input
                  v-model="service.enabled"
                  type="checkbox"
                  :disabled="proxy.running || !!busy" /><span
              /></label>
              <div class="mapping-name">
                <strong>{{ service.name }}</strong
                ><span
                  >NAS 端口 {{ service.nasPort }} <Icon name="arrow" :size="12" />本地
                  {{ service.localPort }}</span
                >
              </div>
              <div class="mapping-address">
                <code>{{ w.localUrl(service.localPort) }}</code
                ><span>{{ service.upstream }}</span>
              </div>
              <span
                v-if="probes[service.id]"
                :class="['badge', probes[service.id]?.reachable ? 'success' : 'neutral']"
                >HTTP {{ probes[service.id]?.status }}</span
              >
              <div class="mapping-actions">
                <button
                  class="icon-button"
                  aria-label="测试服务"
                  :disabled="!!busy || !connection.connected"
                  @click="w.probe(service)"
                >
                  <Icon name="link" /></button
                ><button
                  class="icon-button"
                  aria-label="编辑映射"
                  :disabled="proxy.running || !!busy"
                  @click="w.showEditor(service)"
                >
                  <Icon name="edit" /></button
                ><button
                  class="icon-button"
                  aria-label="删除映射"
                  :disabled="proxy.running || !!busy"
                  @click="w.remove(service)"
                >
                  <Icon name="trash" />
                </button>
              </div>
            </div>
          </section>
          <section v-if="discovered.length" class="card discovered-card">
            <div class="compact-heading">
              <div>
                <h2>NAS 返回的服务入口</h2>
                <p>来自已认证的入口列表，非自行推算的子域名。</p>
              </div>
              <span class="badge success">{{ discovered.length }} 个入口</span>
            </div>
            <div v-for="service in discovered" :key="service.id" class="discovered-row">
              <div>
                <strong>{{ service.name }}</strong>
                <p>
                  <code>:{{ service.nasPort }}</code
                  ><Icon name="arrow" :size="14" /><code>{{ service.fnDomain }}</code>
                </p>
              </div>
              <button
                class="button secondary-button"
                :disabled="
                  proxy.running || profile.services.some((s) => s.upstream === service.upstream)
                "
                @click="w.addDiscovered(service)"
              >
                <Icon name="plus" />{{
                  profile.services.some((s) => s.upstream === service.upstream)
                    ? "已添加"
                    : "添加映射"
                }}
              </button>
            </div>
          </section>
          <section v-if="inventory" class="card inventory-card">
            <div class="compact-heading">
              <div>
                <h2>当前账号完整入口清单</h2>
                <p>保留未映射入口，明确区分“已注册”与“实际可达”。</p>
              </div>
              <span class="badge">{{ inventory.totalEntries }} 个可见入口</span>
            </div>
            <div class="inventory-summary">
              <span
                >有远程配对 <strong>{{ inventory.mappedEntries }}</strong></span
              >
              <span
                >独立服务 <strong>{{ inventory.services.length }}</strong></span
              >
              <span
                >无法建立端口代理 <strong>{{ inventory.unmappedEntries }}</strong></span
              >
            </div>
            <p class="inventory-scope"><Icon name="info" :size="16" />{{ inventory.scope }}</p>
            <details class="inventory-details">
              <summary>查看全部入口、端口、子域名及未映射原因</summary>
              <div class="inventory-table-wrap">
                <table class="inventory-table">
                  <thead>
                    <tr>
                      <th>入口</th>
                      <th>NAS 端口</th>
                      <th>服务子域名前缀 / 路径</th>
                      <th>映射状态</th>
                    </tr>
                  </thead>
                  <tbody>
                    <tr v-for="entry in inventory.entries" :key="entry.id">
                      <td>{{ entry.name }}</td>
                      <td>
                        <code>{{ entry.nasPort ?? "—" }}</code>
                      </td>
                      <td>
                        <code>{{ entry.fnDomain ?? "未提供有效子域名" }}</code>
                        <span v-if="entry.path" class="inventory-path">{{ entry.path }}</span>
                      </td>
                      <td>
                        <span :class="['badge', { success: entry.status === 'mapped' }]">{{
                          entry.status === "mapped"
                            ? "已注册，待测试"
                            : entry.status === "no-port"
                              ? "无独立端口"
                              : entry.status === "no-domain"
                                ? "缺少远程子域名"
                                : "不安全的域名"
                        }}</span>
                        <span class="inventory-reason">{{ entry.reason }}</span>
                      </td>
                    </tr>
                    <tr v-if="!inventory.entries.length">
                      <td colspan="4">当前账号没有可见入口。</td>
                    </tr>
                  </tbody>
                </table>
              </div>
            </details>
          </section>
          <section class="card usage-card">
            <div class="compact-heading">
              <h2>代理启动后，如何访问？</h2>
              <span class="caption">NO NAS COOKIE REQUIRED</span>
            </div>
            <p>
              假设 NAS 的 8084 服务映射到本地 18084，浏览器打开以下地址；API 调用保持原路径即可。
            </p>
            <div class="code-line">
              <code>http://127.0.0.1:18084/</code
              ><button
                class="icon-button"
                aria-label="复制示例地址"
                @click="w.copy('http://127.0.0.1:18084/')"
              >
                <Icon name="copy" />
              </button>
            </div>
            <div class="code-block">
              <span>PowerShell / curl / API 客户端</span
              ><code
                >curl.exe http://127.0.0.1:18084/你的API路径<br /># 应用自己的 API Key 或
                Authorization 仍需保留</code
              >
            </div>
            <p class="muted-help">
              代理不是通用 VPN：目前转发 HTTP、流式响应和 WebSocket，不代理 SSH、SMB 或 UDP。
            </p>
          </section>
        </template>
        <template v-else-if="section === 'protocol'">
          <section class="card protocol-card">
            <div class="compact-heading">
              <h2>已查明的认证链路</h2>
              <span class="badge success">基于实际前端代码</span>
            </div>
            <ol class="protocol-steps">
              <li>
                <span>01</span>
                <div>
                  <h3>解析 FN ID，选择远程中继</h3>
                  <p>
                    获取 FN Connect 的可用地址，使用浏览器式入口握手及 Cookie Jar，不自动跳转内网。
                  </p>
                  <code>POST /api/v1/fn/con → HTTPS /login → Cookie: mode</code>
                </div>
              </li>
              <li>
                <span>02</span>
                <div>
                  <h3>RSA + AES 加密登录</h3>
                  <p>
                    通过 WebSocket 取得 RSA 公钥和 si，使用 AES-256-CBC 加密登录消息，并用 RSA 封装
                    AES 密钥。
                  </p>
                  <code>util.crypto.getRSAPub → user.login</code>
                </div>
              </li>
              <li>
                <span>03</span>
                <div>
                  <h3>建立会话，交换服务凭据</h3>
                  <p>
                    兼容前端中的 Ticket / Cookie 会话与 Legacy 分支；后续 RPC 使用会话 Secret 做
                    HMAC-SHA256 签名。
                  </p>
                  <code>appcgi.sac.entry.v1.exchangeEntryToken → entry-token</code>
                </div>
              </li>
              <li>
                <span>04</span>
                <div>
                  <h3>读取端口与子域名映射</h3>
                  <p>从 NAS 入口列表的 uri 读取配对字段，不假设哈希前缀或 -0 能还原端口。</p>
                  <code>uri.port ↔ uri.fnDomain → https://{fnDomain}.{FN ID}.fnos.net</code>
                </div>
              </li>
            </ol>
          </section>
          <div class="security-grid">
            <section class="card">
              <div class="compact-heading">
                <Icon name="shield" />
                <h2>默认安全边界</h2>
              </div>
              <ul class="feature-list">
                <li>监听地址固定为 127.0.0.1，不开放局域网访问。</li>
                <li>校验 Host、Origin，阻止异常跨站请求。</li>
                <li>远程地址限制为当前 NAS 的 HTTPS 服务子域名。</li>
                <li>登录密码只在 Rust 后端和系统凭据管理器中使用。</li>
                <li>不把 entry-token、NAS Cookie 返回给本地客户端。</li>
              </ul>
            </section>
            <section class="card">
              <div class="compact-heading">
                <Icon name="info" />
                <h2>需要了解的限制</h2>
              </div>
              <ul class="feature-list">
                <li>这是非公开、可能变化的前端认证协议。</li>
                <li>Token 有效期和撤销规则由 NAS / FN Connect 决定。</li>
                <li>不同服务仍可能需要自己的登录或 API Key。</li>
                <li>NAS 账号开启二次验证时，重连可能需要新的验证码。</li>
                <li>同一本机地址的不同端口并不隔离浏览器 Cookie。</li>
              </ul>
            </section>
          </div>
        </template>
        <template v-else
          ><section class="card logs-card">
            <div class="compact-heading">
              <div>
                <h2>连接与代理日志</h2>
                <p>最多保留本次运行的 200 条记录，内存保存，不记录请求业务内容。</p>
              </div>
              <span class="badge neutral">{{ logs.length }} 条</span>
            </div>
            <div v-if="!logs.length" class="empty-state">
              <Icon name="terminal" :size="36" />
              <h3>还没有运行记录</h3>
              <p>测试连接或启动代理后，操作结果会出现在这里。</p>
            </div>
            <div
              v-for="(entry, index) in [...logs].reverse()"
              :key="entry.time + ':' + index"
              class="log-row"
            >
              <time>{{ time(entry.time) }}</time
              ><span :class="['log-level', entry.level]">{{ entry.level.toUpperCase() }}</span
              ><span>{{ entry.message }}</span>
            </div>
          </section></template
        >
        <footer class="page-footer">
          <span>FN Proxy · 独立工具，非飞牛官方客户端</span
          ><span><Icon name="shield" :size="13" />Your NAS. Your connection.</span>
        </footer>
      </main>
    </div>
    <Transition name="toast"
      ><div v-if="notice" role="status" :class="['toast', { error: notice.error }]">
        <Icon :name="notice.error ? 'info' : 'check'" /><span>{{ notice.message }}</span>
      </div></Transition
    >
    <div v-if="editing" class="modal-backdrop" @click.self="editing = false">
      <section class="modal" role="dialog" aria-modal="true" aria-labelledby="mapping-title">
        <div class="compact-heading">
          <h2 id="mapping-title">{{ editor.id ? "编辑服务映射" : "添加服务映射" }}</h2>
          <button class="icon-button" aria-label="关闭" @click="editing = false">
            <Icon name="close" />
          </button>
        </div>
        <form @submit.prevent="w.commitEditor">
          <div class="field">
            <label for="service-name">服务名称</label
            ><input
              id="service-name"
              v-model="editor.name"
              placeholder="例如 Aether / API 服务"
              autofocus
            />
          </div>
          <div class="port-fields">
            <div class="field">
              <label for="nas-port">NAS 服务端口</label
              ><input
                id="nas-port"
                v-model.number="editor.nasPort"
                type="number"
                min="1"
                max="65535"
              />
            </div>
            <div class="field">
              <label for="local-port">本地监听端口</label
              ><input
                id="local-port"
                v-model.number="editor.localPort"
                type="number"
                min="1024"
                max="65535"
              />
            </div>
          </div>
          <div class="field">
            <label for="upstream">FN Connect 服务地址</label
            ><input
              id="upstream"
              v-model="editor.upstream"
              placeholder="https://子域名.你的FNID.fnos.net/"
            />
            <p class="field-help">
              使用浏览器中已经能打开的服务子域名。NAS 端口仅是映射信息，不用于猜测域名。
            </p>
          </div>
          <div class="modal-preview">
            <span>本机访问入口</span><code>{{ w.localUrl(editor.localPort) }}</code>
          </div>
          <div class="form-actions">
            <button type="button" class="button secondary-button" @click="editing = false">
              取消</button
            ><button type="submit" class="button primary"><Icon name="check" />保存映射</button>
          </div>
        </form>
      </section>
    </div>
  </div>
</template>
