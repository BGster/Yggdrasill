---
type: contract
title: 事件总表
description: 39+1 个事件的三种分类（剧情/操作/基础设施）、每个事件的微状态落点与 payload 契约、proposal.submit 尾段链与事件集设计依据。
tags: [kernel, events, payload, registry, contract]
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-cfe8ec9e3ed7fc4dc881025e
    resource: repo://.ygg/mods/agent-seed/mod.lua
  - id: openwiki-source-41a5493ce2fb0d32705844d9
    resource: repo://crates/ygg-core/src/bus.rs
  - id: openwiki-source-5156ec9ec5964484527de455
    resource: repo://crates/ygg-core/src/entity/value.rs
  - id: openwiki-source-8774eec48fcb6d9c1cb2563e
    resource: repo://crates/ygg-core/src/event.rs
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# 事件总表

> **id** kernel/events · **模块** ① ygg 内核 · **触发** 增删事件 / 改事件分类口径之前
> **读不到代价** hard-refusal（事件名或 payload 形状对不上 → 派发被拒）
> **replaces**：DESIGN §2.6（L248–284）（搬运基线 3a2536f）

**定位**：剧情/操作/基础设施三类共 39 个事件与 payload 落点（与 [action-faces](../mod/action-faces.md) 的 registry 口径并存互链）。
<!-- openwiki: broken internal link [../README.md] file "../README.md" does not exist. Fix the href or restore the target, then delete this comment. -->
**上游**：[architecture/README.md](../../architecture/README.md) · **相关**：[wire-protocol.md](wire-protocol.md) · [action-faces.md](../mod/action-faces.md)

事件是唯一推进货币。三类切分：**剧情事件**（推进微状态，20 个）、**操作事件**（增删改查，9 个）、**基础设施事件**（10 个）。注册面另含 `ygg.narrative.goto`（node:render 渲染事件）= 40 个已注册 handler（语义哨兵测试计数口径）。

## 1. 剧情事件（20 个，payload 内管道符均为转义字面量）

| 事件 | 微状态落点 | payload |
| --- | --- | --- |
| `event.start` | live·立局（进入） | `""` 或 `"<title>\|<goal>\|<success>"`（可选章程载荷；不转场） |
| `charter.set` | live·立局（内） | `"<title>\|<goal>\|<success>"`（可反复修订，`run.begin` 后锁定） |
| `run.begin` | 立局 → 协调 | `""`（前置：章程三件套齐备） |
| `requirement.add` | open·立项（进入） | `"<title>\|<description>[#deps]"` |
| `requirement.dispatch` | 立项 → 计划中 / 待开发 → 开发中 | `"rN"` |
| `proposal.submit` | 计划中 → 计划门禁 / 开发中 → 交付受理 | `"<req>\|<rev>\|<stage>\|<summary>\|<artifacts>\|…"` |
| `proposal.accept` | 交付受理 → 验收门禁 | `"pN"` |
| `proposal.reject` | 交付受理 → 待开发 | `"<pN>\|<reason>"` |
| `gate.approve` | 计划门禁 → 待开发 / 验收门禁 → 集成 | `"rN"` |
| `gate.reject` | 计划门禁 → 计划中 / 验收门禁 → 待开发 | `"<rN>\|<feedback>"` |
| `integration.confirm` | 集成 → done | `"<rN>\|develop@<sha>"` |
| `integration.reject` | 集成 → 待开发 | `"<rN>\|<reason>"` |
| `coordinate.submit` | 协调 → 盘点 | `""`（前置见 [state-machine.md](state-machine.md) §盘点前） |
| `verify.submit` | 盘点·待验证 | `"true"` / `"all_green=false;report=…"` |
| `settlement.evolution` | 盘点·待登记 | `"artifact\|summary[\|c=…][\|cls=…][\|sessions=…][\|todo=…]"` |
| `settlement.sessions` | 盘点·待结会话 | `"moved\|remaining\|destination"` |
| `settlement.todo` | 盘点·待清待办 | `"remaining\|basis"` |
| `release.confirm` | 发布 → 批结算 | `"master@<sha>\|vYYYY.MM.DD.N"` |
| `release.skip` | 发布 → 批结算 | `"<reason>"`（必填：本批无需发布的理由） |
| `review.note` | 发布 → 协调（未武装）/ → retired（武装） | `"lesson\|score"` |

`proposal.submit` 尾段链保留固定相对序 **`exec` → `actor` → `actor_session`**（各自可独立缺席）：`exec=<agent_id>[:<reason>]` 必须在链中 `actor=` 之前并同事务绑定执行者台账；`actor=` / `actor_session=` 由**宿主注入**（声明路由），调用方自带同名段在宿主侧即被拒（`ACTOR_FORGED`）——线协议正文见 [wire-protocol.md](wire-protocol.md)，身份语义见 [../mod/identity.md](../mod/identity.md)。

## 2. 操作事件（9 个，不推进任何幕）

`requirement.list` / `requirement.split` / `requirement.merge` / `requirement.amend` / `requirement.conflict` / `requirement.resolve` / `lane.pool` / `lane.claim` / `lane.release`——只改操作对象与标志位（泳道语义见 [../host/lanes.md](../host/lanes.md)）。

## 3. 基础设施事件（10 个）

`config.set` / `config.open` / `config.close` / `map.show` / `ygg.choice_selected` / `vfs.write` / `tool.call` / `tool.result` / `agent.summary` / `stage.advance`。

## 4. 设计依据（事件集为何长这样）

- **事件名的稳定性等于合约的稳定性**：`event` 是推进状态的唯一货币，`op` 是状态之外的宿主端口，`action` 只是适配器的语义入口——三层不得互代（见 [action-faces.md](../mod/action-faces.md)）。
- **废弃的入口不留下静默兼容**：`guide.show` 已物理注销（注册面、菜单挂载、文案键、playbook 引用、孤儿代码零残留），直派旧名不再有假成功——未注册事件在零副作用下拒绝。墓碑兼容（收下旧名、静默当作新名）会把「我调用错了」变成「它恰好也能跑」，错误被推迟到语义层才暴露；精确拒绝（`VALIDATION`）让调用点立刻可见。
- **`map.show` 是单一视图**：操作视图即完整视图，payload 一律忽略。
- **冷启动的章程不进独立阶段**：`intake` 宏观阶段与 `task.intake` 事件已注销，章程改由 `event.start` 的启动载荷承载（`title|goal|success`，`""` 与 `"0"` 同判无载荷）。章程载荷化是因为 `intake` 只装了一件事（写三件套），独立阶段只换来一次多余转场与一套多余拒绝面。
- **本表与运行期 registry 的关系**：本表是设计目标全集，运行期 registry 才是 fail-closed 的运行时唯一事实；两者不一致即缺陷。

**判据**：① 阻断——新增未在 registry 声明的事件、为已注销事件保留兼容分支、或让 `map.show` 按 payload 分岔视图；② 症状——单发派发旧事件名得到 `ok:true`，或事件全集计数与本表不符、`map.show` 两个 payload 的叙事不一致；③ 确认——`cargo run -q --example selftest -- --mod .ygg/mods/agent-seed`（未注册事件拒绝、`map.show` payload 无关性、registry 计数）与 `cargo run -q --example playtest -- .ygg/mods/agent-seed`。

## 5. 内核事件模型与回放纪律

- **来源标志 `Origin`**：`External`（玩家输入 / 宿主注入）是**回放日志的唯一记录对象**；`Derived`（handler 执行期间发出，入 per-transition FIFO）在回放时自动再产生——因此回放日志只含外部事件，派生事件不入档。
- **payload 即规范**：复用 `Value`，数字编码即规范（浮点构造即拒非有限值、`-0.0` 入构造即归一为 `0.0`、编码侧手写 `Serialize` 兑底）——跨平台数字编码分歧通道在类型面上关闭；复合结构属资源面，走 `ygg.data`。
- **注册表规范序 = 注册序**：事件名经 `BTreeMap` 分桶，桶内 `Vec` 保留注册次序；每条注册持稳定键 `(mod id, 注册序号)`——身份、tiebreak 与诊断核对面，全程无哈希容器参与。
- **注册只发生在 Mod 装载期**：handler 集 = 装载的纯函数，这是「回放日志缩为仅外部事件」的前提；运行期注册禁令的权威执行面在 host API（本 crate 不承载运行期拦截）。
- **派发面 fail-closed**：未注册事件名一律 `Err`——装载期注册是唯一入口，不存在静默空跑。
- **两个 sink**：`EventSink` 是派生事件出口（per-transition FIFO），`NarrationSink` 扩展叙述与选择面——`narrate` 为纯输出累积（转换结果的一部分），`choose` 以 `PendingChoice` 设入内核状态并终结当前 handler（挂起即数据，玩家选择 = 新外部事件）。handler 本体不直接触达内核状态。
