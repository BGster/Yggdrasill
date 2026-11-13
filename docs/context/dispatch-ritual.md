# Yggdrasill · 派发仪式（dispatch-ritual）

> **定位**：`subagent_brief` 模板的策略正文知识权威——子代理必读材料纪律、执行者注册、提交仪式、回执纪律与降级说明。mod 的 `beats-harness.subagent_brief` / `config_opened` / `subagent_fallback_note` 是本文的运行时投影（事实头 + 指针）。
> **replaces**：r2 前散置于 `.ygg/mods/agent-seed/data/texts/beats-harness.json` 的派发策略正文。
> **上游**：[../README.md](../README.md) · **相关**：[workflow-acts.md](workflow-acts.md) · [copy-contract.md](copy-contract.md)（submit 命令模板等句式契约） · [../wiki-governance.md](../wiki-governance.md)

## 1. brief 结构（事实头 + 策略正文）

mod 运行时发出的 brief = **事实头**（留 mod，带状态变量）+ **策略正文**（知识权威在本文）：

- 事实头：`Requirement {{id}} (rev {{rev}}, stage={{stage}}): {{title}}` / `Description: {{desc}}` / `Worktree: {{worktree}} (branch={{branch}}, base={{base}})` / `Artifact: {{artifact}}` / `Submit: {{submit}}`。
- 策略正文（以下 §3–§7）随 brief 指针给出，各项目可在自己的 context 副本中本地化改写。

## 2. 派发前环境准备（必须 source .env.local）

在派发任何 subagent（plan / development / analysis）**之前**，主会话必须 `source .env.local`（仓库根目录，git-ignored 不入库）读取环境变量，用于辅助创建 subagent：

```bash
# 派发前必做：加载环境变量（模型/思考级别等 dispatch 参数）
source .env.local  # 若文件不存在，先确认是否需要创建（含 YGG_SUBAGENT_MODEL / YGG_SUBAGENT_THINKING 等键）
```

**纪律**：
- `YGG_SUBAGENT_MODEL`（子代理模型，如 `sonnet` / `provider/model-id`）与 `YGG_SUBAGENT_THINKING`（`off|minimal|low|medium|high|xhigh|max`）从 `.env.local` 读取；未设置时用缺省（不打印值、不写入文档、不 commit）
- 这两个值是 **dispatch 参数**——只在创建 subagent 时使用，**绝不打印、不写入 brief、不进台账**
- 若 `.env.local` 缺失或键未设：按 playbook `dispatch.params-from-env-local` 的缺省语义处理，但**source 动作本身不可省略**（确保后续新增键时被正确加载）
- 无 egg 环境：同样先 source .env.local 再按本文动作清单人工派发

## 3. 必读材料纪律（Mandatory references）

**跨仓库同步的源基线（r5 实证）**：当需求是「从 A 仓同步到 B 仓」时，源仓可能在实施期间前进（主会话并发提交他需求）。brief 里的 `base=<sha>` 只钉住**目标仓**基线；对源仓，要么写「取实施时最新 HEAD」（可变基线），要么写「pin 在 `<sha>`」并声明源仓冻结。默认取**可变基线**——计划稿里的「源基线变化就停机」条件只适用于显式 pin 的场景，否则会因无关提交误停。

在提议或编辑之前，先读 docs wiki：`docs/README.md`（索引）→ `docs/context/*`（领域不变量、原则、决策台账）→ `docs/design/*`（架构与契约）。读完再动手；plan 阶段在 plan artifact 中声明本次变更触碰哪些 wiki 主题文件（Doc sync 职责见 [../wiki-governance.md](../wiki-governance.md)）。

## 4. 工件纪律（Artifact）

交付物通过 harness 存档，**绝不作为散文件落在任何检出里**：子代理在最终汇报中返回全文（或写到 /tmp 下），主会话用 `--artifact-put artifacts/<name>.md` 记入本局存档。一局的所有工件只活在该局存档（`<run-dir>/artifacts/`）；任何检出内的 `artifacts/` 是 git 忽略的草稿区，不是权威副本。

## 5. 提交仪式（Submit）

- 子代理用 `subagent_submit_cmd` 模板提交 `proposal.submit`（模板句式见 [copy-contract.md](copy-contract.md) harness 节），payload 尾段携带 `exec=<agent_id>`（内联执行者注册）与身份段（宿主在声明路由上盖 `actor=` / `actor_session=` 章）。
- 执行者台账：主会话在拿到子代理 id 后 `config.set executor.<rN>.<stage>=<id>`；同一 (req, stage) 换人或跨阶段移交须带原因（`=<id>|<reason>`）。
- 自报 profile 卡：`config.set profile.<handle>.<key>=…`（每键一次 config.set）；profile 是自报声明不是凭据，问责以执行者台账为准。

## 6. 集成纪律（development 阶段收口时）

子代理自己把功能分支集成进 `develop`：对齐 `origin/develop` → `git merge --no-ff` → 集成态门禁全绿 → push `develop` → 在工件中记录 `develop@<sha>`。**永不触碰 `master` 或他人泳道分支**。轮次级 develop→master、reset-develop 与批次 tag 由主流程负责（D-MA22）。

## 7. 文档同步（Doc sync，适用时）

docs wiki 是全仓始终最新的权威——不是变更日志，历史归 git 与结算产物。若变更改变任何子系统的工作方式（内核 crates / egg CLI / Mod / serve / 工具链）：plan 阶段声明触碰面；development 阶段在同一特性分支就地更新对应文件（决策/词汇 → `docs/context/` 对应台账行或词条；行为/契约 → `docs/design/` 对应主题文件；幕台词/仪式/文案句式 → `docs/context/` 的 [workflow-acts.md](workflow-acts.md) / 本文 / [copy-contract.md](copy-contract.md)）。纯缺陷修复与措辞调整不触碰文档。

## 8. 配置面板语义（config_opened 正文的知识权威）

`[CONFIG MENU]` 面板承载 play mode / evolution preference / gates 三类设置：点选一项即生效并停留在本页；选「back」返回幕菜单。注意：切换 play mode 会离开本页（驱动者变了，子页上下文失效）。演进预登记不走菜单——用 `config.set review.evolution.artifact|summary|candidates|classes=<value>` 写设置；verify 通过后 `settlement.evolution` 菜单在同一处组装并校验载荷。

## 9. 降级说明（subagent_fallback_note）

续作（resume）场景：当某阶段更换执行者或跨阶段移交时，brief 尾部附一行「Resume context: prior executor `{{resume_id}}`」，让新执行者知道前任身份；该行只是上下文提示，不改变问责链（仍以执行者台账为准）。
