/// <reference types="node" />
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const css = readFileSync(new URL("../style.css", import.meta.url), "utf8");
function rule(selector: string, source = css) {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = source.replace(/\r\n/g, "\n").match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`));
  expect(match, `Missing CSS rule: ${selector}`).not.toBeNull();
  return match![1]!;
}

describe("application scroll layout", () => {
  it("matches multi-line selectors with LF and CRLF checkouts", () => {
    const lf = css.replace(/\r\n/g, "\n");
    const crlf = lf.replace(/\n/g, "\r\n");
    expect(rule("html,\nbody,\n#app", crlf)).toBe(rule("html,\nbody,\n#app", lf));
  });

  it("prevents document scrolling across the fixed titlebar", () => {
    const root = rule("html,\nbody,\n#app");
    expect(root).toMatch(/height:\s*100%;/);
    expect(root).toMatch(/overflow:\s*hidden;/);
  });

  it("constrains the application to the viewport and reserves titlebar space", () => {
    const shell = rule(".app-shell");
    expect(shell).toMatch(/height:\s*100%;/);
    expect(shell).not.toContain("min-height");
    expect(shell).toMatch(/overflow:\s*hidden;/);
    expect(shell).toContain("padding-top: var(--titlebar-height)");
    expect(rule(".app-shell--desktop")).toContain(
      "--titlebar-height: var(--desktop-titlebar-height)",
    );
    expect(rule(":root")).toContain("--titlebar-height: 0px");
  });

  it("keeps scrolling inside the main content area below the titlebar", () => {
    const main = rule(".main-shell");
    expect(main).toMatch(/min-height:\s*0;/);
    expect(main).toMatch(/overflow-y:\s*auto;/);
    expect(rule(".titlebar")).toContain("position: fixed");
    expect(rule(".sidebar")).toContain("inset: var(--titlebar-height) auto 0 0");
  });
});

describe("theme scrollbars", () => {
  it("uses theme tokens for default, hover, and active colors", () => {
    const root = rule(":root");
    expect(root).toContain("--scrollbar-thumb: var(--text-muted)");
    expect(root).toContain("--scrollbar-thumb-hover: var(--text-secondary)");
    expect(root).toContain("--scrollbar-thumb-active: var(--accent)");
    for (const token of ["--text-muted", "--text-secondary", "--accent"]) {
      expect(rule(':root[data-theme="dark"]')).toContain(token + ":");
    }
    expect(rule("*::-webkit-scrollbar-thumb")).toContain("background: var(--scrollbar-thumb)");
    expect(rule("*::-webkit-scrollbar-thumb:hover")).toContain(
      "background-color: var(--scrollbar-thumb-hover)",
    );
    expect(rule("*::-webkit-scrollbar-thumb:active")).toContain(
      "background-color: var(--scrollbar-thumb-active)",
    );
  });

  it("styles every vertical and horizontal scrollbar, including portaled content", () => {
    const scrollbar = rule("*::-webkit-scrollbar");
    expect(scrollbar).toContain("width: 10px");
    expect(scrollbar).toContain("height: 10px");
    const thumb = rule("*::-webkit-scrollbar-thumb");
    expect(thumb).toContain("border-radius: 999px");
    expect(thumb).toContain("background-clip: padding-box");
    expect(rule("*::-webkit-scrollbar-corner")).toContain("background: transparent");
  });

  it("uses standard properties only when custom scrollbar selectors are unavailable", () => {
    expect(css).toMatch(
      /@supports not selector\(::-webkit-scrollbar\)\s*\{\s*\*\s*\{\s*scrollbar-width: thin;\s*scrollbar-color: var\(--scrollbar-thumb\) transparent;/,
    );
    const webkit = css
      .split("@supports selector(::-webkit-scrollbar)")[1]!
      .split("@supports not")[0]!;
    expect(webkit).not.toContain("scrollbar-color:");
    expect(webkit).not.toContain("scrollbar-width:");
  });

  it("leaves forced-color scrollbars under system control", () => {
    const themed = css.split("@media (forced-colors: none)")[1]!.split("\nhtml,")[0]!;
    expect(themed).toContain("@supports selector(::-webkit-scrollbar)");
    expect(themed).toContain("@supports not selector(::-webkit-scrollbar)");
    expect(css.match(/scrollbar-color:/g)).toHaveLength(1);
  });
});

describe("live service controls", () => {
  it("does not disable service editing, deletion or toggles while the proxy runs", () => {
    const app = readFileSync(new URL("../App.vue", import.meta.url), "utf8");
    const table = app.slice(
      app.indexOf(':model-value="route.enabled"'),
      app.indexOf("</table>", app.indexOf(':model-value="route.enabled"')),
    );
    expect(table).toContain("w.setServiceEnabled");
    expect(table).toContain("newService(route)");
    expect(table).toContain("kind: 'service'");
    expect(table).not.toMatch(/:disabled="[^"]*proxy\.running/);
  });
});
