import { describe, expect, it } from "vitest";
import { dockerPortService, dockerPortStatus, type DockerPortRow } from "../lib/types";

const mapped: DockerPortRow = {
  id: "fixture:0",
  containerId: "fixture-container-id",
  containerName: "fixture service",
  state: "running",
  protocol: "tcp",
  hostIp: "0.0.0.0",
  nasPort: 8084,
  containerPort: 80,
  upstream: "https://fixture-0.my-nas.fnos.net/",
  fnDomain: "fixture-0",
  status: "mapped",
  reason: "fixture",
};
describe("Docker port conversion", () => {
  it("uses the published host port instead of the private container port", () => {
    const service = dockerPortService(mapped);
    expect(service.nasPort).toBe(8084);
    expect(service.name).toBe("fixture service:8084");
    expect(service.upstream).toBe(mapped.upstream);
  });
  it.each([
    "no-domain",
    "registry-unavailable",
    "not-published",
    "unsupported-protocol",
    "ambiguous",
  ] as const)("refuses %s even if a row still contains an old URL", (status) => {
    expect(() => dockerPortService({ ...mapped, status })).toThrow();
  });
  it("refuses UDP and incomplete pairings", () => {
    for (const row of [
      { ...mapped, protocol: "udp" },
      { ...mapped, nasPort: null },
      { ...mapped, upstream: null },
      { ...mapped, fnDomain: null },
    ])
      expect(() => dockerPortService(row)).toThrow();
    expect(dockerPortStatus("registry-unavailable")).toBe("映射来源未知");
    expect(dockerPortStatus("no-domain")).toBe("未匹配到域名");
  });
});
