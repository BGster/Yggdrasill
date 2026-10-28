# ygg-wasm

wasm 壳（B1 实装）：[`ygg-sandbox`](../ygg-sandbox) `KernelFacade` 的
wasm-bindgen 同步导出面（D-A1 线格式）。

## 导出面

| 成员 | 说明 |
| --- | --- |
| `new YggKernel(modSource, manifestJson, schemasJson, resourcesJson)` | 构造内核。`manifestJson` 空串 = 无资源路径（此时资源参数被忽略）；否则资源包装载（schema 文本数组取 `$id` 注册、`path → base64` 资源对象解码） |
| `dispatch(cmd, payload) -> string` | 唯一同步入口，返回 A1 JSON 信封原文。**永不 throw**；域错误码 `FUEL_EXHAUSTED / SANDBOX_DENIED / VALIDATION / KV_LIMIT / INTERNAL` |
| `save() / load(saveJson) / state()` | 保留命令一行镜像（= `dispatch` 同名命令） |
| `stateHash() -> string` | 状态哈希度量面（FR-9）：`hex(SHA-256(canonical_state_bytes))`，A1 信封承载（`{"ok":true,"stateHash":"…"}` / INTERNAL 错误信封）。**永不 throw**（M1 C2） |

构造期失败经 throw 交付：marshal 失败（坏 base64 / 缺 `$id` / 非法 JSON）带
定位文案；装载域失败（ModLoad / 资源包拒绝）抛 **A1 错误信封 JSON 文本**
（消费方在 init 处 catch 一次即可统一按信封分支）。

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
const kernel = new ygg.YggKernel(modSource, manifestJson, schemasJson, resourcesJson);
const envelope = kernel.dispatch("event.start", "0");
```

宿主自带 WASI 实现时，shim 不覆盖已存在的 `wasi_snapshot_preview1` 键。

## 活链验证

```sh
node crates/ygg-wasm/smoke.mjs           # 装载 packages/mods/test-adventure，
                                         # dispatch event.start，断言叙事 +
                                         # 选择挂起 + 存读档往返
node crates/ygg-wasm/verify-determinism.mjs
                                         # FR-9 wasm 侧双跑（M1 C2）：golden-vector
                                         # 同序列经 wasm 内核双跑，哈希必须等于
                                         # fixture 钉死的 native 基线
```

## 体积快照（M1 C2 时点）

- `ygg_wasm_bg.wasm` ≈ 2.9 MB raw（Lua 5.4 VM + mlua + jsonschema/fancy-regex
  + ygg stdlib 嵌入；lto=fat + opt-level=s）。
- **wasm-opt 已兼容（M1 C2 实证）**：wasm-bindgen 剥掉 target-features 元
  数据，须显式传特性旗（脚本头注有实测集）；binaryen 117（wasm-pack 缓存
  版）优化必崩（DCE 对 try_table 未实现路径），binaryen 133 -Oz 成功：
  3,060,338 → 2,734,045 字节（-10.6%），优化产物过 smoke +
  verify-determinism 活链，golden 哈希不变。`scripts/build-wasm.sh` 在
  PATH 上有 wasm-opt 时自动 -Oz（`YGG_WASM_OPT=off/on` 可关/令致命）。
- 体积回归门（twiggy/wasm-tools 入 CI）按 research §7 留 M1 泳道，非门禁。
