export interface ServiceRoute {
  id: string;
  name: string;
  nasPort: number;
  localPort: number;
  upstream: string;
  enabled: boolean;
}
export interface Profile {
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
  name: string;
  nasPort: number | null;
  fnDomain: string | null;
  upstream: string | null;
  path: string | null;
  status: "mapped" | "no-port" | "no-domain" | "invalid-domain";
  reason: string;
}
export interface ServiceInventory {
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
  if (route.localPort < 1024) throw new Error("本地端口必须大于等于 1024");
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
  let port = nasPort > 0 && nasPort < 10000 ? 10000 + nasPort : 18080;
  while (occupied.includes(port) && port < 65535) port += 1;
  if (occupied.includes(port)) throw new Error("没有可建议的本地端口");
  return port;
}
export function localUrl(port: number): string {
  return `http://127.0.0.1:${port}/`;
}
