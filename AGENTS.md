# Yggdrasill · Agent Harness 游玩指南（双层状态机 + 提案验收）

> **本文件只教怎么玩，不装规则。**
> 内核同时是 Agent harness：一次完整的 Agent 工作 = 一个可游玩的 Mod（`.ygg/mods/agent-seed`）。
> 每步回执 = 处境叙事 + 双层进度 + **pendingChoice**（当前允许的动作集——协议约束，不是提示词约束）。
> 项目规则（凭据红线、仓库形态、分支/泳道、集成/部署、门禁命令、证据口径…）是 Mod 资源
> `.ygg/mods/agent-seed/data/playbook.json`，随时用 `guide.show` 现场查，不在本文件复述。
> `CLAUDE.md` 是本文件的软链。

---

## 1. 构建并安装 CLI

```bash
cargo build --release --example agent
mkdir -p .ygg/bin
cp target/release/examples/agent .ygg/bin/ygg-agent
```

`.ygg/bin/`（可执行）与 `.ygg/runs/`（本局产物）在 `.gitignore` 内；`.ygg/mods/` 入库。

---

## 2. 两层状态机（这是全部玩法）

```text
协调器（fsm.stage）
  intake ──task.intake──▶ coordinate ──coordinate.submit──▶ review ──review.note──▶ done
                             ▲                                 │
                             └──── verify.submit=false ────────┘   （门禁失败打回）

需求级流水线（每条需求一条，KV req.<i>.status）
  draft ──dispatch──▶ analyzing ──proposal──▶ analysis_review ──accept──▶ planning
       ──dispatch──▶ planning(job) ──proposal──▶ plan_review ──accept──▶ **plan_gate**
       ──gate.approve──▶ ready ──dispatch──▶ developing(lane) ──proposal──▶ dev_review
       ──accept──▶ **accept**（执行方自检交付）──gate.approve──▶ done
       另有 split / merged / dropped（可追溯终态）与 blocked（冲突）/ stale（提案 rev 过期）
```

**协调器只做四件事**：派活（`requirement.dispatch`）、串行验收提案（`proposal.accept` / `proposal.reject`）、
过两个强制门禁（`gate.approve` / `gate.reject`）、最终复盘（`verify.submit` → `review.note`）。
它**不承载开发本身**，也不允许绕过任何门禁。

### 两个强制主 Agent 门禁（不可跳过）

| 门禁 | 触发状态 | 通过后 | 驳回后 | 校验 |
| --- | --- | --- | --- | --- |
| `plan_gate` | plan 提案验收后 | `ready`（才可 dispatch 进开发） | 回 `planning` 重做 | `gate_rev == req.rev`（否则 stale 不得批） |
| `accept` → accept_gate | 开发提案验收后（执行方交付） | `done` | 回 `ready`（重新进开发队列） | 同上 + **artifacts 必须非空**（不接受只有 summary 的完成自称） |

- `accept` 是**执行方自检并交付**，不等于主 Agent 已验收；`gate.approve` 才是核验。
- 两个门禁都是真实状态 + 动作约束：处于 `plan_gate` / `accept` 的需求**不能派活、不能被跳过**，
  自动推进（`--drive cmd:…`）同样各走一次 `gate.approve`（只是序号由外部决策命令给出）。

---

## 3. 开局与推进方式

```bash
# 开局（冷启动）
.ygg/bin/ygg-agent --mod .ygg/mods/agent-seed \
  --run-id my-task --runs-root .ygg/runs --event event.start
```

交互模式（省略 `--event`）按 `--drive` 运行 —— **推进方式由外部给定，host 不替你做决策**：

| `--drive` | 行为 | 退出 |
| --- | --- | --- |
| `stdin`（缺省） | 读 stdin：每一步推进、每一次提案验收与门禁都要你输入序号 | `q` / 输入结束 |
| `cmd:<路径>` | 不读 stdin：每个待选动作交给决策命令选择；到结局自动结算 | 结算码；返回 HUMAN = 安全暂停（`0`） |
| `none` | 只打印当前回执（含 pendingChoice）就退出，不推进 | `0` |

- **`cmd:` 遇冲突必暂停**：打印冲突双方与说明后退出 `0`，**不自动选路径**；解决后重跑本命令，
  或改用 `--drive stdin` 手动选 `pendingChoice` 的 1/2/3（`keep_both` / `keep_first` / `revise`）。
- **`cmd:` 决策命令协议**：stdin 不参与。命令以 `decide-input.json`（完整当前回执）路径为唯一参数，
  stdout 只允许**一行** `N`（`1..=选项数`）或 `HUMAN[:理由]`；5 秒超时、非零退出、非法输出、
  越界序号一律 fail closed（安全暂停，状态不推进）。`YGG_DECIDE_CMD` 可临时覆盖路径。
- **不造内容**：有 job 在跑却一份待验收提案都没有时，宿主**不生成占位提案**（门禁必须由真实
  交付满足）——打印可读原因、安全暂停并退出 `0`，等外部执行方提交提案后重跑本命令继续。
- **失控防护**：阈值读 Mod 的 `data/config.json` 的 `drive` 段（缺失 / 非法逐字段回落兜底）：
  步数硬顶 = `maxSteps + stepsPerRequirement × 需求数`（兜底 `64 + 16×n`），
  另有「连续 `stallLimit` 步 `pendingChoice` 无变化」判停（兜底 `12`）。
- **`play.mode` 是 Mod 叙事层语义，host 不解释**：`config.set play.mode=<goal|guide>` 仍可切换 Mod
  的提示词 / 旁白口吻，但**推进方式只看 `--drive`**；host 既不读 `data/config.json` 的 `play.mode`，
  也不读存档 KV `settings.play.mode`。

---

## 4. 动作协议（Agent 直接派发；与菜单同一套守卫）

| 动作 | 载荷（JSON 字符串） | 作用 |
| --- | --- | --- |
| `task.intake` | `"title\|description"` | intake → coordinate |
| `requirement.add` | `"title\|description[\|external_id[\|depends_on]]"` | 入队（draft）；同 external_id 幂等 |
| `requirement.list` | `""` / `"0"` / `"full"` | 只读：需求图 + 提案 + 泳道 |
| `requirement.dispatch` | `""` | 按就绪序派活（draft→analysis / planning→plan / ready→development·lane），job 上限 4 |
| `proposal.submit` | `"r1\|1\|analysis\|summary\|artifacts[\|changes[\|external_id]]"` | subagent 提议（stage ∈ analysis/plan/development；changes 可选） |
| `proposal.accept` | `"pN"` / `"rN"` / `""` | **串行**验收一条（空 = requirement_id canonical 序）；rev 不匹配 → stale |
| `proposal.reject` | `"pN\|理由"` / `"rN\|理由"` / `""` | 打回：analysis→draft / plan→planning / development→ready |
| `gate.approve` | `"rN[|说明]"` / `""` | 过门禁（plan_gate → ready；accept → done） |
| `gate.reject` | `"rN\|反馈"` / `""` | 门禁打回（plan_gate → planning；accept → ready） |
| `coordinate.submit` | `""` | 全部叶子需求 done ∧ 无待验收提案 ∧ 无在跑 job ∧ 无冲突 → review |
| `verify.submit` | `"true"` / `"all_green=false;report=…"` | review 内门禁自检（失败打回 coordinate） |
| `review.note` | `"lesson\|score(1-5)"` | 收口到 done（叙事以【结局 收口） |
| `requirement.split` | `"父id\|子标题A\|子标题B[|…]"` | 主 Agent 直改：父置 split，子需求继承依赖（≥2 子） |
| `requirement.merge` | `"源id1,源id2\|合并标题\|合并描述"` | 主 Agent 直改：源置 merged，新需求依赖取并集 |
| `requirement.amend` | `"id\|新标题\|新描述\|新依赖"` | 主 Agent 直改：rev+1；done 修订**派生新需求**；开发中只改说明 |
| `requirement.conflict` | `"id1,id2\|说明"` | 主 Agent 标记冲突（blocked，派活与收口都跳过） |
| `requirement.resolve` | `"id1,id2\|keep_both\|keep_first\|revise[|说明]"` | 冲突解决三条路径 |
| `config.set` / `guide.show` / `map.show` | 同既有语义 | 常驻只读/配置动作 |
| `agent.summary` | `"0"` | 只读：Mod 自报**回执标准字段**（阶段 / 结局 / 待验收提案 / 冲突 / 空等 job） |

- **宿主不读 Mod 私有 KV**：`agent.summary` 回一行 `【摘要】<单行 JSON>`，宿主只认这一行
  （`stage` / `ending` / `verifyPassed` / `nextEndsRun` / `pendingProposals` /
  `requirementCount` / `conflicts` / `awaiting`）。旧 Mod 没有该动作时宿主判为「未知」
  并安全停等，**不回退到读 `ending` / `fsm.stage` / `prop.*` / `req.*`**。

- `changes` 形如 `split=a,b` / `merge=r5` / `amend=标题:描述` / `conflict=r5:说明`：
  subagent 只**建议**，`proposal.accept` 时才落地。
- **stale 与重定基**：验收时 `rev ≠ req.<i>.rev` → 提案置 `stale`、需求置 `stale` 标记（可读拒绝，
  旧提案不得覆盖新决定）。恢复路径：提交**同阶段、当前 rev** 的新提案即可——
  `analysis_review` / `plan_review` / `dev_review` 在「该需求**没有**待验收提案」时接受这种重定基提案；
  仍有正常待验收提案时一律拒绝（`已有一条待验收提案…先 accept / reject`），不覆盖。
  重定基提案落地后需求的 `stale` 标记自动清除。
- 越阶动作 → `【动作被拒】`；载荷不合法 → `【载荷被拒】`；前置不满足 → `【需求待办】`。
  三者都原地重渲染（主状态不变）。
- 收口判据：`req_all_done()`（含 split/merged 的后继全部 done）+ 0 待验收提案 + 0 在跑 job + 0 冲突。

---

## 5. 异步入口（inbox：另一个进程随时进来）

inbox 是 JSONL：`<runs-root>/<run-id>/inbox.jsonl`，每行

```json
{"external_id":"inbox-…","kind":"add|amend|split|merge|conflict|resolve|proposal","title":…,"description":…,"payload":"…","created_at":…}
```

| 命令 | kind | 说明 |
| --- | --- | --- |
| `--requirement '标题\|描述'` | add | 新增需求（title/description 字段） |
| `--proposal 'r1\|1\|analysis\|summary\|artifacts[\|changes]'` | proposal | **外部 subagent 提交提案**；external_id 自动补第 7 段 |
| `--amend 'r1\|新标题\|新描述\|依赖'` | amend | 异步修订（rev+1） |
| `--split 'r1\|子A\|子B'` | split | 异步拆分命令 |
| `--merge 'r2,r3\|标题\|描述'` | merge | 异步合并命令 |
| `--conflict 'r1,r2\|说明'` | conflict | 异步标冲突（`cmd:` 驱动会停下问用户） |
| `--resolve 'r1,r2\|keep_both'` | resolve | 异步解决冲突 |

```bash
.ygg/bin/ygg-agent --run-id my-task --runs-root .ygg/runs \
  --proposal 'r1|1|analysis|影响面：core.rs 两个函数|notes/analysis-r1.md'
```

- **不必给 `--mod`**：这些命令只做 `OpenOptions::append`，不读写 autosave，可与运行中的本局并发。
- **消费时机**：`cmd:` 每次自动推进前、`stdin` 每次提示前、单发每次派发前 drain 新行并按 kind 派发。
- **跨进程写入/收口协议**（协议级互斥，不是「多读一次文件」）：
  run 目录下有 `coordination.lock`（create_new 互斥，陈旧按 mtime 2s 抢占）与
  `coordination.json`（`{phase, writers, owner_pid, heartbeat_at}`）：
  1. 写命令先登记：`phase=running` → 计入 `writers` 并保持到行落盘；
     `phase=closing`（心跳未过期）→ **命令本身非零拒绝（exit 3）**；
     `phase=finalized` → **非零拒绝（exit 3，需开新局）**。
  2. 自动推进在派发最后的 `review.note` 前把 phase 切到 `closing`，**等在途写者落盘归零**，
     然后在同一保护下做最后一次 drain：
     · 有新需求且未 done → 解除 closing、继续 loop（不结局）；
     · 无新需求 → 同一保护下完成 `review.note` + settle，随后 `phase=finalized`。
     因此「写命令成功但运行侧已跨过最后一次 drain」在协议上不可能发生：
     写入要么 happens-before 那次 drain，要么被拒。
  3. 陈旧行与崩溃恢复：closing 用心跳续命（10s 过期），过期后写命令或 `--resume`
     会把它判死并恢回到 `running`；锁也按 mtime 抢占。两者都不会因进程崩溃永久卡死。
  4. 独立 `--settle` 同样走 closing：已有未消费行 → `ok=false` 并退回 running（可 resume 消费）；
     已 finalized → 幂等返回旧报告，不重写。
- **幂等 / 崩溃安全**：cursor（`inbox.cursor` 的 `{consumed:N}`）只在 dispatch 成功且存档落盘后前移；
  重放靠 `external_id`：
  - `add` / `proposal` → Mod 侧 per-entity ext 查重（`【需求重放】` / `【提案重放】`）；
  - `amend` / `split` / `merge` / `conflict` / `resolve` → 载荷末尾 `@ext=` + `inbox.last` 去重（`【异步重放】`）。
- 坏行跳过并前移（否则永久阻塞）；**结算硬门禁**：`--settle` 若发现未消费行 → `ok=false` 且非零
  （写完还要再核一次，结算期间又到的行会让报告作废）。因此「结局后投递」不会静默成功，只能开新局。
- **跨进程时序约定**：自动推进在每次可能收口的动作前都会再 drain 一次（推进前、`coordinate.submit` /
  `verify.submit` / `review.note` 之前、自动 settle 之前），运行期间到达的行**不得漏**；
  但进程退出后的投递属于新局（`inbox` 保留待新局消费，`settlement` 会报错提示）。

---

## 6. 看进度：`requirement.list` 与 `map.show`

```bash
.ygg/bin/ygg-agent --mod .ygg/mods/agent-seed --run-id my-task --runs-root .ygg/runs \
  --resume --event map.show --payload '"full"'
```

- `requirement.list`：需求图逐条（状态 + 依赖 + rev + 泳道 + job + 冲突/stale 追溯）、待验收提案列表、
  泳道台账、图例。
- `map.show`：**双层图**——协调器 4 态节点图 + 需求级流水线（含两个门禁）+ job/泳道台账。
- `pendingChoice` 顺序：解决冲突（如有）→ 主前进动作（`proposal.accept` > `gate.approve` >
  `requirement.dispatch` > `coordinate.submit`）→ `guide.show` → `map.show` → `requirement.list` →
  `requirement.add` → `config.set`（play.mode → evolution.mode → gates）。

---

## 7. runs 产物与局末结算

| 文件 | 内容 |
| --- | --- |
| `autosave.json` | 每步自动存档（实体表 + KV + 待决选择），可直接作 `load` 载荷 |
| `events.jsonl` | 每派发一行：`{seq,event,payload,ok,narrative_count,timestamp,ending,ending_line?}` |
| `run.json` | `{run_id,mod,mod_signature,started_at,updated_at}` |
| `settlement.json` | 局末结算（仅 `--settle` 写出） |
| `inbox.jsonl` | 异步操作队列（只追加；运行侧消费） |
| `inbox.cursor` | 已消费行数（dispatch + 存档成功后才前移） |
| `coordination.json` | 跨进程协议：`{phase, writers, owner_pid, heartbeat_at}`（running / closing / finalized） |
| `coordination.lock` | 短临界区互斥锁（create_new；陈旧按 mtime 抢占，崩溃不卡死） |

```bash
.ygg/bin/ygg-agent --run-id my-task --runs-root .ygg/runs --settle
```

`ok` = 事件日志非空 ∧ 无失败步 ∧ 出现结局叙事行 ∧ **inbox 无未消费行**；退出码与 `ok` 同值。
未消费行存在时报告会带 `结局后仍有 N 条未消费 inbox 行——需开新局` 的错误且 `ok=false`，
写盘后还会再核一次（结算期间新到的行 → 报告重写为 `ok=false`、退出非零）。

固定结算顺序：

1. 确认所有需求完成、inbox 已消费，执行 `--settle` 并核验 `settlement.json`。
2. 完成必要的合流与结果汇报，确认本局没有遗留实现或验收工作。
3. **清空主会话 Todo 列表**（`todo clear`，含已完成与历史任务）后再结束本局；不得把已结束局的任务带入下一局。

---

## 8. 改完 Mod 之后

```bash
cargo run --example pack -- .ygg/mods/agent-seed        # 动了 data/ 或 schemas/ 必跑
cargo run -p ygg-lint -- .ygg/mods/agent-seed/mod.lua   # Lua 静态检查（应 0 error / 0 warning）
cargo run --example playtest -- .ygg/mods/agent-seed    # 冒烟：默认全 1 走查到【结局
```

- **manifest 是内容寻址**（sha256 + byteLength）：改 `data/*.json` 不重算 `pack`，装载会以内容哈希不符拒绝。
- **热载**：单发模式每次新进程按磁盘现状装载；`--resume` 回执里 `hotReload.source="resume"` 即说明改动已生效。
- 两个 Mod（`.ygg/mods/agent-seed` 与 `packages/mods/agent-seed`）现在是**同一份实现与资源**（含 playbook），
  改动要同步两侧并各自重算 manifest。

---

## 9. 本地凭据

本地凭据放 `.env` / `.env.local`（均已 gitignore，权限 600，绝不入库）。Mod 资源、静态配置、动态
`settings.*` 与报告里只出现**键名**，不出现值——完整红线见 `guide.show credentials`。
