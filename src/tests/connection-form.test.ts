/// <reference types="node" />
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const app = readFileSync(new URL("../App.vue", import.meta.url), "utf8");
const form = app.match(/<form @submit\.prevent="saveConnection">([\s\S]*?)<\/form>/)?.[1];

describe("connection form", () => {
  it("does not offer startup auto-connect when adding or editing a connection", () => {
    expect(form).toBeDefined();
    expect(form).not.toContain('id="auto-connect"');
    expect(form).not.toContain("connection.autoConnect");
    expect(form).not.toContain('v-model="profile.autoConnect"');
  });

  it("keeps password saving and connection test/save actions", () => {
    expect(form).toContain('v-model="profile.remember"');
    expect(form).toContain('@click="w.connect()"');
    expect(form).toContain('type="submit"');
    expect(form).toContain("!$event && (profile.autoConnect = false)");
  });
});

describe("connection actions", () => {
  it("saves the form without testing or connecting first", () => {
    const save = app.match(/async function saveConnection\(\) \{([\s\S]*?)\n\}/)?.[1];
    expect(save).toContain("await w.save()");
    expect(save).not.toContain("w.connect()");
    expect(save).not.toContain("formMatchesSession");
  });

  it("offers proxy controls instead of a standalone authentication button", () => {
    const list = app.match(/<TabsContent value="connections"([\s\S]*?)<\/TabsContent>/)?.[1];
    expect(list).toContain('@click="toggleConnectionProxy(c.profile.id)"');
    expect(list).toContain('t("actions.startProxy")');
    expect(list).toContain('t("actions.stopProxy")');
    expect(list).not.toContain('t("connection.connect")');
    expect(app).not.toContain("connectConnection(");
    expect(list).toContain('@click="editConnection(c.profile.id)"');
    expect(list).toContain('t("common.edit")');
    expect(list).not.toContain('t("nav.connections")');
  });

  it("does not expose manual NAS disconnection", () => {
    expect(app).not.toContain('t("common.disconnect")');
    expect(app).not.toContain("w.disconnect()");
  });

  it("places proxy controls before edit and keeps delete last in the connection list", () => {
    const list = app.match(/<TabsContent value="connections"([\s\S]*?)<\/TabsContent>/)?.[1];
    expect(list).toBeDefined();
    const proxy = list!.indexOf('@click="toggleConnectionProxy(c.profile.id)"');
    const edit = list!.indexOf('@click="editConnection(c.profile.id)"');
    const remove = list!.indexOf("actions.deleteConnection");
    expect(proxy).toBeGreaterThanOrEqual(0);
    expect(edit).toBeGreaterThan(proxy);
    expect(remove).toBeGreaterThan(edit);
  });

  it("opens the form when authentication fails while starting the proxy", () => {
    const toggle = app.match(
      /async function toggleConnectionProxy\(id: string\) \{([\s\S]*?)\n\}/,
    )?.[1];
    expect(toggle).toContain("if (busy.value) return");
    expect(toggle).toContain("if (!target || target.connecting) return");
    expect(toggle).toContain("selectConnection(id)");
    expect(toggle).toContain("!target.proxy.running && !w.formMatchesSession.value");
    expect(toggle).toContain("await w.connect(id)");
    expect(toggle).toContain("selectedConnectionId.value === id");
    expect(toggle).toContain("section.value === originSection");
    expect(toggle).toContain("!connectionDialog.value");
    expect(toggle).toContain("editConnection(id)");
    expect(toggle).toMatch(/editConnection\(id\);\s*return;/);
    expect(toggle).toContain("await w.toggleProxy(id)");
    expect(toggle).not.toContain("w.save()");
  });

  it("only displays proxy status rather than NAS authentication status", () => {
    const list = app.match(/<TabsContent value="connections"([\s\S]*?)<\/TabsContent>/)?.[1];
    expect(list).not.toContain('t("connection.status")');
    expect(list).not.toContain('t("connection.authenticated")');
    expect(list).not.toContain('t("connection.notAuthenticated")');
    expect(list).toContain('c.proxy.running ? t("common.running") : t("common.stopped")');
    expect(app).not.toContain('t("common.connected")');
    expect(app).not.toContain('t("common.disconnected")');
    expect(app).not.toContain("connectedCount");
    expect(app).toContain('t("status.runningProxies", { count: runningCount })');
  });
});

describe("non-blocking connection controls", () => {
  it("only disables the connecting button rather than the form or dialog", () => {
    const list = app.match(/<TabsContent value="connections"([\s\S]*?)<\/TabsContent>/)?.[1];
    expect(list).toMatch(/:disabled="\s*!!busy \|\|\s*c\.connecting \|\|/);
    expect(list).toContain("(!c.proxy.running && !c.profile.services.some((s) => s.enabled))");
    expect(list).toContain(':aria-busy="c.connecting"');
    expect(list).toContain('t("actions.startingProxy")');
    expect(form).toContain(':disabled="!!busy || connecting || proxy.running"');
    expect(form).not.toContain('busy === "connect"');
    expect(app).not.toContain(':busy="!!busy || connecting"');
    expect(form).toContain('<UiButton variant="primary" type="submit" :disabled="!!busy">');
  });

  it("keeps proxy startup pinned to the original connection after background login", () => {
    const toggle = app.match(
      /async function toggleConnectionProxy\(id: string\) \{([\s\S]*?)\n\}/,
    )?.[1];
    expect(toggle).toContain("await w.connect(id)");
    expect(toggle).toContain("await w.toggleProxy(id)");
  });
});

describe("service discovery tab", () => {
  it("automatically reads services when the open editor switches to discovery", () => {
    expect(app).toContain("watch(serviceTab, async (tab) => {");
    expect(app).toContain('if (tab === "discovery" && editing.value) await w.discover();');
    expect(app).toContain('@click="w.discover()"');
  });
});
