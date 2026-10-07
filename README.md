# FN Proxy

使用 FN Connect 合法登录会话，将 NAS 上的 HTTP 服务转发到本机。桌面界面使用 **Tauri 2 + Vite + Vue 3 + TypeScript + Reka UI + Tailwind CSS 4**，代码质量工具为 **Oxlint + Oxfmt**。

> 独立工具，非飞牛官方客户端。认证实现来自公开前端代码分析，不是官方稳定 API。它不是通用 VPN，不转发 SSH、SMB、原生 TCP 或 UDP。

## 启动开发

需要 Node.js、Rust stable、Windows C++ 构建工具及 WebView2。项目已通过 `create-tauri-app` 的 Vue TypeScript 模板生成。

```powershell
npm install
npm run desktop:dev
```

仅查看界面（不能登录或启动代理）：

```powershell
npm run dev
# http://127.0.0.1:1420
```

构建安装包：

```powershell
npm run desktop:build
# src-tauri/target/release/bundle/nsis/
```

## 本次实现的验证状态

- 前端：Oxlint、Vue/TypeScript 类型检查、12 项单元测试、Oxfmt 和 Vite 生产构建通过。
- Rust：32 项离线测试通过（另有 1 项真实网络握手测试默认忽略），覆盖加密认证、会话维护、三来源服务盘点与 HTTP/WebSocket 本地代理；Clippy 全 target 检查通过。
- 已完成目标 NAS 的只读 FN ID 解析、HTTPS 入口握手与远程 WebSocket RSA 公钥握手，未发送真实用户名或密码。
- 真实账号自动登录、服务入口列表权限及真实业务 API 尚待用户在桌面应用内验证；模拟测试不替代真实环境验收。

本机调试版可执行文件：`src-tauri/target/debug/fn-proxy.exe`。使用 `tauri build --debug --no-bundle` 构建的版本内嵌界面资源，可以直接启动，不要求 Vite 常驻；源码更新后需要重新构建；如果旧版应用正在运行，请先关闭窗口，再执行 `npm run desktop:dev`。后续 `tauri dev` 会重新构建开发版，此时使用 `npm run desktop:dev` 启动。

## 使用流程

1. 在 **连接** 页点击 **新增连接**，在弹窗中填写 FN ID（不带 `.fnos.net`）、NAS 用户名和密码。账号启用 TOTP 时填写两步验证码；首次绑定请先在 NAS 网页完成。
2. 点击 **保存连接**，自动登录并保存配置；也可先点击 **测试连接**。可选择保存密码和启动时自动连接。密码保存在 Windows 凭据管理器，配置文件不包含密码或 Token。
3. 在 **服务映射** 页选择已保存连接，点击 **新增服务**。在弹窗中手动填写远程根地址、NAS 端口和本地端口，或在 **服务发现** 中点击 **读取服务**、选择有效服务并保存。
4. 服务发现读取桌面入口、Docker 快捷访问注册映射与容器端口；仅明确关联远程域名的 TCP 发布端口可被选择。来源失败不会计为零条，未匹配、歧义、UDP 和仅暴露的容器端口不会自动创建映射。
5. 在 **首页** 查看连接、服务及代理总览，点击对应连接的 **开启代理**。每个启用服务仅监听 `127.0.0.1`。运行中新增服务会自动保存并增量启动监听，不重启已有代理或断开已有 WebSocket；新增失败保留原配置和已有监听。
6. 已有映射的编辑、删除和启用状态调整需先停止该连接的代理，其他连接不受影响。

### 本机如何访问 NAS 服务

假设服务映射如下：

| NAS 服务端口 | FN Connect 服务地址                   | 本地端口 |
| ------------ | ------------------------------------- | -------- |
| 8084         | `https://服务前缀.你的FNID.fnos.net/` | 8084     |

浏览器打开：

```text
http://127.0.0.1:8084/
```

API 客户端的 Base URL 设置为 `http://127.0.0.1:8084`，保持原 API 路径、请求方法、查询参数、请求体和应用本身的认证。例如：

```powershell
curl.exe http://127.0.0.1:8084/你的API路径
# 如果目标应用要求 API Key，继续按该应用的规则发送 Authorization 等 Header。
```

客户端不需要 NAS 账号、Cookie 或 `entry-token`。代理在 Rust 后端为远程请求附加 `entry-token`，不覆盖目标应用自己的 `Authorization`。

WebSocket 客户端使用同一个本地端口，例如 `ws://127.0.0.1:8084/实际WS路径`。HTTP 请求体与响应体以流方式转发，支持下载、上传及 SSE。保持应用运行；关闭应用或停止代理后，本地地址不再可用。

## 自动登录实现

详见 [认证与端口映射分析](docs/protocol.md)。

```text
FN ID 解析
 → HTTPS 入口握手 + Cookie Jar（mode Cookie）
 → WSS /websocket?type=main
 → util.crypto.getRSAPub
 → RSA 封装随机 AES 密钥 + AES-256-CBC 加密 user.login
 → 必要时 user.2fa.loginVerify
 → Ticket / Cookie 或 Legacy 会话
 → 会话 Secret 的 HMAC-SHA256 RPC 签名
 → appcgi.sac.entry.v1.exchangeEntryToken
 → 本地代理在内存中使用 entry-token
```

通过首页或连接页的「新增连接」配置多个 NAS / 账号，连接页以列表展示已保存连接，服务映射页可通过下拉列表切换。每个连接独立维护登录会话、服务映射、代理监听和自动重连；切换、重连或停止一个连接不会停止其他连接。保存登录仅更新当前连接，启动时分别自动登录已勾选的连接。旧版单连接配置会兼容读取，保存后转换为多连接配置。

添加服务映射时，本地端口默认等于 NAS 服务端口（Docker 使用 NAS 发布端口，而不是容器内部端口）。如果本地端口已被其他连接或映射使用，会建议下一个可用端口；手动添加时修改 NAS 端口也会更新默认值，手动指定的本地端口不会被覆盖。其他程序占用端口或系统禁止绑定时，启动代理或运行中新增服务会报错，需要另选端口；已运行的服务不会因此停止。

连接期间每 15 秒保持 WebSocket 活性，每 15 分钟尝试交换服务凭据。认证连接丢失时使用后端内存中的凭据自动重新登录一次；失败后停止自动重试，防止错误密码导致账号锁定。新 TOTP 挑战需要人工输入，不复用旧验证码。

这些定时行为是本工具的维护策略，**不是对 FN Connect Token 有效期的声明**。NAS 与中继服务器仍决定有效期、权限和撤销规则。可手动更新服务凭据。代理不会自动重放已经提交的 POST 等写请求。

## 子域名与端口映射

子域名的哈希前缀和 `-0` 后缀不是已验证的端口编码。实际 NAS 前端的入口 URL 构造读取服务器返回的：

```json
{
  "uri": {
    "port": "8084",
    "fnDomain": "服务前缀",
    "path": "/"
  }
}
```

远程入口按 `https://{fnDomain}.{NAS 主域名}{path}` 拼接。应用分别调用 `appcgi.sac.entry.v1.getEntryList`（桌面）及 `appcgi.sac.entry.v1.dockerList`（Docker 快捷访问）读取配对，不猜测域名、不进行端口扫描，也不在 NAS 上创建或修改入口。当前清单按登录账号权限显示，不承诺涵盖 NAS 所有监听端口；详见 [完整端口/子域名发现调查](docs/service-discovery.md)。

没有 `fnDomain` 的独立端口，不会仅因填写端口而获得 FN Connect 转发能力。需要 NAS 先提供可访问的服务入口，或使用你已经确认的服务远程地址。Docker 快捷访问匹配的是发布到 NAS 的宿主机端口，而不是容器内端口；它先用 `appID` 匹配容器 ID 前缀，再用 `uri.port` 匹配宿主机端口。详见 [Docker 快捷访问调查](docs/docker-discovery.md)。

另外通过只读 `appcgi.dockermgr.containerList`（`all:true`）盘点容器端口，按容器 ID 前缀和宿主机端口关联 Docker 注册映射。分段响应必须等到成功终态才展示；总等待 45 秒、分段空闲 10 秒，超时、响应格式变化或超过安全上限均报告失败，不展示残缺结果。只保留容器 ID、名称、状态和端口元数据，不请求容器详情、日志或环境变量。Host 网络模式、NAS 非 Docker 服务以及原生 UDP 不在该盘点/代理的完整覆盖范围内。

`nasPort` 在手动映射中是说明字段，真正的远程目标由已验证的服务域名决定。

## 安全边界与限制

- 固定绑定 IPv4 loopback；不提供 `0.0.0.0`、公网监听或任意上游 URL。
- 验证 Host、Origin 和浏览器跨站请求，限制远程地址为当前 FN ID 的 HTTPS 服务子域名。
- 保持正常 TLS 证书验证，不使用忽略证书检查的选项。
- 密码只通过本机 Tauri IPC 进入 Rust；不写入前端存储、配置 JSON 或日志。
- Windows 密码使用原生 Credential Manager。非 Windows 平台当前不提供密码持久化。
- Token 与签名 Secret 在后端内存中维护；使用零化容器，日志仅保留状态与错误码。
- 代理不向客户端转发 NAS/网关认证 Cookie；仅按本地访问需要重写应用自己的 Cookie 与同源 Location。
- 本机其他进程也能访问已经启用的代理端口；本工具的本地入口没有额外 API Key。不要在不可信的多人共用机器上使用敏感服务。
- 不同本地端口共享 `127.0.0.1` 的浏览器 Cookie 域，可能导致不同应用的同名 Cookie 冲突。API Token 客户端不依赖浏览器 Cookie 时更适合此模式。
- 不改写 HTML/JS 中硬编码的远程绝对地址；依赖原始域名的 OAuth 回调、CORS 和绝对 URL 可能需要目标应用配置配合。
- 二次验证码首次绑定、验证码/CAPTCHA 与其他新增认证流程不会被绕过。
- “删除保存密码”只删除本机持久凭据，“断开连接”只关闭本机连接与代理。二者均不承诺撤销服务端已经签发的 Token。

## 检查与测试

```powershell
npm run check
npm run test:rust
npm run rust:check
```

Rust 模拟 NAS 测试覆盖真实 RSA/AES 请求加密、HMAC RPC、入口 Cookie 握手、Ticket/Legacy 登录、错误密码、服务映射和凭据交换。代理端到端测试还覆盖 HTTP 方法/查询参数/请求体、应用 Authorization、Cookie 隔离、重定向、WebSocket 文本与二进制双向转发、子协议、Host/Origin 防护、端口冲突的全部回滚与停止释放端口。另有只读公共握手测试，默认忽略，不会发送用户名或密码：

```powershell
$env:FN_PROXY_TEST_ID = '你的FNID'
cargo test --manifest-path src-tauri/Cargo.toml live_fn_connect_handshake -- --ignored
Remove-Item Env:FN_PROXY_TEST_ID
```

实际账号的端到端登录与实际服务 API 仍需要你在桌面应用内验证；测试不会读取浏览器配置、提取现有登录 Cookie，或把聊天里曾出现的凭据写入项目。

## 目录

```text
src/                      Vue 界面、状态管理与前端测试
src-tauri/src/auth.rs     NAS 加密认证、RPC、服务映射读取
src-tauri/src/resolver.rs FN ID 解析与远程入口 HTTP 客户端
src-tauri/src/inventory.rs 完整可见入口清单、映射分类与解析测试
src-tauri/src/docker.rs   容器分段列表、白名单端口元数据与域名关联
src-tauri/src/proxy.rs    loopback HTTP / SSE / WebSocket 代理
src-tauri/src/commands.rs 桌面命令、会话维护和代理生命周期
src-tauri/src/storage.rs  无秘密的配置与 Windows 凭据管理器
src-tauri/src/auth_tests.rs 模拟 NAS 协议测试与显式公共握手测试
src-tauri/src/proxy_tests.rs 本地代理端到端与生命周期测试
docs/protocol.md          来源、协议证据、已知限制
```
