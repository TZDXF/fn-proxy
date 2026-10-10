<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import type { ConnectionInfo } from "../lib/types";
import { formatExpiry, formatTrafficMb, trafficPercent } from "../lib/fn-connect";
import Icon from "./Icon.vue";

const props = defineProps<{ connection: ConnectionInfo }>();
const { t, locale } = useI18n();
const entitlement = computed(() => props.connection.fnConnect?.entitlement);
const tier = computed(() => entitlement.value?.tier ?? "unknown");
const percent = computed(() => (entitlement.value ? trafficPercent(entitlement.value) : null));
const expiry = computed(() => formatExpiry(entitlement.value?.endTime ?? null, locale.value));
const traffic = computed(() => {
  const e = entitlement.value;
  if (!e) return "—";
  if (e.trafficPerMonthMb === 0) return t("fnConnect.unlimited");
  return `${formatTrafficMb(e.trafficUsedMb, locale.value)} / ${formatTrafficMb(e.trafficPerMonthMb, locale.value)}`;
});
const bandwidth = computed(() =>
  entitlement.value?.bandwidthMbps == null ? "—" : `${entitlement.value.bandwidthMbps} Mbps`,
);
const details = computed(() => {
  if (!props.connection.connected || !entitlement.value)
    return `${t("fnConnect.title")} · ${message.value}`;
  const lines = [
    t("fnConnect.title"),
    t(`fnConnect.tiers.${tier.value}`),
    `${t("fnConnect.bandwidth")}: ${bandwidth.value}`,
    `${t("fnConnect.traffic")}: ${traffic.value}`,
  ];
  if (tier.value !== "base")
    lines.push(
      expiry.value
        ? t("fnConnect.expiresAt", { date: expiry.value })
        : `${t("fnConnect.validity")}: —`,
    );
  return lines.join(" · ");
});
const message = computed(() => {
  if (!props.connection.connected) return t("fnConnect.disconnected");
  if (!props.connection.fnConnect) return t("fnConnect.loading");
  return t(
    props.connection.fnConnect.status === "unbound" ? "fnConnect.unbound" : "fnConnect.unavailable",
  );
});
</script>

<template>
  <span class="fn-connect-inline" role="group" :aria-label="details" :title="details">
    <template v-if="connection.connected && entitlement">
      <span :class="['fn-connect-tier', tier]">
        <Icon v-if="tier === 'premium' || tier === 'pro'" name="diamond" :size="12" />
        {{ t(`fnConnect.tiers.${tier}`) }}
      </span>
      <span class="fn-connect-value" :title="`${details} · ${t('fnConnect.bandwidthHint')}`">
        <Icon name="speed" :size="13" />{{ bandwidth }}
      </span>
      <span
        class="fn-connect-value"
        :class="{ exhausted: percent !== null && percent >= 100 }"
        :title="`${details} · ${t('fnConnect.trafficHint')}`"
      >
        <Icon name="transfer" :size="13" />{{ traffic }}
      </span>
    </template>
    <span v-else class="fn-connect-placeholder"><Icon name="globe" :size="13" />—</span>
  </span>
</template>
