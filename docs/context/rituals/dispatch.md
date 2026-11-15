# Yggdrasill · 派发仪式（dispatch-ritual）
> **id** rituals/dispatch · **幕** coordinate（派发前） · **触发** 派发任何 subagent（plan / development / analysis）前
> **读不到代价** hard-refusal（`.env.local` 未 source 即派发、工件落检出、submit 缺 `exec=` 均不合规）
> **ver** 000000

**定位**：`subagent_brief` 模板的策略正文知识权威——子代理必读材料纪律、执行者注册、提交仪式、回执纪律与降级说明。mod 的 `beats-harness.subagent_brief` / `config_opened` / `subagent_fallback_note` 是本文的运行时投影（事实头 + 指针）。

**replaces**：r2 前散置于 `.ygg/mods/agent-seed/data/texts/beats-harness.json` 的派发策略正文。

**上游**：[../../README.md](../../README.md) · **相关**：[acts/](../acts/)（当前幕的入口/必做/出口） · [../copy/harness.md](../copy/harness.md)（submit 命令模板等句式契约） · [../../wiki-governance.md](../../wiki-governance.md)

## 1. brief 结构（事实头 + 策略正文）

mod 运行时发出的 brief = **事实头**（留 mod，带状态变量）+ **策略正文**（知识权威在本文）：

- 事实头：`Requirement {{id}} (rev {{rev}}, stage={{stage}}): {{title}}` / `Description: {{desc}}` / `Worktree: {{worktree}} (branch={{branch}}, base={{base}})` / `Artifact: {{artifact}}` / `Submit: {{submit}}`。
- 策略正文（以下 §3–§7）随 brief 指针给出，各项目可在自己的 context 副本中本地化改写。

## 2. 派发前环境准备（必须 source .env.local，并将结果用于创建）

在派发任何 subagent（plan / development / analysis）**之前**，主会话必须 `source .env.local`（仓库根目录，git-ignored 不入库）读取环境变量，用于辅助创建 subagent。完整闭环是 **读取 → 使用 → 校验**，只有 `source` 而没有传入创建参数不算遵循流程。

```bash
# 派发前必做：在同一 shell 进程加载环境变量（模型/思考级别等 dispatch 参数）
set -a
source .env.local
set +a
# 随后在同一进程创建 subagent：
# Agent(model=${YGG_SUBAGENT_MODEL:+$YGG_SUBAGENT_MODEL}, thinking=${YGG_SUBAGENT_THINKING:+$YGG_SUBAGENT_THINKING}, ...)
# 仅展示调用骨架；工具调用由主会话实际生成，不把变量值写进 brief/文档/提交。
```

**纪律**：
- `YGG_SUBAGENT_MODEL`（子代理模型，如 `sonnet` / `provider/model-id`）与 `YGG_SUBAGENT_THINKING`（`off|minimal|low|medium|high|xhigh|max`）从 `.env.local` 读取；未设置时用缺省（不打印值、不写入文档、不 commit）
- **读取后必须使用**：Agent 工具创建调用须把两个变量实际映射到 `model` / `thinking` 参数；键未设则省略参数并使用 harness 缺省。**不能只 `source`、检查长度，然后仍用工具默认值创建**
- **派发前校验**：在同一 shell 进程验证 `.env.local` 加载成功、变量类型合法、模型引用格式与 thinking 枚举合法；仅输出 `set`/`unset` 状态，绝不输出变量值或含值的完整工具参数。工具返回中可见的参数对象须实际包含目标字段
- **失败可读**：不得用 `2>/dev/null` 掩盖 source 失败；文件缺失/不可读、变量类型不合法时先暂停派发并显式处理。若 `.env.local` 合法但相关键未设，按缺省语义继续，并在主会话内部确认「本次使用缺省」
- 这两个值是 **dispatch 参数**——只在创建 subagent 时使用，**绝不打印、不写入 brief、不进台账**
- 无 egg 环境：同样先 source .env.local 再按本文动作清单人工派发，使用同一套创建参数
- 允许后续新增键时沿用同一闭环：先加载、再按创建工具的实际参数面映射、最后校验生效，不把「读到环境变量」误当「参数已生效」

## 3. 必读材料纪律（Mandatory references）

**跨仓库同步的源基线（r5 实证）**：当需求是「从 A 仓同步到 B 仓」时，源仓可能在实施期间前进（主会话并发提交他需求）。brief 里的 `base=<sha>` 只钉住**目标仓**基线；对源仓，要么写「取实施时最新 HEAD」（可变基线），要么写「pin 在 `<sha>`」并声明源仓冻结。默认取**可变基线**——计划稿里的「源基线变化就停机」条件只适用于显式 pin 的场景，否则会因无关提交误停。

在提议或编辑之前，先读 `docs/README.md`（索引）、[acts/](../acts/)（当前幕的入口/必做/出口）与本文（派发/提交/集成仪式），再读需求点名的 `docs/design/<subsystem>.md`。不得只泛读目录后就开始动手。

- **plan**：对照 playbook `run_machine` 与 `actions`，在 plan artifact 声明触碰的 wiki 主题文件和验收证据。
- **development**：除上述文件外，对照 playbook `gates`、`live_evidence` 与触碰子系统章；在同一特性分支就地更新对应 wiki（Doc sync 见 [../../wiki-governance.md](../../wiki-governance.md)）。

若 brief 未点名具体 subsystem 文件，先沿 `docs/README.md` 的设计导航定位并在计划中写清选择；找不到落点属于 brief 缺口，应返回协调者，不得自行发明新契约文件。

## 4. 工件纪律（Artifact）

交付物通过 harness 存档，**绝不作为散文件落在任何检出里**：子代理在最终汇报中返回全文（或写到 /tmp 下），主会话用 `--artifact-put artifacts/<name>.md` 记入本局存档。一局的所有工件只活在该局存档（`<run-dir>/artifacts/`）；任何检出内的 `artifacts/` 是 git 忽略的草稿区，不是权威副本。

## 5. 提交仪式（Submit）

- 子代理用 `subagent_submit_cmd` 模板提交 `proposal.submit`（模板句式见 [../copy/harness.md](../copy/harness.md)），payload 尾段携带 `exec=<agent_id>`（内联执行者注册）与身份段（宿主在声明路由上盖 `actor=` / `actor_session=` 章）。
- 执行者台账：主会话在拿到子代理 id 后 `config.set executor.<rN>.<stage>=<id>`（stage ∈ analysis / plan / development）。同一 (req, stage) 换人或跨阶段移交不同 id 必须带原因（`=<id>|<reason>`）；amend 不改绑定，split/merge 不继承。`proposal.submit` 不硬查台账，未注册只在 `requirement.list` 显示 unregistered，因此它是自声明审计轨迹而非身份凭证。
- 自报 profile 卡：`config.set profile.<handle>.<key>=…`（每键一次 config.set）；profile 是自报声明不是凭据，问责以执行者台账为准。宿主身份段、三态互证与 opener 契约见 [../../design/identity.md](../../design/identity.md)，本节不复制。

## 6. 配置面板语义（config_opened 正文的知识权威）

`[CONFIG MENU]` 面板承载 play mode / evolution preference / gates 三类设置：点选一项即生效并停留在本页；选「back」返回幕菜单。注意：切换 play mode 会离开本页（驱动者变了，子页上下文失效）。演进预登记不走菜单——用 `config.set review.evolution.artifact|summary|candidates|classes=<value>` 写设置；verify 通过后 `settlement.evolution` 菜单在同一处组装并校验载荷。
