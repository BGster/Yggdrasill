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
- 交互模式：省略 `--event`（按序号推进，`q` 退出；加 `--watch` 每步检查 Mod 改动并热载）。

---

## 3. 单发推进（Agent 脚本化）

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
| `config.set` | `'"gates.require_all_green=false"'`、`'"evolution.mode=suggest"'`（`settings.` 前缀可省） | 任意阶段，**不改变**主状态 |
| `guide.show` | `'"credentials"'`；`'""'` = 本阶段默认节 | 任意阶段，**不改变**主状态 |

- 越阶动作（当前阶段不允许）→ `【动作被拒】` + 当前阶段允许动作清单；载荷不合法 → `【载荷被拒】` + 原因。
  两种拒绝都**原地重渲染**（主状态不变、pendingChoice 保持），改对载荷重发即可。
- 两层配置：`data/config.json` 是静态默认，`config.set` 写 KV `settings.*` 动态覆盖，判定恒为「动态覆盖 > 静态默认」。

---

## 4. 查规则：`guide.show`

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

---

## 5. runs 产物与局末结算

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

## 6. 改完 Mod 之后

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

## 7. 本地凭据

本地凭据放 `.env` / `.env.local`（均已 gitignore，权限 600，绝不入库）。Mod 资源、静态配置、动态
`settings.*` 与报告里只出现**键名**，不出现值——完整红线见 `guide.show credentials`。
