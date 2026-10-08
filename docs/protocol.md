# FN Connect 认证与服务映射证据

> 2026-10-08 更新：服务发现现仅请求 `getEntryList` 与 `dockerList`，不再请求 `containerList` 或展示容器端口盘点。以下容器分段协议描述保留为历史调查记录，不代表当前运行行为。可选服务按 NAS 端口与远程地址去重，同时标记所有匹配来源。来源计数是原始入口记录数；缺少有效端口/远程域名的记录不会进入可选服务列表。

分析日期：2026-10-06。来源为用户拥有的 NAS 公开前端资源、FN Connect 公开入口脚本，以及用户授权的只读服务请求对照测试。本文件不保存真实 Cookie、密码、Token 或业务数据。

## 1. 远程入口握手

普通非浏览器请求与浏览器式请求可能收到不同重定向。使用浏览器 User-Agent、导航 Accept / Sec-Fetch Header 并维护 Cookie Jar 后，观察到 NAS 入口设置名为 `mode` 的 HttpOnly Cookie，随后同地址重定向进入公开登录页面。

这是入口握手，不等于 NAS 用户登录。实现保留正常证书验证，只跟随同主机重定向，并设定次数上限。

## 2. WebSocket 地址及加密

公开资源 `constants-CfymwMO5.js` 构造：

```javascript
const socket = isHttps ? `wss://${location.host}/websocket` : `ws://${location.host}/websocket`;
```

主连接附加 `?type=main`。前端先发 `util.crypto.getRSAPub`，取得 `pub`、`si`。

公开核心脚本使用随机 32 字节字母数字 AES 密钥与 16 字节 IV。当前观察到的 JSEncrypt 分支使用 RSA PKCS#1 v1.5 包装密钥；AES 使用 CBC / PKCS#7。加密外层为：

```json
{ "req": "encrypted", "iv": "BASE64", "rsa": "BASE64", "aes": "BASE64" }
```

`si` 和 `reqid` 位于 AES 内部请求中。该资源同时存在 RSA-OAEP 的另一个代码分支，但所观察构建的选择常量关闭该分支；当前应用实现实际启用的 PKCS#1 v1.5 分支，不冒称支持所有未来版本。

`useTfaLogin-hvWe7ynZ.js` 显示登录参数：

```text
user, password, stay, deviceType, deviceName, did
```

新会话分支由辅助函数附加 `ver: 2`。登录结果可能包含 `ticket`、`token`、`longToken`、加密的 `secret` 及二次验证状态。实现不会保存或输出这些原始响应。

## 3. 会话和后续 RPC

前端存在两类分支：

- Legacy：前端维护 `fnos-token` / `fnos-long-token`。
- Ticket / Cookie：`POST /app/ticket` 提交票据；`GET /app/token` 检查会话。

`secret` 通过本次登录使用的 AES key / IV 解密。前端将明文字节以 Base64 存储，后续签名时 Base64 解码为 HMAC 密钥。

后续常规 RPC 的传输文本为：

```text
Base64(HMAC-SHA256(secretBytes, exactJsonText)) + exactJsonText
```

签名长度为 44 个 Base64 字符。服务端响应按 JSON 读取，使用 `reqid` 匹配请求。通知、WebSocket Ping / Pong 与其他请求的响应不应误认成当前 RPC 结果。

## 4. entry-token 获取和域范围

NAS 桌面中的调用逻辑：

```javascript
const request = { data: {} };
const previous = getCookie("entry-token");
if (previous) request.data = { token: previous };
const result = await api.sac.exchangeEntryToken(request);
if (result.data.token) {
  setCookie("entry-token", result.data.token, {
    domain: "." + location.host,
  });
}
```

方法名对应 `appcgi.sac.entry.v1.exchangeEntryToken`。Domain 覆盖 NAS 主域名及其服务子域名；不能据此断定服务端授权粒度、有效期或 IP 绑定规则。

对一个用户已确认的服务根地址，实际只读对照结果：无 Cookie 返回 FN Connect 403 提示；只携带有效 `entry-token` 返回目标服务 200 页面。该结果验证普通 HTTP 客户端可以复用该凭据，不证明所有 API 或所有网络环境都具有相同行为。

## 5. 子域名和端口的映射

核心脚本的 URL 构造逻辑：

```javascript
if (isFnConnect && uri.fnDomain && uri.port) {
  return `https://${uri.fnDomain}.${location.host}${uri.path}`;
}
```

桌面入口列表来自 `appcgi.sac.entry.v1.getEntryList`；Docker 快捷访问另从 `appcgi.sac.entry.v1.dockerList` 取得注册映射。两者都使用 `uri.port` 与 `uri.fnDomain`。桌面条目通常形如：

```json
{
  "entryKey": "应用入口ID",
  "title": "应用名称",
  "uri": { "port": "8084", "fnDomain": "服务前缀", "path": "/" }
}
```

**结论：应用应读取服务端提供的 `uri.port ↔ uri.fnDomain` 配对。** 当前没有足够证据证明 `fnDomain` 的哈希前缀或数字后缀可自行计算。新端口能否被中继访问由 NAS 入口注册及 FN Connect 规则决定，不由本机代理填写一个端口决定。

当前版本的完整入口清单保留所有返回条目，并明确显示无端口、无远程前缀或不安全域名的原因；只有有效端口与远程前缀配对才能添加为代理映射。手动添加域名需要属于当前 NAS。读取接口是只读的，不调用应用安装、入口编辑、域名注册或系统设置接口。

## 6. 未验证事项

- 新旧 fnOS 版本的所有认证差异，包括 RSA-OAEP 分支。
- entry-token 的 TTL、跨出口/IP 可用性、NAS 退出登录后的撤销行为。
- `dockerList` / `containerList` 在真实账号下的响应结构与权限，以及无 fnDomain 或未发布端口的正式注册机制。
- 真实账号的当前配置、二次验证策略和具体业务 API 的认证。
- OAuth、硬编码绝对地址、特殊 Cookie 与远程服务特有的兼容性。

## 官方参考

- https://developer.fnnas.com/docs/core-concepts/app-entry/
- https://developer.fnnas.com/docs/core-concepts/gateway-registration/
- https://developer.fnnas.com/docs/core-concepts/index-cgi/
- https://fnnas.com/fn-connect

官方 CGI / 统一网关入口和本文的服务子域名代理是相关但不同的访问模型；不能把统一网关 Header 当成 entry-token 的替代品，也不能将 NAS 本地开放 API 的 TRIM_API_TOKEN 当成远程登录凭据。

## 7. 完整清单调查补充

2026-10-06 继续检查公开前端后，确认 `getUserDesktop` 是桌面布局/分组列表，不能替代 `getEntryList` 枚举可见入口；`appcgi.netsvr.domain.list` 在 FN ID 设置中用于域名后缀选择，也不是服务端口映射表。没有确认能够返回 NAS 所有监听端口与远程域名的全局接口。

应用新增 `get_service_inventory` 桌面命令：调用已确认的只读 `getEntryList`、Docker 前端使用的 `dockerList` 与 `containerList`，返回当前账号可见注册入口、容器端口交叉关联、映射/未映射原因及独立服务去重结果。详细来源、候选接口与覆盖范围见 [完整端口/子域名发现调查](service-discovery.md)。

## 8. Docker 快捷访问数据源

用户提供 Docker 快捷访问线索后，进一步在 NAS 公开桌面代码中定位 iframe 根路径 `/apps/docker/`，并取得其独立脚本。脚本调用 `appcgi.sac.entry.v1.dockerList`（无 `data` 参数），读取 `data.list`。快捷访问先按 `container.id.startsWith(entry.appID)` 关联容器，再按容器宿主机端口与 `uri.port` 相等选取 `uri.fnDomain`。这补充了桌面入口列表之外的 Docker 注册映射，不代表全部 NAS 监听端口。详见 [Docker 快捷访问调查](docker-discovery.md)。

## 9. 容器端口盘点与成功终态

`appcgi.dockermgr.containerList` 使用顶层 `all:true`，不附加 `data`。与普通单响应 RPC 不同，它通过多条同 `reqid` 的 `rsp` 数组分段返回。应用独立汇总这些包、按容器 ID 合并端口绑定，直到 `result:"succ"`（兼容 `suc`）才允许展示。`doing` / 无 result 不视为完成；错误状态、断线、总超时、分段空闲或超过安全上限均报告失败，不能返回部分结果冒充全部。

读取只保留容器 ID、名称、状态和端口元数据，不获取详情/环境变量/日志。域名关联必须同时匹配容器前缀和宿主机端口；Docker 注册来源失败时关联为未知，多个匹配域名或容器时为歧义。没有匹配的域名不会自行生成。UDP、仅容器暴露端口、host 网络模式监听端口与 NAS 非 Docker 服务不等价于可经 FN Connect 代理的 HTTP 入口。参数与分段行为来自公开 Docker 前端，真实账号权限与版本结构尚待验收。详见 [Docker 快捷访问调查](docker-discovery.md)。
