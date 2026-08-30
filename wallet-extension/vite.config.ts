import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(new URL(".", import.meta.url));

/**
 * Builds background.js (an ES module - MV3 service workers support
 * "type": "module") and popup.html/popup.js. content-script.js/inject.js
 * are built separately by vite.config.content-script.ts and
 * vite.config.inject.ts as single-entry IIFE bundles, since a content
 * script (either JS world) is always loaded as a classic script, never a
 * module - see those files' comments. All three configs write into the
 * same dist/, chained by the "build" npm script; this one runs first and
 * owns emptyOutDir.
 */
export default defineConfig({
  root: here,
  plugins: [react()],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    rollupOptions: {
      input: {
        background: resolve(here, "src/background.ts"),
        popup: resolve(here, "popup.html")
      },
      output: {
        entryFileNames: "[name].js",
        chunkFileNames: "chunks/[name]-[hash].js",
        assetFileNames: "assets/[name][extname]"
      }
    }
  }
});
