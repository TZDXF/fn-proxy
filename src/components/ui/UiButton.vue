<script setup lang="ts">
import { Primitive } from "reka-ui";
defineProps<{
  variant?: "primary" | "secondary" | "ghost" | "danger";
  type?: "button" | "submit" | "reset";
}>();
</script>
<template>
  <Primitive as="button" :type="type ?? 'button'" :class="['button', variant ?? 'secondary']"
    ><slot
  /></Primitive>
</template>
