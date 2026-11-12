# Yggdrasill · 持久化泳道池与实时进度投影

> **定位**：泳道 worktree 池的非破坏性认领/释放、强制分支命名、绑定窗口与三态实时投影。
> **replaces**：DESIGN §2.10（L340–351）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../README.md) · **相关**：[requirement-story.md](requirement-story.md) · [cli-daemon.md](cli-daemon.md)
> **搬运**：以下正文自 DESIGN.md L340–L351 逐字迁移，不重写语义。

### 2.10 持久化泳道池与实时进度投影

- **非破坏性认领（`worktree-claim`）**：分配 `<repo-parent>/worktrees/<repo>-lane-<k>`（`k ∈ 1..4`）。worktree 已存在且已跟踪工作树干净时 `git checkout -B <branch> HEAD`，使未跟踪构建缓存（`target/`、`node_modules/`）跨任务存活。
- **非破坏性释放（`worktree-release`）**：验证干净后 `git checkout --detach HEAD`，解锁分支而不删除目录。
- **强制分支命名（`branch_problem`，r2）**：泳道分支必须满足 `feature/{4位uuid}-{语义词段}`——小写 4 位 uuid 段 + 语义词段（≥3 连续字母、非纯数字、8–16 位纯 hex 拒绝、总长 ≤48、前缀大小写敏感）。`lane.claim` 与 `git.worktree_claim` 两条入口同规则、拒绝通道不同：前者走 `flow.refuse` 可读拒绝（`reject.count/last` 留痕），后者走叙事拒绝；未自报名 = 可读拒绝，不生成默认名。
- **泳道绑定窗口（R7）**：`待开发 → 开发中` 认领；保持到 集成 confirm/reject、验收门禁 reject、交付受理 reject 或 done 才释放。`交付受理 → 验收门禁 → 集成` 三幕期间**不释放**——执行者仍需该工作树完成 feature→develop 集成（D-MA22）。
- **执行者身份投影（r3）**：`requirement.list` trace 并列三态互证后缀（`[verified]` / `[mismatch:actor=…]` / `[unverified]`，判定只看 `exec=` vs 宿主 `actor=` 章）；`agent.summary.profiles` 数组（≤8 张，handle 升序）投影 profile 注册表（自报身份卡，与账本互链不冗余，见 §2.13）。
- **实时状态投影**：`requirement.list` 与 `summary.jobs[]` 每行需求投影三条从 KV 派生的实时状态串：
  - `worktreeStatus`：`"n/a"` | `"pending"` | `"ready:<path>@<branch>"` | `"bound:lane-<k>@<branch>"`
  - `subagentStatus`：`"idle"` | `"pending"` | `"dispatched:<agent_id>"` | `"delivered:p<N>(<agent_id>)"`
  - `probeStatus`：`"n/a"` | `"pending"` | `"verified:commits=<N>,clean=<0|1>"`

