<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import Icon from "./Icon.vue";
import UiSelect from "./ui/UiSelect.vue";
import UiSwitch from "./ui/UiSwitch.vue";

defineProps<{
  allowLanAccess: boolean;
  networkDisabled: boolean;
  anyProxyRunning: boolean;
}>();
const emit = defineEmits<{ "update:allowLanAccess": [value: boolean] }>();
import { preferences, type AppLocale, type ThemeMode } from "../lib/preferences";

const { t } = useI18n({ useScope: "global" });
const { locale, theme, resolvedTheme } = preferences;
const languages = [
  { value: "zh-CN", label: "简体中文" },
  { value: "en-US", label: "English" },
];
const themes = computed(() => [
  {
    value: "light" as const,
    icon: "sun",
    label: t("settings.light"),
    description: t("settings.lightDescription"),
  },
  {
    value: "dark" as const,
    icon: "moon",
    label: t("settings.dark"),
    description: t("settings.darkDescription"),
  },
  {
    value: "system" as const,
    icon: "monitor",
    label: t("settings.system"),
    description: t("settings.systemDescription"),
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
    <p class="settings-description">{{ t("settings.description") }}</p>
    <section class="panel settings-panel" aria-labelledby="network-heading">
      <div class="panel-heading settings-panel-heading">
        <span class="settings-icon"><Icon name="server" :size="21" /></span>
        <div>
          <h2 id="network-heading">{{ t("settings.network") }}</h2>
          <p>{{ t("settings.networkDescription") }}</p>
        </div>
      </div>
      <div class="settings-language-row settings-network-row">
        <div>
          <label for="allow-lan-access">{{ t("settings.allowLanAccess") }}</label>
          <p>{{ t(allowLanAccess ? "settings.lanEnabled" : "settings.lanDisabled") }}</p>
        </div>
        <UiSwitch
          id="allow-lan-access"
          :model-value="allowLanAccess"
          :disabled="networkDisabled"
          :label="t('settings.allowLanAccess')"
          @update:model-value="emit('update:allowLanAccess', $event)"
        />
      </div>
      <div class="settings-network-hints">
        <p>{{ t("settings.lanAddress") }}</p>
        <p>{{ t("settings.lanWarning") }}</p>
        <p v-if="anyProxyRunning" class="settings-network-warning" role="status">
          {{ t("settings.stopProxiesFirst") }}
        </p>
      </div>
    </section>
    <section class="panel settings-panel" aria-labelledby="language-heading">
      <div class="panel-heading settings-panel-heading">
        <span class="settings-icon"><Icon name="globe" :size="21" /></span>
        <div>
          <h2 id="language-heading">{{ t("settings.language") }}</h2>
          <p>{{ t("settings.languageDescription") }}</p>
        </div>
      </div>
      <div class="settings-language-row">
        <span id="language-label">{{ t("settings.displayLanguage") }}</span>
        <UiSelect
          :model-value="locale"
          :options="languages"
          :label="t('settings.displayLanguage')"
          @update:model-value="selectLocale"
        />
      </div>
    </section>
    <section class="panel settings-panel" aria-labelledby="appearance-heading">
      <div class="panel-heading settings-panel-heading">
        <span class="settings-icon"><Icon name="sun" :size="21" /></span>
        <div>
          <h2 id="appearance-heading">{{ t("settings.appearance") }}</h2>
          <p>{{ t("settings.appearanceDescription") }}</p>
        </div>
      </div>
      <fieldset class="theme-options">
        <legend class="sr-only">{{ t("settings.theme") }}</legend>
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
          <span :class="['theme-preview', `preview-${option.value}`]" aria-hidden="true"
            ><span class="preview-sidebar" /><span class="preview-main"
              ><span /><span /><span /></span
          ></span>
          <span class="theme-option-title"
            ><Icon :name="option.icon" :size="17" />{{ option.label
            }}<Icon v-if="theme === option.value" name="check" :size="16"
          /></span>
          <span class="theme-option-description">{{ option.description }}</span>
        </label>
      </fieldset>
      <p class="theme-current">
        {{ t("settings.currentTheme", { theme: t(`settings.${resolvedTheme}`) }) }}
      </p>
    </section>
    <p class="settings-saved"><Icon name="check" :size="16" />{{ t("settings.saved") }}</p>
  </div>
</template>
