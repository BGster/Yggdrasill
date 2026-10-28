// wasm-stub.ts — vitest 模式下 "@ygg/wasm" 别名的落点（见 vite.config.ts：
// mode === "test" 时 alias 指向本文件）。
//
// 为什么需要：wasm 产物 crates/ygg-wasm/pkg/ 不入 git，真实胶水只在
// build-wasm.sh 后存在；vite 的 import-analysis 在**转换期**静态解析动态
// 导入——产物缺失时 kernel-wasm.ts 的 `import("@ygg/wasm")` 会让所有触链
// 测试文件收集失败。本桩只满足解析面：正常单测（纯逻辑）永不执行
// create()；误触装载路径时 reject → App 降级回放模式（确定性不变）。
//
// 生产构建 / dev（mode != test）不走本文件——alias 指向真实 pkg 胶水。

export default async function init(): Promise<never> {
  throw new Error("测试桩：wasm 胶水在 vitest 下不可用（真实装载走浏览器 / node smoke 路径）");
}

export class YggKernel {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  constructor(..._args: never[]) {
    throw new Error("测试桩：YggKernel 在 vitest 下不可用");
  }

  dispatch(_cmd: string, _payload: string): string {
    throw new Error("测试桩：YggKernel.dispatch 在 vitest 下不可用");
  }

  save(): string {
    throw new Error("测试桩：YggKernel.save 在 vitest 下不可用");
  }

  load(_saveJson: string): string {
    throw new Error("测试桩：YggKernel.load 在 vitest 下不可用");
  }

  state(): string {
    throw new Error("测试桩：YggKernel.state 在 vitest 下不可用");
  }
}
