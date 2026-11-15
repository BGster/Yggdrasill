# 发布幕 · release
> **id** acts/release · **幕** release · **触发** 盘点四项齐备后（`[SETTLE READY]` 自动进入）
> **读不到代价** hard-refusal（未声明发布则 `review.note` 不收口）
> **ver** b7bced

**场景台词（知识权威）**：第 {{batch}} 批 · 任务「{{title}}」{{declared}}。声明本批 develop→master 合并结果（`release.confirm` / `release.skip`）；已声明即可 `review.note` 批结算（未武装）或退役（`run.retire` 武装）。

**阶段策略（release_common）**：`release.confirm` 载荷声明 `master@<sha>` 与批次 tag（`v年.月.日.序号`），或 `release.skip` 声明跳过理由；任一成立即「发布已声明」，`review.note` 才可批结算/退役。

**入口**：盘点四项齐备自动进入。**必做**：发布声明二选一。**出口**：`review.note`——未武装 = 批结算回协调幕（`[BATCH n SETTLED]`，run.batch+1）；武装 = 写 ending 退役（`[ENDING · run retired]`）。

**集成与发布 cadence（集成分工 操作面）**：development 执行者在需求收口自行完成 feature→develop（对齐 `origin/develop` → `git merge --no-ff` → 集成态门禁 → push develop；artifacts 写 `develop@<sha>`），主流程只做轮次 develop→master、reset-develop 与批次 tag，迭代集成不得积压到批末。需求合并入集成分支后 push develop/master 并给集成 merge 打 `vYYYY.MM.DD.N`（N=当日序号）；push/tag 的触发方随 play.mode——guide 由人通过门禁/菜单票确认，goal 由主代理在授权范围内执行——但合并在前置门禁绿、证据齐备时是正常步骤，不是可任意省略的例外。

`develop` 与 `master` 同步只用 reset，禁止反向 merge：轮次合并或任何 master 直接提交后，执行 `git reset --hard master` + `git push --force-with-lease origin develop`。升级（Mod 或 host binary）在批结算后落，再开**新 run-id**；旧局目录不迁移、不 adopt，装载失败即开新局。

**轮次收口与部署（round-level 操作面）**：

- **前置：端到端验收**。本轮每个 feature 都已落 `develop` 之后，先在 `develop` 集成态过端到端验收（示例游戏完整跑通 + save/load 校验）——这是合入 `master` 的前置条件，不是可选项。
- **阶段分离**：迭代集成与 `develop → master` 合并是两个阶段，**永不同步执行**；本轮还有任何 feature 未落 `develop` 时，禁止合并 `master`。
- **合并仪式的硬句**：`git fetch origin` → `master` 与 `develop` **各自**做 divergence-check 并 `--ff-only` 对齐 → 主检出 `git merge --no-ff develop` → `git push origin master`。**trunk 未 push 等于合并未完成**。**禁止 rebase `develop`**——合并拓扑就是回滚历史。
- **tag 的含义**：验收通过后打的 `vYYYY.MM.DD.<n>` 指向那个 no-FF 合并节点，它就是回滚锚点。
- **CHANGELOG**：仓库存在 CHANGELOG 或批次报告时，在批收口一并更新。
- **部署**：任何渠道部署的提交必须**已合入 `master`**——非 master ref 一律拒绝，没有例外开关。部署是写操作：**每次**部署前报告用户并取得批准。

**菜单语义对照**：`release.confirm` / `release.skip`（声明/跳过）、`review_note`（收口）。

**prompts.release 语义**：「声明本批结果（release.confirm / release.skip）后 review.note 批结算；run.retire 武装时它是终局。」

**本幕关联阅读**：[scenes/README.md](../scenes/README.md)（`release_*` / `review_batch_ok` / `endings.done` 句式）；[../story/run-story.md](../story/run-story.md)（发布幕与常驻批周期）；[../req-acts/integration.md](../req-acts/integration.md)（执行者 feature→develop 集成纪律）；集成分工只记录裁决，不复制本操作面。
