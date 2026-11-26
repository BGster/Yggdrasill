---
type: "Reference"
title: "绑定 — Rust / WASM / C ABI 三载体"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T07:10:30.723Z
sources:
  - id: openwiki-source-385bd2486ec412a21045c970
    resource: repo://crates/ygg-capi/src/lib.rs
  - id: openwiki-source-6994426a6fd34e84009762a9
    resource: repo://crates/ygg-capi/tests/parity.rs
  - id: openwiki-source-2525bb44114bad58534d56ee
    resource: repo://crates/ygg-host/src/proto.rs
  - id: openwiki-source-61a53c8e1a9f6204afd357f2
    resource: repo://crates/ygg-wasm/src/lib.rs
  - id: openwiki-source-6428eaa80b3d50daaadfebfd
    resource: repo://crates/ygg-wasm/tests/parity.rs
  - id: openwiki-source-25d1582f0c2b2f36d69217bd
    resource: repo://crates/ygg-wasm/wasi-shim.js
  - id: openwiki-source-fe085e18d1b6157ca615fbb7
    resource: repo://packages/schema/protocol/parity-script.json
  - id: openwiki-source-6a5d7b301b84949a383f444c
    resource: repo://scripts/build-wasm.sh
generated: { by: "pi", at: "2026-11-26T07:10:30.723Z" }
---

# 绑定 — Rust / WASM / C ABI 三载体

> **id** `contracts/host/bindings` · **模块** host · **触发** 接新宿主、加一种语言绑定、改 C ABI 或 wasm 产物
> **读不到代价** `hard-refusal`（绑定里塞语义 → 三条载体在某平台上分叉，且分叉只在宿主侧暴露）

## 1. 三条载体，一个会话

| 载体 | 入口 | 形态 |
| --- | --- | --- |
| Rust | `ygg_host::Session` | 直接调 `handle(&Message) -> Vec<Message>` |
| WASM | `YggSession`（`wasm-bindgen`） | `new(packBytes)` / `handle(json) -> json` / `snapshot() -> Uint8Array` |
| C ABI | `crates/ygg-capi`（`cdylib` + `staticlib`） | 六个 `extern "C"` 符号，字符缓冲区进出 |

三者是**同一个 `Session`** 的三层皮：没有第二份状态机、没有第二份校验、没有第二份错误映射。

## 2. 零语义

绑定层允许做的事只有三件：搬字节、转编码（UTF-8/UTF-16）、把错误字符串交还调用方。

- 不许在绑定里做「顺手」的默认值填充（如缺 `dt` 补 1）——默认值是协议的事，不是绑定的事；
- 不许在绑定里做字段校验或版本判断——判据在 `proto::decode`（未知字段、浮点、未知 tag 全在那一层拒）；
- 不许在绑定里缓存会话状态——状态在 `Session::state`，绑定只持有一个不透明句柄。

违反症状：同一段协议字节在 Rust 通过、在 wasm 被拒（或反之），而 `crates/*/tests/parity.rs` 不会红，因为分叉发生在还没到协议层的地方。

## 3. C ABI 的六个符号与三条边界规则

| 符号 | 面 |
| --- | --- |
| `ygg_protocol_version()` | 返回 `"ygg/1"`（静态区，不要释放） |
| `ygg_session_new(pack, pack_len)` | 包镜像 → 未握手会话；失败返回 `NULL` + `ygg_last_error()` |
| `ygg_session_handle(session, msg_json)` | 一段 JSON 进、一帧或多帧 JSON 数组出 |
| `ygg_session_free(session)` | 释放会话 |
| `ygg_string_free(s)` | 释放本库返回的任一字符串 |
| `ygg_last_error()` | 最近一次「只能返回 NULL」的原因 |

三条规则：

1. **进的是 UTF-8 字节，出的是调用方所有的 NUL 结尾字符串**，用完一律 `ygg_string_free`。
2. **边界上没有结构体过境**：没有 out 参数、没有回调、没有指针数组。结构体一过境，对齐、`bool` 宽度、`size_t` 宽度就会在宿主的第二个平台上变成「协议对不上」。
3. **空指针与非 UTF-8 不 panic**：一律走 `NULL` + `ygg_last_error()`，panic 跨 FFI 是未定义行为。

## 4. WASM 的两个非显然点

- **为什么要 WASI shim**：vendored Lua 会调 `clock_time_get` 等 WASI 导入，而 `wasm32-unknown-unknown` 没有这些导入的实现。`crates/ygg-wasm/wasi-shim.js` 提供确定性的假实现（时钟固定、随机不走），使同一段脚本在浏览器与原生得到同一字节。
- **为什么需要 `__yggBindMemory`**：`scripts/build-wasm.sh` 把 WASI 导入重写成静态 ESM，胶水里的 `instance.exports` 赋值之后必须把 `memory` 交给 shim，否则 shim 拿到的是 `null` 并在第一次 `clock_time_get` 崩。绑定点必须在 `__wbindgen_start()` **之前**（构建脚本用 perl 注入并断言只命中一次）。

## 5. 等价性怎么被机器证明

`packages/schema/protocol/parity-script.json` 是一段**协议字节序列**（hello → submit → step → command → snapshot）+ `expected_types`。

- `crates/ygg-capi/tests/parity.rs` 与 `crates/ygg-wasm/tests/parity.rs` 各自 `include_str!` 同一份脚本（编译期嵌入，改脚本即两边同时变）；
- 两边都断言：逐帧消息类型序列等于 `expected_types`，且**逐帧 JSON 字节相同**；
- `expected_types` 的作用是防退化：脚本若被打空，类型序列先红，不会「三条载体一起同意什么都没发生」。
- 浏览器侧另有一条泳道：`node scripts/wasm-lane.mjs`（默认逐 tick 比对 `packs/example/golden.json`，`--ticks N` 打最终哈希），`scripts/persist-matrix.sh` 用它对原生做 1k/10k/100k 三档同哈希比对。

## 6. 加一种新载体的四步义务

1. 只包 `Session`，不重写协议；
2. 跑同一份 `parity-script.json`，字节相同；
3. 在 `scripts/persist-matrix.sh`（或同形脚本）里加一道长跑比对；
4. 在本文 §1 的表里加一行——否则下一个人会以为只有三条载体。

## 7. 维护责任

- **触发**：改 `Session` 的公开面、改 `proto` 的消息集、加载体、改 wasm 构建链。
- **检查**：`cargo test -p ygg-capi -p ygg-wasm`（两条 parity）、`node scripts/wasm-lane.mjs`、`bash scripts/persist-matrix.sh`。
