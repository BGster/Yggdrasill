# ygg-wasm

`ygg/1` 协议的 WASM 绑定（wasm-bindgen）——**只搬字节，不碰语义**。协议逻辑全在
[`ygg-host`](../ygg-host)：镜像解析（`image`）、状态机（`Session`）、帧切分与帧流打包
（`proto::encode_frames`）。本 crate 的方法体只有 `self.inner.<method>`。

## 导出面

| 成员 | 说明 |
| --- | --- |
| `new YggSession(packBytes)` | 包镜像 → 未握手会话。`packBytes` 是镜像 JSON 的字节：`{"modSource": "<Lua 源码>", "manifest": "<manifest.json 文本；空串 = 无资源>", "schemas": ["<schema 文本>", …], "resources": {"<path>": "<base64>", …}}`（形状由 `ygg_host::image` 定义；wasm 侧没有目录可读，四件装载参数只能随字节过界）。**装载失败 throw**——镜像不在协议面上，没有帧可答 |
| `handle(msgJson) -> string` | 一条消息 JSON → **一个 JSON 数组**的帧流（`step` 两帧 `events` + `delta`，其余一帧） | 
| `snapshot() -> Uint8Array` | 当前世界帧字节（`snapshot` 消息的字节近路；失败答空数组——要「为什么」就走 `snapshot` 消息，那里有 `error` 帧） |

`handle` **永不 throw**：坏 JSON、状态不符、`pack_hash` 不符都答一帧
`error{code,msg}`。错误码决定宿主的动作（`E_PROTOCOL` 宿主自己的 bug、`E_PACK` 换包、
`E_RULE`/`E_BUDGET` 报给包作者、`E_STATE` 回滚重试）——所以错也是协议的一部分，JS 侧
不需要 `try/catch` 才能继续下一步。

消息表与字段见 [`packages/schema/protocol/README.md`](../../packages/schema/protocol/README.md)。
三条载体（Rust `Session` / WASM `YggSession` / C ABI）跑**同一份**脚本
（`packages/schema/protocol/parity-script.json`），回执必须逐帧相同：
[`tests/parity.rs`](tests/parity.rs)。

握手一步（之后才接受世界面消息）：

```js
const session = new ygg.YggSession(new TextEncoder().encode(JSON.stringify({
  modSource, manifest, schemas, resources,
})));
const hello = session.handle(JSON.stringify({
  type: "hello", protocol: "ygg/1", codec: "json",
  pack_id: "ygg:demo", pack_hash: manifestHash,   // manifest 文本的 SHA-256
}));
const [ready] = JSON.parse(hello);                // ready.tick / ready.hash / ready.sealed
const frames = JSON.parse(session.handle('{"type":"step","ticks":1,"dt":16}'));
// frames[0].type === "events"，frames[1].type === "delta"（只含本步被写过的组件）
```

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
const session = new ygg.YggSession(imageBytes);
const reply = session.handle(JSON.stringify({ type: "hello", /* … */ }));
const frame = session.snapshot();        // 世界帧字节（存档面）
```

宿主自带 WASI 实现时，shim 不覆盖已存在的 `wasi_snapshot_preview1` 键。

## 活链验证

```sh
cargo test -p ygg-wasm           # host 同构编译：透传面 + golden 向量 + parity
```

`tests/parity.rs` 在本机（host 目标）跑三载体脚本的 WASM 一支；浏览器侧的双端矩阵
随 M4 落地。

## 体积快照（M1 C2 时点）

- `ygg_wasm_bg.wasm` ≈ 2.9 MB raw（Lua 5.4 VM + mlua + jsonschema/fancy-regex；lto=fat + opt-level=s）。
- **wasm-opt 已兼容（M1 C2 实证）**：wasm-bindgen 剥掉 target-features 元
  数据，须显式传特性旗（脚本头注有实测集）；binaryen 117（wasm-pack 缓存
  版）优化必崩（DCE 对 try_table 未实现路径），binaryen 133 -Oz 成功：
  3,060,338 → 2,734,045 字节（-10.6%）。`scripts/build-wasm.sh` 在
  PATH 上有 wasm-opt 时自动 -Oz（`YGG_WASM_OPT=off/on` 可关/令致命）。
- 体积回归门（twiggy/wasm-tools 入 CI）按 research §7 留 M1 泳道，非门禁。
