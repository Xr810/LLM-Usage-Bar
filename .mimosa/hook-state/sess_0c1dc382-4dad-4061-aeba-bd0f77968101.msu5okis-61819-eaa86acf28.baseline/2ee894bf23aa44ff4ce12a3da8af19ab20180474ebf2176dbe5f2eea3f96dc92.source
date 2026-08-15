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
      // `pnpm rust` / `pnpm tauri` 把 cargo 的 target 目录放在 .cache/cargo-targets/
      // 下（见 AGENTS.md 与 scripts/cargo-cache.mjs）。那里是几 GB、几十万个编译
      // 产物文件，glob 走一遍要几分钟到几十分钟，表现为 vitest 启动后长时间零输出、
      // CPU 0%（卡在磁盘 I/O 而不是在跑测试）。里面不可能有测试，直接排除。
      "**/.cache/**",
    ],
    setupFiles: ["./tests/setupGlobals.ts", "./tests/setupTests.ts"],
    globals: true,
    coverage: {
      reporter: ["text", "lcov"],
    },
  },
});
