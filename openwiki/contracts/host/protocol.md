---
type: "Reference"
title: "集成协议 — ygg/1 消息、状态机与拒绝规则"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T07:10:30.723Z
sources:
  - id: openwiki-source-2525bb44114bad58534d56ee
    resource: repo://crates/ygg-host/src/proto.rs
  - id: openwiki-source-682e02a9e393e886de83488d
    resource: repo://crates/ygg-host/src/session.rs
generated: { by: "pi", at: "2026-11-26T07:10:30.723Z" }
---

# 集成协议 — ygg/1 消息、状态机与拒绝规则

> **id** `contracts/host/protocol` · **模块** host · **触发** 接一个宿主、加/改消息变体、排查「宿主发的帧被拒」
> **读不到代价** `hard-refusal`（宿主按错的时序或字段面发帧 → 每帧都答 `error`，且看不出是时序错还是字段错）

## 1. 三个状态

| 状态 | 进入 | 允许的消息 | 其他消息的答案 |
| --- | --- | --- | --- |
| `Fresh` | `Session::new` / `from_image` | `hello` | `error{E_PROTOCOL}`「未握手：先发送 hello 再发 …」 |
| `Ready` | `hello` 三项对账全过 | 除 `hello` 外全部 | 重复 `hello` 答 `error{E_PROTOCOL}` |
| `Closed` | 宿主调 `close()` | 无 | `error{E_STATE}`「会话已关闭」 |

`hello` 对账三项：`protocol == "ygg/1"`、`codec == "json"`、`pack_hash == 已装载包的 manifest 摘要`。任一不符**不改变状态**（停在 `Fresh`，可改正后重试）；全过后才进 `Ready` 并答 `ready{tick,hash,sealed,components,systems}`。

`hello` 的 `seed` 可选：`None` 沿用内核默认种子；`Some(seed)` 在对账全过后、进 `Ready` 前按该种子重建一条干净世界谱系（重建失败也不改变状态）。这是宿主指定创世种子的唯一入口——先 `hello` 探指纹、再带 `seed` 重发，是 `hosts/h5` 的标准握手。

## 2. 消息表（12 个变体 / 13 个线上 tag —— `snapshot` 两个方向各算一个）

| 消息 | 必填字段 | 前置 | 答案 |
| --- | --- | --- | --- |
| `hello` | `protocol` `codec` `pack_id` `pack_hash`（`seed` 可选） | `Fresh` | `ready` 或 `error` |
| `ready` | `tick` `hash` `sealed` `components` `systems` | 内核 → 宿主 | — |
| `open` | `bytes`（base64 快照） | `Ready` | `ready` |
| `submit` | `ops`（`spawn`/`despawn`/`set`/`insert`/`remove`） | `Ready` | `delta` |
| `command` | `name` `args`（对象） | `Ready` 且包有命令面 | `delta` |
| `step` | `ticks`（≥1）`dt` | `Ready` 且 `sealed == true` | **两帧**：`events` 然后 `delta` |
| `events` | `tick` `hash` `log[]` `errors[]` | 内核 → 宿主 | — |
| `delta` | `tick` `changed[{eid,comps}]` | 内核 → 宿主 | — |
| `snapshot` | `direction`: `request` | `Ready` | `snapshot{direction:reply,tick,hash,bytes}` |
| `replay` | `log{header,frames}` | `Ready` | `replay_result` |
| `replay_result` | `ok` `first_bad_tick` | 内核 → 宿主 | — |
| `error` | `code` `message` | 任一状态 | — |

`snapshot` 的 `reply` 方向若由宿主发来，答 `error{E_PROTOCOL}`「snapshot 的 reply 帧是内核回执，宿主不该发送」。

## 3. 拒绝规则（谁来拒）

| 症状 | 拒在哪 | 答案 |
| --- | --- | --- |
| 多了未知字段 | `proto::decode`（`deny_unknown_fields`） | `error{E_PROTOCOL}` |
| `value` 里塞浮点（`1.5`，或 `1e3`） | 反序列化层 | `error{E_PROTOCOL}` |
| 未知 `type` / 未知 `op` | 反序列化层 | `error{E_PROTOCOL}` |
| `ticks = 0` | 内核 `step` | `error{E_PROTOCOL}` |
| 未 `seal` 就 `step` | `Session::step`（先于内核） | `error{E_RULE}` |
| 包没有命令面 / 未知命令名 | 内核 `Loaded::command` | `error{E_STATE}` |
| 命令被规则拒绝（`false, 原因`） | 内核 | `error{E_RULE}`「E_STEP_REFUSED: …」 |
| 指令预算耗尽 | 内核 | `error{E_BUDGET}` |
| 快照格式不认识 | 内核 | `error{E_STORE_FORMAT}` |

规矩：**能由反序列化判的，绝不进会话**；会话只判状态与时序；内核判世界。三层各答各的 `E_*`，同一个症状不会在两处给出不同码。

## 4. `step` 为什么是两帧

`events` 在 `delta` 之前，顺序即线上顺序：先给「这一步发生了什么」（日志行、错误），再给「世界变成了什么样」（增量单元格）。宿主渲染只要 `delta`，诊断要 `events` —— 但两者必须来自**同一次**推进，所以不能拆成两条消息。

`changed` 只列被写过的单元格（`{eid, comps:[cid…]}`），不含值：值由宿主按需 `snapshot` 或自行镜像。

## 5. `replay` 的判据是哈希，不是「没报错」

`replay` 用日志头的 `seed` 另起一条谱系，逐帧 `apply(ops) → step(1, dt)` 并比对帧哈希。

- **帧的 `tick` 是该帧开跑前的内核时钟**（创世后第一帧是 `tick = 0`），所以复算要求「当前 tick == `frame.tick`」，跳号与倒序都算第一处不符；
- `first_bad_tick` 报的就是这个字段原值；
- v1 的 `replay` 不带起始快照，`start_tick != 0` 直接答 `ok:false`（不猜）；
- 不改变世界的篡改（例如把 `dt` 改小到系统无感知）在 v1 不可检测——这是日志面本来的强度，写在这里是为了不让下一个人以为它是完整性证明。

## 6. 错误码归属

| 码 | 谁产生 | 典型原因 |
| --- | --- | --- |
| `E_PROTOCOL` | 解码层、会话、内核入口 | 未知字段/浮点/未知 tag、时序错、`ticks=0`、`reply` 方向错 |
| `E_PACK` | 装载期 | 包镜像坏、manifest 摘要不符、模块名不合法 |
| `E_RULE` | 规则层 | 未 `seal` 就 `step`、命令被拒、Lua 抛错 |
| `E_BUDGET` | 规则层 | 单次入口超过指令预算 |
| `E_STATE` | 会话/内核 | 会话已关闭、未知命令、内核未挂载 |
| `E_STORE_FORMAT` | 持久层 | 快照帧 `format` 不是 2 |

## 7. 维护责任

- **触发**：加消息变体、改字段、改状态机、改错误码。
- **检查**：`cargo test -p ygg-host`（`session` / `proto_roundtrip` / `schema_conformance` / `replay_tamper`）、`cargo test -p ygg-capi -p ygg-wasm`（parity）。
