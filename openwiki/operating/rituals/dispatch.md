---
type: "Reference"
title: "派发仪式 — brief 怎么写、怎么派、怎么提交"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-ea70eb6c045047448e446296
    resource: repo://.gitignore
  - id: openwiki-source-122f70ffd0740fa8e00a478c
    resource: repo://.pi/tools/git-delivery-check.sh
  - id: openwiki-source-e115ab200a8b2c89a8ff1d41
    resource: repo://.pi/tools/todo-check.sh
  - id: openwiki-source-f6b499648596f0a05b479153
    resource: repo://.ygg/mods/agent-seed/data/texts/beats-harness.json
  - id: openwiki-source-f2294347eab5094b120f7ba9
    resource: repo://.ygg/mods/agent-seed/src/domain/executor.lua
  - id: openwiki-source-d5edd97ca7e6335c0cd02c35
    resource: repo://.ygg/mods/agent-seed/src/handlers/operate/review.lua
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# 派发仪式 — brief 怎么写、怎么派、怎么提交

> **id** rituals/dispatch · **幕** coordinate（派发前） · **触发** 派发任何 subagent（plan / development / analysis）前
> **读不到代价** hard-refusal（`.env.local` 未 source 即派发、工件落检出、submit 缺 `exec=` 均不合规）
> **replaces**：`docs/context/rituals/dispatch.md`（搬运基线 `3a2536f`）
> **上游**：[overview.md](../../overview.md) · **相关**：[read-gate.md](read-gate.md) · [../scenes/README.md](../scenes/README.md)

本页是 `subagent_brief` 模板的**策略正文**权威。mod 的 `beats-harness.subagent_brief` / `config_opened` / `subagent_fallback_note` 只是运行时投影（事实头 + 指针）。

## 1. brief 结构（事实头 + 策略正文）

产物 = **事实头**（留 mod，带状态变量）+ **策略正文**（权威在本页）：

- 事实头：`Requirement {{id}} (rev {{rev}}, stage={{stage}}): {{title}}` / `Description: {{desc}}` / `Worktree: {{worktree}} (branch={{branch}}, base={{base}})` / `Artifact: {{artifact}}` / `Submit: {{submit}}`。
- 策略正文（§3–§9）随 brief 指针给出；各项目可在自己的副本里本地化改写。
- **中断预算（实现类任务必写）**：执行环境在 ≈68k token 处会被上游打断——两次实测同量级（68,376 / 129 次调用；68,303 / 76 次调用），且两次都留下大量已改未提交的改动（12 文件 / 2 文件），全靠 resume 抢救。因此实现类 brief 必须写明：**每完成一块立即提交，任何时刻未提交改动 ≤ ~200 行**；被打断时的损失上限是「一块」，不是「整轮」。
  - 判据：交付回执的 `git log` 是多个提交而非一个巨型提交；若只有一个大 diff 提交，说明中断预算没生效。

## 2. 派发前环境准备（source `.env.local`，并把结果用在创建上）

派发任何 subagent **之前**，主会话必须 `source .env.local`（仓库根，git-ignored 不入库）读取环境变量，用于创建 subagent。闭环是 **读取 → 使用 → 校验**；只有 `source` 而没把参数传进创建调用，不算遵循。

```bash
set -a; source .env.local; set +a
# 同一进程内创建 subagent：
# Agent(model=${YGG_SUBAGENT_MODEL:+$YGG_SUBAGENT_MODEL},
#       thinking=${YGG_SUBAGENT_THINKING:+$YGG_SUBAGENT_THINKING}, ...)
# 仅示意骨架；工具调用由主会话生成，变量值不进 brief / 文档 / 提交。
```

**纪律**：

- `YGG_SUBAGENT_MODEL`（如 `sonnet` / `provider/model-id`）与 `YGG_SUBAGENT_THINKING`（`off|minimal|low|medium|high|xhigh|max`）从 `.env.local` 读；未设则用缺省（**不打印值、不写入文档、不 commit**）。
- **读取后必须使用**：创建调用须实际把两个变量映射到 `model` / `thinking` 参数；键未设则省略参数用 harness 缺省。**不能只 source、查长度，然后仍用工具默认值创建**。
- **派发前校验**：同一 shell 进程内验证加载成功、类型合法、模型引用与 thinking 枚举合法；只输出 `set` / `unset` 状态，绝不输出值或含值的完整参数。工具返回里可见的参数对象须实际包含目标字段。
- **失败可读**：不得用 `2>/dev/null` 掩盖 source 失败；文件缺失 / 不可读 / 类型非法时先暂停派发并显式处理。合法但键未设，按缺省语义继续，并在主会话内部确认「本次使用缺省」。
- 这两个值是 **dispatch 参数**——只用于创建 subagent，**绝不打印、不写 brief、不进台账**。
- 无 egg 环境：同样先 source `.env.local`，再按本页动作清单人工派发，用同一套创建参数。

## 3. 必读材料纪律（Mandatory references）

**跨仓库同步的源基线**：需求是「从 A 仓同步到 B 仓」时，源仓可能在实施期间前进（别人并发提交）。brief 的 `base=<sha>` 只钉**目标仓**；对源仓要么写「取实施时最新 HEAD」（可变基线），要么写「pin 在 `<sha>`」并声明源仓冻结。**默认取可变基线**——「源基线变化就停机」只适用于显式 pin 的场景，否则会因无关提交误停。

提方案或编辑之前，先读 [overview.md](../../overview.md)（索引）、[../acts/](../acts/)（当前幕的入口 / 必做 / 出口）与本页，再读需求点名的 `contracts/<subsystem>/<topic>.md`。不得只泛读目录就动手。

**推进动作的读签判定不在本页**：「签名消费正文还是核对版本、窗口怎么算」见 [read-gate.md](read-gate.md)（支配协调者自己的推进动作，不限于派发）。两页不互抄规则。

- **plan**：在计划稿里声明触碰的知识库页面与验收证据。
- **development**：除上述外，对照触碰子系统的契约页与证据规范；在同一特性分支就地更新对应页面（Doc sync 见 [../../governance.md](../../governance.md)）。

<!-- openwiki: broken internal link [../../contracts/README.md] file "../../contracts/README.md" does not exist. Fix the href or restore the target, then delete this comment. -->
若 brief 未点名具体子系统页，先沿 [../../contracts/README.md](../../architecture/README.md) 的索引定位，并在计划里写清选择；找不到落点属 brief 缺口，应返回协调者，**不得自行发明新契约页**。

## 4. 工件纪律（Artifact）

交付物通过 harness 存档，**绝不作散文件落在任何检出里**：子代理在最终汇报里返回全文（或写 `/tmp`），主会话用 `--artifact-put artifacts/<name>.md` 记入本局存档。一局的工件只活在该局存档（`<run-dir>/artifacts/`）；检出里的 `artifacts/` 是 git 忽略的草稿区，不是权威副本。

### 取证标准（可证伪断言）

工件里的**否定性断言**（「X 不存在 / 本仓从未有 Y」）与**数量断言**（「16 处」）必须先跑一条**本来可能给出反例**的方法并贴原始输出，结论旁写明方法名。方法必须与断言对象的**形态**匹配：

| 断言形态 | 方法（会失败的那种） | 反例（看着像取证、其实不可能给出「存在」） |
| --- | --- | --- |
| PATH 上的命令不存在 | `command -v <tool>; echo "exit=$?"`（exit=1 才算不存在） | `which <tool>`；`ls scripts/*<tool>*`（只扫本仓脚本目录，对 cargo crate / 全局命令恒空） |
| cargo crate 不存在 | `cargo metadata --no-deps --format-version 1`（取 `packages[].name`），或 `ls crates/` | `command -v ygg-lint`（crate 不是 PATH 命令，恒空 → 假否定） |
| 文件 / 目录存在性 | `test -e <path> && echo EXISTS` | `ls <glob>`（通配不匹配时不报错，易读成「没有」） |
| 数量 N（次数 / 文件数 / 行数） | 一条可复算命令 + 贴原始输出；写清分母（口径） | 目测、抽样、只数一屏、换口径不声明（16 vs 23 键 24 处） |

**断言「不存在」时必须写明「我用什么方法找过」**——未写方法的否定结论视同「我没往那儿看」。实录：某需求曾写「`ygg-lint` 本仓从未存在」，取证用的是 `command -v` 与 `ls scripts/*lint*`，两者都不适用于 cargo crate；实测 `crates/ygg-lint` 存在且 `cargo run -q -p ygg-lint -- --pack .ygg/mods/agent-seed` 输出 `0 error(s), 0 warning(s)`。

## 5. 提交仪式（Submit）

- 子代理用 `subagent_submit_cmd` 模板提交 `proposal.submit`（句式见 [../scenes/README.md](../scenes/README.md)），payload 尾段带 `exec=<agent_id>`（内联执行者注册）与身份段（宿主在声明路由上盖 `actor=` / `actor_session=` 章）。
- **执行者台账**：主会话拿到子代理 id 后 `config.set executor.<rN>.<stage>=<id>`，live stage ∈ **plan / development**（`analysis` 是旧版兼容槽位，不作 live 阶段、不为新工作登记）；一个子代理覆盖两个 live 阶段时逐阶段登记同一 id。同一 (req, stage) 换人、或**跨阶段移交**（development 的 id ≠ plan 的 id）**必须带原因**（`=<id>|<reason>`），不得当「首次绑定」静默覆盖；成功移交写 `id|reason` 并向 `req.<i>.hist` 追加一条。台账不带 rev：`amend` 不改绑定，`split` / `merge` 不继承。`proposal.submit` 不硬查台账，未注册只在 `requirement.list` 显示 unregistered——它是自声明审计轨迹，不是身份凭证。
- **自报 profile 卡**：`config.set profile.<handle>.<key>=…`（每键一次）；profile 是自报声明不是凭据，问责以台账为准（宿主身份段与三态互证见 [../../contracts/mod/identity.md](../../contracts/mod/identity.md)）。
- **交付洁净度自检（提交前必跑）**：`sh .pi/tools/git-delivery-check.sh --repo <checkout> --text` 必须 `ok=1`（五项计数全 0），**回执贴该行原文**——「工作树 clean」这句话不再是证据。

## 6. 配置面板语义（`config_opened` 正文权威）

`[CONFIG MENU]` 承载 play mode / evolution preference / gates 三类设置：点选即生效并停留在本页；选「back」返回幕菜单。切换 play mode 会离开本页（驱动者变了，子页上下文失效）。演进预登记不走菜单——用 `config.set review.evolution.artifact|summary|candidates|classes=<value>`；verify 通过后 `settlement.evolution` 菜单在同一处组装并校验载荷。

## 7. 收尾自检（交付洁净度五问）

| # | 问 | 计数键（`ok=1` 当且仅当五项全 0） | 红灯修法 |
| --- | --- | --- | --- |
| 1 | 有未提交改动吗？ | `dirty.count`（拆 `tracked` / `untracked` / `unmerged`） | 提交或丢弃；**不许用 `stash` 了事** |
| 2 | 有提交没进上游吗？ | `unpushed.count`；**无 upstream 记 `-1` = 失败** | `git push -u origin HEAD` |
| 3 | 有悬置 stash 吗？ | `stash.count` | 提交或 `git stash drop`；stash 栈是**仓库级共享**的（泳道池与主检出共用），先看 `stash=` 明细是不是自己的 |
| 4 | 有冲突 / 编辑残留吗？ | `conflict_debris.count`（basename 含 `~`、`*.orig`、`*.rej`） | `rm`；尤其防 `git add -A` 把它扫进提交 |
| 5 | 格式化干净吗？ | `fmt.status=clean`；非 Rust 仓 `n/a`（显式可见） | `cargo fmt` 后提交 |

**为什么是这五问**：曾连续三次交付都留未提交的 rustfmt 差异，而三次回执都写了「工作树 clean」——形容词没有对照物，计数有。修法就是把这一条写进 brief，不依赖协调者每次记得。

**纪律**：回执贴 `ok=` 行原文；`n/a` / `--no-fmt` / `--no-stash` 是豁免，须在同回执写明理由（脚本记进 `waived=`，不静默）。它**不是门禁**：不进 `gates.gate_problem()`、不改 KV，红灯由执行者修，不阻塞其他需求。`unpushed` 记 `-1` 是刻意的：`0` 表示「比过且一致」，`-1` 表示「没法比」——「本地干净、远端没有」的半交付只能靠这两个值可分辨。脚本**只读**：不改 refs / index / worktree，不 push、不 reset、不 stash。

## 8. 决策就地落档（不编号、不建台账）

一条真正新的架构决策**不携带编号**：在实现它的**同一个提交**里，就地写进它所治理页面或文档的「设计依据」段（规则 + 依据 + 三问判据），需求描述与 brief 只点名那个文件与段。两条泳道触及同一决策时就地并入既有段落。不编号、不建台账、没有需要自分配或回交协调者的标识符。

**历史依据**：曾用「预分配编号」协调决策，两泳道从同一基线各取下一号，导致同一编号被双重认领（git `d0b4a7c` / `6848138`）——**标识符不是协调机制，锚点才是**。

## 9. 后台执行与同步等待例外

- 每个 subagent 任务都在**后台**运行；主会话**不在原地等待**——继续推进其他泳道、验证或规划，完成通知到达后再核验与合并。
- **同步等待只在一种情况允许**：下一个动作**严格依赖**该结果，且没有其他事可并行；即便如此也先把等待时间花在其他泳道上。

## 10. 判据

① 阻断——未 source `.env.local` 就派发、`source` 了却不把参数传进创建、工件写成检出里的散文件、回执只写「工作树 clean」而无 `ok=` 行、否定性断言无方法。② 症状——交付回执是单个巨型提交、`unpushed.count=-1`、`conflict_debris.count>0`、需求描述出现「从未存在」而无人复算。③ 确认——创建调用返回里可见 `model` / `thinking` 字段、`git-delivery-check.sh --text` 的 `ok=1` 行原文在回执中、每个否定断言旁有方法名与原始输出。

维护责任 = 改 brief 结构、派发纪律或提交仪式的执行者；触发 = 终止信封、台账语义、脚本参数或证据标准变化；检查 = 本页命令与 `.pi/tools/git-delivery-check.sh`、`data/texts/beats-harness.json` 投影一致。
