<script setup lang="ts">
import { useI18n } from "vue-i18n";
import {
  SelectRoot,
  SelectTrigger,
  SelectValue,
  SelectIcon,
  SelectPortal,
  SelectContent,
  SelectViewport,
  SelectItem,
  SelectItemText,
  SelectItemIndicator,
} from "reka-ui";
import Icon from "../Icon.vue";
defineProps<{
  modelValue: string;
  options: { value: string; label: string }[];
  disabled?: boolean;
  label: string;
}>();
const emit = defineEmits<{ "update:modelValue": [value: string] }>();
const { t } = useI18n({ useScope: "global" });
</script>
<template>
  <SelectRoot
    :model-value="modelValue"
    :disabled="disabled"
    @update:model-value="emit('update:modelValue', String($event))"
  >
    <SelectTrigger class="select-trigger" :aria-label="label"
      ><SelectValue :placeholder="label || t('connection.select')" /><SelectIcon
        class="select-chevron"
        >⌄</SelectIcon
      ></SelectTrigger
    >
    <SelectPortal
      ><SelectContent class="select-content" position="popper" :side-offset="6"
        ><SelectViewport>
          <SelectItem
            v-for="option in options"
            :key="option.value"
            :value="option.value"
            class="select-item"
            ><SelectItemText>{{ option.label }}</SelectItemText
            ><SelectItemIndicator><Icon name="check" :size="16" /></SelectItemIndicator
          ></SelectItem> </SelectViewport></SelectContent
    ></SelectPortal>
  </SelectRoot>
</template>
