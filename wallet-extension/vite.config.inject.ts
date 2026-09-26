import { defineConfig } from "vite";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { copyFileSync } from "node:fs";

const here = fileURLToPath(new URL(".", import.meta.url));

/**
 * Builds inject.js as a standalone IIFE bundle - see src/inject.ts's
 * comment for why this can't be a module, and vite.config.content-script.ts
 * / vite.config.ts for the other two entries. Runs last in the "build" npm
 * script (does not empty dist/), so manifest.json is copied here.
 */
export default defineConfig({
  root: here,
  plugins: [
    {
      name: "copy-manifest",
      closeBundle() {
        copyFileSync(resolve(here, "manifest.json"), resolve(here, "dist/manifest.json"));
      }
    }
  ],
  build: {
    outDir: "dist",
    emptyOutDir: false,
    rollupOptions: {
      input: { inject: resolve(here, "src/inject.ts") },
      output: { format: "iife", entryFileNames: "[name].js" }
    }
  }
});
