import { readFileSync } from "node:fs";
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

describe("NAS main entry placement", () => {
  const app = readFileSync(new URL("../App.vue", import.meta.url), "utf8");
  const manual = app.split('<TabsContent value="manual"')[1]!.split("</TabsContent")[0]!;
  const discovery = app.split('<TabsContent value="discovery"')[1]!.split("</TabsContent")[0]!;

  it("offers the NAS main entry in the discovery list instead of the manual form", () => {
    expect(manual).not.toContain("chooseNasMain");
    expect(manual).not.toContain("service.nasMainHint");
    expect(discovery).toContain('@click="chooseNasMain"');
    expect(discovery).not.toContain("service.nasMainHint");
    expect(discovery.indexOf('class="discovered-list"')).toBeLessThan(
      discovery.indexOf('@click="chooseNasMain"'),
    );
  });

  it("keeps the main entry available without inventory and prevents duplicate port mappings", () => {
    expect(discovery.indexOf('@click="chooseNasMain"')).toBeLessThan(
      discovery.indexOf('<template v-if="inventory">'),
    );
    expect(discovery).toContain(
      "!!busy || connecting || profile.services.some((s) => s.nasPort === 443)",
    );
    expect(discovery).not.toContain('t("service.noAvailable")');
  });
});
