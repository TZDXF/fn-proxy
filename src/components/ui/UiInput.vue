<script setup lang="ts">
import { Primitive } from "reka-ui";
defineProps<{ modelValue?: string | number; type?: string }>();
const emit = defineEmits<{ "update:modelValue": [value: string | number] }>();
function update(event: Event) {
  const input = event.target as HTMLInputElement;
  emit(
    "update:modelValue",
    input.type === "number" && input.value !== "" ? input.valueAsNumber : input.value,
  );
}
</script>
<template>
  <Primitive as="input" :type="type ?? 'text'" :value="modelValue" class="input" @input="update" />
</template>
