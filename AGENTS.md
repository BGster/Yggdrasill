# Yggdrasill · Agent Harness 强制约束与冷启动

> 本文件只写两件事：不能违反的约束，以及怎么开始 / 继续 / 暂停 / 结束。玩法细节（状态机 / 动作协议 / 异步 inbox /
> 进度查看 / runs 产物 / Mod 维护 / host 契约）见 `.ygg/mods/agent-seed/README.md` 与 playbook 的 `harness` 章；
> 现场规则用 `guide.show <section_id>` 按章节取。`CLAUDE.md` 是本文件的软链。

## 一、强制约束（不可协商）

**0. 数据红线**：不得把真实业务数据（`*.xlsx` / `*.xls` / `*.csv`、涉密挂载目录下的任何文件）读入仓库或上下文，
也不得对其目录做 `ls` / `stat` / `find` / `du` / `wc` 之类的元信息读取（行数、列名、枚举值同样涉密）。
需要真实数据的验证由用户运行并回贴输出；`/tmp` 下自造的合成数据不受限。

1. **局内一切产出必须先有事件承载**：写文件、改代码、跑命令产出交付物之前，必须先有对应事件把它纳入
   状态机（`requirement.dispatch` → 提案 → 门禁）。**判定只看事件，不看产物落在哪**——落在 `~/tmp`、
   落在 `/tmp`、落在对话里都算局内产出；“事小”“顺手”“用户没说要走流程”都不构成例外。
2. **主 Agent 在协调层只做三件事**：派活、串行验收提案、过两个强制门禁；不替泳道写实现。
   需求建单后第一个动作是 `requirement.dispatch`，不是自己先干。
   - **派发后登记执行者**（自声明账本，不是身份认证）：拿到 subagent id 后
     `config.set executor.<rN>.<stage>=<id>`（stage ∈ analysis / plan / development）。同一 (req,stage) 换人、
     或与前一阶段 id 不同的**跨阶段换人**，都必须带原因（`=<id>|<原因>`）；`amend` 不改绑定，`split` / `merge`
     不继承；`proposal.submit` 不做硬校验——未登记只在 `requirement.list` 显示「未登记」。
3. **回执整段呈现，禁止解析式截断**：不得 `narrative[0][:N]`、不得只取首行、不得按 `startswith` 做行级过滤、
   不得把回执塞进 `grep` / `python` 后只看摘要。每次派发把 narrative **全部行**打到对话里，
   并把 `pendingChoice` 原文交给用户。要规则就 `guide.show <section_id>` 取整章，不靠记忆。
4. **单发 ≠ 合规**：不批量刷 `--event` 只防脚本刷状态，替代不了 `requirement.dispatch` → proposal → gate 这条链；
   不手改 `autosave.json` / `events.jsonl`。**goal 下直派是合法路径**（见 §一·8），但产出仍须有事件链；
   `--drive cmd` 不得用来伪装决策（它是宿主通道，不是决策者）。
5. **违规即停**：自查或被告知绕过时，停止一切推进，如实列出「派发了什么 / 自己做了什么 / 哪些产出没有事件」，
   等用户指示；**不得先补齐流程再报告**——补票会把违规洗成合规记录。
6. **凭据只在 `.env` / `.env.local`**：Mod 资源、配置、报告只出现键名，不出现值。
7. **自进化只走两个出口：`AGENTS.md` 或 Mod；不得写旁路脚本、不得改 CLI 源码**：
   - 发现纪律没被遵守、要“自我进化”时，**只有两个合法落点**——本文件（写成可判定判据）或
     `.ygg/mods/agent-seed/`（闸门改 `src/flow.lua` 与 `src/handlers/*.lua`，提示词与剧本改 `data/*.json`；
     `mod.lua` 只是装配入口，不装业务逻辑）。两者可同时改，但**不得新增第三种载体**。
   - **禁止写检查脚本 / wrapper / hook 当补丁**：不得新增 “check-*.py”、“pre-*-hook”之类“帮自己守纪律”的脚本——
     那只是把纪律外包给一个自己写的、同样会被自己绕过的检查器。脚本只允许做**只读查询**
     （`requirement.list` / `map.show` / `events.jsonl` 只读统计），且**不得成为约束载体**。
   - **禁止改 ygg-agent CLI 源码**（`crates/ygg-sandbox/examples/agent.rs` 与 host 契约）。CLI 是不可协商的宿主，
     不是本仓库的实现面；确有必须由宿主解决的能力缺口，写清现象与复现交用户定，不自行 patch。
   - **判据三问，缺一不可**：① 它拦的是哪个**具体动作**？② 违规时**看得见的表现**是什么
     （拒绝文案 / 状态不变 / 载荷被拒）？③ 下次用什么**确认它生效**（一条可跑的命令或一次探针）？
     三问答不全 = 没写完，不得声称已进化完毕。
   - **口头承诺不算进化**：不得用“我会注意 / 下次读全”作结论；没有落在上面两个落点的，一律视为未处理。
   - **先筛后写**：局末建议进 backlog 前先按 Mod 的 `evolution` 章筛——长期优先（换任务 / 换仓库还会遇到吗）；
     已有未用必须查根因（调用链 / 可发现性 / 契约适配 / 主动不用 / 确无场景 / 冗余退役，六选一）；
     逐项独立复核（artifact 必含`文件:行号` + 可跑命令与实际输出 + 语义依据 + 根因分类与结论）；
     场景先于技术（实际场景 → 流程哪一步不满足 → 代码与复现 → 根因 → 长期收益 → 改造面与验证）。
     单次任务约束、临时工具与探针口径、已在当次解决的问题默认不进。
   - **声明 ≠ 证明**：`review.note` 的 `ev=` / `c=` / `cls=` 只是声明登记（Mod 只校路径形状、
     计数与分类自洽）；沙箱读不到 artifact，也不认识 subagent 会话——不得据此声称已证明建议质量或复核独立性。

### 8. goal / guide：决策权在谁手里

- `play.mode` **只由 Mod 解释**：`settings.play.mode`（运行期覆盖）> `data/config.json` 的静态默认；
  宿主 CLI 不读它，也不解释它。
- **guide**（默认）：里程碑动作（`proposal.accept` / `gate.approve` / `coordinate.submit` / `verify.submit` /
  `review.note`）只认菜单票据——必须经 `ygg.choice_selected`（回序号）进来，单发直派一律【动作被拒】。
  打回类（`*.reject`）不在闸内：打回不产生通过效果，是唯一的纠错通道。
- **goal**：主 Agent 读全回执后**自行派发**（票据不参与判定）。不是「自动选序号 1」，也不经任何外部脚本；
  `pendingChoice` 照常完整输出，证据标准不变。
- **切模式是决策权移交**：用户当次授权后，主 Agent 可自行执行
  `--event config.set --payload '"play.mode=goal"'`；Mod 会播 `handover_goal`（代办范围 + 证据标准不变 +
  收局复盘要交代）/ `handover_guide`（决策权回到人）。菜单只提供「切到另一模式」的边。
- **goal 授权是 run 级记录**：`config.set goal.authorization=<单行说明>`（trim ≤200、无换行；空值 = 显式清除）。
  只有**运行期进入 goal** 的局（`settings.play.mode=goal`）在 `review.note` 收口前校验：缺记录则旁白阻断、
  不写 `ending`、不推进；静态 `data/config.json` 预设 goal 视为仓库所有者常设配置，**不在校验范围**；
  切回 guide 不校验，resume 保留。该记录只证明「登记过一句话」，不证明用户说过——不得据此声称身份认证。
- `--drive stdin|cmd:<路径>|none` 是**宿主**的驱动通道（人工选择 / 外部命令 / 只看不推进），与 `play.mode` 正交；
  goal 不依赖它，删掉外部决策命令不影响 goal 可用。

### 9. 验收与收口边界（状态机强制，不靠自觉）

- **需求级**：`proposal.accept` 只按阶段推进——`analysis` → `planning`；`plan` → **`plan_gate`**；
  `development` → **`accept`**（执行方交付，同时释放泳道）。
- **两个强制门禁**：`gate.approve` 在 `plan_gate` → `ready`，在 `accept` → `done`；
  `gate.reject` 分别回 `planning` / `ready`。
  批准前校验两条硬约束：`gate_rev == req.rev`（待批期间被 amend → 判 stale，旧批准不得生效）、
  `accept` 类还要求 `artifacts` 非空（不接受只有 summary 的完成自称）。
- **收口前件**（`coordinate.submit` / `verify.submit=true` / `review.note` 共用同一套）：0 条待验收提案 ∧
  0 个在跑 job ∧ 0 个未解决冲突 ∧ 全部需求处于终态（`done`，或已 split / merged / dropped 且后继全都 done）。
- `coordinate.submit` 只在 `coordinate` 阶段可用，成功即进 `review` 并把 `verify.passed` 置 false；
  `verify.submit` 在 `review` 记录全绿（`all_green=false` 在 `gates.require_all_green=true` 时打回 `coordinate`）；
  `review.note` 写 `review.lesson` / `review.score` / `review.evolution.*`（`ev=` / `c=` / `cls=` 可选段，
  缺省即显式 0/0）/ `ending` 后进 `done`（【结局）。**`c` 的 `screened > 0` 时 `ev` 与 `cls` 必填**（收口前拒），
  0/0 可只带 `ev`（复核后无候选）。**运行期 goal 局**还要求 `settings.goal.authorization` 非空（缺则旁白阻断，
  静态默认 goal 不校验，见 §8）。
- 宿主靠 `agent.summary` 的 `nextEndsRun` 判定「下一步会收口」：仅当 review 阶段菜单首项是 `review.note` 时为真，
  此时先切 `closing`（阻止新写入 + 最后一次 drain 排在收口之前）再派发。

## 二、冷启动

### 构建与安装
CLI 来源只有一个：本仓库的 `crates/ygg-sandbox/examples/agent.rs`（宿主本体，契约驱动 + fail-closed）：

```bash
YGGDRASILL_ROOT="${YGGDRASILL_ROOT:-.}"                  # 本仓库根（或任意检出路径）
(cd "$YGGDRASILL_ROOT" && cargo build --release --example agent)
mkdir -p .ygg/bin && cp "$YGGDRASILL_ROOT/target/release/examples/agent" .ygg/bin/ygg-agent
```

宿主与 Mod 版本不匹配时装载期直接失败（错误文案点名缺失字段 + 给出应有形状），不静默降级；
`.ygg/bin/*` 与 `.ygg/runs/*` 在 `.gitignore` 内，`.ygg/mods/` 入库。

### 开始
```bash
.ygg/bin/ygg-agent --mod .ygg/mods/agent-seed --run-id <局名> --runs-root .ygg/runs --event event.start
```

### 继续
```bash
A=".ygg/bin/ygg-agent --mod .ygg/mods/agent-seed --run-id <局名> --runs-root .ygg/runs"
$A --resume --drive stdin                                     # 交互：人来选（缺省）
$A --resume --event config.set --payload '"play.mode=goal"'   # 自行接管决策：读全回执、按处境直接派发（可随时切回 guide）
$A --resume --drive none                                      # 只看当前处境，不推进
$A --resume --event guide.show --payload '"harness"'          # 查规则：按章节 id 定点查阅（harness / credentials / gates …）
$A --resume --event map.show                                  # 当前节点 + 出边；加 --payload '"full"' 看完整设计图
.ygg/bin/ygg-agent --run-id <局名> --runs-root .ygg/runs --requirement '<标题>|<描述>'   # 异步补需求/提案，只追加 inbox，可与运行中的局并发
```

### 暂停
- 交互模式输入 `q`，或 stdin 结束。
- 外部驱动（`--drive cmd:<路径>`）返回 `HUMAN[:理由]`、超时、非零退出、非法输出、越界序号——一律安全暂停，
  状态不推进，不会回退选 1。**goal 不依赖该通道**。
- 暂停后重新执行「继续」里的命令即可，不会丢状态。

### 结束
结束用 `.ygg/bin/ygg-agent --run-id <局名> --runs-root .ygg/runs --settle`，固定结算顺序（不可跳步）：

1. 确认所有需求进入 done / split / merged / dropped 等终态、inbox 无未消费行，并通过 `verify.submit`
   记录全绿证据（收口前件见 §一·9）。
2. `coordinate.submit` → `review`，再 `review.note` 收口到 `done`（写复盘经验、评分与进化候选声明；
   候选先按 `evolution` 章筛，登记用 `ev=artifacts/<file>.md|c=<kept>/<screened>|cls=<kind>:<n>,...`；
   guide 下可先 `config.set review.evolution.*` 登记，菜单会把已登记值带进载荷）。
3. 执行 `--settle` 并核验 `settlement.json` 的 `ok`（四项开关：事件非空 / 无失败步 / 出现结局行 / inbox 已消费）。
4. **清空主会话 Todo 列表**（`todo clear`，含已完成与历史任务）后再结束本局；不得把已结束局的任务带入下一局。
