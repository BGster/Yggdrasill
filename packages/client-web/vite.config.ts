import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import { generatedModsMiddleware } from "./vite.plugin.mods";

// B2 面板：Vite 构建 + vitest（jsdom）单测；信封 fixture 由 vite 原生 JSON 导入承载。
//
// B3 追加：
// - alias "@ygg/wasm"      → crates/ygg-wasm/pkg/ygg_wasm.js（wasm-bindgen --target web
//   胶水；产物 pkg/ 不入 git，kernel-wasm.ts 以**动态导入**消费——产物缺失时
//   仅该动态路径失败，模块加载 / 回放模式不受影响）；
// - alias "@ygg/wasm-shim" → crates/ygg-wasm/wasi-shim.js（preview-1 shim，静态导入）；
// - **test 模式例外**：vitest（mode === "test"）把 "@ygg/wasm" 指向本地测试桩
//   src/wasm-stub.ts——vite 的 import-analysis 在转换期静态解析动态导入，
//   产物缺失（未跑 build-wasm.sh）会让全部触链测试文件收集失败；单测门禁
//   不得依赖 wasm 构建序。生产构建 / dev 走真实胶水；
// - server.fs.allow 放行仓库根（dev / vitest 需跨包读 mods 资源与 crates 胶水；
//   pnpm-workspace.yaml 本就使仓库根成为缺省 workspace 根，此处显式固化）；
// - base "./"：相对 base——GitHub Pages 项目站（/<repo>/ 前缀）安全；
//   build.outDir dist（Pages workflow 上传目录）。

const repoRoot = fileURLToPath(new URL("../..", import.meta.url));

export default defineConfig(({ mode }) => {
  const wasmAlias =
    mode === "test"
      ? fileURLToPath(new URL("./src/wasm-stub.ts", import.meta.url))
      : fileURLToPath(new URL("../../crates/ygg-wasm/pkg/ygg_wasm.js", import.meta.url));
  return {
    plugins: [react(), generatedModsMiddleware()],
    base: "./",
    resolve: {
      alias: {
        "@ygg/wasm": wasmAlias,
        "@ygg/wasm-shim": fileURLToPath(
          new URL("../../crates/ygg-wasm/wasi-shim.js", import.meta.url),
        ),
      },
    },
    server: {
      fs: {
        allow: [repoRoot],
      },
    },
    build: {
      outDir: "dist",
    },
    test: {
      environment: "jsdom",
      include: ["src/**/*.test.{ts,tsx}"],
    },
  };
});
