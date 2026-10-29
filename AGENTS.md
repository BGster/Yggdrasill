# Yggdrasill · Agent Harness 游玩指南

> **本文件只教怎么玩，不装规则。**
> 内核同时是 Agent harness：一次完整的 Agent 工作 = 一个可游玩的 Mod（`.ygg/mods/agent-seed`）。
> 每步回执 = 处境叙事 + 进度 + **pendingChoice**（当前允许的动作集——协议约束，不是提示词约束）。
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

## 2. 开局（冷启动）

```bash
.ygg/bin/ygg-agent --mod .ygg/mods/agent-seed \
  --run-id my-task --runs-root .ygg/runs --event event.start
```

- `--run-id` 只允许字母数字与 `-` `_` `.`；产物落 `<runs-root>/<run-id>/`（`--runs-root` 缺省 `./runs`，本项目约定 `.ygg/runs`）。
- 回执是 **stdout 单行 JSON**：

  ```json
  {"ok":true,"event":"event.start","narrative":["…"],"pendingChoice":{"prompt":"…","options":["…"]},
   "autosave":".ygg/runs/my-task/autosave.json","seq":0,"revision":0}
  ```

- **怎么读**：`narrative` = 这一步发生了什么（叙事行，最后一行若是 `【结局…` 即本局收口）；
  `pendingChoice` = 下一步的合法动作（`null` = 局末无待选）；加性键 `warnings`（内核警告 + 宿主提示）、
  `hotReload`（本次派发前的热载结果）、`ending`（本步出现结局行）；失败时 `error` 带稳定码 + 文案，
  并补一次 `state` 回填当前 `pendingChoice`（出错后仍能接着走）。
- 退出码：`0` ok / `1` 内核错误或装载失败 / `2` 用法错误。
- 交互模式：省略 `--event`（按序号推进，`q` 退出；加 `--watch` 每步检查 Mod 改动并热载）；
  它按 `play.mode` 运行——`guide` 逐步确认（缺省）/ `goal` 挂机，见 §3。

---

## 3. 运行模式：`goal` 挂机 / `guide` 逐步确认

交互模式（省略 `--event`）按 **`play.mode`** 运行：`guide`（缺省）逐步确认，`goal` 挂机自动推进。

| 模式 | 行为 | 退出 |
| --- | --- | --- |
| `guide` | 每一步推进都要你输入（提示符 `[guide] >`），Mod 不会自动连跑 | `q` / 输入结束 |
| `goal` | 不读 stdin：自动取 `pendingChoice` 序号 1（推荐主动作）推进，到结局自动结算 | 结算码（与 `ok` 同值） |

```bash
# goal 挂机：先写动态配置，然后交互模式直接跑完一局（无需任何 stdin）
.ygg/bin/ygg-agent --mod .ygg/mods/agent-seed --run-id my-task --runs-root .ygg/runs \
  --resume --event config.set --payload '"play.mode=goal"'
.ygg/bin/ygg-agent --mod .ygg/mods/agent-seed --run-id my-task --runs-root .ygg/runs \
  --resume < /dev/null

# guide 逐步确认（缺省）：同样一条命令、不给 goal——每一步都要你输入序号
.ygg/bin/ygg-agent --mod .ygg/mods/agent-seed --run-id my-task --runs-root .ygg/runs --resume
```

- **两层配置**：静态 `data/config.json` 的 `play.mode`（缺省 `guide`）+ 动态 KV
  `settings.play.mode`（`config.set play.mode=<goal|guide>` 写入），判定恒为「动态覆盖 > 静态默认」。
  CLI **每步现读**——中途 `config.set` 下一步即生效（guide 会话里改成 goal 就转入挂机，一直到结局）。
- **分阶段提示词差异**：每个阶段两版文案，按当前模式取一（goal = `<stage>_goal`，
  guide = `<stage>_guide`），每阶段回执顶部固定一行 `【运行模式】play.mode = …`：
  - guide：`逐步确认（guide）：… **这一步由你确认后提交** …`
  - goal：`挂机（goal）：无需确认——宿主自动…`（各阶段写各自的推进动作，不是一句全局套话）
- **失控防护**（goal）：最多 64 步；派发失败 / 无 `pendingChoice` 且未到结局 → 可读错误 + 退出码 `1`。
- **单发模式不受影响**：`--event` 一次只派发一个事件，不自动连跑（Agent 的调用语义）。

---

## 4. 单发推进（Agent 脚本化）

```bash
.ygg/bin/ygg-agent --mod .ygg/mods/agent-seed --run-id my-task \
  --runs-root .ygg/runs --resume --event <动作> --payload '<JSON 标量>'
```

`--payload` 必须是 **JSON 标量**（整数/浮点/布尔/字符串；缺省 `"0"`）——内核 payload 只有四标量，
复合载荷压成**单个字符串**：

| 动作 | 载荷（注意外层 JSON 引号） | 迁移 |
| --- | --- | --- |
| `task.intake` | `'"title\|description"'`（空串 = 默认占位任务） | intake → decompose |
| `plan.submit` | `'"s1=标题; s2=标题; s3=标题"'`（1–8 片；只给标题则自动编号） | decompose → implement |
| `slice.report` | `'"s1\|summary\|a.md,b.md"'`（顺序上报，跳片会被拒） | implement → implement / verify |
| `verify.submit` | `'"true"'` 或 `'"all_green=false;report=门禁输出摘要"'` | verify → review（全绿）/ implement（要求全绿时打回） |
| `review.note` | `'"lesson\|4"'`（评分 1–5，缺省 3） | review → done（叙事以 `【结局` 收口） |
| `config.set` | `'"play.mode=goal"'`、`'"gates.require_all_green=false"'`、`'"evolution.mode=suggest"'`（`settings.` 前缀可省） | 任意阶段，**不改变**主状态 |
| `guide.show` | `'"credentials"'`；`'""'` = 本阶段默认节 | 任意阶段，**不改变**主状态 |
| `map.show` | `'""'` / `'"0"'` / `'"full"'`（三者同一张全图） | 任意阶段，**不改变**主状态 |

- 越阶动作（当前阶段不允许）→ `【动作被拒】` + 当前阶段允许动作清单；载荷不合法 → `【载荷被拒】` + 原因。
  两种拒绝都**原地重渲染**（主状态不变、pendingChoice 保持），改对载荷重发即可。
- 两层配置：`data/config.json` 是静态默认，`config.set` 写 KV `settings.*` 动态覆盖，判定恒为「动态覆盖 > 静态默认」。

---

## 5. 查规则与节点图：`guide.show` · `map.show`

```bash
.ygg/bin/ygg-agent --mod .ygg/mods/agent-seed --run-id my-task \
  --runs-root .ygg/runs --resume --event guide.show --payload '"gates"'
```

- 12 个 section id：`credentials`、`repository`、`branches`、`lanes`、`dispatch`、`integration`、
  `gates`、`live_evidence`、`evidence`、`output`、`pointers`、`hard_constraints`。
- 回执 = 该节全文：标题 + 摘要 + 每条原子规则（带「必须 / 禁止 / 应当」标记与依据），
  以及该节附带的验证命令、仓库布局、文档指针、备注。
- 载荷为空（`'""'`）→ 取**本阶段默认节**；非法 id → `【指南被拒】` + 全部合法 id，主状态不变。
- 每阶段回执已带一行 `现场规则 · <节标题>：<摘要>` 提示，`pendingChoice` 里紧随主前进动作也有
  `guide.show` 入口。默认节按阶段轮换（不写状态，纯按进度计数）：

  | 阶段 | 默认节 |
  | --- | --- |
  | `intake` | `output`（语言与输出） |
  | `decompose` | `integration`（收口流程） |
  | `implement` | `repository` → `lanes` → `dispatch`（逐片轮换） |
  | `verify` | `gates`；被打回后转 `live_evidence` |
  | `review` | `evidence` / `hard_constraints`（按计划规模） |

- 想一次看全：连续 `guide.show` 12 个 id，或直接读 `.ygg/mods/agent-seed/data/playbook.json`。

### 看节点图：`map.show`

本 Mod 的「剧情」就是那台六态工作流，`map.show` 把它渲染成一张节点图（节点 + 带动作名的边），
常驻只读——不推进任何状态。

```bash
.ygg/bin/ygg-agent --mod .ygg/mods/agent-seed --run-id my-task \
  --runs-root .ygg/runs --resume --event map.show --payload '"full"'
```

```text
【剧情节点图 · 六态工作流】当前 >> [implement] << ← 你在这里｜进度 1/3 片
  [intake]    ──task.intake──▶ [decompose]
  [decompose] ──plan.submit──▶ [implement]
▶ [implement] ├─slice.report──▶ [implement]（非最后一片，同态继续）
              └─slice.report──▶ [verify]（全部切片后）
  [verify]    ├─verify.submit=true──▶ [review]
              └─verify.submit=false──▶ [implement]（打回，重做最后一片）
  [review]    ──review.note──▶ [done]（局末【结局）
  [done]      ──（局末·无出边）
常驻动作（不推进主图，任意阶段可用）：config.set · guide.show · map.show——任务/切片/门禁/配置一律不变
```

- 头行读三件事：`>> [<stage>] << ← 你在这里` = KV `fsm.stage` 的当前节点；`｜进度 X/Y 片` =
  已上报切片 / 计划片数（`plan.submit` 之前不显示）；行首 `▶` = 当前节点的出边行
  （`├─` / `└─` 是同一节点的分支，缩进行是续行）。
- 载荷只接空 / `0` / `"full"`（同一个全图，不做局部视图）；其他值 → `【地图被拒】` + 用法，
  主状态不变。看完照旧按 `pendingChoice` 继续——地图是旁挂只读，不占掉主前进动作。
- `pendingChoice` 顺序固定为「主前进 → `guide.show` → `map.show` → `config.set`（`play.mode` →
  `evolution.mode` → `gates`）」，两个只读动作都在改配置之前；每阶段回执顶部还有一行
  `【运行模式】play.mode = …`（当前模式与它的推进纪律，见 §3）。

---

## 6. runs 产物与局末结算

| 文件 | 内容 |
| --- | --- |
| `autosave.json` | 每步自动存档（引擎版本 + 实体表 + KV + 待决选择），可直接作 `load` 载荷 |
| `events.jsonl` | 每派发一行：`{seq, event, payload, ok, narrative_count, timestamp, ending, ending_line?}` |
| `run.json` | `{run_id, mod, mod_signature, started_at, updated_at}`（热载比对与结算取 `mod`） |
| `settlement.json` | 局末结算报告（仅 `--settle` 写出） |

```bash
.ygg/bin/ygg-agent --run-id my-task --runs-root .ygg/runs --settle
```

`ok` = 事件日志非空 ∧ 无失败步 ∧ 出现结局叙事行；退出码与 `ok` 同值。结算摘要会打出终局面
（`ending` / `score` / `exp`）与 KV 摘要。

---

## 7. 改完 Mod 之后

```bash
cargo run --example pack -- .ygg/mods/agent-seed        # 必跑：动了 data/ 或 schemas/ 就重算 manifest
cargo run -p ygg-lint -- .ygg/mods/agent-seed/mod.lua   # 可选：Lua 静态检查（choice 后 emit 等规则）
cargo run --example playtest -- .ygg/mods/agent-seed    # 冒烟：全 1 贪心应通关到【结局
```

- **manifest 是内容寻址**（`sha256` + `byteLength`）：手改 `data/*.json` 后不重算 `pack`，装载会以
  「内容哈希不符」拒绝。
- **热载**：单发模式每次新进程、开局即按磁盘现状装载——`--resume` 回执里 `hotReload.source="resume"`
  即说明改动已生效；交互模式加 `--watch`。重载失败会**沿用旧 Mod 继续**（回执 `warnings` 提示），不中断局。
- **存档只带状态**（实体表 / KV / 待决选择），资源不入档：改资源后 `--resume` 即刻用新资源 + 旧状态续局。

---

## 8. 本地凭据

本地凭据放 `.env` / `.env.local`（均已 gitignore，权限 600，绝不入库）。Mod 资源、静态配置、动态
`settings.*` 与报告里只出现**键名**，不出现值——完整红线见 `guide.show credentials`。
