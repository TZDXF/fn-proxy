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
