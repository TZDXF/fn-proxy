import { version as packageVersion } from "../../package.json";
import { describe, expect, it, vi } from "vitest";
import { createUpdates, RELEASES_URL, type UpdateInfo } from "../lib/updates";

function fixture(desktop = true) {
  const result: UpdateInfo = {
    currentVersion: "0.1.0",
    latestVersion: "0.2.0",
    updateAvailable: true,
    releaseUrl: RELEASES_URL + "/tag/v0.2.0",
  };
  const dependencies = {
    desktop,
    getVersion: vi.fn().mockResolvedValue("0.1.0"),
    check: vi.fn().mockResolvedValue(result),
    install: vi.fn().mockImplementation(async (_version, onEvent) => {
      onEvent({ event: "Started", data: { contentLength: 100 } });
      onEvent({ event: "Progress", data: { chunkLength: 40 } });
      onEvent({ event: "Finished", data: {} });
    }),
    restart: vi.fn().mockResolvedValue(undefined),
    open: vi.fn().mockResolvedValue(undefined),
  };
  return { updates: createUpdates(dependencies), dependencies, result };
}

describe("version and update checks", () => {
  it("formats the shared version label from the installed version and update response", async () => {
    const { updates, dependencies, result } = fixture();
    expect(updates.versionLabel.value).toBe("");
    dependencies.getVersion.mockResolvedValueOnce("1.2.3");
    await updates.initialize();
    expect(updates.versionLabel.value).toBe("v1.2.3");
    dependencies.check.mockResolvedValueOnce({ ...result, currentVersion: "1.2.4" });
    await updates.check();
    expect(updates.versionLabel.value).toBe("v1.2.4");
  });
  it("reads the installed desktop version", async () => {
    const { updates, dependencies } = fixture();
    await updates.initialize();
    expect(updates.version.value).toBe("0.1.0");
    expect(dependencies.getVersion).toHaveBeenCalledOnce();
  });
  it("handles version lookup failure and recovers on a successful check", async () => {
    const { updates, dependencies } = fixture();
    dependencies.getVersion.mockRejectedValueOnce(new Error("unavailable"));
    await updates.initialize();
    expect(updates.versionFailed.value).toBe(true);
    await updates.check();
    expect(updates.versionFailed.value).toBe(false);
    expect(updates.version.value).toBe("0.1.0");
  });
  it("shows new releases and opens the exact release page", async () => {
    const { updates, dependencies, result } = fixture();
    await updates.check();
    expect(updates.state.value).toBe("available");
    expect(updates.latest.value).toEqual(result);
    await updates.openRelease();
    expect(dependencies.open).toHaveBeenCalledWith(result.releaseUrl);
  });
  it("distinguishes up-to-date and no-release states", async () => {
    const { updates, dependencies, result } = fixture();
    dependencies.check.mockResolvedValueOnce({ ...result, updateAvailable: false });
    await updates.check();
    expect(updates.state.value).toBe("upToDate");
    dependencies.check.mockResolvedValueOnce({
      ...result,
      latestVersion: null,
      updateAvailable: false,
      releaseUrl: RELEASES_URL,
    });
    await updates.check();
    expect(updates.state.value).toBe("noRelease");
  });
  it("prevents duplicate checks and clears stale release data on retry", async () => {
    const { updates, dependencies } = fixture();
    await updates.check();
    let finish!: (result: UpdateInfo) => void;
    dependencies.check.mockImplementationOnce(
      () =>
        new Promise<UpdateInfo>((resolve) => {
          finish = resolve;
        }),
    );
    const pending = updates.check();
    await updates.check();
    expect(dependencies.check).toHaveBeenCalledTimes(2);
    expect(updates.state.value).toBe("checking");
    expect(updates.latest.value).toBeNull();
    finish({
      currentVersion: "0.1.0",
      latestVersion: null,
      updateAvailable: false,
      releaseUrl: RELEASES_URL,
    });
    await pending;
    expect(updates.checking.value).toBe(false);
  });
  it.each([
    ["network", "updates.networkError"],
    ["rateLimited", "updates.rateLimited"],
    ["invalidResponse", "updates.invalidResponse"],
    ["unexpectedResponse", "updates.unexpectedResponse"],
  ])("handles %s errors and allows retry", async (error, key) => {
    const { updates, dependencies } = fixture();
    dependencies.check.mockRejectedValueOnce(error);
    await updates.check();
    expect(updates.state.value).toBe("failed");
    expect(updates.errorKey.value).toBe(key);
    await updates.check();
    expect(updates.state.value).toBe("available");
    expect(updates.errorKey.value).toBe("");
  });
  it("does not use native IPC in browser preview", async () => {
    const { updates, dependencies } = fixture(false);
    await updates.initialize();
    await updates.check();
    expect(updates.version.value).toBe(packageVersion);
    expect(updates.versionLabel.value).toBe("v" + packageVersion);
    expect(dependencies.getVersion).not.toHaveBeenCalled();
    expect(dependencies.check).not.toHaveBeenCalled();
  });
  it("can open releases before checking and handles opener failure", async () => {
    const { updates, dependencies } = fixture();
    dependencies.open.mockRejectedValueOnce(new Error("no browser"));
    await updates.openRelease();
    expect(dependencies.open).toHaveBeenCalledWith(RELEASES_URL);
    expect(updates.openFailed.value).toBe(true);
    expect(updates.opening.value).toBe(false);
    await updates.openRelease();
    expect(updates.openFailed.value).toBe(false);
  });
  it.each([
    "https://evil.example/releases",
    "https://github.com/other/repo/releases",
    "https://user@github.com/TZDXF/fn-proxy/releases",
    "http://github.com/TZDXF/fn-proxy/releases",
  ])("rejects an untrusted release URL: %s", async (releaseUrl) => {
    const { updates, dependencies, result } = fixture();
    dependencies.check.mockResolvedValueOnce({ ...result, releaseUrl });
    await updates.check();
    await updates.openRelease();
    expect(dependencies.open).not.toHaveBeenCalled();
    expect(updates.openFailed.value).toBe(true);
  });
});

describe("in-app signed update install", () => {
  it("installs the checked version with progress and offers restart", async () => {
    const { updates, dependencies } = fixture();
    await updates.check();
    await updates.download();
    expect(dependencies.install).toHaveBeenCalledWith("0.2.0", expect.any(Function));
    expect(dependencies.open).not.toHaveBeenCalled();
    expect(updates.progress.value).toBe(40);
    expect(updates.installing.value).toBe(true);
    expect(updates.installed.value).toBe(true);
    await updates.restart();
    expect(dependencies.restart).toHaveBeenCalledOnce();
  });
  it("reports install failures and permits retry", async () => {
    const { updates, dependencies } = fixture();
    await updates.check();
    dependencies.install.mockRejectedValueOnce("versionChanged");
    await updates.download();
    expect(updates.downloadError.value).toBe("updates.versionChanged");
    expect(updates.installed.value).toBe(false);
    await updates.download();
    expect(updates.downloadError.value).toBe("");
    expect(updates.installed.value).toBe(true);
  });
  it("does not install before checking or in browser preview", async () => {
    for (const desktop of [true, false]) {
      const { updates, dependencies } = fixture(desktop);
      await updates.download();
      expect(dependencies.install).not.toHaveBeenCalled();
    }
  });
});
