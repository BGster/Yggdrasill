// M1 闭环集成（真实三道门子进程，需要 Rust 工具链；真实 LLM 不在环——
// provider 契约由 mock 承载）。单独脚本跑：pnpm -C packages/pipeline run test:integration
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "node",
    include: ["test/integration.generate.test.ts"],
    testTimeout: 300_000,
  },
});
