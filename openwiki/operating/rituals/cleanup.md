---
type: "Reference"
title: "盘点清理政策 — 会话与待办清零的唯一权威"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-940625457f02ee1c08fc18a4
    resource: repo://.pi/tools/sessions-count.sh
  - id: openwiki-source-e115ab200a8b2c89a8ff1d41
    resource: repo://.pi/tools/todo-check.sh
  - id: openwiki-source-90c75edb710a356b1957d6e9
    resource: repo://.ygg/mods/agent-seed/src/domain/wire/settlement.lua
  - id: openwiki-source-55ac8b91f1869ef9305b6b62
    resource: repo://crates/ygg-sandbox/src/host_contract/settlement.rs
  - id: openwiki-source-3709884c1dbc8a072f364d79
    resource: repo://docs/context/rituals/fallback.md
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# 盘点清理政策 — 会话与待办清零的唯一权威

> **id** rituals/cleanup · **幕** settle · **触发** `review.note` 批结算 / 退役前
> **读不到代价** hard-refusal（未清理则 `[SESSIONS CLEANUP PENDING]` 阻塞收口；误触他人会话是越界）
> **replaces**：`docs/context/rituals/cleanup.md`（搬运基线 `3a2536f`）
> **上游**：[overview.md](../../overview.md) · **相关**：[../acts/settle.md](../acts/settle.md) · [../scenes/README.md](../scenes/README.md)

本页是两项清理（子代理会话 / 主会话待办）的**政策正文**。mod 的 `settlement_sessions_unsetted_hint` 与 `settlement_todo_segment` 只是本页的运行时投影（判据短句 + 指针），不构成第二份规范。

## 1. 子代理会话清理（settlement.sessions）

**触发**：`review.note` 批结算 / 退役前，若本局子代理会话记录未清理（`remaining > 0`），`[SESSIONS CLEANUP PENDING]` 阻塞收口。

**政策正文**：

1. 清理是 **prompt 驱动**的：用你自己环境的原生会话管理能力，删除 / 移出本局产生的子代理会话记录。
2. **范围严格限定本局**：主会话与无关会话绝不可触碰；`moved=0` 是合法结果（没有可清理项时如实声明 0）。
3. 完成后用 `settlement.sessions`（载荷 `moved|remaining|destination`）或 `settlement.evolution` 的 `sessions=<moved>/<remaining>/<destination>` 可选段声明；`remaining` 必须为 0，`destination` 非空（如 `<your-destination>/<batch>`）。文件改名进 trash 目录而非永久删除；不保证逐个文件哪些被保留——目的地自行核验。
4. **未声明 = 诚实地未清理**：缺失声明保持 unsettled 并持续阻塞，不允许假装已清。
5. **成功判据**：清理后本局的子代理会话不再出现在你自己的会话历史 / resume 列表里。
6. **识别注意**：子会话记录可能与主会话并排存于同一会话存储——移动前按 spawn 时间与 session id 逐文件识别，绝不误触主会话或无关会话；不要把临时目录下的任务 I/O 拷贝当作记录本体——记录是你的会话历史实际列出的那些条目。

## 2. 主会话待办清理（settlement.todo）

**触发**：同上，`review.note` 前主会话 todo 列表必须清零。

**政策正文**：结束本局前清空主会话 todo 列表（`todo clear`，含已完成与历史项）；绝不把已结束局的任务带进下一局。声明载荷为 `remaining|verified_by`：`remaining` 必须为 0，`verified_by` 非空且 ≤120 字符（写明复核依据，如「主会话 todo 列表复核为 0 项（含完成与历史）」）。也可并入 `settlement.evolution` 的 `todo=<remaining>/<verified_by>` 一次事务完成。

## 3. 无 egg 时的清理

无 egg 时两项清理由主会话自律执行并写入批结算记录（判据同 §1 / §2）；缺的只是机器阻塞（`review.note` 的强制 gate），政策本身不降级。本仓自己改代码时的会话清理同样按 §1：只动自己这轮 spawn 的子会话，主会话与无关会话不碰。

## 4. 判据

① 阻断——`review.note` 前不声明 `settlement.sessions` / `settlement.todo`、把 `remaining` 写成非 0 却声称已清、把主会话或无关会话当成清理对象。② 症状——`[SESSIONS CLEANUP PENDING]` 反复出现、`destination` 为空、`verified_by` 写「已清理」而无复核依据。③ 确认——`settlement.sessions` 的 `remaining=0` 且 `destination` 非空、`settlement.todo` 的 `verified_by` 能在 todo 列表副本上复核、trash 目录里能看到本局会话条目。

维护责任 = 改清理政策或声明载荷的执行者；触发 = 会话存储位置 / 声明键变化；检查 = 本页 §1 / §2 与 `data/texts/beats-settlement.json` 的投影句式一致（投影只留短句 + 指针），且投影里的路径字面量随本页改名同步。
