# Yggdrasill · 盘点清理政策（cleanup-policy）

> **定位**：盘点幕两项清理（子代理会话清理 / 主会话待办清理）的政策正文知识权威。mod 的 `beats-settlement.settlement_sessions_unsetted_hint` 与 `settlement_todo_segment` 是本文的运行时投影（判据短句 + 指针）。
> **replaces**：r2 前散置于 `.ygg/mods/agent-seed/data/texts/beats-settlement.json` 的清理政策正文。
> **上游**：[../README.md](../README.md) · **相关**：[workflow-acts.md](workflow-acts.md)（盘点幕） · [copy-contract.md](copy-contract.md)（清理相关句式契约）

## 1. 子代理会话清理（settlement.sessions）

**触发**：`review.note` 批结算/退役前，若本局子代理会话记录未清理（`remaining > 0`），`[SESSIONS CLEANUP PENDING]` 阻塞收口。

**政策正文（知识权威）**：

1. 清理是 **prompt 驱动**的：用你自己环境的原生会话管理能力，删除/移出本局产生的子代理会话记录。
2. **范围严格限定本局**：主会话与无关会话绝不可触碰；`moved=0` 是合法结果（没有可清理项时如实声明 0）。
3. 完成后通过 `settlement.sessions`（载荷 `moved|remaining|destination`）或 `settlement.evolution` 的 `sessions=<moved>/<remaining>/<destination>` 可选段声明结果；`remaining` 必须为 0，`destination` 非空（如 `<your-destination>/<batch>`）。文件改名进 trash 目录，不是永久删除；不保证逐个文件哪些被保留——目的地请自行核验。
4. **未声明 = 诚实地未清理**：缺失声明保持 unsettled 状态并持续阻塞，不允许假装已清。
5. **成功判据**：清理后，本局的子代理会话不再出现在你自己的会话历史 / resume 列表中。
6. **识别注意**：子会话记录可能与主会话并排存于同一会话存储——移动前按 spawn 时间与 session id 逐文件识别，绝不误触主会话或无关会话；不要把临时目录下的任务 I/O 拷贝当作会话记录本体——记录是你的会话历史实际列出的那些条目。

**运行时投影**：mod 键只保留「[SESSIONS CLEANUP PENDING] remaining={{remaining}} — review.note blocked；政策见 docs/context/cleanup-policy.md；完成后 settlement.sessions 或 sessions= 段声明（remaining 须为 0）」量级。

## 2. 主会话待办清理（settlement.todo）

**触发**：同上，`review.note` 前主会话 todo 列表必须清零。

**政策正文（知识权威）**：结束本局前清空主会话 todo 列表（`todo clear`，含已完成与历史项）；绝不把已结束局的任务带进下一局。声明载荷为 `remaining|verified_by` 两段：`remaining` 必须为 0，`verified_by` 非空且 ≤120 字符（写明复核依据，如「主会话 todo 列表复核为 0 项（含完成与历史）」）。也可并入 `settlement.evolution` 的 `todo=<remaining>/<verified_by>` 可选段一次事务完成。

**运行时投影**：`settlement_todo_segment` 键保留 `todo=0/<verified_by：一句话复核依据，含完成与历史项>` 的短示例形态，长示例文案以本节为准。

## 3. 无 egg 降级

无 egg 时两项清理由主会话自律执行并写入批结算记录（同第 1、2 节判据）；缺的只是机器阻塞（`review.note` 的强制 gate），政策本身不降级。
