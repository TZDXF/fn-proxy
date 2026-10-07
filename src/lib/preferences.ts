import { computed, ref, watch } from "vue";

export const PREFERENCES_KEY = "fn-proxy.preferences";
export type AppLocale = "zh-CN" | "en-US";
export type ThemeMode = "light" | "dark" | "system";
export interface Preferences {
  locale: AppLocale;
  theme: ThemeMode;
}
type PreferenceStorage = Pick<Storage, "getItem" | "setItem">;

export function normalizePreferences(value: unknown): Preferences {
  const candidate = value && typeof value === "object" ? (value as Partial<Preferences>) : {};
  return {
    locale: candidate.locale === "en-US" ? "en-US" : "zh-CN",
    theme: candidate.theme === "light" || candidate.theme === "dark" ? candidate.theme : "system",
  };
}

export function readPreferences(storage?: PreferenceStorage): Preferences {
  try {
    return normalizePreferences(JSON.parse(storage?.getItem(PREFERENCES_KEY) ?? "null"));
  } catch {
    return normalizePreferences(null);
  }
}

export function writePreferences(storage: PreferenceStorage | undefined, value: Preferences) {
  try {
    storage?.setItem(PREFERENCES_KEY, JSON.stringify(normalizePreferences(value)));
  } catch {
    // Storage may be disabled or full. Keep the preference active for this session.
  }
}

function browserStorage(): Storage | undefined {
  try {
    return typeof window === "undefined" ? undefined : window.localStorage;
  } catch {
    return undefined;
  }
}

export function resolveTheme(mode: ThemeMode, systemDark: boolean): "light" | "dark" {
  return mode === "system" ? (systemDark ? "dark" : "light") : mode;
}

export function createPreferences(storage?: PreferenceStorage) {
  const initial = readPreferences(storage);
  const locale = ref<AppLocale>(initial.locale);
  const theme = ref<ThemeMode>(initial.theme);
  const systemDark = ref(false);
  const resolvedTheme = computed(() => resolveTheme(theme.value, systemDark.value));

  function initialize(root: HTMLElement, media?: MediaQueryList) {
    systemDark.value = media?.matches ?? false;
    const syncSystemTheme = (event: MediaQueryListEvent) => {
      systemDark.value = event.matches;
    };
    media?.addEventListener("change", syncSystemTheme);
    const stop = watch(
      [locale, theme, resolvedTheme],
      () => {
        root.lang = locale.value;
        root.dataset.theme = resolvedTheme.value;
        root.style.colorScheme = resolvedTheme.value;
        writePreferences(storage, { locale: locale.value, theme: theme.value });
      },
      { immediate: true, flush: "sync" },
    );
    return () => {
      stop();
      media?.removeEventListener("change", syncSystemTheme);
    };
  }

  return { locale, theme, resolvedTheme, initialize };
}

export const preferences = createPreferences(browserStorage());
