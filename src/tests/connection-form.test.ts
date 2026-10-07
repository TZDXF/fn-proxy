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

  it("offers distinct connect and edit buttons in the saved connection list", () => {
    const list = app.match(/<TabsContent value="connections"([\s\S]*?)<\/TabsContent>/)?.[1];
    expect(list).toContain('@click="connectConnection(c.profile.id)"');
    expect(list).toContain('t("connection.connect")');
    expect(list).toContain('@click="editConnection(c.profile.id)"');
    expect(list).toContain('t("common.edit")');
    expect(list).not.toContain('t("nav.connections")');
  });

  it("places disconnect before edit and keeps delete last in the connection list", () => {
    const list = app.match(/<TabsContent value="connections"([\s\S]*?)<\/TabsContent>/)?.[1];
    expect(list).toBeDefined();
    const connect = list!.indexOf('@click="connectConnection(c.profile.id)"');
    const disconnect = list!.indexOf('t("common.disconnect")');
    const edit = list!.indexOf('@click="editConnection(c.profile.id)"');
    const remove = list!.indexOf("actions.deleteConnection");
    expect(connect).toBeGreaterThanOrEqual(0);
    expect(disconnect).toBeGreaterThan(connect);
    expect(edit).toBeGreaterThan(disconnect);
    expect(remove).toBeGreaterThan(edit);
  });

  it("opens the form when credentials are missing or connecting fails", () => {
    const connect = app.match(
      /async function connectConnection\(id: string\) \{([\s\S]*?)\n\}/,
    )?.[1];
    expect(connect).toContain("if (busy.value) return");
    expect(connect).toContain("selectConnection(id)");
    expect(connect).toContain("if (!hasSavedPassword.value)");
    expect(connect).toContain("await w.connect(id)");
    expect(connect).toContain("selectedConnectionId.value === id");
    expect(connect).toContain("section.value === originSection");
    expect(connect).toContain("!connectionDialog.value");
    expect(connect).toContain("editConnection(id)");
    expect(connect).not.toContain("w.save()");
  });
});

describe("non-blocking connection controls", () => {
  it("only disables the connecting button rather than the form or dialog", () => {
    const list = app.match(/<TabsContent value="connections"([\s\S]*?)<\/TabsContent>/)?.[1];
    expect(list).toContain(':disabled="!!busy || c.connecting"');
    expect(list).toContain('c.connecting ? t("connection.connecting")');
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
