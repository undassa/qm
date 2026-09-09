import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

/**
 * Новая поверхность живёт на том же входе, что и старая: `/next` на порту 8096.
 * Так она получает вход в систему и заголовок личности даром, без второго сервера.
 */
export default defineConfig({
  plugins: [react()],
  base: "/next/",
  build: { outDir: "dist", emptyOutDir: true },
  server: { proxy: { "/api": "http://127.0.0.1:8096" } },
});
