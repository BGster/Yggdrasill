---
type: "Reference"
title: "Yggdrasill 内核 SDK — 定位与宪法"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T07:10:30.723Z
sources:
  - id: openwiki-source-8037e2358a2c4f9b2c722a11
    resource: repo://AGENTS.md
  - id: openwiki-source-651d1fb6c9e49916a916ab51
    resource: repo://Cargo.toml
  - id: openwiki-source-682e02a9e393e886de83488d
    resource: repo://crates/ygg-host/src/session.rs
  - id: openwiki-source-683c84ca41d2dbee8643a1d8
    resource: repo://crates/ygg-lint/src/rules.rs
  - id: openwiki-source-3ce02e4845420a1c74760ca3
    resource: repo://crates/ygg-sandbox/src/modload.rs
  - id: openwiki-source-8c02dd48d2aed1554766f57a
    resource: repo://crates/ygg-sim/src/lib.rs
  - id: openwiki-source-fe085e18d1b6157ca615fbb7
    resource: repo://packages/schema/protocol/parity-script.json
  - id: openwiki-source-dfe0e9598ad418e75d354f01
    resource: repo://packs/example/golden.json
  - id: openwiki-source-c105d9c13b085511821f0969
    resource: repo://scripts/wasm-lane.mjs
generated: { by: "pi", at: "2026-11-26T07:10:30.723Z" }
---

# Yggdrasill 内核 SDK — 定位与宪法

> **id** `overview` · **模块** 中枢 · **触发** 第一次进本仓、判断「这件事该不该由内核做」、给新人讲这是什么
> **读不到代价** `silent-drift`（把渲染、联网、Agent 编排塞进内核，界一破就很难收回）

Yggdrasill 是一个**可嵌入的游戏内核 SDK**：宿主引擎驱动它，它负责世界与规则，两者之间只有一份 JSON 协议。

内核不渲染、不含物理、不碰输入与音频、不做资源导入、不联网、不编排 Agent。它只保证一件事：**同一个包、同一个种子、同一串命令，任何载体跑出逐字节相同的轨迹。**

## 1. 三层结构

```
宿主引擎（Godot / Unity / H5 / TapTap / 你自己的服务）
        │  ygg/1 协议（JSON 帧，13 个 tag）
        ▼
ygg-host      会话、状态机、镜像装载、错误映射          ← 集成层
        ▼
ygg-sandbox   Lua 沙箱：预算、白名单、命令面、原子 step  ← 规则运行时
        ▼
ygg-sim       列式世界：archetype、定点、RNG 流、哈希    ← 世界权威
ygg-core      KV
```

`ygg-pack`（原 `ygg-modules`）管包格式与纯性扫描，`ygg-lint` 管静态检查，`ygg-schema` 管 JSON Schema，`ygg-wasm` / `ygg-capi` 是两个绑定载体。**只有 `ygg-host` 面向宿主**，其余 crate 不认识「宿主」这个词。

## 2. 宪法表

| 内核负责 | 内核不负责 |
| --- | --- |
| 世界的权威状态（实体/组件/archetype/KV） | 渲染、动画、UI |
| 时间推进（`step`，tick 与 dt 由宿主给） | 物理、碰撞、寻路 |
| 确定性（Q32.32 定点、规范字节、逐 tick 哈希、命名 RNG 流） | 输入采集、音频播放 |
| Lua 规则沙箱（能力授予、指令预算、原子回滚） | 资源导入/解码（贴图、模型、音频） |
| 快照 / 事件日志 / 回放 | 网络同步、房间、匹配 |
| 包格式与装载序 | Agent 编排、prompt、工具调用 |
| 集成协议 `ygg/1` 与三载体绑定 | 观察面 UI（面板、SSE、地图） |

**判定法**：换一个宿主引擎（比如把 H5 换成 Godot），这段代码要不要改？要改 → 宿主侧；不改 → 内核侧。

## 3. 与 Agent 编排的分工

本仓**不做 Agent**。Agent 的协议、工具、结算、账本在另一个仓（sak）实现，它对本仓的关系只有一个：**它是若干宿主之一**，通过同一份 `ygg/1` 协议驱动世界。因此本仓里没有任何「派发」「提案」「门禁」「结算」的代码，也没有为 Agent 特化的 pack 或 runner。

## 4. 工作区地图

| crate | 源码行数 | 一句话 |
| --- | --- | --- |
| `crates/ygg-sim` | 5439 | 列式世界、定点、哈希、RNG、快照、日志 |
| `crates/ygg-sandbox` | 7946 | Lua 沙箱、`world` 面 17 键、装载、命令面、原子 step |
| `crates/ygg-host` | 2263 | 协议帧、会话状态机、镜像、错误映射 |
| `crates/ygg-lint` | 1828 | 静态规则（含 `YGG-STD` AST 检查） |
| `crates/ygg-schema` | 1213 | manifest / 组件 / 存档的 JSON Schema |
| `crates/ygg-modules` | 552 | 包的发现、字节锁、纯性扫描 |
| `crates/ygg-core` | 346 | KV 与值类型 |
| `crates/ygg-wasm` | 273 | wasm-bindgen 绑定 |
| `crates/ygg-capi` | 240 | C ABI（6 个符号） |

配套：`packs/example`（参考包）、`hosts/h5`（零构建浏览器宿主）、`packages/schema/protocol`（协议 schema 与 parity 脚本）、`scripts/`（构建与门禁）。

## 5. 三条能力线（谁会成为宿主）

| 线 | 宿主 | 本仓提供 |
| --- | --- | --- |
| 单机/网页游戏 | H5、TapTap | `hosts/h5` + wasm 绑定 |
| 引擎内嵌 | Godot、Unity | C ABI + 协议帧 |
| 服务端权威 | 自己的服务（含 Agent 世界后端） | Rust 绑定 + 快照/回放 |

## 6. 现在的数字

- 9 个 crate，`cargo test --workspace` **63 套全绿**；参考包自测 **12/12**。
- 金值：50 tick 逐帧哈希钉在 `packs/example/golden.json`。
- 长跑：10 万 tick 最终 hash `15126981819641904924`，原生 2668ms / wasm 3764ms，两载体逐帧相同。

## 7. 权威顺序

源码与测试 > `contracts/**`（内核/宿主/包的行为契约）> `operating/**`（怎么构建、怎么跑、怎么评）> `AGENTS.md` > 其他一切（含本页）。

本页只做导航与宪法；任何具体行为以对应契约页为准，契约页与源码冲突时以源码为准并立刻改页。

## 8. 维护责任

- **触发**：新增/删除 crate、宪法边界变更（哪一类职责进或出内核）、能力线变化、规模数字变化。
- **检查**：§4 行数由 `git ls-files 'crates/*/src/*.rs' | xargs wc -l` 复算；§6 的测试套数与金值来自 `cargo test --workspace` 与 `scripts/wasm-lane.mjs`。
