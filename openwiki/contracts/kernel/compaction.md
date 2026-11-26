---
type: contract
title: 上下文压缩与检查点交接
description: 显式压缩双入口、三级水位线（normal/soft_brake/hard_brake）、稳定检查点谓词、批边界天然检查点与交接产物字段。
tags: [kernel, compaction, handoff, context, contract]
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-faa4fb02781cc381dc2c84d5
    resource: repo://.pi/tools/ctx-usage.sh
  - id: openwiki-source-d2367da63a39ecbad8c167be
    resource: repo://crates/ygg-core/src/save.rs
  - id: openwiki-source-93ae92d89530c45c6be76959
    resource: repo://crates/ygg-core/src/snapshot.rs
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# 上下文压缩与检查点交接

> **id** kernel/compaction · **模块** ① ygg 内核 · **触发** 改压缩水位 / 检查点判定 / 交接产物之前
> **读不到代价** silent-drift（压缩点选错 → 回执丢失与上下文腐化）
> **replaces**：DESIGN §3.6（L590–605）（搬运基线 3a2536f）

**定位**：显式压缩双入口、三级水位线与稳定检查点谓词（批边界是天然检查点）。
<!-- openwiki: broken internal link [../README.md] file "../README.md" does not exist. Fix the href or restore the target, then delete this comment. -->
**上游**：[architecture/README.md](../../architecture/README.md) · **相关**：[action-faces.md](../mod/action-faces.md) · [cli-daemon.md](../host/cli-daemon.md)

## 1. 压缩是显式的

它只在两个入口发生：① `action=handoff_compact`；② `ygg_run_lanes` 的编排层刹车。**`dispatch` 只报告水位，不触发压缩**（水位字段随 `summary` 回执给出，判定留给编排层）。

## 2. 水位线

- **有效窗口上限**：默认 `min(modelWindow, 200_000)` token（可经 `YGG_EFFECTIVE_CONTEXT_WINDOW` 覆盖）。
- `ratio < 0.60` → `normal`；
- `0.60 ≤ ratio < 0.75` → **`soft_brake`**：先让当前活跃子代理波在其门禁处落地，再写交接并压缩；
- `ratio ≥ 0.75` → **`hard_brake`**：不再派生新子代理；立即写交接并压缩。

## 3. 稳定检查点谓词

仅当 `summary.available == true` ∧ `pendingTools == 0` ∧（`gates.length > 0` ∨ `jobs.length == 0` ∨ `conflicts.length > 0`）时才允许压缩，除非经 `handoff_compact` 显式强制。

常驻语义下**批边界是天然稳定检查点**：批收口后 `stage=coordinate ∧ jobs=0 ∧ gates=0 ∧ conflicts=0`，谓词常真；常驻长局推荐在批边界做压缩交接。

## 4. 常驻批排空暂停（drive 泵）

`summary.lifecycle == "resident"` ∧ `summary.batchDrained` ∧ inbox 无未消费行 → 打印等待文案后安全暂停（exit 0）：空批绝不连点 `coordinate.submit`。`lifecycle` 缺失（旧 mod）不触发。

## 5. 交接产物（`artifacts/handoff-latest.md`）

在压缩前持久化进 `run.sqlite` VFS 并在压缩后重新注入，包含：运行 ID、阶段、token 比例、任务表（含 `executor` / `planExecutor`）、待决门禁、未解决冲突、VFS 产物清单，以及精确的下一步 `ygg_session` / `ygg_run_lanes` 命令。

**判据**：① 阻断——在 `dispatch` 里就地压缩，或在 `pendingTools != 0` 时压缩（不满足谓词且无强制）；② 症状——压缩后同一需求被重新提案、门禁回执丢失、交接文件缺命令段；③ 确认——压缩发生在 `handoff_compact` 或编排层刹车处，交接文件七段齐全。

## 6. 存档与规范快照的边界（压缩之外的「持久化」）

压编是上下文面的事，存档是状态面的事，两者不共享语义：

- **存档**（`ygg-core::save`）= 实体表 + 持久 KV + `pending_choice`——挂起选择是**内核状态**（挂起即数据），读档后必须原样重现待决选项；handler 是代码不入档（重装载即恢复）；输出缓冲属瞬态诊断面，旁路不入档。
- **双版本号**：`format_version` = 存档格式自身版本（读档即校验）、`engine_version` = 写出它的引擎版本；两者各自演进、不共用。
- **确定性序**：实体表 ID 升序 + KV 键字典序（容器天然序），同一状态无论构造次序如何，存档字节逐字节一致。
- **规范快照**（`ygg-core::snapshot`）：每个事件边界取一次，`serde_json` 默认 `BTreeMap`（**禁开 `preserve_order`**）；属性键序与实体序由排序决定；`-0.0 → 0.0` 写入路径已规范化、编码侧 `Value` 手写 `Serialize` 兜底；非有限值（NaN/±∞）**拒绝为错误**而非静默改写；本 crate 只产 canonical bytes，SHA-256 归 `ygg-sandbox`。
