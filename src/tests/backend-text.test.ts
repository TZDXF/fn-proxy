import { afterEach, describe, expect, it } from "vitest";
import { isBackendText, localize, localizeError } from "../lib/text";
import { preferences } from "../lib/preferences";

const originalLocale = preferences.locale.value;
afterEach(() => {
  preferences.locale.value = originalLocale;
});

describe("backend message localization", () => {
  it("detects structured backend texts only", () => {
    expect(isBackendText({ code: "auth.tfaRequired" })).toBe(true);
    expect(isBackendText({ code: "x", params: { port: "8084" } })).toBe(true);
    expect(isBackendText("auth.tfaRequired")).toBe(false);
    expect(isBackendText({ code: 42 })).toBe(false);
    expect(isBackendText(null)).toBe(false);
  });
  it("translates backend codes with interpolation in both locales", () => {
    preferences.locale.value = "zh-CN";
    expect(localize({ code: "workspace.portInUse", params: { port: "8084" } })).toBe(
      "本地端口 8084 已被另一个连接使用",
    );
    preferences.locale.value = "en-US";
    expect(localize({ code: "workspace.portInUse", params: { port: "8084" } })).toBe(
      "Local port 8084 is already used by another connection.",
    );
    expect(localize({ code: "storage.savedPasswordMissing" })).toBe(
      "No saved password was found. Enter it again.",
    );
  });
  it("passes through plain strings and unknown codes", () => {
    expect(localize("plain")).toBe("plain");
    expect(localize(null)).toBe("");
    expect(localize({ code: "" })).toBe("");
    expect(localize({ code: "missing.code" })).toBe("missing.code");
  });
  it("reuses existing top-level keys when a backend code references them", () => {
    preferences.locale.value = "en-US";
    expect(localize({ code: "docker.source" })).toBe(
      "Docker port and quick-access mapping association",
    );
  });
  it("normalizes Tauri command rejections", () => {
    preferences.locale.value = "zh-CN";
    expect(localizeError(new Error("boom"))).toBe("boom");
    expect(localizeError("raw string")).toBe("raw string");
    expect(localizeError({ code: "workspace.loginRequired" })).toBe("请先登录 NAS");
  });
});
