import path from "node:path";
import { configDefaults, defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  test: {
    environment: "jsdom",
    exclude: [
      ...configDefaults.exclude,
      "scripts/**/*.test.mjs",
      "**/.pnpm-store/**",
      // Nested worktrees ship their own node_modules, so collecting their copy
      // of a test loads a second React and every render dies on a null
      // dispatcher. Both layouts are in use: `.worktrees/` at the root and
      // `.claude/worktrees/` for agent branches.
      "**/.worktrees/**",
      "**/.claude/worktrees/**",
    ],
    setupFiles: ["./tests/setupGlobals.ts", "./tests/setupTests.ts"],
    globals: true,
    coverage: {
      reporter: ["text", "lcov"],
    },
  },
});
