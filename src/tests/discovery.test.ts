import { describe, expect, it } from "vitest";
import { discoverySources, type DiscoveredService, type InventoryEntry } from "../lib/types";

const service: DiscoveredService = {
  id: "fixture",
  name: "Fixture",
  nasPort: 8084,
  upstream: "https://fixture.my-nas.fnos.net/",
  fnDomain: "fixture",
  source: { code: "inventory.serviceSourceDesktop" },
};
function entry(source: string, overrides: Partial<InventoryEntry> = {}): InventoryEntry {
  return {
    id: "fixture",
    source: { code: source },
    appId: null,
    name: "Fixture",
    nasPort: service.nasPort,
    fnDomain: service.fnDomain,
    upstream: service.upstream,
    path: null,
    status: "mapped",
    reason: { code: "inventory.entryMapped" },
    ...overrides,
  };
}
describe("registered service discovery", () => {
  it("preserves both source labels while deduplicating repeated entries", () => {
    expect(
      discoverySources(service, [
        entry("inventory.sourceDesktop"),
        entry("inventory.sourceDocker"),
        entry("inventory.sourceDocker"),
      ]).map((s) => s.code),
    ).toEqual(["inventory.sourceDesktop", "inventory.sourceDocker"]);
  });
  it("does not label a service using unconfigured entries or a different domain/port", () => {
    expect(
      discoverySources(service, [
        entry("inventory.sourceDocker", { status: "no-port", nasPort: null }),
        entry("inventory.sourceDocker", { upstream: "https://other.my-nas.fnos.net/" }),
        entry("inventory.sourceDocker", { nasPort: 9999 }),
      ]),
    ).toEqual([]);
  });
});
