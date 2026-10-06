import { defineConfig } from "vitest/config";
import vue from "@vitejs/plugin-vue";
import tailwindcss from "@tailwindcss/vite";
export default defineConfig({
  plugins: [vue(), tailwindcss()],
  clearScreen: false,
  server: {
    host: "127.0.0.1",
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**", "**/.scaffold/**"] },
  },
  test: { include: ["src/tests/**/*.test.ts"] },
});
