import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { resolve } from "node:path";
export default defineConfig({plugins:[react(),tailwindcss()],resolve:{alias:[{find:/^@tauri-apps\/.*/,replacement:resolve("reports/module-ui-2026-11-02/preview/tauri.ts")}]},server:{host:"127.0.0.1",port:5196,strictPort:true,hmr:false}});
