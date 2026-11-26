---
type: "Reference"
title: "持久化 — 快照帧、输入日志与回放定位"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T07:10:30.723Z
sources:
  - id: openwiki-source-6cc1016021f1acaff9cc0b8b
    resource: repo://crates/ygg-core/src/kv.rs
  - id: openwiki-source-2525bb44114bad58534d56ee
    resource: repo://crates/ygg-host/src/proto.rs
  - id: openwiki-source-682e02a9e393e886de83488d
    resource: repo://crates/ygg-host/src/session.rs
  - id: openwiki-source-2696e2f7febd691f8cbc479e
    resource: repo://crates/ygg-host/tests/replay_tamper.rs
  - id: openwiki-source-0ddc079b1076812ea4f7471c
    resource: repo://crates/ygg-sim/src/canonical.rs
  - id: openwiki-source-b888e3dc95ca0a07465ab295
    resource: repo://crates/ygg-sim/src/snapshot.rs
  - id: openwiki-source-b0d308427c9d0c9eb125b7ed
    resource: repo://scripts/persist-matrix.sh
generated: { by: "pi", at: "2026-11-26T07:10:30.723Z" }
---

# 持久化 — 快照帧、输入日志与回放定位

> **id** `contracts/kernel/persistence` · **模块** kernel · **触发** 改存档格式、加字段、写回放/续跑、排查「存档读不回来」
> **读不到代价** `irreversible`（改格式而不升版本 → 旧存档静默读成另一个世界；回放语义写错 → 篡改无法定位）

## 1. 两件东西，两种用途

| 产物 | 是什么 | 用来做什么 |
| --- | --- | --- |
| **快照帧** | 某一 tick 的完整世界字节 | 存档、中途接管、跨进程对账 |
| **输入日志** | 每 tick 的 dt + 宿主操作 + 该 tick 的哈希 | 回放复算、篡改定位、观战 |

两者都不含规则代码：快照 + 同一个包 = 同一个世界；日志 + 同一个种子 = 同一条轨迹。

## 2. 快照帧逐字节布局

```text
magic        8B   "YGGSIM\0\x01"
fmt_version  2B   LE = 2（读取器收 {1,2}）
flags        2B   LE = 0
tick         8B   LE
next_entity  8B   LE
comp_n       4B   LE
  × comp_n: name_len 2B | name UTF-8 | type 1B(1..6) | frac_bits 1B     ← 按 name 字节序升序
res_n        4B   LE                                                   ← 键升序
  × res_n: key 8B | value 8B
rng_seed     8B   LE
rng_n        4B   LE                                                   ← 流名升序
  × rng_n: name_len 2B | name UTF-8 | w0..w3（各 4B LE）
arch_n       4B   LE                     ← 按「升序组件名串联」的字节序升序
  × arch_n: comp_cnt 2B | comp_idx 2B×comp_cnt | row_n 4B
           | ids: u64×row_n | 逐列载荷（顺序 = comp_idx 顺序）:
               I64/U64/F64Bits 8B×row_n · U8 1B×row_n · Bytes (len 4B | bytes)×row_n
digest       8B   LE  = fnv1a64(前面全部字节) = state hash
```

三条被测试钉死的性质：**字节稳定**（同态两次编码逐字节相同）、**往返幂等**（`decode(b).encode()==b`）、**名称键控**（注册顺序不同但组件集相同的两个进程，快照字节与哈希完全相同）。

**为什么自描述而不是编译期 schema**：存档要被另一个进程读，而内部组件 id 与注册序是进程局部的；把名字与类型写进帧，就少一条「两边必须同序注册」的隐式契约。

## 3. 版本策略：`YGG-SIM-FORMAT` vs `YGG-SIM-FRAME`

| 症状 | 码 | 处置 |
| --- | --- | --- |
| `fmt_version` 不在 `{1,2}` | `YGG-SIM-FORMAT` → 协议面 `E_STORE_FORMAT` | **升级存档**：旧代码读不了新格式，这是版本问题 |
| magic / 长度 / 尾 digest / 列类型不符 | `YGG-SIM-FRAME` → `E_PACK`/`E_STATE` | **文件坏了**：重存或放弃 |

- v1（2026-11-21）：列型 1..5；v2（本版）加列型 6 = `F64Bits`（8B 位串），其余布局不变；
- 新代码读 v1 帧是**原语义**（Number 列照旧 `I64`），读回后以 v2 重编码为准；
- **跨 `fmt_version` 的 state hash 不承诺相等**——这正是帧版本化的本义；
- 加字段就必须升 `fmt_version` 并同步 `canonical::FMT_VERSION`，否则就是「同版本两种字节」。

## 4. 输入日志与回放定位

```text
Log { header: { seed, start_tick }, frames: [ { tick, dt, ops: [Op], hash } ] }
```

- 回放 = 在**同源内核**（`from_bundle` 的显式种子变体，种子取自 `header.seed`）上按帧重放 `ops`，每帧重算哈希并与日志里的 `hash` 比；
- 第一处不等即停，答 `ReplayResult { ok: false, first_bad_tick: Some(t) }`——**定位到帧**，不是「日志不一致」；
- `start_tick != 0` 直接 `ok:false`（没有起点状态的日志无法独立复算）；
- 截断的日志是**合法前缀**：回放到第 k 帧仍 `ok:true`，只是没跑完；
- 四类篡改各有一个用例（未篡改 / 改操作值 / 改哈希 / 截断）：`crates/ygg-host/tests/replay_tamper.rs`。

## 5. KV 为什么不进快照

`world.kv` / `ygg.kv` 存的是**不属于世界**的簿记（宿主注入的环境量、规则自己的小状态）。它不进快照、不进 `state_hash`、不被回放复算——理由只有一条：**能影响玩法的东西必须在列存里**，否则「同一份日志重放出不同世界」。这条不是优化，是回放语义的前提。

## 6. 工程验证

| 检查 | 命令 |
| --- | --- |
| 快照往返与字节稳定 | `cargo test -p ygg-sim --test snapshot_roundtrip` |
| 篡改定位 | `cargo test -p ygg-host --test replay_tamper` |
| 10 万 tick 三端一致 | `bash scripts/persist-matrix.sh`（原生 / wasm / 墙钟三档同 hash `15126981819641904924`） |
| 帧重生成 | `cargo run -q --example golden -p ygg-sandbox -- packs/example --ticks 100000 --final-only` |

## 7. 维护责任

- **触发**：改帧布局或列型、改 `FMT_VERSION`、改回放判据、改日志线格式。
- **检查**：上面四条命令，外加 `cargo test -p ygg-sandbox --test example_pack`（golden 逐 tick）与 `cargo test -p ygg-wasm`（三绑定 parity）。
