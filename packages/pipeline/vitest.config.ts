// 单测门禁（快）：mock provider / fake runner / 纯函数；不真跑 cargo，
// 不依赖 Rust 工具链。集成闭环（真实三道门子进程）在 vitest.integration.config。
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "node",
    include: ["test/*.test.ts"],
    exclude: ["**/node_modules/**", "test/integration.generate.test.ts"],
  },
});
