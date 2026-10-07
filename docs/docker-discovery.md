# Docker 快捷访问端口与 FN Connect 子域名

分析日期：2026-10-06。用户指出 Docker 应用的“快捷访问”可显示部署服务的端口与远程域名，本轮沿独立 Docker 前端验证了数据来源，纠正了仅分析 NAS 主站脚本得出的不完整判断。

## 已取得的公开证据

- NAS 主站公开脚本为 `trim.docker` 配置 iframe 根路径 `/apps/docker/`。
- 匿名访问该根路径并维护正常远程入口 Cookie 握手后，取得标题为 Docker 的 HTML。
- HTML 引用 `/apps/docker/assets/index-C3r93lML.js`，本轮取得 JavaScript 638,669 字节。
- 脚本 SHA-256：`3de7ac8916fecc9d410445ce1870261df5e63938c12be09afaeb478841811555`。
- 该脚本明确调用 `appcgi.sac.entry.v1.dockerList`，并读取响应的 `data.list`；此调用确实存在于独立 Docker 前端，不能因为 NAS 主站的适配器未导出就当作无效或遗留定义。

本轮只读了公开静态资源，没有读取浏览器 Cookie、获取真实密码或向 NAS 发送真实账号登录。新增认证后调用已在模拟 NAS 中验证，实际账号响应仍须在应用内读取。

## 实际请求与结构

Docker 布局在 FN Connect 模式下加载注册映射，前端提交的参数对象为：

```json
{ "req": "appcgi.sac.entry.v1.dockerList" }
```

此方法的前端调用不附加 `data`；最终 WebSocket 传输仍需要正常 NAS 会话与签名，不能直接把未签名 JSON 当作可匿名调用的 API。

前端会读取这些字段：

```text
data.list[].appID
 data.list[].uri.port
 data.list[].uri.fnDomain
```

其中 `appID` 用于匹配容器 ID 的前缀，`uri.port` 是发布到 NAS 的宿主机端口。`title`、入口路径等附加字段的真实返回情况尚未读取验证，解析器不会假定一定存在标题；没有标题时使用“Docker 服务 :端口”的名称。

## 快捷访问的关联算法

用等价伪代码描述，不猜测服务端如何生成域名：

```text
针对当前容器：
  找到 appID 是当前容器 ID 前缀的注册条目
  枚举当前容器 ports 中的 TCP 端口
  用 publicPort（宿主机端口）匹配条目的 uri.port
  取出匹配条目的 uri.fnDomain
  若存在远程前缀：拼接前缀 + NAS 主域名，使用当前页面协议访问
  否则：回退到 NAS 主机名 + publicPort
```

注意：前端本地访问回退不等于通过中继开放了那个端口。在外网环境缺少 `fnDomain` 时，不应该用主域名加端口猜测访问。

快捷访问展示 `publicPort:privatePort`，但关联远程域名时只比较 `publicPort`。例如宿主机 8084 映射到容器 80，查询域名的匹配值是 8084，不是 80。

菜单过滤 TCP 不证明所有端口都是 HTTP 服务；本工具只进行 HTTP/WebSocket 代理，不因此获得原生 TCP/UDP 穿透能力。

## Docker 已发布端口与远程映射是两个来源

独立 Docker 前端还存在以下读取逻辑：

- `appcgi.dockermgr.containerList`，参数包含 `all:true`，多条响应通过 `rsp` 聚合，收到成功终态后去重。
- 容器条目中 `ports` 的 `publicPort` / `privatePort` / `type`，用于快捷访问端口菜单。
- 容器详情也能从 `hostConfig.portBindings` 提取 TCP 宿主机与容器端口。

当前应用已接入只读容器列表，正确汇总分段响应，再与 `dockerList` 按容器前缀与宿主机端口关联；不能仅按全局端口号关联不同容器，更不能把第一个分段当作完整列表。本工具不请求容器详情、环境变量或日志，也不修改容器。对列表响应采用字段白名单，只保留 `id`、`names`、`state`、`ports`；未知字段不会存盘、写入日志或传到界面。

## 容器列表的分段协议

```json
{ "req": "appcgi.dockermgr.containerList", "all": true }
```

与 Docker 公开前端一致，不包装 `data`。每个包先匹配 `reqid`，再读取 `rsp` 数组。`doing` 或没有 `result` 的包可以是进度；只有 `result:"succ"`（兼容 `suc`）才允许展示完整结果。终态包里的记录也要汇总。同一容器 ID 的端口绑定合并去重，不因重复分段丢失端口；`fail`、`cancel`、非零 `errno`、未知结果状态及错误结构明确失败。

总等待上限 45 秒，单次等待空闲 10 秒；单文本帧最多 2 MiB、文本包最多 1,024 个、累计容器记录最多 10,000 条、累计端口绑定最多 20,000 条。触及上限、断线、超时或结构变化时不返回分段残留，不把截断结果冒充完整列表。支持 WebSocket Ping/Pong，非本次 `reqid` 的通知不作为容器记录。

这些是本工具的保护策略，不是服务端协议承诺；真实版本结构仍需在应用内验证。列表不提供 host 网络模式内全部监听端口，也不能覆盖 NAS 非 Docker 服务。

## 当前实现

“从 NAS 读取”依次通过已登录的 RPC 连接查询：

1. `getEntryList`：桌面/应用入口。
2. `dockerList`：Docker 快捷访问注册映射。
3. `containerList`：容器及已发布/仅暴露端口的元数据。

三个来源分别标注成功/失败及计数，再呈现两个独立集合：

- **注册入口清单**保留桌面和 Docker 的全部记录、来源与未映射原因；可配置服务按宿主机端口 + 远程根地址去重。
- **Docker 容器端口盘点**显示容器名称/状态、宿主机 IP、宿主机→容器端口、协议及对应远程域名。
- 一个来源失败不丢弃其他成功来源；失败计数为未知，不是 0。三者均失败才返回整体错误。
- 容器盘点中途失败不展示部分容器；已取得的注册映射仍保留。
- Docker 注册来源不可读取时，容器关联状态为“映射来源未知”，不能断言没有域名。
- 未匹配域名、同一前缀匹配多个容器、或同一容器端口对应多个域名时不自动选择目标。
- UDP/未知协议、无宿主机端口的记录仅展示，不创建代理。只有明确关联的 TCP 行可添加映射；TCP 仍不保证目标是 HTTP 服务。
- 上游仍须满足当前 FN ID 的 HTTPS 域名限制；不扫描端口、不创建入口、不猜测域名。

离线测试覆盖分段汇总、重复绑定、成功终态要求、错误 reqid、控制 Ping、错误结构、断线/超时/超限、来源权限失败、前缀歧义，以及敏感字段不进入报告。实际账号当前的容器数量、注册映射、权限和 HTTP 可达性仍待用户在应用中登录并点击“从 NAS 读取”验证，模拟测试不等于真实 NAS 验收。

## 官方文档交叉核对

Context7 可查到飞牛 Docker/资源文档中的 Compose 发布端口示例与 manifest 的 `service_port`，但没有提供这个内部快捷访问 RPC 的稳定 API 文档。本记录的内部方法和关联规则来自 NAS 当前公开前端，具有版本相关性。

- https://developer.fnnas.com/docs/core-concepts/resource
- https://developer.fnnas.com/docs/core-concepts/docker
