/// <reference types="node" />
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const app = readFileSync(new URL("../App.vue", import.meta.url), "utf8");
const row = app.slice(
  app.indexOf('<tr v-for="route in profile.services"'),
  app.indexOf("</tbody>", app.indexOf('<tr v-for="route in profile.services"')),
);

describe("service mapping actions", () => {
  it("provides named copy and open actions for both addresses", () => {
    for (const action of ["copyLocal", "copyRemote", "openLocal", "openRemote", "edit", "delete"]) {
      expect(row).toContain(`:title="t('actions.${action}', { name: route.name })"`);
    }
    expect(row).toContain('@click.prevent="w.open(route.upstream)"');
    expect(row).toContain('@click.prevent="w.open(route.localPort)"');
    expect(row).toContain('@click="w.copy(route.upstream)"');
    expect(row).toContain('@click="w.copy(w.localUrl(route.localPort))"');
  });
  it("removes test and jump buttons from the operations column", () => {
    const operations = row.slice(row.indexOf('<div class="row-actions">'));
    expect(operations).not.toContain("w.probe");
    expect(operations).not.toContain("w.open");
  });
  it("loads icons through the local proxy with a default fallback", () => {
    expect(row).toContain("<ServiceIcon");
    expect(row).toContain("favicon.ico");
    const icon = readFileSync(new URL("../components/ServiceIcon.vue", import.meta.url), "utf8");
    expect(icon).toContain('@error="failed = true"');
    expect(icon).toContain('name="server"');
    expect(icon).toContain('v-if="!loaded || failed || !url"');
  });
});
