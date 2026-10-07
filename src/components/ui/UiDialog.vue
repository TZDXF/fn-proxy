<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { watch } from "vue";
import {
  DialogRoot,
  DialogPortal,
  DialogOverlay,
  DialogContent,
  DialogTitle,
  DialogClose,
} from "reka-ui";
import Icon from "../Icon.vue";
const props = defineProps<{
  modelValue: boolean;
  title: string;
  busy?: boolean;
  wide?: boolean;
  notice?: { message: string; error: boolean } | null;
}>();
const emit = defineEmits<{ "update:modelValue": [value: boolean] }>();
let opener: HTMLElement | null = null;
watch(
  () => props.modelValue,
  (open) => {
    if (open) opener = document.activeElement as HTMLElement;
  },
  { flush: "sync" },
);
function closeFocus(event: Event) {
  event.preventDefault();
  if (opener?.isConnected) opener.focus();
}
function preventWhileBusy(event: Event) {
  if (props.busy) event.preventDefault();
}
const { t } = useI18n({ useScope: "global" });
</script>
<template>
  <DialogRoot :open="modelValue" @update:open="!busy && emit('update:modelValue', $event)">
    <DialogPortal>
      <DialogOverlay class="dialog-overlay" />
      <DialogContent
        :class="['dialog-content', { wide }]"
        :aria-describedby="undefined"
        @escape-key-down="preventWhileBusy"
        @pointer-down-outside="preventWhileBusy"
        @close-auto-focus="closeFocus"
      >
        <div class="dialog-heading">
          <DialogTitle>{{ title }}</DialogTitle
          ><DialogClose class="icon-button" :disabled="busy" :aria-label="t('common.closeDialog')"
            ><Icon name="close"
          /></DialogClose>
        </div>
        <div
          v-if="notice"
          :class="['dialog-notice', { error: notice.error }]"
          :role="notice.error ? 'alert' : 'status'"
        >
          <Icon :name="notice.error ? 'info' : 'check'" /><span>{{ notice.message }}</span>
        </div>
        <slot />
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
