# Yggdrasill · 身份与权限（不进剧情）
> **id** mod/identity · **模块** ③ agent-seed mod · **触发** 改执行者台账 / 身份判定 / profile 之前
> **读不到代价** silent-drift（把自声明当身份证据）

> **定位**：执行者台账、提交者身份章、profile 卡与人机票据门四机制及其「不改变任何微状态」定性。
> **replaces**：DESIGN §2.13（L372–382）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../../README.md) · **相关**：[wire-protocol.md](../kernel/wire-protocol.md) · [lanes.md](../host/lanes.md)
> **搬运**：以下正文自 DESIGN.md L372–L382 逐字迁移，不重写语义。

### 2.13 身份与权限（不进剧情）

两套身份机制 + 一套人机门禁。**定性**：它们**不改变任何微状态**——是事件的**准入校验与留痕**，落在 payload 尾段（§2.6）、进度投影（§2.10）与事件日志；剧情推进只认 §2.4 转场表。每个微状态的「主人」字段 = 人 ⇒ 该动作走票据。

- **执行者台账**（`settings.executor.<rN>.<stage>`，stage ∈ `plan` / `development`）：自声明的审计轨迹，值 `<agent_id>[:<reason>]`；同一 (需求， 阶段) 换人或跨阶段交接到不同 id 必须带原因。占位 id（`main` / `default` / `unregistered`）在恢复决策中被过滤。台账是**权责权威**（谁执行了这一幕），不做身份认证。
- **提交者身份章**（宿主注入 `actor=` / `actor_session=`）：子代理在 `proposal.submit` 载荷尾段携带 `exec=<agent_id>`，并在环境中声明 `YGG_ACTOR=<agent_id>`；派发侧已知 session 时再声明 `YGG_ACTOR_SESSION=<session_id>`。解析链 `--actor` > `YGG_ACTOR` > daemon 请求字段（session 对称），宿主只在声明路由盖章；调用方伪造保留段 = `ACTOR_FORGED`，零 dispatch、零持久化。Mod 对 `exec=` vs `actor=` 做 `[verified]` / `[mismatch:actor=…]` / `[unverified]` 三态互证，**识别不拒绝**——主代理无宿主章代交也只显示 unverified，裁决权留在人与门禁。`actor_session` 是纯审计字段，永不参与判定。诚实威胁模型：防误记与张冠李戴、让冒用可见可审计，不做密码学。首条带身份的成功写先到先得认领不可变 `opener_actor`；失败或拒绝步不认领，同 actor 重写为 no-op。
- **profile 卡**（`profile.<handle>.*`）：子代理按 brief 的 Register 行以 `config.set profile.<handle>.harness=…` 等**每键一次** `config.set` 自报身份卡（`harness` / `session_id` / `role` / `assignments`）；主会话注册只是 fallback，数据面无差异，可信度差别留在 `registered_by` 约定与事件日志。profile 回答「这个句柄是谁」，不是凭据；与台账互链不冗余，权责始终以执行者台账为准。
- **人机门禁**（`HUMAN.ACTIONS`，5 个，维持不动，R5）：`proposal.accept` / `gate.approve` / `coordinate.submit` / `verify.submit` / `review.note`。`guide` 模式下**只收菜单票**（`ygg.choice_selected` 回复索引），直发一律拒绝；`goal` 模式下主代理读完整回执后自行派发（`goal.authorization` 逐局登记，证据标准不变）。新事件（`charter.set` / `run.begin` / `release.*` / `integration.*`）不进票据门：`integration.*` 由执行者发（执行者结构上无菜单），`release.*` 由主代理直发（与 `settlement.*` 同口径）。
- **stamp-only 路由**（r9，`pack.json` 的 `inboxRoutes`）：`config.set` / `gate.approve` / `coordinate.submit` 三条各一张路由，只声明 `actor: true`、**不声明 `write`**——宿主据此为直接 `--event` 通道在载荷末尾盖 `|actor=<id>`，却不会长出任何 CLI 写命令（`write_binding` 只匹配带 `write` 的路由）。即：身份章的**准入面**扩到三个推进动作，**写面**一字不动（inbox 不是绕过门禁的旁路）。代价两条：① 这三条配置值不得再含字面 `|actor=`（`ACTOR_FORGED`，与 `proposal.submit` 同口径）；② 盖章段必须先剥后解析——`config.set` / `gate.approve` / `coordinate.submit` 三个 handler 顶部调 `wire.strip_identity_tail`，否则 `read.x=<ver>|actor=me` 会被当成坏 ver、`note` 会吞掉一个尾巴。
- **按人读签**（r9）：读签的语义是「**执行这个动作的人**读过」，不是「本局有人读过」——多代理局是常态，不区分签署人的台账会把别人的签名记成自己的（实证：r6 子代理的签名被主代理的 `gate.approve` 计入 `2/2 signed`）。载体是**侧键** `settings.read_actor.<doc>`（值形状 `ver|scene|seq` 与 `settings.read_index` 一并冻结：翻倍值形状会打红既有断言并让销票多一处要清的字段），由 `readsig.sign` 同事务写、`forget` / 4 个 handler 里的 7 处 `clear()` 同事务清。判定侧（`gates.read_problem`）的身份链：宿主章（直接派发）→ 无章但 `HUMAN.entry == "ticket"`（人工菜单点击）→ **降级**为 any-signer 并叙事告明「不区分签署人」（人工点击结构上无章，若也 fail-closed 就把 `guide` 锁死）；无章且非票据 → **拒**（能带而不带 = 门被绕开）。新增第五维 `read-gate/other-actor`：签了但不是你签的——空签署人（r3 旧存档）不参与这条判定，旧存档不得因机制升级被一夜判死。回执把两个口径并列（`k/n` = 本幕覆盖几件，`mine` = 其中**你自己**签了几件），因为单报 `k/n` 就是「像我已签全」的歧义本身。
- **amend 由销票覆盖，不再单设主体维（r11）**：`req_bump_rev` 在同一事务里调 `readsig.clear()`，所以 amend / 两路打回推进 rev 之后签名**已被销光**，下一个推进动作报的是**覆盖维 `read-gate/missing`**——「签名早于新 rev」这个组合在 amend 路径上**结构性不可达**。旧的主体维（`read-gate/subject`）因此是从未生效的假保护，r11 已删除该维与它的拒台词 beat。它当年唯一可达的路径（签名 → 之后 `requirement.add`新需求）现在由**新鲜维** `read-gate/stale` 接管，所以删除没有开口子；守护用例 `read_gate_handles_a_signature_taken_before_a_new_requirement_without_a_subject_dimension` 把这一维加回去即转红。现行判据 = **覆盖 / 新鲜 / 遗忘 + 按人**。

维护责任 = 修改提交模板、宿主身份注入、执行者登记、profile 协议或读签归属的执行者；触发 = 任一上述变更；检查 = inline-exec、ACTOR_FORGED、profile roundtrip、opener 与三态互证语义用例，外加 r9 的四条归属用例（他人签署拒 / 无身份直派拒 / 票据降级 / 侧键同生共死）与 `read_actor_side_key_dies_with_the_signature`。把三态提示升级为自动拒绝、把 session 当判定依据、把 profile 当凭据、或把「本局有人读过」当成「执行者读过」，均属契约回归。 r11 另加守护用例 `read_gate_handles_a_signature_taken_before_a_new_requirement_without_a_subject_dimension`（把主体维加回去即转红）与 `read_gate_amendment_loses_signatures_through_the_clear_not_a_subject_dimension`（amend 报覆盖维的机械证据）。

### 批边界的身份载体：覆盖检查，不再是硬门

`coordinate.submit` 在批边界的身份判定降为**覆盖检查**（本幕必读是否由执行者签过），不再额外要求一个身份载体。它曾是 `goal` 模式的死锁源：批收口动作由主代理直发、结构上不带宿主章，若按缺章 fail-closed，`goal` 局永远关不掉这一批。降级后它与人机门禁共用同一个 `gates.read_problem` 判定，票据与直派两条通道各自拿到正确的降级语义。

**依据**：身份章的用途是“让冒用可见”，不是“拦住推进”；把一个它本来拿不到的载体当作硬前置，换来的不是安全而是死锁。反方案（为批收口单独造一个身份载体）会多出一条只能由主代理走的身份路径，而那正是本机制要消除的“两套真相”。

**判据**：① 阻断——给 `coordinate.submit` 加回“必须有身份章”的前置；② 症状——`goal` 模式批收口被拒且 `pendingChoice` 始终停在 `coordinate.submit`；③ 确认——`cargo test -p ygg-sandbox --test agent_seed_semantics` 的批收口与门禁独立性用例。

### 设计依据（身份与权限为什么这样分）

四类载体、四份职责，不得互相顶替：**执行者台账**（`settings.executor.<rN>.<stage>`）= 权责权威（谁执行了这一幕，自声明）；**提交者身份章**（宿主注入 `actor=` / `actor_session=`，仅在声明路由盖章）= 准入证据；**profile 卡** = 句柄自报（“这个句柄是谁”，不是凭据）；**人机票据门**（5 个 pass 型动作）= 人类的通过权。三条不可动摇：

1. **读签的语义是“执行这个动作的人”读过**（按人，不是按局）——多代理局不区分签署人的台账会把别人的签名记成自己的（实证：子代理的签名被主代理的门禁动作计入 2/2 signed）。载体是侧键 `settings.read_actor.<doc>`，与读签同事务写、同事务清；
2. **签名随 `rev` 推进被同事务销光**——`req_bump_rev` 在同一事务里调 `readsig.clear()`，所以 amend / 两路打回之后下一个推进动作报的是**覆盖维**，「签名早于新 rev」**结构性不可达**。因此旧的主体维是从未生效的假保护，已删除该维与其拒台词 beat；它当年唯一可达的路径现在由**新鲜维**接管，删除没有开口子。
3. **`actor_session` 是纯审计字段，永不参与判定**——它能带而不带、带了也可能是别的会话，拿它做裁决等于把审计日志当门锁。

**依据**：身份在这里解决的是“责任可追溯 + 冒用可见”，不是“防伪造”——Mod 与宿主跑在同一个沙箱里，能改配置的人也能改路由声明，真正的边界只有沙箱本身。因此每加一道“身份门”都只是把一个可读的失信改成一次不可解释的拒绝。反方案（把三态提示升级为自动拒绝、把 profile 当凭据、把 session 当判定依据）均会先锁死合法路径，再因为绕过手段存在而无法补回安全。

**判据**：① 阻断——把三态提示改成自动拒绝、把 `session` 当判定依据、把 profile 当凭据，或把“本局有人读过”当成“执行者读过”；② 症状——`read-gate/other-actor` 不报却实际由他人签名；amend 后报的是“签名过期”而不是“未签”；`guide` 模式下人工点选被身份门锁死；③ 确认——`cargo test -p ygg-sandbox --test agent_seed_semantics` 的 inline-exec / `ACTOR_FORGED` / profile roundtrip / opener / 三态互证 / 四条归属用例 / `read_actor_side_key_dies_with_the_signature` / 两条主体维守护用例全组。

---

