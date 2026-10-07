import { watch } from "vue";
import { createI18n } from "vue-i18n";
import zhCN from "../locales/zh-CN.json";
import enUS from "../locales/en-US.json";
import { preferences } from "./preferences";

export const i18n = createI18n({
  legacy: false,
  locale: preferences.locale.value,
  fallbackLocale: "zh-CN",
  messages: { "zh-CN": zhCN, "en-US": enUS },
});

const stop = watch(
  preferences.locale,
  (locale) => {
    i18n.global.locale.value = locale;
  },
  { flush: "sync" },
);

if (import.meta.hot) import.meta.hot.dispose(stop);
