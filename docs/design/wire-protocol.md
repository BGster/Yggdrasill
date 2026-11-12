# Yggdrasill · 事件 Payload 线协议契约

> **定位**：管道分隔 payload 的逐事件格式与校验规则（含 exec/actor/actor_session 尾段链）。
> **replaces**：DESIGN §3.3（L420–445）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../README.md) · **相关**：[events.md](events.md) · [identity.md](identity.md)
> **搬运**：以下正文自 DESIGN.md L420–L445 逐字迁移，不重写语义。

### 3.3 事件 Payload 线协议契约

派发到 `agent-seed` 的所有事件使用管道分隔（`|`）的 UTF-8 字符串 payload（在支持处附带可选尾随 `@ext=<id>` 幂等键）：

| 事件名 | 线协议 Payload 格式 | 契约与校验规则 |
| --- | --- | --- |
| `event.start` | `""`/`"0"` = 无载荷 boot；`"<title>\|<description>"` = 章程载荷 | 无载荷照常 boot（章程显示 (untitled task)）；有载荷同拍写 `task.title`/`task.description` 并落 `coordinate`；已在 `coordinate` 再带载荷 = 幂等补写不过段（r2 rev2 B 裁决）。 |
| `requirement.add` | `"<title>\|<description>[\|<ext_id>[\|<dep1,dep2>]]"` | 在 `draft` 中添加 `rN`。校验 DAG 无环与 `ext_id` 幂等。 |
| `requirement.dispatch` | `""` \| `"0"` \| `"rN"` | 派发就绪需求，最多达 `job_capacity`（4）。`draft` 行进入 `planning`；`ready`（`planning` + `plan_ok`）行认领泳道并进入 `developing`。 |
| `proposal.submit` | `"<req_id>\|<rev>\|<stage>\|<summary>\|<artifacts>\|[<changes>]\|[<ext_id>][\|exec=<agent_id>[:<reason>]][\|actor=<actor_id>][\|actor_session=<session_id>]"` | 提交针对 `analysis`、`plan` 或 `development` 的提案。保留尾段链固定相对序 `exec → actor → actor_session`（各自可独立缺席）：`exec=` 必须在链中 `actor=` 之前并同事务绑定执行者台账；`actor=` / `actor_session=` 由**宿主注入**（声明路由 + `--actor` / `YGG_ACTOR` / daemon 请求字段），调用方自带同名段在宿主侧即被拒（`ACTOR_FORGED`），mod 侧错位段逐段点名拒绝；三态互证（verified / mismatch / unverified）识别不拒绝，`session` 是纯审计字段永不参与判定（见 §3.7）。 |
| `proposal.accept` | `""` \| `"0"` \| `"pN"` \| `"rN"` | 接受头部（或指定的）待决提案。在 `guide` 模式下需要菜单票据。 |
| `proposal.reject` | `"<pN_or_rN>[\|<reason>]"` | 拒绝待决提案并将需求返回其阶段队列。 |
| `gate.approve` | `""` \| `"0"` \| `"rN[\|<note>]"` | 通过 `plan_gate`（$\to$ `ready` + 响应式自动派发）或 `accept_gate`（$\to$ `done` + 内联泳道释放）。要求 `gate_rev == req.rev`，且对 `accept_gate` 要求非空 `artifacts`。 |
| `gate.reject` | `"<rN>[\|<feedback>]"` | 拒绝 `plan_gate`（`plan_ok=false`）或 `accept_gate`（保留 `plan_ok`），递增 `rev`，并返回 `planning`。 |
| `requirement.amend` | `"<req_id>\|<title>\|<description>\|<deps>"` | 空段保留当前值；`deps="-"` 清空依赖。递增 `rev`。 |
| `requirement.split` | `"<parent_id>\|<child_title_1>\|<child_title_2>[\|...]"` | 将一个开发前需求拆分为 $\ge 2$ 个 `draft` 子需求。 |
| `requirement.merge` | `"<rA,rB>\|<merged_title>\|<merged_desc>"` | 将 $\ge 2$ 个开发前需求合并为一个带并集依赖的新 `draft` 需求。 |
| `requirement.conflict` | `"<rA,rB>\|<note>"` | 将两个需求标记为 `blocked`，直到解决。 |
| `requirement.resolve` | `"<rA,rB>\|<keep_both\|keep_first\|revise>[\|<note>]"` | 清除冲突并应用所选解决策略。 |
| `coordinate.submit` | `""` \| `"0"` | 当所有关闭前置条件满足时，从 `coordinate -> review` 推进（**批边界**）。 |
| `verify.submit` | `"all_green=<true\|false>[;report=<text>]"` | 在 `review` 中记录验证结果。 |
| `settlement.evolution` | `"<artifact>\|<summary>[\|c=<kept>/<screened>[\|cls=<kind>:<n>,...]][\|sessions=<moved>/0/<dest>][\|todo=0/<verified_by>]"` | 在一个事务中注册演进筛选并结算会话/todo 清理（**每批一次**）。 |
| `review.note` | `"<lesson>\|<score_1_to_5>"` | 将 `settlement.evolution.*` 归档进 `review.evolution.*`；未武装 = 批收口回 `coordinate`（`run.batch` +1，无 `[ENDING`），`run.retire` 武装 = 退役终局（`[ENDING · run retired]`）。 |
| `config.set` | `"<key>=<value>"` | 更新运行时 `settings.<key>`（`play.mode`、`goal.authorization`、`run.retire`（`true`/`""` 退役武装/解除）、`executor.<rN>.<stage>`、`evolution.mode`、`gates.require_all_green`、`review.evolution.*`）。 |
| `vfs.write` | `"artifacts/<name>.md\|<content>"` | 经 `ygg.fs.write` 将内容写入 `run.sqlite`（`vfs_files`），并在 `artifact.*` 中记录 SHA-256 + 字节长度。 |

