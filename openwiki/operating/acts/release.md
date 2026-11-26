---
type: "Reference"
title: "发布幕 · release — 声明本批结果，然后才是收口"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-1bea6517c507ab7384eef79c
    resource: repo://.ygg/mods/agent-seed/src/handlers/operate/release.lua
  - id: openwiki-source-d5edd97ca7e6335c0cd02c35
    resource: repo://.ygg/mods/agent-seed/src/handlers/operate/review.lua
  - id: openwiki-source-fa9d37e9da91d2dbb8c7f8a8
    resource: repo://docs/context/acts/release.md
  - id: openwiki-source-12391f341f5cb8a477d9598a
    resource: repo://docs/context/acts/settle.md
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# 发布幕 · release — 声明本批结果，然后才是收口

> **id** acts/release · **幕** release · **触发** 盘点四项齐备后（`[SETTLE READY]` 自动进入）
> **读不到代价** hard-refusal（未声明发布则 `review.note` 不收口）
> **replaces**：`docs/context/acts/release.md`（搬运基线 `3a2536f`）
> **上游**：[settle.md](settle.md) · **相关**：[retired.md](retired.md) · [../req-acts/integration.md](../req-acts/integration.md) · [../story/run-story.md](../story/run-story.md)

**场景台词（知识权威）**：第 {{batch}} 批 · 任务「{{title}}」{{declared}}。声明本批 develop→master 合并结果（`release.confirm` / `release.skip`）；已声明即可 `review.note` 批结算（未武装）或退役（`run.retire` 武装）。

## 1. 必做与出口

**入口**：盘点四项（`verify.submit` → `settlement.evolution` → `settlement.sessions` → `settlement.todo`）齐备后自动进入，不花一个事件。

**必做——发布声明二选一**：

- `release.confirm`：载荷声明 `master@<sha>` 与批次 tag（`v年.月.日.序号`）。
- `release.skip`：声明跳过理由。

**任一成立即「发布已声明」**，此时 `review.note` 才可收口。

**出口**：`review.note`——未武装 = 批结算回协调幕（`[BATCH n SETTLED]`，`run.batch+1`）；`run.retire` 武装 = 写 ending 退役（`[ENDING · run retired]`）。

## 2. 集成分工与两段合并

**development 执行者**在需求收口时自行完成 feature→develop：对齐 `origin/develop` → `git merge --no-ff` → 集成态门禁 → push develop；artifacts 写 `develop@<sha>`。**迭代集成不得积压到批末**。

**主流程只做轮次 develop→master、reset-develop 与批次 tag**。合并在前置门禁绿、证据齐备时是正常步骤，不是可任意省略的例外；push / tag 的触发方随 `play.mode`——guide 由人通过门禁 / 菜单票确认，goal 由主代理在授权范围内执行。

## 3. reset 单向同步（两条不可协商）

- `develop` 与 `master` 同步**只用 reset**：轮次合并后或任何 master 直接提交后，`git reset --hard master` + `git push --force-with-lease origin develop`。
- **禁止 rebase `develop`，禁止反向 merge（master→develop）**：合并拓扑就是回滚历史，rebase 把它抹平。

## 4. 轮次收口（round-level）

1. **前置：端到端验收**。本轮每个 feature 都已落 `develop` 之后，先在集成态过端到端验收（示例游戏完整跑通 + save/load 校验）——这是合入 `master` 的前置条件，不是可选项。
2. **阶段分离**：迭代集成与 `develop → master` 合并是两个阶段，**永不同步执行**；本轮还有任何 feature 未落 `develop` 时，禁止合并 `master`。
3. **合并仪式的硬句**：`git fetch origin` → `master` 与 `develop` **各自**做 divergence-check 并 `--ff-only` 对齐 → 主检出 `git merge --no-ff develop` → `git push origin master`。**trunk 未 push 等于合并未完成**。
4. **tag 的含义**：验收通过后打的 `vYYYY.MM.DD.<n>` 指向那个 no-FF 合并节点——它就是回滚锚点。
5. **CHANGELOG**：仓库存在 CHANGELOG 或批次报告时，在批收口一并更新。

## 5. 部署与升级

- **部署只认 master**：任何渠道部署的提交必须**已合入 `master`**——非 master ref 一律拒绝，没有例外开关。
- **部署是写操作**：**每次**部署前报告用户并取得批准。
- **升级在批结算后落**（Mod 或 host binary），然后开**新 run-id**；旧局目录不迁移、不 adopt，装载失败即开新局。

## 6. 判据

① 阻断——未 `release.confirm` / `release.skip` 就 `review.note`、在 feature 未全部落 `develop` 时合并 `master`、对 `develop` 做 rebase、部署非 master ref。② 症状——`review.note` 回执带「未声明发布」、合并后 `git log --graph` 里出现 master→develop 的反向边、tag 指向的不是合并节点。③ 确认——`git log --first-parent -1 master` 是 `Merge branch 'develop'`、`git rev-parse origin/master` 等于本地、`git tag --contains <merge>` 含本批 tag。

维护责任 = 改本幕正文或合并 / 部署纪律的执行者；触发 = 合并仪式、reset 规则或 tag 语义变更；检查 = 上面第 6 节三条确认命令。
