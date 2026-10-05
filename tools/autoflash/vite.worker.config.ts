import { defineConfig } from "vite";

export default defineConfig({
  build: {
    ssr: "worker/index.ts",
    outDir: "dist/server",
    emptyOutDir: false,
    rollupOptions: {
      output: {
        entryFileNames: "index.js",
      },
    },
  },
});
