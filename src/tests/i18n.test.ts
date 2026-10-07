/// <reference types="node" />
import { readFileSync } from "node:fs";
import { afterEach, describe, expect, it } from "vitest";
import { i18n } from "../lib/i18n";
import { preferences } from "../lib/preferences";
import { dockerPortStatus, normalizeFnId } from "../lib/types";
import zhCN from "../locales/zh-CN.json";
import enUS from "../locales/en-US.json";

function flatten(messages: object, prefix = ""): Record<string, string> {
  return Object.fromEntries(
    Object.entries(messages).flatMap(([key, value]) => {
      const path = prefix ? `${prefix}.${key}` : key;
      return typeof value === "string" ? [[path, value]] : Object.entries(flatten(value, path));
    }),
  );
}
const zh = flatten(zhCN);
const en = flatten(enUS);
const originalLocale = preferences.locale.value;
afterEach(() => {
  preferences.locale.value = originalLocale;
});

describe("application internationalization", () => {
  it("has matching message keys and interpolation parameters in both languages", () => {
    expect(Object.keys(en).sort()).toEqual(Object.keys(zh).sort());
    for (const key of Object.keys(zh)) {
      expect(en[key]).toBeTypeOf("string");
      expect(en[key]!.match(/\{\w+\}/g)?.sort() ?? []).toEqual(
        zh[key]!.match(/\{\w+\}/g)?.sort() ?? [],
      );
    }
  });
  it("provides every static translation key referenced by the UI and workspace", () => {
    for (const file of [
      "App.vue",
      "components/SettingsPage.vue",
      "components/Titlebar.vue",
      "components/Titlebar.vue",
      "components/ui/UiDialog.vue",
      "components/ui/UiSelect.vue",
      "lib/workspace.ts",
      "lib/types.ts",
      "main.ts",
    ]) {
      const text = readFileSync(new URL(`../${file}`, import.meta.url), "utf8");
      for (const match of text.matchAll(/\bt\(["']([\w.]+)["']/g))
        expect(zh, `${file}: ${match[1]}`).toHaveProperty(match[1]!);
    }
  });
  it("switches navigation, settings, counters and action labels immediately", () => {
    preferences.locale.value = "en-US";
    expect(i18n.global.locale.value).toBe("en-US");
    expect(i18n.global.t("nav.settings")).toBe("Settings");
    expect(i18n.global.t("settings.dark")).toBe("Dark");
    expect(i18n.global.t("status.onlineConnections", { count: 2 })).toBe("2 connections online");
    expect(i18n.global.t("actions.copyLocal", { name: "API" })).toBe("Copy local URL for API");
    preferences.locale.value = "zh-CN";
    expect(i18n.global.t("nav.settings")).toBe("设置");
    expect(i18n.global.t("status.onlineConnections", { count: 2 })).toBe("2 个连接在线");
  });
  it("localizes validation errors and Docker status labels outside components", () => {
    preferences.locale.value = "en-US";
    expect(() => normalizeFnId("nas--id")).toThrow(
      "FN ID only supports letters, digits, and single hyphens",
    );
    expect(dockerPortStatus("no-domain")).toBe("No matching domain");
    preferences.locale.value = "zh-CN";
    expect(dockerPortStatus("no-domain")).toBe("未匹配到域名");
  });
  it("compiles every message in both languages without falling back to a key", () => {
    for (const locale of ["zh-CN", "en-US"] as const) {
      preferences.locale.value = locale;
      for (const key of Object.keys(zh)) {
        expect(i18n.global.t(key, { count: 2, name: "API", theme: "Dark" })).not.toBe(key);
      }
    }
  });
});
