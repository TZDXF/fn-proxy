export interface ServiceRoute {
  id: string;
  name: string;
  nasPort: number;
  localPort: number;
  upstream: string;
  enabled: boolean;
}
export interface Profile {
  id: string;
  fnId: string;
  username: string;
  remember: boolean;
  autoConnect: boolean;
  services: ServiceRoute[];
}
export interface ConnectionInfo {
  connected: boolean;
  fnId: string;
  username: string;
  relay: string;
  authMode: string;
  message: string;
}
export interface ListenerInfo {
  name: string;
  localUrl: string;
  upstream: string;
  nasPort: number;
}
export interface ProxyStatus {
  running: boolean;
  listeners: ListenerInfo[];
  requests: number;
}
export interface AppSnapshot {
  connections: ConnectionSnapshot[];
}
export interface ConnectionSnapshot {
  id: string;
  connection: ConnectionInfo;
  proxy: ProxyStatus;
}
export interface DiscoveredService {
  id: string;
  name: string;
  nasPort: number;
  upstream: string;
  fnDomain: string;
  source: string;
}
export interface InventoryEntry {
  id: string;
  source: string;
  appId: string | null;
  name: string;
  nasPort: number | null;
  fnDomain: string | null;
  upstream: string | null;
  path: string | null;
  status: "mapped" | "no-port" | "no-domain" | "invalid-domain";
  reason: string;
}
export interface InventorySource {
  id: string;
  name: string;
  status: "ok" | "unavailable";
  count: number | null;
  message: string;
}
export interface DockerPortRow {
  id: string;
  containerId: string;
  containerName: string;
  state: string;
  protocol: string;
  hostIp: string | null;
  nasPort: number | null;
  containerPort: number | null;
  upstream: string | null;
  fnDomain: string | null;
  status:
    | "mapped"
    | "no-domain"
    | "registry-unavailable"
    | "not-published"
    | "unsupported-protocol"
    | "ambiguous";
  reason: string;
}
export interface DockerPortInventory {
  containers: number;
  publishedPorts: number;
  mappedPorts: number;
  unmappedPorts: number;
  unconfirmedPorts: number;
  registryAvailable: boolean;
  rows: DockerPortRow[];
  scope: string;
}
export interface ServiceInventory {
  docker: DockerPortInventory | null;
  sources: InventorySource[];
  totalEntries: number;
  mappedEntries: number;
  unmappedEntries: number;
  services: DiscoveredService[];
  entries: InventoryEntry[];
  scope: string;
}
export interface LogEntry {
  time: number;
  level: "info" | "success" | "warn" | "error";
  message: string;
}
export interface RouteProbe {
  status: number;
  reachable: boolean;
  message: string;
}
export function normalizeFnId(input: string): string {
  const id = input.trim().toLowerCase();
  if (!/^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/.test(id) || id.includes("--")) {
    throw new Error("FN ID 仅支持字母、数字和单个连字符");
  }
  return id;
}
export function validateService(route: ServiceRoute, fnId: string): void {
  if (!route.name.trim()) throw new Error("请输入服务名称");
  for (const port of [route.nasPort, route.localPort]) {
    if (!Number.isInteger(port) || port < 1 || port > 65535)
      throw new Error("端口需为 1–65535 的整数");
  }
  let url: URL;
  try {
    url = new URL(route.upstream);
  } catch {
    throw new Error("请输入完整的 HTTPS 服务地址");
  }
  if (
    url.protocol !== "https:" ||
    !url.hostname.endsWith(`.${normalizeFnId(fnId)}.fnos.net`) ||
    url.username ||
    url.password ||
    url.search ||
    url.hash ||
    (url.port && url.port !== "443") ||
    url.pathname !== "/"
  ) {
    throw new Error("服务地址必须是当前 NAS 的 FN Connect 子域名根地址，且不含凭据或查询参数");
  }
}
export function suggestedLocalPort(nasPort: number, occupied: number[] = []): number {
  if (!Number.isInteger(nasPort) || nasPort < 1 || nasPort > 65535)
    throw new Error("端口需为 1–65535 的整数");
  const used = new Set(occupied);
  for (let offset = 0; offset < 65535; offset += 1) {
    const port = ((nasPort - 1 + offset) % 65535) + 1;
    if (!used.has(port)) return port;
  }
  throw new Error("没有可建议的本地端口");
}
export function localUrl(port: number): string {
  return `http://127.0.0.1:${port}/`;
}

export function dockerPortService(row: DockerPortRow): DiscoveredService {
  if (
    row.status !== "mapped" ||
    row.protocol !== "tcp" ||
    !row.nasPort ||
    !row.upstream ||
    !row.fnDomain
  ) {
    throw new Error("该记录尚未取得明确的 TCP 端口与远程域名关联，不能创建代理映射");
  }
  return {
    id: row.id,
    name: `${row.containerName}:${row.nasPort}`,
    nasPort: row.nasPort,
    upstream: row.upstream,
    fnDomain: row.fnDomain,
    source: "Docker 容器端口与快捷访问映射关联",
  };
}
export function dockerPortStatus(status: DockerPortRow["status"]): string {
  return {
    mapped: "已注册，待测试",
    "no-domain": "未匹配到域名",
    "registry-unavailable": "映射来源未知",
    "not-published": "无宿主机端口",
    "unsupported-protocol": "协议不支持",
    ambiguous: "关联存在歧义",
  }[status];
}
