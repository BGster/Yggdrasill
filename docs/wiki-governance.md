# Yggdrasill · 活文档闭环与文档边界治理

> **定位**：docs wiki 的三职责（注入 / 同步 / 核验）、**文件边界判据**（分类轴 = 召回时点）、**design / context 二分判据**、**幕进必读与推进必签**、**ver 与兼容层**、**批结算自动维护**的权威定义。
> **replaces**：DESIGN §4 活文档闭环（原 L733–739；r2 载体指称由两单文件改述为 wiki 树，三职责语义逐条保留）。
> **上游**：[README.md](README.md) · **相关**：[context/README.md](context/README.md)（召回入口） · [design/README.md](design/README.md)（契约树入口）
> **无编号**：本文只写规则与判据。任何"某年某月某裁决"的编年史归 git 与结算产物，不进本文。

## 1. 工作流即 Mod（治理总纲）

**软件工程协调本身被建模为一个可玩的 Mod（`agent-seed`），协调者通过事件信封逐步与状态机交互。**
所有产物都必须先由事件带入状态机（`requirement.dispatch` → proposal → gate）——**判定看事件，不看字节落在哪里**：`~/tmp`、`/tmp`、聊天记录都算"已产出"。运行级是常驻自循环（`boot → charter → coordinate ⇄ settle → release → coordinate`，终局只有显式退役可达 `retired`），需求级走九幕（`draft → planning → plan_gate → ready → developing → dev_review → accept_gate → integrating → done`）。

**依据**：agent 的失败模式不是"不会写代码"，而是"长上下文里规则漂移"——静态提示词随距离衰减，而状态机每一步都要重新读当前状态，因此规则不会因为对话变长而失效。反方案（一次性长提示词 + 人工检查）在早期迭代里反复出现"同一规则三次不同执行"。

**判据**：① 阻断——跳过事件直接产出交付物（写文件、改代码、出工件而事件账里查不到对应 dispatch/proposal/gate）；② 症状——`requirement.list` 里某需求停在 `draft` 却有对应工件、或事件账与磁盘产物数量对不上；③ 确认——`<egg> open <run> --event requirement.list` 的每行状态与 `git log` 的每次提交能一一对上。

## 2. 活文档三职责（闭环契约）

`docs/` 的 wiki（顶层导航 [README.md](README.md)；领域与决策在 `docs/context/*`，系统行为与接口契约在 `docs/design/*`）在每次运行中都作为闭环契约强制执行：

1. **执行前注入**：Mod 的 `subagent_brief` 模板向每个 `plan` 与 `development` 子代理 brief 注入强制指令：*"在提议或编辑之前，先阅读 docs wiki（`docs/README.md` 索引；`docs/context/*` 领域不变量；`docs/design/*` 架构与契约）。"* 以及 Doc sync 职责（见下）。
2. **执行中同步（Doc sync）**：wiki 是**整个仓库始终最新的设计原则与设计方案权威**（内核 / CLI / Mod / serve / 工具链按主题分文件），不是流水账。子代理职责分阶段：`plan` 阶段在其 plan artifact 中声明本次变更触碰哪些 wiki 主题文件；`development` 阶段在同一特性分支上**就地更新**对应文件——**决策写进它所治理的那份文档的「设计依据」段**（规范 + 依据 + 三问判据），词汇写进 `docs/context/cold-start/vocabulary.md` 词条，行为与契约写进 `docs/design/` 对应子系统主题文件。不追加任何变更叙事。
3. **结算前核验**：主会话在 `accept_gate` 核验文档更新已随特性分支集成（契约类需求对应主题文件、规则类需求对应「设计依据」段），并在每批的 `review` 盘点结算（常驻局的批收口或退役）前复查 wiki 仍与当前代码一致——`settlement.evolution` 前是最后期限。机器侧闭环见 §6。

**结构与维护纪律**：每主题一文件（≤300 行）、文件头部带定位 / `replaces` / 上下游索引头、双向链接；顶层导航只有 `README.md` 一处。

## 3. 文件边界 = 召回时点边界

**每个文件或目录必须先回答一个问题才能存在：谁会在运行的哪个时刻打开它？** 若答案是**内容类别**——"决策""叙事""杂项""common""utils"——那它是**残余桶**，不许创建：把内容**溶进治理它的文档**里。`触发` 头字段写成话题（"查台账""看叙事""有需要时"）而不是情境，是同一个错误换了顶帽子。

**两条就地例子**（都发生在本仓，都带合格头部、都过了全部机检、都把规则与需要它的时刻分开了）：
- `docs/context/narrative/`：按**内容类型**切（"行为叙事放一起"）→ r6 推翻，三份文件按读者时点归位到 `context/story/` 与 `context/cold-start/`。
- `docs/context/archive/`：按"这是一个台账"切 → r12 解体，逐条裁决溶进它治理的文档。**它的召回是自指的**（"查裁决时读它"），而真正的时刻是"取决策号前"——那个时刻没有任何机制指向它，主代理连漏五次。

**放不下的时候，新桶永远不是答案**：要么它属于治理它的那份文档（一条决策就写进它约束的契约），要么现有切分本身错了、必须按召回时点重切。**重切允许新建文件（时点变了就是新文件），但一个文件的正当性不能是"它是一类内容"。**

**行数上限是切分判据，不是排版目标**：主流程幕 1:1 一文件（`acts/`，≤40 行），需求级九幕按共享时点聚合三份（`req-acts/intake` = 1-3、`delivery` = 4-6、`integration` = 7-9，≤45 行）。超限说明切分单元选错了（实证：`delivery` 若含第 7 幕为 51 行、超 45，故第 7 幕归 `integration`）。

**AGENTS.md 同样有预算：≤ 500 行**，由 `docs-consistency.sh` 逐次报告（`agents=<当前>/500`，超限则 `ok=0`）。它是每会话自动注入的面，膨胀直接摊薄所有约束的注意力——历史上 r12 前它在 277 行，那个超限正是「操作程序混进硬约束」的可见信号。**分类对错不能机械化，但「何时该重新审视分类」可以。** 超限前先逐行过 `AGENTS.md` §1.8 的 Q1/Q2，或把内容迁到它真正被需要的那一刻。

**判据**：① 阻断——新建目录/文件，或把"放不下的东西"停进一个新桶；② 症状——出现名为 `archive`/`misc`/`common`/`narrative` 的目录，`触发` 头写的是话题，或两个互不相关的读者时点必须打开同一份文件；③ 确认——`bash scripts/docs-ver.sh --check` 返回 `ok=1`，`wc -l docs/context/acts/*.md docs/context/req-acts/*.md` 全部落在上限内，且每个目录能用一句话说清一个情境、该情境出现在 `scenes.json` 或 `AGENTS.md §3` 的路由里。

## 4. design / context 二分判据

`docs/` 的两棵树不是按「主题」分的，而是按**读者的使用方式**分的。一处内容该进哪棵树，看下面这条判据，不看它「像不像设计」。

- **`docs/design/` = 改之前必须遵守的契约。** 判据：**有代码消费者**——playbook JSON 的 `basis`/`source`、`.ygg/mods/agent-seed/src/**` 的注释、`crates/**` 的测试与注释会**按路径引用它**（断言它存在，或让执行者照着它改代码）。断了会有人照着空路径动手。
- **`docs/context/` = 我要理解 / 执行这个工作流。** 判据：**无代码消费者**——只有文档读者（人，以及按 brief 读 wiki 的 agent）会打开它；没有任何 JSON / Lua / Rust 按路径指向它。

**判定方法（三步，全部可机械复核）**

1. `grep -rn 'docs/design/<name>\.md' .ygg/mods crates`——有命中 → 契约面，留 `design/`。
2. 只有 `docs/**` 与 `AGENTS.md` 命中 → 行为面，归 `context/`。
3. 两边都不命中 → 先问「这份东西该不该存在」；**不许**把它塞进任何一边充数。

「按路径引用」要分清：playbook 是**提示词素材**（读它的是人与 agent 的知识面），crates 测试里 `read_to_string("docs/…")` 才是**代码消费者**。前者不构成留在 design 的理由。

但「代码读它」还不足以下结论，要看**代码拿它做什么**：

| 代码对文档的关系 | 例子 | 判据归属 |
| --- | --- | --- |
| **实现它**——照它写逻辑、按它判定 | `design/kernel/state-surface.md`（KV 键就是代码真的会写的） | `design/` |
| **校验它**——测试读它与投影比对，保证不漂移 | 逐字文案比对 | **不构成**「代码消费者」；它是 4.2 描述的症状，不是归档依据 |

（2026-11-15 校准：曾据旧口径把 `docs/context/scenes/` 判为归档错位，属误判——它是**源**、mod 是**投影**，代码只是校验。）

### 4.1 三分：契约 / 知识 / 输出（mod 与 docs 的边界）

上一节只回答「进哪棵树」。**一段内容该不该出现在 mod 里**是另一问，用三分回答：

| 类 | 判据 | 唯一副本位置 |
| --- | --- | --- |
| **契约** | 代码照它**实现**（按它写逻辑、按它判定） | `docs/design/` |
| **知识** | 离开当前时刻也该知道（规则、为什么、怎么用、有几种用法、注意事项） | `docs/context/`（按幕归位；剧本面在 `scenes/`） |
| **输出** | 离开当前状态就无意义（状态计数、回执、门禁/拒绝提示、指针行） | mod `data/texts/`（唯一副本） |

- **mod 不承载知识**：mod 只约束流程、渲染输出、指向文件。知识型散文出现在 beats 或 playbook 正文里即为**漂移**。
- **拒绝/门禁提示属于输出**，必须当场说清原因与下一步——它出现在「操作者正在做事、被打断」的那一刻，塌成指针会多一次往返（用户 2026-11-15 裁定：保留在 mod）。
- **`[READ FIRST]` 块属于输出**：只有路径、版本号与签名命令，无解释；权威在 `docs/context/scenes.json`，mod 内是 `gen-scene-readfirst.py` 的**生成物**，不构成第二份手写副本。

### 4.2 单副本纪律（duplicate 是待消除形态，不是「需要同步的两份」）

**同一段话出现两处即为缺陷。** 同步机制（逐字比对测试、生成脚本）只能缓解症状，不能使其合规：

- 两处**手写** → 必须消除一处：合并到唯一副本后删除另一处；
- 一处手写 + 一处**生成** → 合规（生成物受 `--check` 约束，无从漂移）。

推论：`docs/context/` 里若存在只是 mod 输出镜像的「逐字文案表」，即违反本节——无 Mod 的读者真正需要的是**流程与判据**（`acts/` / `req-acts/` / `rituals/` 已覆盖），不是逐字回执。

### 4.3 树的最高目的：无 Mod 也自洽可读

`docs/context/` 必须能在**没有安装 CLI 与 Mod** 的情况下被读通，并据此执行工作流。任何判据、任何「代码有引用」的技术理由，都不得把内容搬走而使这条承诺失效。

`docs/context/scenes/` 曾按此目的留在 context（用户明确：这是刻意设计，不是归档失误）。2026-11-15 更名为 `scenes/`——`copy` 是「文案」直译、无业务含义，而这里装的是**剧本**：每一刻该发生什么、操作者该做什么、判据在哪。逐字回执仍归 mod 单副本（4.1）。

**机检（不是口号）**：`.ygg/mods/agent-seed/tests/` 的三例把这条判据钉在状态里——`design_pointer_anchors_exist_under_docs_design`（逐来源指针硬账）、`design_tree_index_is_complete_and_ids_match_paths`（索引集合双向相等 + `docs/design/*.md` 只允许 `README.md` + 一级子目录恰好三模块）、`agents_design_routes_resolve`（`AGENTS.md §3` 每条 design 路由可解析）。把行为面搬回 design、或在 design 里留兼容壳、或新增第四目录，都会转红。

## 5. 幕进必读与推进必签（scene gate）

**任何推进动作执行前，必须先读该幕的权威正文，并以 `config.set read.<doc-id>=<ver>` 就位签名。** 签名四维缺一即拒：**覆盖**（该幕必读文件未签）、**新鲜**（签的 ver ≠ 磁盘 ver）、**遗忘**（跨幕未重签，或距上次正文通读超过 `readGate.window`）、**按人**（签名者 ≠ 本动作执行者；人工菜单点击结构上无宿主章，降级为 any-signer 并在回执告明）。

**依据**：摘要替代正文是首要失效模式——回执 `[READ FIRST]`、beats 文本、playbook 摘要、上一局习惯都只是**路由与摘要，永不是规则**；没有机器可验的签名，"读过"无法审计，也无法证明读的是当前版本。反方案（只靠自律核对）在多轮迭代里被证伪：漏读时门禁全绿，缺陷只在真实运行里冒出来。

**判据**：① 阻断——`requirement.dispatch` / `proposal.submit` / `gate.approve` / `coordinate.submit` 在缺签或签名过期时执行；② 症状——回执 `[READ FIRST]` 的路径不存在或 ver 与磁盘不符，`--attest` 报 `forged` / `stale` 大于 0；③ 确认——`bash .pi/tools/docs-consistency.sh --attest --run <run-dir>` 返回 `claims>0` 且 `forged=0 stale=0`。

**读多少由事实决定，不由习惯决定**（判定细则见 [context/rituals/read-gate.md](context/rituals/read-gate.md)）：磁盘 ver 变了 → 通读正文；ver 相同且距上次通读在窗口内 → 版本核对后签名；ver 相同但超窗口或上下文丢失（压缩 / 新会话 / 交接）→ 通读。**版本核对永不延展窗口，只有正文通读会重置它。**

**这个文件不进 `readPlan`，也不产生签名义务**——它是"怎么签名"的规则，若自身成为必读并需要签名，就构成「先签名才能读到怎么签名」的自指门禁。它登记在 `scenes.json` 的 `reference` 段以便机检可达。

## 6. 批结算文档维护义务（机器闭环，主会话自办）

**未武装的 `review.note` 批结算，会在 `[BATCH n SETTLED]` 之后、同一 handler 事务内登记 `docs.maintenance.due=<batch>`**，并在回执里叙述 `[DOCS MAINTENANCE DUE]`（含 wiki 的分文件指引与解除命令）。**它不派生需求行**——CONTEXT / DESIGN 的维护是**主会话的自身工作**。

**为什么不是需求**（用户 2026-11-15 裁定）：subagent 缺跨批次上下文。r11（68,376 token / 129 次调用）与 r12（68,303 token / 76 次调用）两次实测，执行者在 ≈68k token 处被上游打断，且产出的文档仍需主会话逐条复核。派发它，等于用一条「看起来在治理」的需求掩盖「没人有上下文做这件事」。

**闭环靠阻塞而不是靠占位行**：义务未解除时，下一次 `coordinate.submit` 直接拒（`docs-maintenance/due`，附解除命令）；主会话做完复核后 `config.set docs.maintenance.done=<batch>` 解除——**只解除等值批次**：旧批次号清不掉新义务，空值是显式放弃（自担后果）。

**判据**：① 阻断——下一批 `coordinate.submit`（义务未解除即无法跨批边界）；② 症状——批结算后 `docs.maintenance.due` 未置位，或 `req.count` 因维护而 +1（说明又派生了需求），或拿旧批次号就能清掉；③ 确认——语义用例 `batch_settle_marks_docs_maintenance_due_and_blocks_the_next_boundary` 与 `docs_maintenance_clear_requires_the_outstanding_batch`。


## 7. ver 定义与旧路径兼容层两分法

**① `ver` = 把固定头 `> **ver** <hex6>` 的 token 换成定宽占位 `000000`（字节数不变）、其余字节原样后 `git hash-object` 取前 6 位。② 旧路径兼容层两分法：拆分型 → 薄壳（≤10 行指针壳，不自带规则）、1:1 迁移 → 软链（同字节入口）。③ 每条兼容层必须写明退场条件。**

**依据**：ver 若直接哈希文件本体，则"文件含 ver、ver 由哈希决定"构成自引用循环，永不收敛；定宽占位同时保证字节长度不变（不引起行位移）。兼容层不是风格选择：当冻结消费者（mod 字面指针、crates 按路径读表）还在时直接删旧路径会让锚点校验与逐域键集断言同时转红；但薄壳与软链都不携带第二份权威，因此不违反"一条规则一个权威载体"。

**判据**：① 阻断——"改了正文不改 ver"与"删旧路径但没补兼容层"两类提交；② 症状——`docs-ver --check` 报 `changed>0` / `missing_header>0`；`selftest` 的指针锚点用例或 `copy_contract_domain_fragments_cover_every_text_key` 转红；③ 确认——`bash scripts/docs-ver.sh --check` 连跑两次 `changed=0`、`cargo test -p ygg-sandbox --test selftest` 全绿、`ls -l docs/context/*.md | grep -c -- '->'` 为 0。

## 8. 文案单向同步纪律

mod 文案（叙事台词 / 仪式正文 / 句式模板）的**知识权威源**在 `docs/context/` 运行流程层（`context/acts/` + `context/req-acts/` / `context/rituals/` / `context/scenes/README.md` 及其七个消费域文件），mod `data/texts/*.json` 是其运行时投影。任何文案改动**先改 context 对应文件，再同步 mod 键值**；对应域契约表的句式模板列与 mod 键值必须逐字一致（允许 `{{变量}}` 原样保留），验收与每批的维护需求复核该一致性（双向核对无多无缺）。context 流程层（台词/仪式/句式）与 design 契约层（状态机/事件/线协议）语义不重复讲述，以双向链接互指。

## 9. 方案呈报纪律（top-down layering）

凡向人类呈报方案（计划审批卡、架构提案、需求澄清），一律自上而下四层展开，读者可在一层停住而不失完整图景——① **业务背景**（为什么需要：现状的痛与触发场景，一段话）；② **业务逻辑设计**（新世界里操作与规则怎么运转，不含实现词）；③ **架构设计**（职责如何分界、机制如何成立、与既有系统的接缝）；④ **模块设计**（落到文件/接口/测试的改动面，仅在读者继续追问时展开）。与 playbook `proposal.plan-presentation` 规则同源：summary 是裁决面，技术细节归 artifacts；四层顺序不可倒置——先结论后细节是给执行者的 brief，先图景后细节才是给裁决者的呈报。
