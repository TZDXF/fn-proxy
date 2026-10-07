import { describe, expect, it, vi } from "vitest";
import {
  PREFERENCES_KEY,
  createPreferences,
  normalizePreferences,
  readPreferences,
  resolveTheme,
  writePreferences,
} from "../lib/preferences";

function memoryStorage(initial: string | null = null) {
  let value = initial;
  return {
    getItem: vi.fn(() => value),
    setItem: vi.fn((_key: string, next: string) => {
      value = next;
    }),
  };
}
function browserFixture(dark = false) {
  const root = { lang: "", dataset: {} as Record<string, string>, style: { colorScheme: "" } };
  const media = Object.assign(new EventTarget(), { matches: dark });
  return {
    root: root as unknown as HTMLElement,
    media: media as unknown as MediaQueryList,
    change(matches: boolean) {
      media.matches = matches;
      media.dispatchEvent(Object.assign(new Event("change"), { matches }));
    },
  };
}

describe("appearance and language preferences", () => {
  it("uses Chinese and the system theme by default", () => {
    expect(readPreferences()).toEqual({ locale: "zh-CN", theme: "system" });
    expect(normalizePreferences(null)).toEqual({ locale: "zh-CN", theme: "system" });
  });
  it("restores valid preferences while rejecting invalid or partial values", () => {
    const storage = memoryStorage('{"locale":"en-US","theme":"dark"}');
    expect(readPreferences(storage)).toEqual({ locale: "en-US", theme: "dark" });
    expect(storage.getItem).toHaveBeenCalledWith(PREFERENCES_KEY);
    expect(normalizePreferences({ locale: "fr", theme: "sepia" })).toEqual({
      locale: "zh-CN",
      theme: "system",
    });
    expect(normalizePreferences({ locale: "en-US" })).toEqual({ locale: "en-US", theme: "system" });
    expect(normalizePreferences([])).toEqual({ locale: "zh-CN", theme: "system" });
  });
  it("recovers from corrupted JSON and unavailable storage", () => {
    expect(readPreferences(memoryStorage("{broken"))).toEqual({ locale: "zh-CN", theme: "system" });
    const storage = {
      getItem: () => {
        throw new Error("denied");
      },
      setItem: () => {
        throw new Error("full");
      },
    };
    expect(readPreferences(storage)).toEqual({ locale: "zh-CN", theme: "system" });
    expect(() => writePreferences(storage, { locale: "en-US", theme: "dark" })).not.toThrow();
    expect(() => writePreferences(undefined, { locale: "en-US", theme: "dark" })).not.toThrow();
  });
  it("resolves system themes but keeps explicit light and dark choices", () => {
    expect(resolveTheme("system", false)).toBe("light");
    expect(resolveTheme("system", true)).toBe("dark");
    expect(resolveTheme("light", true)).toBe("light");
    expect(resolveTheme("dark", false)).toBe("dark");
  });
  it("applies restored preferences before rendering and persists language and theme changes", () => {
    const storage = memoryStorage('{"locale":"en-US","theme":"dark"}');
    const preferences = createPreferences(storage);
    const { root, media } = browserFixture();
    const cleanup = preferences.initialize(root, media);
    expect(root.lang).toBe("en-US");
    expect(root.dataset.theme).toBe("dark");
    expect(root.style.colorScheme).toBe("dark");
    preferences.locale.value = "zh-CN";
    preferences.theme.value = "light";
    expect(root.lang).toBe("zh-CN");
    expect(root.dataset.theme).toBe("light");
    expect(readPreferences(storage)).toEqual({ locale: "zh-CN", theme: "light" });
    expect(createPreferences(storage).theme.value).toBe("light");
    cleanup();
  });
  it("tracks system changes only in system mode and removes its listener on cleanup", () => {
    const storage = memoryStorage();
    const preferences = createPreferences(storage);
    const { root, media, change } = browserFixture(true);
    const removeListener = vi.spyOn(media, "removeEventListener");
    const cleanup = preferences.initialize(root, media);
    expect(root.dataset.theme).toBe("dark");
    change(false);
    expect(root.dataset.theme).toBe("light");
    preferences.theme.value = "dark";
    change(false);
    expect(root.dataset.theme).toBe("dark");
    preferences.theme.value = "light";
    change(true);
    expect(root.dataset.theme).toBe("light");
    preferences.theme.value = "system";
    expect(root.dataset.theme).toBe("dark");
    expect(readPreferences(storage).theme).toBe("system");
    cleanup();
    expect(removeListener).toHaveBeenCalledWith("change", expect.any(Function));
    change(false);
    expect(root.dataset.theme).toBe("dark");
  });
  it("keeps changes working when storage writes fail", () => {
    const preferences = createPreferences({
      getItem: () => null,
      setItem: () => {
        throw new Error("denied");
      },
    });
    const { root } = browserFixture();
    const cleanup = preferences.initialize(root);
    expect(() => {
      preferences.theme.value = "dark";
      preferences.locale.value = "en-US";
    }).not.toThrow();
    expect(root.dataset.theme).toBe("dark");
    expect(root.lang).toBe("en-US");
    cleanup();
  });
});
