# Yggdrasill · 持久化泳道池与实时进度投影

> **定位**：泳道 worktree 池的分池认领/释放（r1/D-MA24：默认池 + 具名池）、强制分支命名、绑定窗口与三态实时投影。
> **replaces**：DESIGN §2.10（L340–351；r1/D-MA24 分池语义并入）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../README.md) · **相关**：[requirement-story.md](requirement-story.md) · [cli-daemon.md](cli-daemon.md)
> **搬运**：以下正文自 DESIGN.md L340–L351 逐字迁移，不重写语义。

### 2.10 持久化泳道池与实时进度投影

- **非破坏性认领（`worktree-claim`）**：分配 `<pool-root>/<repo>-lane-<k>`（`k ∈ 1..4`；默认池根 `$HOME/tmp/worktrees`，`--root` / `$YGG_WORKTREE_ROOT` 可覆盖）。worktree 已存在且已跟踪工作树干净时 `git checkout -B <branch> HEAD`，使未跟踪构建缓存（`target/`、`node_modules/`）跨任务存活。
- **分池泳道（r1，D-MA24）**：台账以池为一级键——默认池（""，运行仓 CWD toplevel 自己的池）沿用旧键 `lane.<k>.*`（字节级兼容，旧局可读）；具名池为 `lane.<label>.<k>.*`，label 形状 `^[a-z][a-z0-9-]{0,23}$`（首字符字母，与纯数字 k 键空间零冲突；保留字 `seq`/`pools`）。`lane.pool "label|path"` 幂等注册池（`lane.pools` 插入序 + `lane.pool.<label>.path`，path 形状校验、不验是否 git 仓）；`lane.claim`/`lane.release` 载荷尾部带可选池段（缺省 = 默认池）；需求侧双写 `req.<i>.lane` + `req.<i>.lane_pool`。并行度**按池计**（每池 k ∈ 1..job_capacity）；自动派活只从默认池取道，多仓需求由主 Agent 先注册+预挂牌。工具桥新增 `git.worktree_claim_pool` / `git.worktree_release_pool` / `git.lane_probe_pool`（args 固定尾参 `--repo {{repo}}`，池归属从**请求快照** `tool.<id>.args` 的 repo 段反查——宿主输出行键序冻结不携带 `--repo`；反查失败 fail-closed 拒绝且账本不动）。冲突雷达只在**同池**内两两比对探针文件（跨池文件树无关，同名文件不再误报）。宿主侧已知限制：目录前缀是派生值（toplevel basename，两个同名 basename 的检出会共享池目录）；`--repo` 只收本地已存在的检出路径。
- **非破坏性释放（`worktree-release`）**：验证干净后 `git checkout --detach HEAD`，解锁分支而不删除目录。
- **强制分支命名（`branch_problem`，r2）**：泳道分支必须满足 `feature/{4位uuid}-{语义词段}`——小写 4 位 uuid 段 + 语义词段（≥3 连续字母、非纯数字、8–16 位纯 hex 拒绝、总长 ≤48、前缀大小写敏感）。`lane.claim` 与 `git.worktree_claim` 两条入口同规则、拒绝通道不同：前者走 `flow.refuse` 可读拒绝（`reject.count/last` 留痕），后者走叙事拒绝；未自报名 = 可读拒绝，不生成默认名。
- **泳道绑定窗口（R7）**：`待开发 → 开发中` 认领；保持到 集成 confirm/reject、验收门禁 reject、交付受理 reject 或 done 才释放。`交付受理 → 验收门禁 → 集成` 三幕期间**不释放**——执行者仍需该工作树完成 feature→develop 集成（D-MA22）。
- **执行者身份投影（r3）**：`requirement.list` trace 并列三态互证后缀（`[verified]` / `[mismatch:actor=…]` / `[unverified]`，判定只看 `exec=` vs 宿主 `actor=` 章）；`agent.summary.profiles` 数组（≤8 张，handle 升序）投影 profile 注册表（自报身份卡，与账本互链不冗余，见 §2.13）。
- **实时状态投影**：`requirement.list` 与 `summary.jobs[]` 每行需求投影三条从 KV 派生的实时状态串：
  - `worktreeStatus`：`"n/a"` | `"pending"` | `"ready:<path>@<branch>"` | `"bound:<lane标签>@<branch>"`（默认池 `lane-<k>`，具名池 `lane-<label>-<k>`）
  - `subagentStatus`：`"idle"` | `"pending"` | `"dispatched:<agent_id>"` | `"delivered:p<N>(<agent_id>)"`
  - `probeStatus`：`"n/a"` | `"pending"` | `"verified:commits=<N>,clean=<0|1>"`

