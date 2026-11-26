# ygg-wasm

wasm 壳（B1 实装）：[`ygg-sandbox`](../ygg-sandbox) `Loaded` 的
wasm-bindgen 同步导出面（M1-E 收紧：**只有装载 / 推进 / 快照**，没有事件面）。

## 导出面

| 成员 | 说明 |
| --- | --- |
| `new YggWorld(imageBytes)` | 装载 Mod 镜像。`imageBytes` 是镜像 JSON 的字节：`{"modSource": "<Lua 源码>", "manifest": "<manifest.json 文本；空串 = 无资源>", "schemas": ["<schema 文本>", …], "resources": {"<path>": "<base64>", …}}`（无文件系统的 wasm 侧没有目录可读，四件装载参数只能随字节过界） |
| `step(ticks, dt) -> string` | 推进 `ticks` 个 tick（每 tick `dt` 整数秒），返回推进后的状态哈希（64 位小写 hex；与 native `hash::state_hash` 同一字节序）。时间只由宿主推进 |
| `snapshot() -> Uint8Array` | 世界帧规范字节（同一状态 ⇒ 同一字节；哈希与存档共用这一个序列化面） |
| `YggWorld.restore(imageBytes, frame) -> YggWorld` | 按世界帧重建：镜像（世界定义来源）+ 帧（世界面）两个参数，对应 native `Loaded::restore(dir, bytes)`。坏帧先校验后改 |

**没有** `submit` / `events` / `drive` / `dispatch` 导出面：协议层（ygg/1）在 M3
落成后才经类型化接口加回来，现在不留占位壳。

失败一律 throw（构造、推进、坏帧都是）：现在没有稳定的错误线格式值得钉死，
信封是 M3 协议层的事。构造期输入失败（坏 JSON / 坏 base64 / 缺 `$id`）与装载域
失败（manifest / 资源包 / 入口 chunk / 模块表）都携带定位文案。

## 构建

```sh
scripts/build-wasm.sh          # 产物：crates/ygg-wasm/pkg/
```

前置与 WASI SDK 安装见脚本头注（实证链：WASI SDK 34 + rustc 1.99 + mlua
@ff96ed7 + wasm-bindgen 0.2.129）。关键点：

- **两步链替代 wasm-pack**：模块含 wasm 异常指令（wasi-libc EH setjmp），
  wasm-bindgen 0.2.129 的 catch wrappers 要求 externref 表——CLI 旗
  `--reference-types` 开启；wasm-pack 0.15 不透传该旗。
- **交叉编译环境**：`CC_wasm32_unknown_unknown` 等旗面固化在仓库
  `.cargo/config.toml`（缺省路径 `/opt/wasi-sdk`，`force = false` 允许环境
  覆盖）；本机 SDK 在别处时导出 `WASI_SDK_PATH` 即可（脚本处理）。
- `pkg/` 为构建产物（`.gitignore` 内），不进版本库。

## 宿主集成（WASI shim）

模块 import `wasi_snapshot_preview1`（wasi C runtime 链路面），宿主必须提供。
`wasi-shim.js` 是实测 import 面的最小实现（无文件系统、stdout/stderr 转发
console、未知 import 兜底 ENOSYS）：

```js
import { installWasiShim } from "./wasi-shim.js";
installWasiShim();                       // 必须先于 pkg 胶水 init
const ygg = await import("<pkg>/ygg_wasm.js");
await ygg.default(wasmBytes);            // 或无参（浏览器 fetch 相对路径）
const world = new ygg.YggWorld(new TextEncoder().encode(JSON.stringify({
  modSource, manifest, schemas, resources,
})));
const stateHash = world.step(1, 16);      // 推进一个 tick，取状态哈希
const frame = world.snapshot();          // 世界帧字节
const rebuilt = ygg.YggWorld.restore(image, frame);
```

宿主自带 WASI 实现时，shim 不覆盖已存在的 `wasi_snapshot_preview1` 键。

## 活链验证

捆绑模块（`wasm-pack` 产物）由宿主侧用例驱动；跨端哈希等价用例随 M4.3 双端矩阵落地。
目前在仓库内可跑的验证是 crate 自身用例：

```sh
cargo test -p ygg-wasm
```

## 体积快照（M1 C2 时点）

- `ygg_wasm_bg.wasm` ≈ 2.9 MB raw（Lua 5.4 VM + mlua + jsonschema/fancy-regex；lto=fat + opt-level=s）。
- **wasm-opt 已兼容（M1 C2 实证）**：wasm-bindgen 剥掉 target-features 元
  数据，须显式传特性旗（脚本头注有实测集）；binaryen 117（wasm-pack 缓存
  版）优化必崩（DCE 对 try_table 未实现路径），binaryen 133 -Oz 成功：
  3,060,338 → 2,734,045 字节（-10.6%）。`scripts/build-wasm.sh` 在
  PATH 上有 wasm-opt 时自动 -Oz（`YGG_WASM_OPT=off/on` 可关/令致命）。
- 体积回归门（twiggy/wasm-tools 入 CI）按 research §7 留 M1 泳道，非门禁。
