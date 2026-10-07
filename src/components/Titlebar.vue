<script setup lang="ts">
import { onMounted, onUnmounted } from "vue";
import { useI18n } from "vue-i18n";
import { getCurrentWindow } from "@tauri-apps/api/window";
import Icon from "./Icon.vue";
import { createWindowControls } from "../lib/windowControls";

const { t } = useI18n({ useScope: "global" });
const controls = createWindowControls(getCurrentWindow());
const { maximized, focused, failed } = controls;
onMounted(controls.initialize);
onUnmounted(controls.dispose);
</script>

<template>
  <header :class="['titlebar', { 'titlebar--inactive': !focused }]">
    <div class="titlebar-drag-region" @mousedown="controls.drag">
      <img src="/app-icon.svg" alt="" width="20" height="20" draggable="false" />
      <span v-if="failed" class="titlebar-error" role="alert">{{ t("window.actionFailed") }}</span>
    </div>
    <div class="titlebar-controls" role="group" :aria-label="t('window.controls')">
      <button
        type="button"
        class="titlebar-button"
        :aria-label="t('window.minimize')"
        :title="t('window.minimize')"
        @click="controls.minimize"
      >
        <Icon name="window-minimize" :size="16" />
      </button>
      <button
        type="button"
        class="titlebar-button"
        :aria-label="maximized ? t('window.restore') : t('window.maximize')"
        :title="maximized ? t('window.restore') : t('window.maximize')"
        @click="controls.toggleMaximize"
      >
        <Icon :name="maximized ? 'window-restore' : 'window-maximize'" :size="16" />
      </button>
      <button
        type="button"
        class="titlebar-button titlebar-button--close"
        :aria-label="t('window.close')"
        :title="t('window.close')"
        @click="controls.close"
      >
        <Icon name="close" :size="16" />
      </button>
    </div>
  </header>
</template>
