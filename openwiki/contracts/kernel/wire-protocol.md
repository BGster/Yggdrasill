---
type: "Reference"
title: "事件 Payload 线协议契约"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-55f7cfd3b64c4982c69dd5df
    resource: repo://.ygg/mods/agent-seed/src/domain/wire/intake.lua
  - id: openwiki-source-a4428d3a250a714e2b67c998
    resource: repo://.ygg/mods/agent-seed/src/domain/wire/proposal.lua
  - id: openwiki-source-90c75edb710a356b1957d6e9
    resource: repo://.ygg/mods/agent-seed/src/domain/wire/settlement.lua
  - id: openwiki-source-1687a5fc04658c13a78d4590
    resource: repo://.ygg/mods/agent-seed/src/handlers/requirements/gate_actions.lua
  - id: openwiki-source-a320e603ef2519a7baf859a5
    resource: repo://crates/ygg-sandbox/examples/egg/identity/tests.rs
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# 事件 Payload 线协议契约

> **id** kernel/wire-protocol · **模块** ① ygg 内核 · **触发** 改 payload 形状 / 改 exec-actor 尾段链之前
> **读不到代价** hard-refusal（段位错位 → 载荷被拒或 `ACTOR_FORGED`）
> **replaces**：DESIGN §3.3（L420–445）（搬运基线 `3a2536f`）
> **上游**：[architecture/README.md](../../architecture/README.md) · **相关**：[events.md](events.md) · [../mod/identity.md](../mod/identity.md) · [state-surface.md](state-surface.md)

## 1. 总则

派发到 `agent-seed` 的所有事件使用**管道分隔**（`|`）的 UTF-8 字符串 payload，在支持处附带可选尾随 `@ext=<id>` 幂等键。空段保留当前值、`"-"` 表示清空（仅 `deps` 用）、`""`/`"0"` 表示「无载荷」或「不限目标」。

## 2. 逐事件格式

| 事件 | Payload 格式 | 契约 |
| --- | --- | --- |
| `event.start` | `""` / `"0"` / `"<title>\|<description>"` | 无载荷照常 boot（章程显示 `(untitled task)`）；有载荷同拍写 `task.title`/`task.description` 并落 `coordinate`；已在 `coordinate` 再带载荷 = 幂等补写不过段。 |
| `requirement.add` | `"<title>\|<description>[\|<ext_id>[\|<dep1,dep2>]]"` | 在 `draft` 中添加 `rN`；校验 DAG 无环与 `ext_id` 幂等。 |
| `requirement.dispatch` | `""` / `"0"` / `"rN"` | 派发就绪需求，最多达 `job_capacity`（4）；`draft` 进 `planning`，`ready`（`planning` + `plan_ok`）认领泳道进 `developing`。 |
| `proposal.submit` | `"<req_id>\|<rev>\|<stage>\|<summary>\|<artifacts>\|[<changes>]\|[<ext_id>][\|exec=<agent_id>[:<reason>]][\|actor=<actor_id>][\|actor_session=<session_id>]"` | 见 §3。 |
| `proposal.accept` | `""` / `"0"` / `"pN"` / `"rN"` | 接受头部（或指定）待决提案；`guide` 模式需菜单票据。 |
| `proposal.reject` | `"<pN_or_rN>[\|<reason>]"` | 拒绝待决提案并把需求退回其阶段队列。 |
| `gate.approve` | `""` / `"0"` / `"rN[\|<note>]"` | 通过 `plan_gate`（→ `ready` + 响应式自动派发）或 `accept_gate`（→ `done` + 内联泳道释放）；要求 `gate_rev == req.rev`，`accept_gate` 另要求非空 `artifacts`。 |
| `gate.reject` | `"<rN>[\|<feedback>]"` | 拒 `plan_gate`（`plan_ok=false`）或 `accept_gate`（保留 `plan_ok`），递增 `rev`，返回 `planning`。 |
| `requirement.amend` | `"<req_id>\|<title>\|<description>\|<deps>"` | 空段保留当前值；`deps="-"` 清空依赖；递增 `rev`。 |
| `requirement.split` | `"<parent_id>\|<child_title_1>\|<child_title_2>[\|...]"` | 开发前需求拆为 ≥ 2 个 `draft` 子需求。 |
| `requirement.merge` | `"<rA,rB>\|<merged_title>\|<merged_desc>"` | ≥ 2 个开发前需求合并为一个带并集依赖的新 `draft`。 |
| `requirement.conflict` | `"<rA,rB>\|<note>"` | 两需求标记 `blocked` 直到解决。 |
| `requirement.resolve` | `"<rA,rB>\|<keep_both\|keep_first\|revise>[\|<note>]"` | 清除冲突并应用所选策略。 |
| `coordinate.submit` | `""` / `"0"` | 所有关闭前置条件满足时从 `coordinate → review` 推进（**批边界**）。 |
| `verify.submit` | `"all_green=<true\|false>[;report=<text>]"` | 在 `review` 中记录验证结果。 |
| `settlement.evolution` | `"<artifact>\|<summary>[\|c=<kept>/<screened>[\|cls=<kind>:<n>,...]][\|sessions=<moved>/0/<dest>][\|todo=0/<verified_by>]"` | 一个事务内注册演进筛选并结算会话 / todo 清理（**每批一次**）。 |
| `review.note` | `"<lesson>\|<score_1_to_5>"` | 归档 `settlement.evolution.*` 进 `review.evolution.*`；未武装 = 批收口回 `coordinate`（`run.batch` +1），`run.retire` 武装 = 退役终局。 |
| `config.set` | `"<key>=<value>"` | 更新 `settings.<key>`：`play.mode`、`goal.authorization`、`run.retire`（`true`/`""` 武装 / 解除）、`executor.<rN>.<stage>`、`evolution.mode`、`gates.require_all_green`、`review.evolution.*`。 |
| `vfs.write` | `"artifacts/<name>.md\|<content>"` | 经 `ygg.fs.write` 写入 `run.sqlite`（`vfs_files`），并在 `artifact.*` 记 SHA-256 + 字节长度。 |

## 3. exec / actor / actor_session 尾段链

保留尾段链的相对序**固定**为 `exec → actor → actor_session`，各自可独立缺席：

- `exec=` 必须在链中 `actor=` 之前，并**同事务**绑定执行者台账（`executor.<rN>.<stage>`）；
- `actor=` / `actor_session=` 由**宿主注入**（声明路由 + `--actor` / `YGG_ACTOR` / daemon 请求字段）；调用方自带同名段，宿主侧即拒（`ACTOR_FORGED`），不 dispatch、不持久化；
- Mod 侧对错位段逐段点名拒绝：`req_prop_exec_misplaced` / `req_prop_actor_misplaced` / `req_prop_actor_session_misplaced`；
- 三态互证（`verified` / `mismatch` / `unverified`）**识别不拒绝**；`session` 是纯审计字段，永不参与判定。

判据：① 阻断——在 payload 里手写 `actor=` / `actor_session=`，或把 `exec=` 放到 `actor=` 之后；② 症状——`ACTOR_FORGED` 主机拒、或 Mod 侧 `*_misplaced` 拒绝；③ 确认——`cargo test -p ygg-sandbox --test agent_seed_semantics` 之外的身份用例在 `crates/ygg-sandbox/examples/egg/identity/tests.rs`，Mod 侧错位用例在 `.ygg/mods/agent-seed/tests/wire.lua`。
