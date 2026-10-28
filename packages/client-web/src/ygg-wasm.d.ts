// ============================================================================
// ygg-wasm.d.ts — @ygg/* 别名模块的环境类型面（tsc --noEmit 用）。
//
// 为什么手写：wasm 产物 crates/ygg-wasm/pkg/ 不入 git（.gitignore），真实
// 类型源 pkg/ygg_wasm.d.ts 只在构建后存在——typecheck 门禁不得依赖构建序。
// 本文件声明 vite resolve.alias（vite.config.ts）指向的两个模块的最小消费面：
//   "@ygg/wasm"      → crates/ygg-wasm/pkg/ygg_wasm.js（wasm-bindgen --target web 胶水）
//   "@ygg/wasm-shim" → crates/ygg-wasm/wasi-shim.js（preview-1 shim）
// 字段/方法名与壳 crate 文档（crates/ygg-wasm/src/lib.rs 模块注）逐字对齐；
// 真实产物 d.ts 与本文件的漂移由 B1 smoke（node 活链）兜底。
// ============================================================================

declare module "@ygg/wasm" {
  /**
   * 胶水默认导出 = init（wasm-bindgen --target web）：拉取并实例化
   * ygg_wasm_bg.wasm。必须在 installWasiShim() 之后调用（实例化期的
   * preview-1 import 合并与模块 memory 捕获依赖该补丁）。
   */
  export default function init(module_or_path?: unknown): Promise<void>;

  /**
   * 壳导出结构（KernelFacade 的 wasm 绑定镜像）。构造期域失败 /
   * marshal 失败 **throw**（值 = A1 错误信封 JSON 文本或定位文案）；
   * dispatch / save / load / state 永不 throw，返回信封 JSON 原文。
   */
  export class YggKernel {
    constructor(
      mod_source: string,
      manifest_json: string,
      schemas_json: string,
      resources_json: string,
    );
    /** 唯一同步入口（A1）：命令分发，返回信封 JSON 原文。 */
    dispatch(cmd: string, payload: string): string;
    /** `save` 镜像（= dispatch("save","")）：完整状态快照信封。 */
    save(): string;
    /** `load` 镜像（= dispatch("load", save_json)）：存档 JSON 载入。 */
    load(save_json: string): string;
    /** `state` 镜像（= dispatch("state","")）：诊断信封。 */
    state(): string;
  }
}

declare module "@ygg/wasm-shim" {
  /**
   * 幂等安装：补丁 WebAssembly.instantiate(/Streaming)，把 preview-1 shim
   * 并入 importObject（宿主已带 wasi_snapshot_preview1 时不覆盖）并捕获
   * 模块 memory。任何 init 调用前必须先执行一次。
   */
  export function installWasiShim(): void;
}
