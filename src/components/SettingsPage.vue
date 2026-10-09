<script setup lang="ts">
import { computed } from "vue";
import type { createUpdates } from "../lib/updates";
import UiButton from "./ui/UiButton.vue";
import { useI18n } from "vue-i18n";
import Icon from "./Icon.vue";
import UiSelect from "./ui/UiSelect.vue";
import UiSwitch from "./ui/UiSwitch.vue";

const { updates } = defineProps<{
  updates: ReturnType<typeof createUpdates>;
  launchAtLogin: boolean;
  launchAtLoginDisabled: boolean;
  autoStartProxy: boolean;
  startupDisabled: boolean;
  allowLanAccess: boolean;
  networkDisabled: boolean;
  anyProxyRunning: boolean;
}>();
const emit = defineEmits<{
  "update:launchAtLogin": [value: boolean];
  "update:allowLanAccess": [value: boolean];
  "update:autoStartProxy": [value: boolean];
}>();
import { preferences, type AppLocale, type ThemeMode } from "../lib/preferences";

const { t } = useI18n({ useScope: "global" });
const { versionLabel, state, latest, errorKey, checking, opening, openFailed, versionFailed } =
  updates;
const { locale, theme } = preferences;
const languages = [
  { value: "zh-CN", label: "简体中文" },
  { value: "en-US", label: "English" },
];
const themes = computed(() => [
  {
    value: "light" as const,
    icon: "sun",
    label: t("settings.light"),
  },
  {
    value: "dark" as const,
    icon: "moon",
    label: t("settings.dark"),
  },
  {
    value: "system" as const,
    icon: "monitor",
    label: t("settings.system"),
  },
]);
function selectLocale(value: string) {
  if (value === "zh-CN" || value === "en-US") locale.value = value as AppLocale;
}
function selectTheme(event: Event) {
  theme.value = (event.target as HTMLInputElement).value as ThemeMode;
}
</script>

<template>
  <div class="settings-page">
    <section class="panel settings-panel" aria-labelledby="general-heading">
      <h2 id="general-heading">{{ t("settings.general") }}</h2>
      <div class="settings-row">
        <div>
          <label for="launch-at-login">{{ t("settings.launchAtLogin") }}</label>
          <p class="settings-hint">{{ t("settings.launchAtLoginDescription") }}</p>
        </div>
        <UiSwitch
          id="launch-at-login"
          :model-value="launchAtLogin"
          :disabled="launchAtLoginDisabled"
          :label="t('settings.launchAtLogin')"
          @update:model-value="emit('update:launchAtLogin', $event)"
        />
      </div>
      <div class="settings-row">
        <span>{{ t("settings.displayLanguage") }}</span>
        <UiSelect
          :model-value="locale"
          :options="languages"
          :label="t('settings.displayLanguage')"
          @update:model-value="selectLocale"
        />
      </div>
      <div class="settings-row">
        <span id="theme-label">{{ t("settings.theme") }}</span>
        <fieldset class="theme-options" aria-labelledby="theme-label">
          <label
            v-for="option in themes"
            :key="option.value"
            :class="['theme-option', { selected: theme === option.value }]"
          >
            <input
              type="radio"
              name="theme"
              :value="option.value"
              :checked="theme === option.value"
              @change="selectTheme"
            />
            <Icon :name="option.icon" :size="16" />
            {{ option.label }}
          </label>
        </fieldset>
      </div>
    </section>
    <section class="panel settings-panel" aria-labelledby="proxy-heading">
      <h2 id="proxy-heading">{{ t("settings.proxy") }}</h2>
      <div class="settings-row">
        <div>
          <label for="auto-start-proxy">{{ t("settings.autoStartProxy") }}</label>
          <p class="settings-hint">{{ t("settings.autoStartProxyDescription") }}</p>
        </div>
        <UiSwitch
          id="auto-start-proxy"
          :model-value="autoStartProxy"
          :disabled="startupDisabled"
          :label="t('settings.autoStartProxy')"
          @update:model-value="emit('update:autoStartProxy', $event)"
        />
      </div>
      <div class="settings-row">
        <div>
          <label for="allow-lan-access">{{ t("settings.allowLanAccess") }}</label>
          <p class="settings-hint">{{ t("settings.lanWarning") }}</p>
          <p v-if="allowLanAccess" class="settings-hint">{{ t("settings.lanAddress") }}</p>
          <p v-if="anyProxyRunning" class="settings-warning" role="status">
            {{ t("settings.stopProxiesFirst") }}
          </p>
        </div>
        <UiSwitch
          id="allow-lan-access"
          :model-value="allowLanAccess"
          :disabled="networkDisabled"
          :label="t('settings.allowLanAccess')"
          @update:model-value="emit('update:allowLanAccess', $event)"
        />
      </div>
    </section>
    <section class="panel settings-panel" aria-labelledby="updates-heading">
      <h2 id="updates-heading">{{ t("updates.title") }}</h2>
      <div class="settings-row">
        <div>
          <span>{{ t("updates.currentVersion") }}</span>
          <strong class="update-version">{{ versionLabel || t("common.reading") }}</strong>
        </div>
        <div class="update-actions">
          <UiButton :disabled="opening" @click="updates.openRelease">
            {{ t(state === "available" ? "updates.download" : "updates.releasePage") }}
          </UiButton>
          <UiButton
            variant="primary"
            :disabled="checking || !updates.desktop"
            @click="updates.check"
          >
            <Icon name="refresh" :size="16" />
            {{ t(checking ? "updates.checking" : "updates.check") }}
          </UiButton>
        </div>
      </div>
      <div class="update-content">
        <p
          v-if="state !== 'idle' || !updates.desktop"
          class="update-status"
          role="status"
          aria-live="polite"
        >
          {{
            !updates.desktop
              ? t("updates.preview")
              : state === "failed"
                ? t(errorKey)
                : t(`updates.${state}`, { version: latest?.latestVersion ?? "" })
          }}
        </p>
        <p v-if="versionFailed" class="update-error" role="alert">
          {{ t("updates.versionError") }}
        </p>
        <p v-if="openFailed" class="update-error" role="alert">{{ t("updates.openError") }}</p>
      </div>
    </section>
  </div>
</template>

<style scoped>
.settings-page {
  max-width: 920px;
}
.settings-panel {
  margin-bottom: 20px;
  padding: 0 24px;
}
.settings-panel h2 {
  padding: 18px 0 4px;
  font-size: 14px;
}
.settings-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 20px;
  padding: 18px 0;
  font-size: 13px;
}
.settings-row + .settings-row {
  border-top: 1px solid var(--line);
}
.settings-row .select-trigger {
  min-width: 210px;
}
.settings-row .switch {
  flex-shrink: 0;
}
.settings-hint,
.settings-warning,
.update-status,
.update-error {
  margin-top: 6px;
  font-size: 12px;
  line-height: 1.7;
  overflow-wrap: anywhere;
}
.settings-hint,
.update-status {
  color: var(--text-muted);
}
.settings-warning {
  color: var(--accent);
}
.theme-options {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  padding: 4px;
  margin: 0;
  min-width: 0;
  border: 1px solid var(--line);
  border-radius: 9px;
}
.theme-option {
  position: relative;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 7px 12px;
  border-radius: 6px;
  cursor: pointer;
  color: var(--text-secondary);
}
.theme-option:hover {
  background: var(--surface-subtle);
}
.theme-option.selected {
  background: var(--accent-soft);
  color: var(--accent);
}
.theme-option:focus-within {
  outline: 2px solid var(--focus);
  outline-offset: 2px;
}
.theme-option input {
  position: absolute;
  opacity: 0;
  width: 1px;
  height: 1px;
}
.update-version {
  margin-left: 10px;
}
.update-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
.update-content:has(> p) {
  padding-bottom: 18px;
}
.update-error {
  color: var(--danger);
}
@media (max-width: 600px) {
  .settings-panel {
    padding: 0 18px;
  }
  .settings-row {
    flex-wrap: wrap;
    gap: 12px;
  }
  .settings-row:has(.switch) {
    flex-wrap: nowrap;
  }
  .settings-row .select-trigger {
    min-width: 150px;
  }
}
</style>
