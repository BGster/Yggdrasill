---
type: "Reference"
title: "架构总览 — 模块与契约页索引"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T17:33:41.429Z
sources:
  - id: openwiki-source-8037e2358a2c4f9b2c722a11
    resource: repo://AGENTS.md
  - id: openwiki-source-ef4b88be1d64012bd99b7527
    resource: repo://crates/ygg-core/src/lib.rs
  - id: openwiki-source-682e02a9e393e886de83488d
    resource: repo://crates/ygg-host/src/session.rs
  - id: openwiki-source-d2b71bd24583ba41d15160e6
    resource: repo://crates/ygg-sandbox/src/lib.rs
  - id: openwiki-source-3ce02e4845420a1c74760ca3
    resource: repo://crates/ygg-sandbox/src/modload.rs
  - id: openwiki-source-8c02dd48d2aed1554766f57a
    resource: repo://crates/ygg-sim/src/lib.rs
  - id: openwiki-source-06a44f5a944119bb5b73eaf6
    resource: repo://crates/ygg-sim/src/sim.rs
  - id: openwiki-source-88bd460316cc9f4d5e371ae1
    resource: repo://scripts/wiki-consistency.sh
  - id: openwiki-source-219a851abf50e3acdd3f032c
    resource: repo://scripts/wiki-ver.sh
generated: { by: "pi", at: "2026-11-26T17:33:41.429Z" }
---

# 架构总览 — 模块与契约页索引

> **id** `architecture` · **模块** 索引 · **触发** 开工前定位、改动跨 crate 前
> **读不到代价** `silent-drift`（路由指错 → 改错 crate、或把契约当成实现细节）

## 1. 三族与三段

内核按「谁在使用」分三段，而不是按代码目录分：

| 段 | 这一段的职责 | 不属于它 |
| --- | --- | --- |
| **host** | 把外部的请求翻成一次世界推进，把结果翻成帧 | 任何规则、任何渲染 |
| **rules** | 用 Lua 描述规则，只通过 `world.*` 触碰世界 | 任何 I/O、任何时间源 |
| **world** | 列存状态、定时推进、随机流、快照与哈希 | 任何 Lua 语义、任何呈现 |

判定法一句话：**换掉宿主引擎这段代码要不要改** —— 要改的属于 host，不改的属于 rules/world。

## 2. crate → 契约

| crate | 行数（源码） | 一句话职责 | 契约页 |
| --- | --- | --- | --- |
| `ygg-sim` | 5.4k | 列存世界：组件/查询/调度/tick/快照/回放/哈希 | [world](../contracts/kernel/world.md) · [determinism](../contracts/kernel/determinism.md) · [persistence](../contracts/kernel/persistence.md) |
| `ygg-sandbox` | 6.6k | 规则运行时：Lua 沙箱、`world.*` 面、包装载、命令面 | [sandbox](../contracts/kernel/sandbox.md) · [lua-api](../contracts/kernel/lua-api.md) · [packaging](../contracts/kernel/packaging.md) |
| `ygg-host` | 2.3k | 协议信封：`ygg/1` 消息、会话状态机、包镜像 | [protocol](../contracts/host/protocol.md) · [image](../contracts/host/image.md) |
| `ygg-capi` / `ygg-wasm` | 0.5k | 同一会话的 C ABI 与 WASM 绑定，零协议逻辑 | [bindings](../contracts/host/bindings.md) |
| `ygg-core` | 0.35k | 不属于列存世界的标量状态（环境 KV + 四型值），不进 tick 哈希 | [determinism](../contracts/kernel/determinism.md) §5 |
| `ygg-lint` / `ygg-modules` / `ygg-schema` | 3.6k | Lua 静态检查、模块名/路径解析、JSON Schema 校验 | [packaging](../contracts/kernel/packaging.md) §6 |

## 3. 契约页索引

**kernel**（改内核语义前必读）

- [world](../contracts/kernel/world.md) — 组件、archetype、槽位、查询面、结构延迟、调度与 seal
- [determinism](../contracts/kernel/determinism.md) — 定点、canonical 字节、状态哈希输入面、随机流
- [sandbox](../contracts/kernel/sandbox.md) — 白名单、math 收窄、指令预算、内存上限
- [lua-api](../contracts/kernel/lua-api.md) — `world.*` 签名与命令返回值契约
- [persistence](../contracts/kernel/persistence.md) — 快照帧、`E_STORE_FORMAT`、日志与回放定位
- [packaging](../contracts/kernel/packaging.md) — `pack.json` v2、manifest 锁、装载序
- [errors](../contracts/kernel/errors.md) — 三层错误码与映射

**host**（接宿主前必读）

- [protocol](../contracts/host/protocol.md) — `ygg/1` 消息、状态机、拒绝规则
- [bindings](../contracts/host/bindings.md) — Rust / WASM / C ABI 三载体等价
- [image](../contracts/host/image.md) — 无文件系统宿主的包镜像

**operating**（怎么跑、怎么写）

- [build](../operating/build.md) · [example-pack](../operating/example-pack.md) · [h5-host](../operating/h5-host.md) · [dev-workflow](../operating/dev-workflow.md) · [vocabulary](../operating/vocabulary.md)

## 4. 索引与实文件的关系

- 本页表格是**指路**，不是正文：任何一条路由的规则以目标页为准。
- `scripts/wiki-ver.sh` 校验每页的 `id` 唯一、`ver` 与账本 `openwiki/ver.json` 一致、站内链接可解析。
- `scripts/wiki-consistency.sh` 校验 `AGENTS.md` 的行数预算、路由目标存在、claims 证据可解析。
- 新增契约页时必须同时改本页与 `AGENTS.md` §3 —— 两处都是「消费者在这一刻会看到的路由」，缺一处就是断链。
