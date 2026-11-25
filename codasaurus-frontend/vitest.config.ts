import { mergeConfig } from "vite"
import { configDefaults, defineConfig } from "vitest/config"

import viteConfig from "./vite.config.ts"

export default defineConfig(
  mergeConfig(viteConfig, {
    test: {
      environment: "jsdom",
      setupFiles: "./src/tests/setup.ts",
      exclude: [...configDefaults.exclude, "src/tests/e2e/**"],
    },
  })
)
