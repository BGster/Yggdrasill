---
type: "Reference"
title: "知识库治理 — 权威、页面生命周期、ver 账本与维护义务"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-613d195031e01848a443efc6
    resource: repo://.pi/tools/docs-consistency.sh
  - id: openwiki-source-c1b4b807feda7647ba638e19
    resource: repo://.ygg/mods/agent-seed/data/config.json
  - id: openwiki-source-9aedb7453b0c6f64c43d9da5
    resource: repo://.ygg/mods/agent-seed/data/texts/core.json
  - id: openwiki-source-1c80d889f00dcfd1c85bbf4a
    resource: repo://.ygg/mods/agent-seed/src/domain/readsig.lua
  - id: openwiki-source-d5edd97ca7e6335c0cd02c35
    resource: repo://.ygg/mods/agent-seed/src/handlers/operate/review.lua
  - id: openwiki-source-b640c22b435c5b19de7f36dd
    resource: repo://.ygg/mods/agent-seed/src/handlers/system/config.lua
  - id: openwiki-source-7cfc9f62361f222eb87eaef8
    resource: repo://scripts/docs-ver.sh
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# 知识库治理 — 权威、页面生命周期、ver 账本与维护义务

> **id** governance · **模块** 直达页 · **触发** 新增/搬动/删除一页、裁决「这条规则该写在哪」、或改 ver 与签名机制之前
> **读不到代价** irreversible（同一条规则出现两个权威副本，或页面被手改后生成链断裂，后续每一批都按错的规则执行）
> **replaces**：`docs/wiki-governance.md`（搬运基线 `3a2536f`）
> **上游**：[overview.md](overview.md) · **相关**：[quickstart.md](quickstart.md)（召回入口）· [architecture/README.md](architecture/README.md)（契约树入口）· [operating/rituals/cleanup.md](operating/rituals/cleanup.md)（批结算维护）

本仓只有一个知识库：`openwiki/`。它不是随笔集，而是**被机器校验的契约面**——页面的存在、路径、路由与版本都在测试与脚本的账上。

## 1. 权威顺序（读到冲突时按此裁决）

1. **源码与测试**：能跑的行为以 `crates/**`、`packages/**`、`.ygg/mods/**` 与它们的测试为准。
2. **本知识库的契约页**（`contracts/**`）：代码照它**实现**的规则，改动前必读、改完必改。
3. **本知识库的操作页**（`operating/**`）：离开「某一刻」也该知道的判据与流程。
4. **`AGENTS.md`**：只留冷启动级的不可逆 / 硬拒绝约束与逐字命令，其余一律路由到本库。
5. **其余一切**（回执、beats、摘要、上一局习惯）：**路由与摘要，永不是规则**。

**一条规则只有一个权威载体**。别处出现同一句话就是缺陷，不是「需要同步的第二份」：两份手写 → 删一份；一份手写 + 一份生成 → 合规（生成物受 `--check` 约束）。

## 2. 页面生命周期（禁止手改）

页面是 OpenWiki 的**生成产物**，只能走生命周期：`openwiki_begin(init|update)` → `openwiki_submit_plan` → 逐页 `openwiki_next_page` → 写页 + `openwiki_submit_page`（Claims 带 `repo://` 证据）→ `openwiki_finish`。

- **不手改**已生成的页面，不在页面里写「本次改了什么」的叙事；刷新走 `update` 生命周期。
- **证据只指仓内文件**：`repo://<repo-relative-path>`；目录、`openwiki/**` 自身、`.claude` / `artifacts` / `.rpiv` 等非跟踪目录都不可指。
- **一页一个人写**：不把页面生成委派给 subagent（页面须在同一上下文里连贯成形）。
- **Claims 是页面的锚**：每条 claim 至少一条证据；页面刷新时 claim 走同样的确认 / 撤回流程。

## 3. 文件边界 = 召回时点边界

**每一页必须先回答：谁会在运行的哪个时刻打开它？** 若答案是**内容类别**——「决策」「叙事」「杂项」「common」——那它是**残余桶**，不许建：把内容溶进治理它的那一页。

- **`触发` 头写情境，不写话题**：「改实体索引之前」合格，「查台账」不合格。
- **行数上限是切分判据，不是排版目标**：契约页 ≤ 300 行；同幕操作页按共享时点聚合（如需求九幕三页：受理 1–3 / 交付 4–6 / 集成 7–9）。超限说明切分单元选错。
- **放不下时新桶永远不是答案**：要么它属于治理它的那一页，要么现有切分本身错了、按召回时点重切。重切允许新建页（时点变了就是新页），但**一页的正当性不能是「它是一类内容」**。
- **`AGENTS.md` 有硬预算 ≤ 160 行**，由 `scripts/wiki-consistency.sh` 逐次报告（`agents=<当前>/160`，超限 `ok=0`）。它是每会话自动注入的面，膨胀直接摊薄所有约束的注意力——超限前先逐行过 §1.8 的 Q1/Q2。

## 4. 契约页 / 操作页二分

两棵树不是按「主题」分的，而是按**读者拿它做什么**分的：

- **`contracts/` = 改之前必须遵守的契约。** 判据：**有代码消费者**——`crates/**`、`.ygg/mods/agent-seed/src/**`、playbook `basis`/`source` 会按路径引用它，或照它实现逻辑。
- **`operating/` = 我要理解 / 执行这个工作流。** 判据：**无代码消费者**——只有人与按 brief 读库的 agent 会打开它。

**三步判定（可机械复核）**：① `grep -rn 'openwiki/contracts/<name>\.md' .ygg/mods crates` 有命中 → 契约面；② 只有 `AGENTS.md` 与 `openwiki/**` 命中 → 操作面；③ 两边都不命中 → 先问「这一页该不该存在」，**不许**塞进任何一边充数。

**「代码读它」还不足以下结论，要看代码拿它做什么**：

| 代码对页面的关系 | 例子 | 归属 |
| --- | --- | --- |
| **实现它**——照它写逻辑、按它判定 | `contracts/kernel/state-surface.md`（KV 键就是代码真的会写的） | `contracts/` |
| **校验它**——测试读它与投影比对，保证不漂移 | 逐字文案比对 | **不构成**「代码消费者」；它描述的是「实现它」那一侧的投影关系 |

### 4.1 三分：契约 / 知识 / 输出

上一节只回答「进哪棵树」。**一段内容该不该出现在 mod 里**是另一问：

| 类 | 判据 | 唯一副本位置 |
| --- | --- | --- |
| **契约** | 代码照它**实现** | `openwiki/contracts/` |
| **知识** | 离开当前时刻也该知道（规则、为什么、怎么用、注意事项） | `openwiki/operating/`（按幕归位；剧本面在 `scenes/`） |
| **输出** | 离开当前状态就无意义（状态计数、回执、门禁/拒绝提示、指针行） | mod `data/texts/`（唯一副本） |

- **mod 不承载知识**：mod 只约束流程、渲染输出、指向文件。知识型散文出现在 beats 或 playbook 正文里即为漂移。
- **拒绝 / 门禁提示属于输出**，必须当场说清原因与下一步——操作者正被打断的那一刻，塌成指针会多一次往返。
- **`[READ FIRST]` 块属于输出**：只有路径、版本号与签名命令，无解释；权威是 `scripts/gen-scene-readfirst.py` 的输入（`openwiki/scenes.json` + 页头），mod 内是生成物。

### 4.2 树的最高目的：无 Mod 也自洽可读

`operating/` 必须能在**没有安装 CLI 与 Mod** 的情况下被读通，并据此执行工作流。任何判据、任何「代码有引用」的技术理由，都不得把内容搬走而使这条承诺失效。逐字回执仍归 mod 单副本。

## 5. ver 账本与签名（scene gate）

页面**不再自带 ver 行**：ver 是「该页正文的版本指纹」，集中在 `openwiki/ver.json`（`scripts/wiki-ver.sh` 维护），值 = 正文把 `ver` 字段留空后 `git hash-object` 取前 6 位。

**任何推进动作执行前，必须先读该幕的权威正文，并 `config.set read.<doc-id>=<ver>` 就位签名。** 签名四维缺一即拒：**覆盖**（该幕必读页未签）、**新鲜**（签的 ver ≠ 账本 ver）、**遗忘**（跨幕未重签，或距上次正文通读超过 `readGate.window`，缺省 80 次派发）、**按人**（签名者 ≠ 本动作执行者；人工菜单点击结构上无宿主章，降级为 any-signer 并在回执告明）。

**读多少由事实决定，不由习惯决定**：磁盘 ver 变了 → 通读正文；ver 相同且距上次通读在窗口内、无上下文丢失 → 版本核对后签名；ver 相同但超窗口或上下文丢失（压缩 / 新会话 / 交接）→ 通读。**版本核对永不延展窗口，只有正文通读会重置它。**

**本页不进 `readPlan`，也不产生签名义务**——它是「怎么签名」的规则，若自身成为必读并需要签名，就构成「先签名才能读到怎么签名」的自指门禁。它登记在 `scenes.json` 的 `reference` 段以便机检可达。

## 6. 批结算维护义务（机器闭环，主会话自办）

**未武装的 `review.note` 批结算，会在 `[BATCH n SETTLED]` 之后、同一 handler 事务内登记 `docs.maintenance.due=<batch>`**，并在回执里叙述 `[DOCS MAINTENANCE DUE]`（含分页指引与解除命令）。**它不派生需求行**——知识库维护是**主会话的自身工作**。

**为什么不是需求**：subagent 缺跨批次上下文。r11（68,376 token / 129 次调用）与 r12（68,303 token / 76 次调用）两次实测，执行者在 ≈68k token 处被上游打断，且产出的文档仍需主会话逐条复核。派发它，等于用一条「看起来在治理」的需求掩盖「没人有上下文做这件事」。

**闭环靠阻塞而不是靠占位行**：义务未解除时，下一次 `coordinate.submit` 直接拒（`docs-maintenance/due`，附解除命令）；主会话做完复核后 `config.set docs.maintenance.done=<batch>` 解除——**只解除等值批次**：旧批次号清不掉新义务，空值是显式放弃（自担后果）。

## 7. 兼容层两分法与退场条件

**旧路径兼容层只有两形态：拆分型 → 薄壳（≤ 10 行指针壳，不自带规则）、1:1 迁移 → 软链（同字节入口）。每条兼容层必须写明退场条件。**

从 `docs/` 到 `openwiki/` 的迁移**不留兼容层**：`docs/` 整树删除、引用一次性改写（`git log` 是历史，不是第二入口）。理由是旧路径已无冻结消费者——mod 指针、工具脚本与测试在本批同一提交内改完；任何「先留薄壳」都会把一个已死的引用面变成永久债。

## 8. 文案单向同步

mod 文案（叙事台词 / 仪式正文 / 句式模板）的**知识权威源**在 `operating/` 运行流程层（`operating/acts/` + `operating/req-acts/` / `operating/rituals/` / `operating/scenes/README.md` 及其七个消费域文件），mod `data/texts/*.json` 是其运行时投影。任何文案改动**先改 `operating/` 对应页，再同步 mod 键值**；对应域契约表的句式模板列与 mod 键值必须逐字一致（`{{变量}}` 原样保留），每批维护复核该一致性（双向核对无多无缺）。

## 9. 方案呈报纪律（top-down layering）

凡向人类呈报方案（计划审批卡、架构提案、需求澄清），一律自上而下四层，读者可在一层停住而不失完整图景——① **业务背景**（现状的痛与触发场景，一段话）；② **业务逻辑设计**（新世界里操作与规则怎么运转，不含实现词）；③ **架构设计**（职责如何分界、机制如何成立、与既有系统的接缝）；④ **模块设计**（落到文件 / 接口 / 测试的改动面，仅在读者继续追问时展开）。四层顺序不可倒置——先结论后细节是给执行者的 brief，先图景后细节才是给裁决者的呈报。

## 10. 机器闭环

| 检查 | 断言什么 |
| --- | --- |
| `bash scripts/wiki-ver.sh --check` | 每页账本 ver 与正文一致（`ok=1`）、无缺账、无孤儿账 |
| `bash scripts/wiki-consistency.sh` | 页面是否存在、`AGENTS.md` 预算 ≤ 160、操作页 ≤ 45 / 契约页 ≤ 300 行、`[READ FIRST]` 指针可解析 |
| `cargo test -p ygg-sandbox --test selftest`（或 `-p ygg-modules`） | 契约树索引完备、`AGENTS.md` 每条路由可解析、场景 id 全可达 |
| `scripts/gen-scene-readfirst.py --check` | `data/config.json` 的 readPlan / readGate 与 `scenes.json` 无漂移 |
| `--attest --run <run-dir>` | 签名未被伪造（`forged=0 stale=0`） |

**判据（①②③，本页自身）**。① 阻断——手改生成页、把规则写进第二个载体、新增按内容类型命名的目录、页面无 `id` / `触发` 头。② 症状——同一句话在两页都能搜到、`wiki-ver --check` 报 `changed>0`、`[READ FIRST]` 指向不存在的页、`AGENTS.md` 超过 160 行。③ 确认——上表五行逐条有输出，且 `openwiki_finish` 后页面与 Claims 在 git 里成对出现。

维护责任 = 改 `openwiki/**`、`scripts/wiki-*.sh`、`scripts/gen-scene-readfirst.py` 或 `AGENTS.md` 的执行者；触发 = 任一相关 commit；检查 = 上表五行。
