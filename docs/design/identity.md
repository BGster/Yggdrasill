# Yggdrasill · 身份与权限（不进剧情）

> **定位**：执行者台账、提交者身份章、profile 卡与人机票据门四机制及其「不改变任何微状态」定性。
> **replaces**：DESIGN §2.13（L372–382）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../README.md) · **相关**：[wire-protocol.md](wire-protocol.md) · [lanes.md](lanes.md)
> **搬运**：以下正文自 DESIGN.md L372–L382 逐字迁移，不重写语义。

### 2.13 身份与权限（不进剧情）

两套身份机制 + 一套人机门禁。**定性**：它们**不改变任何微状态**——是事件的**准入校验与留痕**，落在 payload 尾段（§2.6）、进度投影（§2.10）与事件日志；剧情推进只认 §2.4 转场表。每个微状态的「主人」字段 = 人 ⇒ 该动作走票据。

- **执行者台账**（`settings.executor.<rN>.<stage>`，stage ∈ `plan` / `development`）：自声明的审计轨迹，值 `<agent_id>[:<reason>]`；同一 (需求， 阶段) 换人或跨阶段交接到不同 id 必须带原因。占位 id（`main` / `default` / `unregistered`）在恢复决策中被过滤。台账是**权责权威**（谁执行了这一幕），不做身份认证。
- **提交者身份章**（宿主注入 `actor=` / `actor_session=`）：解析链 `--actor` > `YGG_ACTOR` > daemon 请求字段；mod 侧三态互证（`exec=` vs `actor=`：`[verified]` / `[mismatch]` / `[unverified]`）**识别不拒绝**——裁决权留在人与门禁。`actor_session` 是纯审计字段，永不参与判定。诚实威胁模型：防误记与张冠李戴、让冒用可见可审计，不做密码学（D-MA12）。
- **profile 卡**（`profile.<handle>.*`）：`config.set` 逐键注册的自报身份卡（`harness` / `session_id` / `role` / `assignments`），回答「这个句柄是谁」；与台账互链不冗余（渲染层字符串 join，权责仍以台账为准）。
- **人机门禁**（`HUMAN.ACTIONS`，5 个，维持不动，R5）：`proposal.accept` / `gate.approve` / `coordinate.submit` / `verify.submit` / `review.note`。`guide` 模式下**只收菜单票**（`ygg.choice_selected` 回复索引），直发一律拒绝；`goal` 模式下主代理读完整回执后自行派发（`goal.authorization` 逐局登记，证据标准不变）。新事件（`charter.set` / `run.begin` / `release.*` / `integration.*`）不进票据门：`integration.*` 由执行者发（执行者结构上无菜单），`release.*` 由主代理直发（与 `settlement.*` 同口径）。

---

