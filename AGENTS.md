# mxbc-data-gov-workbench · Agent Harness 强制约束与冷启动

> 本文件只写两件事：不能违反的约束，以及怎么开始 / 继续 / 暂停 / 结束。玩法细节（状态机 / 动作协议 / 异步 inbox /
> 进度查看 / runs 产物 / Mod 维护 / host 契约）见 `.ygg/mods/agent-seed/README.md` 与 playbook 的 `harness` 章；项目规则现场查 `map.show`。`CLAUDE.md` 是本文件的软链。

## 一、强制约束（不可协商）

**0. 数据访问红线（最高优先级）**：**禁止读取、列出、搜索、统计、校验或写入任何真实数据文件**——涉密挂载
（`/Volumes/WenshuSpace/**`）、任何持有真实指标的 `*.xlsx` / `*.xls` / `*.csv` 及其副本，以及上述文件所在目录的
`ls` / `stat` / `find` / `du` / `wc`。「只读」「只看结构」「只数计数」「不打印数值」**都不是许可**——行数、列名、月份范围、空值计数同样涉密。

- 纯代码工作不受限；`/tmp` 下自造合成数据可任意读写；涉及真实数据的命令**由用户运行**并回贴输出；
- 仅当用户在**当次指令**里明确指定某个文件与动作时，才可对该文件执行该动作（授权不跨指令、不跨文件）。

1. **局内一切产出必须先有事件承载**：写文件、改代码、跑命令产出交付物之前，必须先有对应事件把它纳入
   状态机（`requirement.dispatch` → 提案 → 门禁）。**判定只看事件，不看产物落在哪**——落在 `~/tmp`、
   落在 `/tmp`、落在对话里都算局内产出；“事小”“顺手”“用户没说要走流程”都不构成例外。
2. **主 Agent 在协调层只做三件事**：派活、串行验收提案、过两个强制门禁；不替泳道写实现。
   需求建单后第一个动作是 `requirement.dispatch`，不是自己先干。
3. **回执整段呈现，禁止解析式截断**：不得 `narrative[0][:N]`、不得只取首行、不得把回执塞进
   `grep` / `python` 后只看摘要。每次派发把 narrative **全部行**打到对话里。硬判据：出现
   「本步规则 · …（N 条，随 map.show 输出）」就必须当场 `map.show` 取规则，再决定下一步。
4. **单发 ≠ 合规**：不批量刷 `--event` 只防脚本刷状态，替代不了 `requirement.dispatch` → proposal
   → gate 这条链；不手改 `autosave.json` / `events.jsonl`，不用 `--drive cmd` 伪装决策。
5. **违规即停**：自查或被告知绕过时，停止一切推进，如实列出「派发了什么 / 自己做了什么 / 哪些产出
   没有事件」，等用户指示；**不得先补齐流程再报告**——补票会把违规洗成合规记录。
6. **凭据只在 `.env` / `.env.local`**：Mod 资源、配置、报告只出现键名，不出现值。
7. **自进化只走两个出口：`AGENTS.md` 或 Mod；不得写旁路脚本、不得改 CLI 源码**：
   - 发现纪律没被遵守、要“自我进化”时，**只有两个合法落点**——本文件（写成可判定判据）或
     `.ygg/mods/agent-seed/`（`mod.lua` 加闸门 + `data/*.json` 改提示词与剧本）。两者可同时改，
     但**不得新增第三种载体**。
   - **禁止写检查脚本 / wrapper / hook 当补丁**：不得新增 “check-*.py”、“pre-*-hook”之类“帮自己
     守纪律”的脚本——那只是把纪律外包给一个自己写的、同样会被自己绕过的检查器。脚本只允许做
     **只读查询**（`requirement.list` / `map.show` / `events.jsonl` 只读统计），且**不得成为约束载体**。
   - **禁止改 ygg-agent CLI 源码**（`Yggdrasill` 仓库的 `examples/agent` 与 host 契约）。CLI 是不可
     协商的宿主，不是本项目的实现面；确有必须由宿主解决的能力缺口，写清现象与复现交用户定，不自行 patch。
   - **判据三问，缺一不可**：① 它拦的是哪个**具体动作**？② 违规时**看得见的表现**是什么
     （拒绝文案 / 状态不变 / 载荷被拒）？③ 下次用什么**确认它生效**（一条可跑的命令或一次探针）？
     三问答不全 = 没写完，不得声称已进化完毕。
   - **口头承诺不算进化**：不得用“我会注意 / 下次读全”作结论；没有落在上面两个落点的，一律视为未处理。

**推进动作已内置规则闸（不靠自觉）**：`task.intake` / `requirement.dispatch` / `integration.submit` /
`merge.submit` / `deploy.submit`——本阶段规则未经 `map.show` 看过之前，**一律返回【先读规则】被拒**；
标记按阶段记，进新阶段自动重新上闸。所以「先读规则再推进」不是纪律建议，而是状态机的前置条件；
回执末尾的「本步规则 · …」也已写明「未取则派活 / 报证据一律被拒」。

## 二、冷启动

### 构建与安装
CLI 来源只有一个：`Yggdrasill` 仓库的 `examples/agent`（宿主本体，契约驱动 + fail-closed）：

```bash
YGGDRASILL_ROOT="${YGGDRASILL_ROOT:-../Yggdrasill}"      # 可换成本机任意检出路径
(cd "$YGGDRASILL_ROOT" && cargo build --release --example agent)
mkdir -p .ygg/bin && cp "$YGGDRASILL_ROOT/target/release/examples/agent" .ygg/bin/ygg-agent
```

宿主与 Mod 版本不匹配时装载期直接失败（错误文案点名缺失字段 + 给出应有形状），不静默降级；`.ygg/bin/*` 与 `.ygg/runs/*` 在 `.gitignore` 内，`.ygg/mods/` 入库。

### 开始
```bash
.ygg/bin/ygg-agent --mod .ygg/mods/agent-seed --run-id <局名> --runs-root .ygg/runs --event event.start
```

### 继续
```bash
A=".ygg/bin/ygg-agent --mod .ygg/mods/agent-seed --run-id <局名> --runs-root .ygg/runs"
$A --resume --drive stdin                                # 交互：人来选（缺省）
$A --resume --drive cmd:.ygg/mods/agent-seed/decide.py   # 挂机：Mod 自带决策命令；HUMAN 即安全暂停
$A --resume --drive none                                 # 只看当前处境，不推进
$A --resume --event guide.show --payload '"harness"'   # 查规则：按章节 id 定点查阅（harness / credentials / gates …）
.ygg/bin/ygg-agent --run-id <局名> --runs-root .ygg/runs --requirement '<标题>|<描述>'   # 异步补需求/提案，只追加 inbox，可与运行中的局并发
```

### 暂停
- 交互模式输入 `q`，或 stdin 结束。
- 挂机模式决策命令返回 `HUMAN[:理由]`、超时、非零退出、非法输出、越界序号——一律安全暂停，状态不推进，不会回退选 1。
- 暂停后重新执行「继续」里的命令即可，不会丢状态。

### 结束
结束用 `.ygg/bin/ygg-agent --run-id <局名> --runs-root .ygg/runs --settle`，固定结算顺序（不可跳步）：

1. 确认所有需求完成、inbox 无未消费行 → 执行 `--settle` 并核验 `settlement.json` 的 `ok`。
2. 完成必要的合流与结果汇报，确认本局没有遗留实现或验收工作。
3. **清空主会话 Todo 列表**（`todo clear`，含已完成与历史任务）后结束本局；不得把已结束局的任务带入下一局。
