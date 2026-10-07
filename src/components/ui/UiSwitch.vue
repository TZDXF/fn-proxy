<script setup lang="ts">
import { SwitchRoot, SwitchThumb } from "reka-ui";
defineProps<{ modelValue: boolean; disabled?: boolean; id?: string; label: string }>();
const emit = defineEmits<{ "update:modelValue": [value: boolean] }>();
</script>
<template>
  <SwitchRoot
    :id="id"
    :model-value="modelValue"
    :disabled="disabled"
    :aria-label="label"
    class="switch"
    @update:model-value="emit('update:modelValue', $event)"
  >
    <SwitchThumb class="switch-thumb" />
  </SwitchRoot>
</template>
