# agent-seed · 双层状态机 + 提案验收的元工作流 Mod

M-Agent 泳道的 seed mod：把「一次完整的 Agent 工作」配置成一个可游玩的 Mod——协调器派活/验收，
每条需求各自跑 analyze → plan → 门禁 → develop → 交付 → 门禁。任何 Agent 装载它 + 注入任务描述
即可开工，**零定制**。怎么玩（构建 / 开局 / 单发 / 异步 / 结算）见仓库根 `AGENTS.md`；本文件只说
Mod 内部结构。

## 一、协调器（`fsm.stage`，单机单事件 `stage.advance`）

```text
intake ──task.intake──▶ coordinate ──coordinate.submit──▶ review ──review.note──▶ done
                           ▲                                 │
                           └──────── verify.submit=false ─────┘（门禁失败打回）
```

- `coordinate.submit` 有硬门禁：全部叶子需求 `done` ∧ 0 待验收提案 ∧ 0 在跑 job ∧ 0 未解决冲突。
- `review` 里 `verify.submit=true` 只记录门禁证据（要求全绿时才允许之后收口），`review.note` 才收口。

## 二、需求级流水线（`req.<i>.status`）

```text
draft ─dispatch→ analyzing ─proposal→ analysis_review ─accept→ planning
      ─dispatch→ planning(job) ─proposal→ plan_review ─accept→ plan_gate
      ─gate.approve→ ready ─dispatch→ developing(lane) ─proposal→ dev_review
      ─accept→ accept(执行方交付) ─gate.approve→ done
```

| 状态 | 含义 |
| --- | --- |
| `draft` / `planning` / `ready` | 可派活（分别派 analysis / plan / development） |
| `analyzing` / `developing` | job 在跑（`req.<i>.job` 标记阶段；开发另占 `lane.<k>`） |
| `analysis_review` / `plan_review` / `dev_review` | 提案已到、待主 Agent 串行验收 |
| `plan_gate` | **强制门禁**：plan 批了才能 `ready` → 开发 |
| `accept` | **执行方已交付**（自检 + artifacts），不等于验收；等 accept_gate |
| `done` / `split` / `merged` / `dropped` | 终态（拆并/弃用保留可追溯性） |

另有标记：`conflict`（+`conflict_peer`，展示为 `blocked`）、`needs_amend`（待修订）、
`stale`（提案 rev 过期）、`gate_rev`（门禁绑定 revision）。

**两个强制门禁**（真实状态 + 动作约束，goal 也不能跳）：

1. `plan_gate`：`gate.approve` → `ready`；`gate.reject` → `planning`（重做）。
2. `accept` → accept_gate：`gate.approve` 要求 **artifacts 非空** → `done`；`gate.reject` → `ready`（重进开发队列）。
   两者都校验 `gate_rev == req.rev`，待批期间被 amend 即判 stale，不得批准。

## 三、提案协议（subagent 只建议，不改共享状态）

```text
proposal.submit  "r1|1|analysis|summary|artifacts[|changes[|external_id]]"
proposal.accept  "pN" / "rN" / ""      → 空 = requirement_id canonical 序取一条（串行验收）
proposal.reject  "pN|理由" / "rN|理由" / ""
```

- `stage ∈ analysis | plan | development`，必须与需求当前状态匹配（`analyzing` / `planning`(job) / `developing`）。
- `changes` 是**拟议变更**，验收时才应用：`split=a,b` / `merge=r5` / `amend=标题:描述` / `conflict=r5:说明`。
- 验收时 `rev ≠ req.<i>.rev` → 提案置 `stale`、需求置 `stale` 标记，可读拒绝（旧提案不得覆盖新决定；
  重提新 rev 即可）。`external_id` 幂等（`【提案重放】`）。
- 多个需求可以同时在 analyze / plan / develop（job 上限 4），**验收一次只处理一条需求**。

## 四、主 Agent 的需求图命令（串行）

| 动作 | 载荷 | 说明 |
| --- | --- | --- |
| `requirement.split` | `父id\|子A\|子B[|…]` | 父置 `split`，子需求 `draft` 并**继承父依赖**（≥2 子） |
| `requirement.merge` | `id1,id2\|标题\|描述` | 源置 `merged`（`into` 指向新需求），新需求依赖取**并集** |
| `requirement.amend` | `id\|标题\|描述\|依赖` | rev+1；`draft/planning/ready` 直改、开发中只改说明、`done` **派生新需求** |
| `requirement.conflict` | `id1,id2\|说明` | 双向标 `blocked`；冲突需求不派活、不许收口 |
| `requirement.resolve` | `id1,id2\|keep_both\|keep_first\|revise` | 保留双方 / 放弃第二条 / 保留但置待修订 |

终态（`split`/`merged`/`dropped`）不再接受修订与冲突；`coordinate.submit` 会把拆并的**后继**一并要求 done。

## 五、job 槽位与开发泳道

- `tuning.job_capacity`（默认 4）：分析 / 规划 / 开发共用 job 槽位（`req.<i>.job` 非空即占用）。
- 开发额外占 lane（`lane.<k>.req`）；只有「`ready` ∧ 依赖全部满足 ∧ 无冲突 ∧ 不需要修订」的需求能进开发。
- 相互声明冲突的需求不可能同时 `developing`（冲突标记让两者都不可调度）。
- **内核仍单线程 dispatch + 单 autosave**：泳道只是宿主侧并行工作单元，结果一律经 inbox 回来，
  宿主按 `requirement_id` / `external_id` canonical 序合流。

## 六、增量配置

- 静态 `data/config.json` + 动态 KV `settings.*`（`config.set`）：`play.mode`（guide/goal）、
  `gates.require_all_green`、`evolution.mode` / `evolution.max_per_run`。
- `tuning`：`req_capacity`（需求上限 32）、`job_capacity`（4）、`min_slices` / `max_slices`（历史遗留）、
  `default_score`。

## 七、文件

| 文件 | 作用 |
| --- | --- |
| `mod.lua` | Mod 本体：协调器 FSM / 需求流水线 / 提案与门禁 / 泳道 / 渲染 / 解析 |
| `data/scenes.json` | 文本资源（scenes / endings / prompts / options / beats / tuning） |
| `data/playbook.json` | 现场规则资源（`guide.show` 按 section id 取用） |
| `data/config.json` | 静态配置默认值 |
| `pack.json` / `manifest.json` | 打包声明与内容寻址清单（`cargo run --example pack` 生成） |
| `schemas/*.json` | 资源 schema（与 `packages/schema/schemas/` 单源一致） |

## 八、走查

```bash
# 默认全 1 贪心：intake → coordinate.submit → verify → review.note → 【结局
cargo run --example playtest -- .ygg/mods/agent-seed

# 多需求 multi-stage（外部提案 / goal 占位提案均可）：
#   .ygg/bin/ygg-agent --run-id t --runs-root .ygg/runs --requirement '需求A|描述'
#   .ygg/bin/ygg-agent --run-id t --runs-root .ygg/runs --proposal 'r1|1|analysis|结论|a.md'
#   .ygg/bin/ygg-agent --mod .ygg/mods/agent-seed --run-id t --runs-root .ygg/runs --resume

# 改完 data/ 或 schemas/ 必跑（manifest 内容寻址）
cargo run --example pack -- .ygg/mods/agent-seed
```
