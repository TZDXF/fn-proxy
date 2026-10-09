/// <reference types="node" />
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const page = readFileSync(new URL("../components/SettingsPage.vue", import.meta.url), "utf8");

describe("compact settings page", () => {
  it("groups controls into three sections without decorative previews or repeated descriptions", () => {
    expect(page.match(/<section /g)).toHaveLength(3);
    expect(page).not.toContain("theme-preview");
    expect(page).not.toContain('t("settings.description")');
    expect(page).not.toContain('t("settings.saved")');
  });

  it("adds launch at login to general settings without coupling it to proxy startup", () => {
    const general = page.slice(
      page.indexOf('aria-labelledby="general-heading"'),
      page.indexOf('aria-labelledby="proxy-heading"'),
    );
    expect(general).toContain('id="launch-at-login"');
    expect(general).toContain(':model-value="launchAtLogin"');
    expect(general).toContain(':disabled="launchAtLoginDisabled"');
    expect(general).toContain("emit('update:launchAtLogin', $event)");
    expect(general).toContain('t("settings.launchAtLoginDescription")');
    expect(general).not.toContain('id="auto-start-proxy"');
  });

  it("preserves proxy controls, safety guidance and running-state restrictions", () => {
    expect(page).toContain(':disabled="startupDisabled"');
    expect(page).toContain(':disabled="networkDisabled"');
    expect(page).toContain("emit('update:autoStartProxy', $event)");
    expect(page).toContain("emit('update:allowLanAccess', $event)");
    expect(page).toContain('t("settings.lanWarning")');
    expect(page).toContain('v-if="allowLanAccess"');
    expect(page).toContain('v-if="anyProxyRunning"');
  });

  it("retains update actions and error feedback while hiding the idle prompt", () => {
    expect(page).toContain('@click="updates.check"');
    expect(page).toContain('@click="updates.openRelease"');
    expect(page).toContain("v-if=\"state !== 'idle' || !updates.desktop\"");
    expect(page).toContain('v-if="versionFailed"');
    expect(page).toContain('v-if="openFailed"');
  });
});
