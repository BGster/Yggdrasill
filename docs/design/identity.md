# Yggdrasill · 身份与权限（不进剧情）

> **定位**：执行者台账、提交者身份章、profile 卡与人机票据门四机制及其「不改变任何微状态」定性。
> **replaces**：DESIGN §2.13（L372–382）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../README.md) · **相关**：[wire-protocol.md](wire-protocol.md) · [lanes.md](lanes.md)
> **搬运**：以下正文自 DESIGN.md L372–L382 逐字迁移，不重写语义。

### 2.13 身份与权限（不进剧情）

两套身份机制 + 一套人机门禁。**定性**：它们**不改变任何微状态**——是事件的**准入校验与留痕**，落在 payload 尾段（§2.6）、进度投影（§2.10）与事件日志；剧情推进只认 §2.4 转场表。每个微状态的「主人」字段 = 人 ⇒ 该动作走票据。

- **执行者台账**（`settings.executor.<rN>.<stage>`，stage ∈ `plan` / `development`）：自声明的审计轨迹，值 `<agent_id>[:<reason>]`；同一 (需求， 阶段) 换人或跨阶段交接到不同 id 必须带原因。占位 id（`main` / `default` / `unregistered`）在恢复决策中被过滤。台账是**权责权威**（谁执行了这一幕），不做身份认证。
- **提交者身份章**（宿主注入 `actor=` / `actor_session=`）：子代理在 `proposal.submit` 载荷尾段携带 `exec=<agent_id>`，并在环境中声明 `YGG_ACTOR=<agent_id>`；派发侧已知 session 时再声明 `YGG_ACTOR_SESSION=<session_id>`。解析链 `--actor` > `YGG_ACTOR` > daemon 请求字段（session 对称），宿主只在声明路由盖章；调用方伪造保留段 = `ACTOR_FORGED`，零 dispatch、零持久化。Mod 对 `exec=` vs `actor=` 做 `[verified]` / `[mismatch:actor=…]` / `[unverified]` 三态互证，**识别不拒绝**——主代理无宿主章代交也只显示 unverified，裁决权留在人与门禁。`actor_session` 是纯审计字段，永不参与判定。诚实威胁模型：防误记与张冠李戴、让冒用可见可审计，不做密码学（D-MA12）。首条带身份的成功写先到先得认领不可变 `opener_actor`；失败或拒绝步不认领，同 actor 重写为 no-op。
- **profile 卡**（`profile.<handle>.*`）：子代理按 brief 的 Register 行以 `config.set profile.<handle>.harness=…` 等**每键一次** `config.set` 自报身份卡（`harness` / `session_id` / `role` / `assignments`）；主会话注册只是 fallback，数据面无差异，可信度差别留在 `registered_by` 约定与事件日志。profile 回答「这个句柄是谁」，不是凭据；与台账互链不冗余，权责始终以执行者台账为准。
- **人机门禁**（`HUMAN.ACTIONS`，5 个，维持不动，R5）：`proposal.accept` / `gate.approve` / `coordinate.submit` / `verify.submit` / `review.note`。`guide` 模式下**只收菜单票**（`ygg.choice_selected` 回复索引），直发一律拒绝；`goal` 模式下主代理读完整回执后自行派发（`goal.authorization` 逐局登记，证据标准不变）。新事件（`charter.set` / `run.begin` / `release.*` / `integration.*`）不进票据门：`integration.*` 由执行者发（执行者结构上无菜单），`release.*` 由主代理直发（与 `settlement.*` 同口径）。

维护责任 = 修改提交模板、宿主身份注入、执行者登记或 profile 协议的执行者；触发 = 任一上述变更；检查 = inline-exec、ACTOR_FORGED、profile roundtrip、opener 与三态互证语义用例。把三态提示升级为自动拒绝、把 session 当判定依据或把 profile 当凭据，均属契约回归。

---

