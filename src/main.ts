import { createApp, watch } from "vue";
import { i18n } from "./lib/i18n";
import { preferences } from "./lib/preferences";
import App from "./App.vue";
import "./style.css";

if (import.meta.env.PROD) {
  document.addEventListener("contextmenu", (event) => event.preventDefault(), { capture: true });
}

const cleanupPreferences = preferences.initialize(
  document.documentElement,
  window.matchMedia("(prefers-color-scheme: dark)"),
);
const stopTitle = watch(
  preferences.locale,
  () => {
    document.title = i18n.global.t("settings.title");
  },
  { immediate: true, flush: "sync" },
);
if (import.meta.hot)
  import.meta.hot.dispose(() => {
    cleanupPreferences();
    stopTitle();
  });

createApp(App).use(i18n).mount("#app");
