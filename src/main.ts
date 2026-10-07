import { createApp } from "vue";
import App from "./App.vue";
import "./style.css";

if (import.meta.env.PROD) {
  document.addEventListener("contextmenu", (event) => event.preventDefault(), { capture: true });
}

createApp(App).mount("#app");
