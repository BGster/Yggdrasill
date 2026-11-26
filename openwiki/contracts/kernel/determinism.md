---
type: "Reference"
title: "确定性 — 定点、规范化字节、哈希与随机流"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T07:10:30.723Z
sources:
  - id: openwiki-source-6cc1016021f1acaff9cc0b8b
    resource: repo://crates/ygg-core/src/kv.rs
  - id: openwiki-source-2525bb44114bad58534d56ee
    resource: repo://crates/ygg-host/src/proto.rs
  - id: openwiki-source-2696e2f7febd691f8cbc479e
    resource: repo://crates/ygg-host/tests/replay_tamper.rs
  - id: openwiki-source-f5131ab478fa718e924dbf48
    resource: repo://crates/ygg-sandbox/src/inject.rs
  - id: openwiki-source-0ddc079b1076812ea4f7471c
    resource: repo://crates/ygg-sim/src/canonical.rs
  - id: openwiki-source-86c13e4901257c152b5f1bfe
    resource: repo://crates/ygg-sim/src/fixed.rs
  - id: openwiki-source-40bf7934333e8b316826ca7d
    resource: repo://crates/ygg-sim/src/hash.rs
  - id: openwiki-source-98e9e76231844f76a1652472
    resource: repo://crates/ygg-sim/src/rng.rs
  - id: openwiki-source-b888e3dc95ca0a07465ab295
    resource: repo://crates/ygg-sim/src/snapshot.rs
  - id: openwiki-source-b0d308427c9d0c9eb125b7ed
    resource: repo://scripts/persist-matrix.sh
generated: { by: "pi", at: "2026-11-26T07:10:30.723Z" }
---

# 确定性 — 定点、规范化字节、哈希与随机流

> **id** `contracts/kernel/determinism` · **模块** kernel · **触发** 加数值字段、改快照布局、改随机、排查「两端哈希不同」
> **读不到代价** `irreversible`（确定性一旦破，历史存档与回放全部作废，且破点通常只在别的平台上暴露）

## 1. 三条纪律

1. **没有浮点**：状态里只有 `i64` / `u64` / 字符串 / 布尔 / 引用 / 标签，以及 `F64Bits` 这种**位串**（按字节原样存，不做算术）。`proto::Value` 干脆没有浮点变体，解码期拒。
2. **没有未定义序**：组件表按注册序、archetype 按首次出现序、流按名称字典序；哈希输入里没有 `HashMap` 迭代序。
3. **没有隐式环境**：不读时钟、不读环境变量、不读文件系统；随机只从世界自己的流里取。

## 2. 定点：Q32.32

`Fixed = i64`，`FRAC_BITS = 32`，`ONE = 1 << 32`（`crates/ygg-sim/src/fixed.rs`）。

- `add` / `sub` 直接加减（回绕由 `i64` 定义）；
- `mul` / `div` 用 `i128` 中间量再收窄，`div` 除零返回错误而不是 panic；
- `isqrt` 是**整数**开方（牛顿迭代 + 收敛判据），不是 `f64::sqrt`；
- `to_decimal` / `parse_decimal` 是十进制文本 ↔ 定点的**唯一**通道，供 JSON 边界使用（协议层对 `value` 只收整数，小数需走这两个函数）。

规则：**任何进入状态或哈希的数值都必须是 `Fixed` 或整数**。`F64Bits` 只允许作为「从外部原样带入的位串」存在，不参与运算、不参与比较大小。

## 3. 规范化字节：一份读写器

`crates/ygg-sim/src/canonical.rs` 的 `FrameWriter` / `FrameReader` 是定宽小端读写的**唯一**实现，快照与批量帧共用。这不是风格问题：

- 快照能解的列，批量帧也必须能解 —— 共用读写器让这条性质成为**结构性质**（`FrameReader::finish` 一次校验尾 digest，坏字节 fail-closed）；
- 帧头固定 `MAGIC = b"YGGSIM\0\x01"`（8 字节）+ `FMT_VERSION` + `flags`（保留位，非 0 的语义必须先加契约）；
- 每帧以 `fnv1a64(前面全部字节)` 的 8 字节小端结尾。

## 4. 状态哈希的精确输入面

`state_hash(snapshot) == fnv1a64(snapshot_payload)`，其中 `snapshot_payload` 是完整帧**去掉尾部 8 字节 digest**。

快照帧的段序（`crates/ygg-sim/src/snapshot.rs`）：

```text
MAGIC(8) │ FMT_VERSION(2) │ flags(2) │ tick(8) │ next_entity(8)
        │ 组件表 (cid, name, kind, type_params…) │ 资源表
        │ rng_seed(8) │ rng 流表 (name, w0..w3) │ archetype 段（按首现序，列按 cid 升序）
        └ tail: fnv1a64(以上全部) 小端 8 字节
```

因此「同一状态同一字节」的论证是构造性的：

1. 段序与列序都由规范序决定，没有可自由排列的位置；
2. 每个值定宽小端，字符串长度前置；
3. 唯一变量 `rng` 也在帧内 —— **随机流的当前位置属于状态**，所以「同 seed 同输入，第 N tick 后哈希相同」不依赖任何进程外状态；
4. 尾 digest 与 `hash()` 取同一个数，`restore(b)` 的校验与 `hash()` 的取值是同一次计算，不会出现「restore 过了但 hash 不同」。

算法选择 FNV-1a 64 的三个理由（不是安全面、十行可审、跨语言同源）写在 `crates/ygg-sim/src/hash.rs` 的模块注释里；公开参考向量（`fnv1a64("foobar") == 0x85944171F73967E8`）在同一个文件的用例里钉死。

## 5. `ygg-core` 的 KV 不在哈希里

`crates/ygg-core` 的 `kv` 存的是**不属于列存世界**的标量状态（宿主注入的环境量、规则的自定义簿记）。它不进快照、不进 `state_hash`。这条边界在 `contracts/kernel/lua-api` 里对 Lua 侧同样成立：`world.kv.*` 的写入不改变 tick 哈希，也不会被 `replay` 复算 —— 需要被复算的东西必须放进组件。

## 6. 随机：多流 + 派生

```text
derive_state(seed_text, stream) -> [u32; 4]      // w[i] = mix64(fnv64("{seed}/{stream}/{i}")) & 0xFFFFFFFF
next32(state)                     -> u32          // 每个流各自的 4 状态
stream_name(system, channel)      -> "{system}/{channel}"
```

- 流第一次被用到时**按名称派生**，所以「哪个系统先跑」不影响任何流的取值 —— 这是把「顺序依赖」从随机里彻底摘掉的办法；
- 流表按名称字典序写进快照，`Rng::from_parts(seed, streams)` 可原样恢复；
- `math.random` 在 Lua 侧绑到 `RANDOM_STREAM` 这条流（见 `contracts/kernel/sandbox` §3），`math.randomseed` 缺席 —— 种子只能由宿主在 `hello.seed` 里给。
- 内核自己的锚种子 `KERNEL_RNG_SEED = 0x5947_4452`（“YGDR”）在 `crates/ygg-sandbox/src/inject.rs`，是 S12 golden 的基线，改动等于换基线。

## 7. 双端一致怎么证明

| 面 | 证据 |
| --- | --- |
| 单进程逐 tick | `packs/example/golden.json`（50 tick 哈希序列）由 native 生成、由 `crates/ygg-sandbox/tests/example_pack.rs` 与 `scripts/wasm-lane.mjs` 同时断言 |
| 长跑 | `bash scripts/persist-matrix.sh` 在 1k / 10k / 100k 三档比对 native 与 wasm 的最终哈希 |
| 快照往返 | `restore(snapshot())` 后 `hash()` 不变；快照往返逐字节稳定 |
| 篡改定位 | `crates/ygg-host/tests/replay_tamper.rs` 断言 `first_bad_tick` 指到被改的那一帧 |

## 8. 维护责任

- **触发**：加组件类型、改帧布局/段序、改随机派生、改哈希算法、动 `FRAC_BITS`。
- **检查**：`cargo test -p ygg-sim`、`cargo test -p ygg-sandbox --test example_pack`、`node scripts/wasm-lane.mjs`、`bash scripts/persist-matrix.sh`。改任何一项都必须**同时重新生成** `packs/example/golden.json`，并在提交信息里写明「基线已换」。
