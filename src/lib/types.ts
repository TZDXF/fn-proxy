import { i18n } from "./i18n";
import type { BackendText } from "./text";

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
  message: BackendText;
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
  services: ServiceRoute[];
  connection: ConnectionInfo;
  proxy: ProxyStatus;
}
export interface DiscoveredService {
  id: string;
  name: string;
  nasPort: number;
  upstream: string;
  fnDomain: string;
  source: BackendText;
}
export interface InventoryEntry {
  id: string;
  source: BackendText;
  appId: string | null;
  name: string;
  nasPort: number | null;
  fnDomain: string | null;
  upstream: string | null;
  path: string | null;
  status: "mapped" | "no-port" | "no-domain" | "invalid-domain";
  reason: BackendText;
}
export interface InventorySource {
  id: string;
  name: BackendText;
  status: "ok" | "unavailable";
  count: number | null;
  message: BackendText;
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
  reason: BackendText;
}
export interface DockerPortInventory {
  containers: number;
  publishedPorts: number;
  mappedPorts: number;
  unmappedPorts: number;
  unconfirmedPorts: number;
  registryAvailable: boolean;
  rows: DockerPortRow[];
  scope: BackendText;
}
export interface ServiceInventory {
  docker: DockerPortInventory | null;
  sources: InventorySource[];
  totalEntries: number;
  mappedEntries: number;
  unmappedEntries: number;
  services: DiscoveredService[];
  entries: InventoryEntry[];
  scope: BackendText;
}
export interface LogEntry {
  time: number;
  level: "info" | "success" | "warn" | "error";
  label: string;
  message: BackendText;
}
export interface RouteProbe {
  status: number;
  reachable: boolean;
  message: BackendText;
}
export function normalizeFnId(input: string): string {
  const id = input.trim().toLowerCase();
  if (!/^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/.test(id) || id.includes("--")) {
    throw new Error(i18n.global.t("validation.fnId"));
  }
  return id;
}
export function validateService(route: ServiceRoute, fnId: string): void {
  if (!route.name.trim()) throw new Error(i18n.global.t("validation.serviceName"));
  for (const port of [route.nasPort, route.localPort]) {
    if (!Number.isInteger(port) || port < 1 || port > 65535)
      throw new Error(i18n.global.t("validation.port"));
  }
  let url: URL;
  try {
    url = new URL(route.upstream);
  } catch {
    throw new Error(i18n.global.t("validation.https"));
  }
  if (
    url.protocol !== "https:" ||
    (url.hostname !== `${normalizeFnId(fnId)}.fnos.net` &&
      !url.hostname.endsWith(`.${normalizeFnId(fnId)}.fnos.net`)) ||
    url.username ||
    url.password ||
    url.search ||
    url.hash ||
    (url.port && url.port !== "443") ||
    url.pathname !== "/"
  ) {
    throw new Error(i18n.global.t("validation.upstream"));
  }
}
export function suggestedLocalPort(nasPort: number, occupied: number[] = []): number {
  if (!Number.isInteger(nasPort) || nasPort < 1 || nasPort > 65535)
    throw new Error(i18n.global.t("validation.port"));
  const used = new Set(occupied);
  for (let offset = 0; offset < 65535; offset += 1) {
    const port = ((nasPort - 1 + offset) % 65535) + 1;
    if (!used.has(port)) return port;
  }
  throw new Error(i18n.global.t("validation.noPort"));
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
    throw new Error(i18n.global.t("validation.dockerMapping"));
  }
  return {
    id: row.id,
    name: `${row.containerName}:${row.nasPort}`,
    nasPort: row.nasPort,
    upstream: row.upstream,
    fnDomain: row.fnDomain,
    source: { code: "docker.source" },
  };
}
export function dockerPortStatus(status: DockerPortRow["status"]): string {
  return {
    mapped: i18n.global.t("docker.mapped"),
    "no-domain": i18n.global.t("docker.noDomain"),
    "registry-unavailable": i18n.global.t("docker.registryUnavailable"),
    "not-published": i18n.global.t("docker.notPublished"),
    "unsupported-protocol": i18n.global.t("docker.unsupportedProtocol"),
    ambiguous: i18n.global.t("docker.ambiguous"),
  }[status];
}

/** Preserve both registry labels when the selectable service is deduplicated. */
export function discoverySources(
  service: DiscoveredService,
  entries: InventoryEntry[],
): BackendText[] {
  const sources = new Map<string, BackendText>();
  for (const entry of entries) {
    if (
      entry.status === "mapped" &&
      entry.nasPort === service.nasPort &&
      entry.upstream === service.upstream
    ) {
      sources.set(entry.source.code, entry.source);
    }
  }
  return [...sources.values()];
}
