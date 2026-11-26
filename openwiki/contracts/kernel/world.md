---
type: "Reference"
title: "世界模型 — 列存 ECS、查询面与调度"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T07:10:30.723Z
sources:
  - id: openwiki-source-682e02a9e393e886de83488d
    resource: repo://crates/ygg-host/src/session.rs
  - id: openwiki-source-9df12303ee4d047b7a6adef7
    resource: repo://crates/ygg-sim/src/component.rs
  - id: openwiki-source-9251356232b87676d619b6d7
    resource: repo://crates/ygg-sim/src/query.rs
  - id: openwiki-source-06a44f5a944119bb5b73eaf6
    resource: repo://crates/ygg-sim/src/sim.rs
  - id: openwiki-source-b888e3dc95ca0a07465ab295
    resource: repo://crates/ygg-sim/src/snapshot.rs
  - id: openwiki-source-d2391d71ffa8516188253d73
    resource: repo://crates/ygg-sim/src/store.rs
  - id: openwiki-source-29178b053729e870307f2d91
    resource: repo://crates/ygg-sim/src/system.rs
generated: { by: "pi", at: "2026-11-26T07:10:30.723Z" }
---

# 世界模型 — 列存 ECS、查询面与调度

> **id** `contracts/kernel/world` · **模块** kernel · **触发** 加组件/系统、写查询、排查「读到旧值」「实体凭空消失」「delta 里少了东西」
> **读不到代价** `hard-refusal`（把结构操作当成即时生效 → 同一 tick 内读到半成品世界，且不可复现）

## 1. 一个世界的形状

```text
Sim { tick, registry, store: ArchetypeStore, rng, resources, pending, changed, recorder }
```

- **组件**：`registry` 里按名注册，`ComponentDef { name, kind }`；kind 在线格式里是 1 字节（v1 = 1..5，v2 增加 `6 = F64Bits`）。Lua 面只暴露 `i64` / `u64` / `u8` / `bytes` 四种，**没有浮点**。
- **实体**：`EntityId = u64`，`next_entity` 单调递增，**id 不复用**。
- **archetype**：同一组件集合的行存在一起；`archetypes` 按「升序组件名串联」的字节序排序，槽位（`(archetype, row)`）是运行时概念，**不进快照**。
- **资源表**：`BTreeMap<u64,u64>`，内核内部的少量标量，独立于实体。

## 2. 两种写，两种生效时机

| 写 | 生效 | 谁负责 |
| --- | --- | --- |
| **值写**（改已有列的值） | **立即**，并记 undo 日志 | `world.set` / `each_mut` 回写 |
| **结构写**（`spawn` / `despawn` / `insert` / `remove`） | 本 tick 内只入 `pending` 队列，**tick 边界落地** | `Sim::step` 在系统跑完后统一结算 |

为什么分开：结构变更会搬行、换 archetype、改 `entity_index`，在系统迭代中途做会让「正在迭代的集合」瞬间失效。延迟到 tick 边界，迭代就有了稳定的世界视图。

## 3. tick 的事务边界

`Sim::step(dt)` 是一个事务：

1. 取三个便宜的标记：RNG 流状态、日志长度、世界 undo 日志开场；
2. 跑 `pre → rules → post` 三阶段；
3. **成功**：先取增量面 `written_cells()`（必须在 commit 之前——commit 会丢掉整本 undo 日志），再 commit，再结算 `pending`；
4. **失败**：回滚值写 → 恢复 RNG 流 → 丢本 tick 新投的 `pending` → 截断日志 → 清空增量面 → 原样返回错误。

失败即「什么都没发生」：tick 不前进、hash 不变、`delta` 为空。

## 4. 查询面

| API | 语义 |
| --- | --- |
| `for_each(&[ComponentId], f)` | 只读迭代**同时具备全部列**的实体，返回条数 |
| `for_each_mut(&mut self, &[ComponentId], f)` | 写迭代，回调拿到 `RowSet`，写入立即生效并记 undo |
| `RowSet::id/row/len/present/fixed/u64v/bytes` | 单行视图，读写按列序 `k` 取 |
| `ids(&[ComponentId])` | 只取 id 的规范序数组（Lua 面 1 基） |

`RowSet::set_fixed/set_u64` 只能写**已存在**的列；写不存在或类型不符的列报 `YGG-SIM-SCHEMA`，不会静默扩列。

## 5. 调度器与 `seal`

- 三阶段固定序：`pre → rules → post`；阶段内按**注册序**执行，不排序、不重排。
- `seal(registry)` 只校验不排序，六项按序先命中先返回：

| # | 校验 | 破坏时 |
| --- | --- | --- |
| 1 | 系统名重复 | `YGG-SIM-SCHEMA` |
| 2 | `reads`/`writes` 里的组件未注册 | `YGG-SIM-SCHEMA` |
| 3 | 同阶段两个系统写同一组件 | `YGG-SIM-SCHEMA` |
| 4 | `after` 指向不存在的系统 | `YGG-SIM-SCHEMA` |
| 5 | `after` 指向别的阶段 | `YGG-SIM-SCHEMA` |
| 6 | 同阶段 `after` 成环 | `YGG-SIM-SCHEMA` |

同阶段「一个写、另一个只读」是合法的：读不产生冲突。真需要先后，放进不同阶段或用 `after` 表达。

`seal` 之后再 `add` 一律 `YGG-SIM-SEALED`——追加一条未校验的声明会让 seal 的结论失效。

## 6. 世界变更集（`delta.changed`）

`changed` 来自 `written_cells()`：本 tick 真正被写过的 `(eid, cid)` 对，按 **eid 升序、cid 升序**去重排序。它**只含**本 tick 成功提交的写：

- 值写：每写一格记一条；
- 结构写：在 tick 边界结算后计入（`spawn` 计入新实体的全部列）；
- 回滚或拒绝后为空——不许出现「看起来写了」的回执。

## 7. 不变量与症状对照

| 不变量 | 被破坏时的可观察症状 |
| --- | --- |
| id 不复用 | 旧快照里的实体 id 突然指向新实体 |
| 结构写只在 tick 边界落地 | 同一 tick 内 `for_each` 迭代到刚 spawn 的实体 → 迭代条数在两次调用间跳变 |
| 值写立即 + undo 可回滚 | 一次失败的命令留下半改的世界 → 哈希与快照对不上 |
| 同阶段无写写冲突 | 两个系统写同一列，结果取决于注册序 → 改注册序就改结果 |
| `changed` 与真实写一致 | delta 缺失被写的格 → 宿主渲染停在旧值，且越跑越偏 |
| 组件名而非 id 键控快照 | 两个进程同态不同 hash → 跨进程对账失败 |

## 8. 维护责任

- **触发**：加组件种类、改 tick 语义、改 `pending` 结算时机、改 `seal` 校验项、改 `changed` 口径。
- **检查**：`cargo test -p ygg-sim`（`store` / `query` / `system` / `transaction` 各套）、`cargo test -p ygg-sandbox --test world_facade`、`cargo test -p ygg-sandbox --test example_pack`。
