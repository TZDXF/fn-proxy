/// <reference types="node" />
import { Buffer } from "node:buffer";
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const root = new URL("../../", import.meta.url);
const read = (path: string) => readFileSync(new URL(path, root));

// The UI and desktop bundles should retain one shared, editable icon source.
describe("application branding", () => {
  it("uses the same icon in the sidebar and browser tab", () => {
    expect(read("public/app-icon.svg").toString()).toContain('viewBox="0 0 512 512"');
    expect(read("src/App.vue").toString()).toMatch(/class="brand-mark" src="\/app-icon\.svg"/);
    expect(read("index.html").toString()).toContain('href="/app-icon.svg"');
  });

  it("rebuilds native resources when application icons change", () => {
    expect(read("src-tauri/build.rs").toString()).toContain("cargo:rerun-if-changed=icons");
  });

  it("provides every icon referenced by the desktop bundle", () => {
    const config = JSON.parse(read("src-tauri/tauri.conf.json").toString()) as {
      bundle: { icon: string[] };
    };
    for (const icon of config.bundle.icon) {
      expect(read(`src-tauri/${icon}`).length).toBeGreaterThan(0);
    }
    expect(read("src-tauri/icons/icon.ico").subarray(0, 4)).toEqual(Buffer.from([0, 0, 1, 0]));
    expect(read("src-tauri/icons/icon.icns").subarray(0, 4).toString()).toBe("icns");
  });

  it("generates square RGBA PNGs at the required desktop sizes", () => {
    for (const [name, size] of [
      ["32x32.png", 32],
      ["64x64.png", 64],
      ["128x128.png", 128],
      ["128x128@2x.png", 256],
      ["icon.png", 512],
    ] as const) {
      const png = read(`src-tauri/icons/${name}`);
      expect(png.subarray(0, 8)).toEqual(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]));
      expect(png.readUInt32BE(16)).toBe(size);
      expect(png.readUInt32BE(20)).toBe(size);
      expect(png[24]).toBe(8);
      expect(png[25]).toBe(6);
    }
  });
});
