<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { remoteStatus, type RemoteCheck } from "../lib/serviceAvailability";
import { localize } from "../lib/text";

const props = defineProps<{ connected: boolean; check?: RemoteCheck }>();
const { t } = useI18n({ useScope: "global" });
const status = computed(() => remoteStatus(props.check));
const detail = computed(() => {
  const result = props.check?.result;
  if (!result) return t("service.remote.checkingHint");
  return `${localize(result.message)}${result.status ? ` (HTTP ${result.status})` : ""}`;
});
</script>

<template>
  <span
    v-if="connected && check?.result && !check.result.reachable"
    :class="['badge', status.tone]"
    :title="detail"
    role="status"
    >{{ t(`service.remote.${status.label}`) }}</span
  >
</template>
