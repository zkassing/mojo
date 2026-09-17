import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "path";

// Tauri 期望固定端口；开发时由 tauri dev 注入环境变量
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // Rust 代码变动交给 tauri，不让 vite 重启
      ignored: ["**/src-tauri/**"],
    },
  },
});
