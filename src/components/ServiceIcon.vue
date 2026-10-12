<script setup lang="ts">
import { ref, watch } from "vue";
import Icon from "./Icon.vue";

const props = defineProps<{ url: string }>();
const loaded = ref(false);
const failed = ref(false);
watch(
  () => props.url,
  () => {
    loaded.value = false;
    failed.value = false;
  },
);
</script>

<template>
  <span class="service-icon" aria-hidden="true">
    <img
      v-if="url && !failed"
      v-show="loaded"
      :key="url"
      :src="url"
      alt=""
      referrerpolicy="no-referrer"
      @load="loaded = true"
      @error="failed = true"
    />
    <Icon v-if="!loaded || failed || !url" name="server" :size="20" />
  </span>
</template>
