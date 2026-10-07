# Repository Guidelines

## 项目结构与模块组织

FN Proxy 是基于 Vue 3、TypeScript 与 Tauri 2 的 NAS 认证反向代理桌面应用。

- `src/components/` 存放界面组件，`ui/` 存放通用控件；`src/lib/` 管理工作区状态、类型和偏好设置。
- `src/locales/` 包含中英文翻译；`src/tests/` 存放前端测试。
- `src-tauri/src/` 包含 Rust 后端：`auth.rs` 处理认证，`proxy.rs` 处理代理，`commands.rs` 管理 IPC 与生命周期，`storage.rs` 管理配置与凭据，`desktop.rs` 管理托盘和窗口。
- `public/` 存放静态资源，`src-tauri/icons/` 存放原生图标；`docs/` 记录协议与服务发现。勿提交 `dist/`、`src-tauri/target/` 或 `.scaffold/`。

## 构建、测试与开发命令

桌面开发需要 Node.js、Rust stable、Windows C++ 构建工具及 WebView2。在仓库根目录执行：

- `npm ci`：按锁文件安装前端依赖。
- `npm run desktop:dev`：启动完整桌面开发环境；`npm run dev` 仅启动前端，不能验证原生功能。
- `npm run build`：检查前端类型并生成静态产物；`npm run desktop:build`：构建桌面安装包。
- `npm run check`：运行前端 lint、类型检查、测试与格式检查。
- `npm run test:rust`：运行 Rust 测试；`npm run rust:check`：检查 Rust 格式并运行 Clippy，警告视为错误。
- `npm run format`：格式化前端文件；`cargo fmt --manifest-path src-tauri/Cargo.toml`：格式化 Rust。

## 编码风格与命名约定

前端遵循现有两空格缩进、双引号、分号及 100 字符行宽，使用 Oxfmt、Oxlint 和严格 TypeScript 检查。Vue 组件采用 PascalCase，如 `UiButton.vue`；变量和函数采用 camelCase。Rust 使用四空格缩进、snake_case 函数与模块、PascalCase 类型，以 rustfmt 为准。新增界面文案必须同步两份语言资源的键及插值参数。

## 测试规范

前端使用 Vitest，测试命名为 `src/tests/*.test.ts`，通过 `npm run test` 执行。Rust 使用内置测试机制，包含模块内测试及 `*_tests.rs` 文件。目前没有配置覆盖率门槛；行为变更应补充回归测试，重点覆盖认证失败、Cookie 隔离、HTTP/WebSocket 转发、端口冲突和停止释放资源。优先使用本地模拟服务；默认忽略的真实网络握手测试仅在显式配置后运行。窗口、托盘和局域网功能另需桌面手动验证。

## 提交与 Pull Request 规范

历史提交采用 `feat: ...`、`fix: ...` 格式，使用简短英文祈使句描述单一改动。PR 应说明目的、行为变化、关联问题及实际执行的验证命令；界面修改附截图，代理或认证修改说明安全影响。提交前运行前端检查、Rust 测试与 Rust 检查；依赖变更同步对应锁文件。

## 安全与配置

密码只能通过本机 IPC 进入 Rust，持久化使用 Windows 凭据管理器；不得写入前端存储、配置、日志或测试夹具。勿提交 `.env`、HAR、真实凭据或 Token。保持 TLS、Host、Origin 和上游域名校验；默认仅监听 `127.0.0.1`。局域网监听必须显式开启，并仅用于可信网络。
