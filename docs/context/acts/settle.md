# 盘点幕 · settle
> **id** acts/settle · **幕** settle · **触发** `coordinate.submit` 成功（进入 settle 相位）
> **读不到代价** hard-refusal（载荷形状错、`c=`/`cls=` 计数不自洽被拒）
> **ver** 124a0d

**场景台词（知识权威）**：任务「{{title}}」：{{count}} 条需求（{{done}} 条已终态）｜require_all_green {{gates}}。四项齐备（验证、演进登记、会话清理、待办清理）自动进发布幕。

**阶段策略（settle_common）**：盘点四项——`verify.submit` 先记全绿 → `settlement.evolution` 登记演进（可并入 `sessions=` / `todo=` 可选段一次事务）→ `settlement.sessions` → `settlement.todo`；四项齐备自动转发布幕（不占事件）。会话清理与待办清理的政策正文见 [rituals/cleanup.md](../rituals/cleanup.md)。

**入口**：`coordinate.submit` 成功（verify.passed 复位为 false，本批 settlement 键清空）。**必做**：① `verify.submit` 记全绿（`all_green=false` 且 `require_all_green=true` 时打回协调幕返工）；② `settlement.evolution` 登记（summary 必填）；③④ 会话与待办清理声明。**出口**：四项齐备 → 发布幕（`[SETTLE READY]`，R9 自动转场）。

**登记载荷与拒绝面**：`settlement.evolution` 仅在 verify 绿后登记，载荷为 `artifact|summary[|c=<kept>/<screened>[|cls=<kind>:<n>,...]][|sessions=<moved>/<remaining>/<destination>][|todo=<remaining>/<verified_by>]`。summary trim 后必填；`c=0/0` 可空 artifact；screened>0 时 artifact/cls 必填且六类计数与 kept/screened 自洽。sessions/todo 可同事务完成，独立 `settlement.sessions` / `settlement.todo` 仍保留。guide 可预登记 `config.set review.evolution.artifact|summary|candidates|classes=<value>`（逐键查形状，登记时查跨字段完整性）。`review.note` 只收 `lesson|score`，旧 ev/c/cls 段拒绝；它先跑 `gates.settlement_problem()`，把本批键单源归档到 `review.evolution.*` 后再批结算或退役。

**演进筛选（登记前逐项完成）**：逐项问长期优先级（换任务/仓库是否仍出现）；既有未用规则必须从 call chain / discoverability / contract mismatch / deliberately unused / genuinely no scenario / redundant 六类中选唯一根因；每项独立评审，工件含 `file:line`、可运行命令及真实输出、语义依据、根因类别与结论；按实际场景→未满足的流程步骤→代码与复现→根因→长期收益→改动面与验证排序。单任务约束、临时工具/探针约定、同任务已解决项不得入册。`settlement.evolution` 只证明 summary 非空、路径形状、计数/类别自洽；沙箱不能读工件或子会话，声明绝不证明建议质量或评审独立性，报告不得越界声称。

**不得新增第二载体**：演进流程**永不**新增第二个决策/执行入口、外部检查脚本或宿主改动——唯一登记动作是 `settlement.evolution`，唯一裁决与执行归属仍是 `AGENTS.md`（判据面）与 Mod 资源（`src/**` 门禁逻辑、`data/*.json` 提示面）两处。想「写个脚本帮你守纪律」只会在同一个地方被绕过；想加第三个入口就等于建第二套规范面。

**宿主收口提示**：`agent.summary.nextEndsRun` 仅在 settle 首项为 `review.note` 且 `run.retire` 已武装时为 true；无预登记时默认空 artifact + 中性 summary + `c=0/0`。宿主随后进入 closing（禁新写、最后 drain）；未武装批结算不得预关闭。常驻局在 `batchDrained` 且 inbox 无未消费行时泵安全暂停 exit 0，不在空批上连发 `coordinate.submit`。

**批边界即重启点（升级纪律）**：升级（Mod 演进或宿主二进制）**只在批结算之后**落地，随后开**新的 run-id**；旧 run 目录**永不迁移、永不 adopt**——装载失败即开新局。批边界是天然重启点，因为它已是一个自洽的收口点（本批需求全部终态、四项齐备、已声明发布）。

**判据**：① 阻断——在批中间替换 Mod/宿主后继续推进同一个局，或写迁移/兼容代码去读旧 run 目录；② 症状——同一 `run-id` 下出现两个 Mod 版本的产物，或为“让旧局能开”而降低装载校验；③ 确认——新局是新的 `.ygg/runs/<id>`，且 `run.json` 的 `store` / `engineVersion` 与当前二进制一致。

**菜单语义对照**：`verify_green` / `verify_fail`（记绿/记不绿）、`settlement_evolution`（登记演进）、`review_note`（批结算/退役）、`map.show`。

**prompts.settle 语义**：「门禁全绿了吗？依次验证 → 演进登记 → 会话 → 待办；四项齐备自动进发布幕。」

**本幕关联阅读**：[rituals/cleanup.md](../rituals/cleanup.md)（会话/待办清理政策）；[scenes/README.md](../scenes/README.md)（`verify_*` / `settlement_*` / `review_*` 句式）；[../../design/kernel/state-surface.md](../../design/kernel/state-surface.md)（`verify.*` / `settlement.*` / `review.evolution.*` 存储键契约）。
