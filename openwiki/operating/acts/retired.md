---
type: "Reference"
title: "退役终态 · retired — 唯一的真转移，唯一的收尾"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-e115ab200a8b2c89a8ff1d41
    resource: repo://.pi/tools/todo-check.sh
  - id: openwiki-source-8339725b1afcf7133cd86688
    resource: repo://.ygg/mods/agent-seed/pack.json
  - id: openwiki-source-d5edd97ca7e6335c0cd02c35
    resource: repo://.ygg/mods/agent-seed/src/handlers/operate/review.lua
  - id: openwiki-source-58ff0a3a16c1d28f634ce8b3
    resource: repo://crates/ygg-core/src/transition.rs
  - id: openwiki-source-0dc4a97640b5d4baa335a5a2
    resource: repo://crates/ygg-sandbox/examples/egg/paths.rs
  - id: openwiki-source-ca81618bb0d44dfaa8caf0e2
    resource: repo://crates/ygg-sandbox/examples/egg/settlement.rs
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# 退役终态 · retired — 唯一的真转移，唯一的收尾

> **id** acts/retired · **幕** retired · **触发** `run.retire` 武装后的最后一次 `review.note`
> **读不到代价** silent-drift（终态只读，误读不会推进也不会损坏状态）
> **replaces**：`docs/context/acts/retired.md`（搬运基线 `3a2536f`）
> **上游**：[release.md](release.md) · **相关**：[../rituals/cleanup.md](../rituals/cleanup.md) · [../scenes/README.md](../scenes/README.md) · [../../contracts/kernel/state-machine.md](../../contracts/kernel/state-machine.md)

**场景台词（知识权威）**：本局已退役。

## 1. 怎么进来，进来之后

**入口**：`config.set run.retire=true` 武装，然后走最后一轮 `coordinate.submit → verify.submit → settlement.* → review.note`。回执出现 `[ENDING · run retired]` 即已退役。

**终态语义**：业务事件全拒；`map.show` / `requirement.list` 只读查询仍可用。这是内核 FSM 唯一的一条真转移（`live → retired`），其余「相位」都是同一状态机内的自循环。

**ending 的渲染**：`[ENDING · run retired]` 前缀由 pack `agent.endingPrefix` 声明；结算汇总（任务、需求计数、批数、评分、演进候选、一句话教训）由 `endings.done` 键渲染，句式契约归 [scenes/README.md](../scenes/README.md)。

## 2. 退役核验（四开关）

出现 ending 后运行 `--settle`，确认 `settlement.json` 的 `ok=true`，且四个开关同时成立：

| 开关 | 断言 |
| --- | --- |
| events 非空 | 事件账里有真实动作，不是空跑出来的 ending |
| 无 failed step | 没有一步是失败后被跳过 |
| ending 存在 | ending 已写盘，不是只 narrate 过 |
| inbox 已消费 | 收件箱没有遗留的 `--requirement` / `--proposal` |

对**开放**常驻局提前 `--settle` 无害但不会结算（`reached=open, ok=false`，回到 running）——所以「跑了 settle 就一定收口」是错的。

## 3. 收尾

- 批结算后，待新 `--requirement` 进料或武装退役；两者都不做，局就停在那里（不腐坏）。
- **主会话结束前按 cleanup-policy 清空 todo**（含完成与历史），不得带入下一局；清理判据见 [../rituals/cleanup.md](../rituals/cleanup.md)。
- 升级后**开新 run-id**，旧局目录不迁移、不 adopt。

## 4. 判据

① 阻断——退役后仍派发业务事件、或把「提前 `--settle` 返回 `ok=false`」当成结算成功。② 症状——`settlement.json` 有一项开关为假却声称收口、todo 带着上一局条目进新局。③ 确认——`cat <run>/settlement.json` 四项全真且 `ok=true`，且新局目录里 `events.jsonl` 从空开始。

维护责任 = 改退役判定、ending 渲染或 cleanup 判据的执行者；触发 = 终态语义、四开关或清理范围变更；检查 = 上面第 4 节三条确认命令。
