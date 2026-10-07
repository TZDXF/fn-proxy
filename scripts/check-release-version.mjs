import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const root = new URL("../", import.meta.url);
const read = (path) => readFileSync(new URL(path, root), "utf8");
const pkg = JSON.parse(read("package.json"));
const config = JSON.parse(read("src-tauri/tauri.conf.json"));
const cargoVersion = read("src-tauri/Cargo.toml").match(/^version\s*=\s*"([^"]+)"/m)?.[1];
const cargoLockVersion = read("src-tauri/Cargo.lock").match(
  /\[\[package\]\]\s+name = "fn-proxy"\s+version = "([^"]+)"/,
)?.[1];
const lock = JSON.parse(read("package-lock.json"));
assert.match(pkg.version, /^\d+\.\d+\.\d+$/, "Release version must be stable semver");
assert.equal(config.version, pkg.version, "Tauri and npm versions must match");
assert.equal(cargoVersion, pkg.version, "Rust and npm versions must match");
assert.equal(cargoLockVersion, pkg.version, "Cargo lockfile app version must match");
assert.equal(lock.version, pkg.version, "npm lockfile version must match");
assert.equal(lock.packages[""].version, pkg.version, "npm root package lock version must match");
if (process.env.RELEASE_TAG) {
  assert.equal(process.env.RELEASE_TAG, "v" + pkg.version, "Release tag must match app version");
}
console.log("Release version verified: v" + pkg.version);
