---
type: "Reference"
title: "身份与权限（不进剧情）"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-f2294347eab5094b120f7ba9
    resource: repo://.ygg/mods/agent-seed/src/domain/executor.lua
  - id: openwiki-source-20786f1cdc5649019aa1a3b9
    resource: repo://.ygg/mods/agent-seed/src/domain/gates.lua
  - id: openwiki-source-d57e21e5bbbce0b80489fd49
    resource: repo://.ygg/mods/agent-seed/src/domain/requirements.lua
  - id: openwiki-source-a4428d3a250a714e2b67c998
    resource: repo://.ygg/mods/agent-seed/src/domain/wire/proposal.lua
  - id: openwiki-source-b640c22b435c5b19de7f36dd
    resource: repo://.ygg/mods/agent-seed/src/handlers/system/config.lua
  - id: openwiki-source-a320e603ef2519a7baf859a5
    resource: repo://crates/ygg-sandbox/examples/egg/identity/tests.rs
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# 身份与权限（不进剧情）

> **id** mod/identity · **模块** ③ agent-seed mod · **触发** 改执行者台账 / 身份判定 / profile 之前
> **读不到代价** silent-drift（把自声明当身份证据）
> **replaces**：DESIGN §2.13（L372–382）+ 批边界身份载体（搬运基线 `3a2536f`）
<!-- openwiki: broken internal link [state-machine.md] file "state-machine.md" does not exist. Fix the href or restore the target, then delete this comment. -->
> **上游**：[architecture/README.md](../../architecture/README.md) · **相关**：[../kernel/wire-protocol.md](../kernel/wire-protocol.md) · [../host/lanes.md](../host/lanes.md) · [state-machine.md](../kernel/state-machine.md)

## 1. 定性：四类载体，不改变任何微状态

身份机制是事件的**准入校验与留痕**，落在 payload 尾段、进度投影与事件日志；剧情推进只认状态机契约。四类载体四份职责，**不得互相顶替**：

| 载体 | 存在形式 | 职责 | 不是什么 |
| --- | --- | --- | --- |
| **执行者台账** | `settings.executor.<rN>.<stage>`（stage ∈ `plan` / `development`） | **权责权威**：谁执行了这一幕（自声明审计轨迹，值 `<agent_id>[:<reason>]`） | 不是身份认证 |
| **提交者身份章** | 宿主注入 `actor=` / `actor_session=` | **准入证据**：仅在被声明为 `actor: true` 的路由盖章 | 不是调用方可写字段 |
| **profile 卡** | `profile.<handle>.*` | **句柄自报**：这个句柄是谁（`harness` / `session_id` / `role` / `assignments`） | 不是凭据 |
| **人机票据门** | `HUMAN.ACTIONS`（5 个 pass 型动作） | **人类的通过权** | 不是状态推进 |

每个微状态的「主人」字段 = 人 ⇒ 该动作走票据，不是走身份章。

## 2. 执行者台账

同一 (需求, 阶段) 换人或跨阶段交接到不同 id **必须带原因**；占位 id（`main` / `default` / `unregistered`）在恢复决策中被过滤。台账不做认证，只做权责归属；`profile` 与台账互链但不冗余——**权责始终以台账为准**。

## 3. 提交者身份章

子代理在 `proposal.submit` 尾段携带 `exec=<agent_id>`，并在环境声明 `YGG_ACTOR=<agent_id>`（已知 session 时再声明 `YGG_ACTOR_SESSION=<session_id>`）。解析链 `--actor` > `YGG_ACTOR` > daemon 请求字段（session 对称），宿主**只在声明路由盖章**；调用方伪造保留段 = `ACTOR_FORGED`，零 dispatch、零持久化。

Mod 对 `exec=` vs `actor=` 做三态互证：`[verified]` / `[mismatch:actor=…]` / `[unverified]`，**识别不拒绝**——主代理无宿主章代交只显示 unverified，裁决权留在人与门禁。`actor_session` 是纯审计字段，**永不参与判定**。首条带身份的成功写先到先得认领不可变 `opener_actor`；失败或拒绝步不认领，同 actor 重写为 no-op。

**诚实威胁模型**：防误记与张冠李戴、让冒用可见可审计，**不做密码学**——Mod 与宿主跑在同一沙箱里，能改配置的人也能改路由声明，真正的边界只有沙箱本身。因此每加一道「身份门」都只是把一个可读的失信改成一次不可解释的拒绝。

## 4. 人机票据门（维持 5 个）

`proposal.accept` / `gate.approve` / `coordinate.submit` / `verify.submit` / `review.note`。`guide` 模式**只收菜单票**（`ygg.choice_selected` 回复索引），直发一律拒绝；`goal` 模式主代理读完整回执后自行派发（`goal.authorization` 逐局登记，证据标准不变）。新事件（`charter.set` / `run.begin` / `release.*` / `integration.*`）不进票据门：`integration.*` 由执行者发（执行者结构上无菜单），`release.*` 由主代理直发（与 `settlement.*` 同口径）。

**stamp-only 路由**：`config.set` / `gate.approve` / `coordinate.submit` 三条各一张 `inboxRoutes` 路由，只声明 `actor: true`、**不声明 `write`**——宿主据此为直接 `--event` 通道在载荷末尾盖 `|actor=<id>`，却不会长出任何 CLI 写命令（`write_binding` 只匹配带 `write` 的路由）。即：身份章的**准入面**扩到三个推进动作，**写面一字不动**。代价两条：① 这三条配置值不得再含字面 `|actor=`（`ACTOR_FORGED`）；② 盖章段必须先剥后解析——三个 handler 顶部调 `wire.strip_identity_tail`，否则 `read.x=<ver>|actor=me` 会被当成坏 ver、`note` 会吞掉一个尾巴。

## 5. 按人读签与四个判定维

读签的语义是「**执行这个动作的人**读过」，不是「本局有人读过」——多代理局是常态，不区分签署人的台账会把别人的签名记成自己的（实证：r6 子代理的签名被主代理的 `gate.approve` 计入 `2/2 signed`）。

- 载体是**侧键** `settings.read_actor.<doc>`（值形状 `ver|scene|seq`，与 `settings.read_index` 一并冻结），由 `readsig.sign` 同事务写、`forget` / 4 个 handler 里 7 处 `clear()` 同事务清。
- 判定侧（`gates.read_problem`）身份链：宿主章（直接派发）→ 无章但 `HUMAN.entry == "ticket"`（人工菜单点击）→ **降级** any-signer 并叙事告明；无章且非票据 → **拒**（能带而不带 = 门被绕开）。
- `read-gate/other-actor`：签了但不是你签的；空签署人（r3 旧存档）不参与这条判定，旧存档不得因机制升级被一夜判死。
- 回执把两个口径并列：`k/n`（本幕覆盖几件）、`mine`（其中**你自己**签了几件）——单报 `k/n` 就是「像我已签全」的歧义本身。

**amend 由销票覆盖，不单设主体维**：`req_bump_rev` 在同一事务里调 `readsig.clear()`，amend / 两路打回推进 rev 之后签名**已被销光**，下一个推进动作报的是**覆盖维** `read-gate/missing`——「签名早于新 rev」在 amend 路径上**结构性不可达**。旧的主体维 `read-gate/subject` 是从未生效的假保护，已删除；它当年唯一可达的路径（签名之后 `requirement.add` 新需求）现由**新鲜维** `read-gate/stale` 接管，删除没有开口子。**现行判据 = 覆盖 / 新鲜 / 遗忘 + 按人**。

守护用例：`read_gate_handles_a_signature_taken_before_a_new_requirement_without_a_subject_dimension`（把主体维加回去即转红）、`read_gate_amendment_loses_signatures_through_the_clear_not_a_subject_dimension`、`read_actor_side_key_dies_with_the_signature`。

## 6. 批边界：覆盖检查，不再是硬门

`coordinate.submit` 在批边界的身份判定降为**覆盖检查**（本幕必读是否由执行者签过），不再额外要求身份载体。它曾是 `goal` 模式的死锁源：批收口由主代理直发、结构上不带宿主章，若按缺章 fail-closed，`goal` 局永远关不掉这一批。降级后它与人机门禁共用同一个 `gates.read_problem` 判定，票据与直派两条通道各自拿到正确的降级语义。

## 7. 判据

① 阻断——把三态提示改成自动拒绝、把 `session` 当判定依据、把 profile 当凭据、把「本局有人读过」当成「执行者读过」、或给 `coordinate.submit` 加回「必须有身份章」的前置；② 症状——`read-gate/other-actor` 不报却实际由他人签名 / amend 后报「签名过期」而非「未签」/ `guide` 模式人工点选被身份门锁死 / `goal` 批收口被拒且 `pendingChoice` 停在 `coordinate.submit`；③ 确认——Mod 套件 `.ygg/mods/agent-seed/tests/identity.lua` + `tests/gates.lua`，跑 `cargo run -q --example selftest -- --mod .ygg/mods/agent-seed`（`cargo test -p ygg-sandbox --test selftest` 这个 target 不存在——旧文档的过期命令，已就地更正）。
