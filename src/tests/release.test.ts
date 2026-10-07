/// <reference types="node" />
import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { version } from "../../package.json";

const root = new URL("../../", import.meta.url);
const script = new URL("scripts/check-release-version.mjs", root);
function validate(tag: string) {
  return spawnSync(process.execPath, [fileURLToPath(script)], {
    env: { ...process.env, RELEASE_TAG: tag },
    encoding: "utf8",
  });
}

describe("release safety gates", () => {
  it("accepts the matching release tag and synchronized version files", () => {
    const result = validate("v" + version);
    expect(result.status, result.stderr).toBe(0);
  });
  it("rejects a tag that differs from the built app version", () => {
    const result = validate("v999.0.0");
    expect(result.status).not.toBe(0);
    expect(result.stderr).toContain("Release tag must match app version");
  });
  it("checks all code before building and uploads assets before publishing", () => {
    const workflow = readFileSync(new URL(".github/workflows/release.yml", root), "utf8");
    for (const command of [
      "npm run release:check",
      "npm run check",
      "npm run test:rust",
      "npm run rust:check",
    ]) {
      expect(workflow.indexOf(command)).toBeGreaterThan(-1);
      expect(workflow.indexOf(command)).toBeLessThan(workflow.indexOf("npm run desktop:build"));
    }
    expect(workflow).toContain("--verify-tag --draft");
    expect(workflow).toContain("refusing to overwrite it");
    expect(workflow.indexOf("gh release upload")).toBeLessThan(workflow.indexOf("--draft=false"));
    expect(workflow).toContain("Get-FileHash");
  });
});
