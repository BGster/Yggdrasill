---
type: "Reference"
title: "知识库治理 — 权威、生命周期、账本与维护义务"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T17:33:41.429Z
sources:
  - id: openwiki-source-8037e2358a2c4f9b2c722a11
    resource: repo://AGENTS.md
  - id: openwiki-source-88bd460316cc9f4d5e371ae1
    resource: repo://scripts/wiki-consistency.sh
  - id: openwiki-source-219a851abf50e3acdd3f032c
    resource: repo://scripts/wiki-ver.sh
generated: { by: "pi", at: "2026-11-26T07:10:30.723Z" }
---

# 知识库治理 — 权威、生命周期、账本与维护义务

> **id** `governance` · **模块** 中枢 · **触发** 写/删/改任何 openwiki 页、改 `AGENTS.md`、跑一致性门禁、判「这条该写哪一页」
> **读不到代价** `silent-drift`（没有权威顺序与账本，页面会逐步退化成过期散文，而所有门禁仍显示绿）

## 1. 权威顺序

```text
源码与测试  >  openwiki/contracts  >  openwiki/operating  >  AGENTS.md  >  其他一切（含生成页与外部文档）
```

- **源码与测试是唯一事实**：页面与代码冲突时，页面错；
- `contracts/` 描述**必须保持不变的东西**（数据形状、不变量、API 面、错误码）；
- `operating/` 描述**怎么做**（建、跑、验、发布、排查）；
- `AGENTS.md` 是冷启动索引：只放「读不到就会做错事」的约束与命令，细节一律指向本库；
- OpenWiki 自动生成的 `index.md`（每目录一份）是**导航投影**，不是权威，不进引用、不改。

## 2. 页面生命周期

1. **计划**：新页/删页先进入 run 的 plan（`openwiki_submit_plan`），路径相对 `openwiki/`，禁止 `index.md`/`log.md`/`instructions.md` 这些根级保留名；
2. **写页**：一页一次写完整正文（frontmatter 由工具生成，正文中文）；契约页四行头 `id / 模块 / 触发 / 读不到代价`，操作页把「模块」换成「幕/场景」；
3. **主张**：每页提交 claims（`openwiki_submit_page`），每条至少一个 `repo://` 证据；
4. **删除**：删页必须同时清 `ver.json` 条目、`.claims/` 对应文件、以及所有指向它的链接——只删文件不算删页。

**禁止手改生成物**：`openwiki/**/index.md`、`.run.json`、`.last-update.json`、`.page-manifest.json` 都是工具状态。

## 3. 契约 vs 操作的二分判据

三步，任一命中即归 `contracts/`：

1. 换一个宿主（Godot / H5 / Unity）后，这段内容是否需要**逐字改**？→ 需要 = `contracts/`；
2. 它约束的是**数据形状、不变量、API 签名、错误码**吗？→ 是 = `contracts/`；
3. 它描述的是**一次动作的顺序、命令、验收**吗？→ 是 = `operating/`。

边界情形：性能预算、版本策略、装载序属于 `contracts/`（它们约束实现而非操作者）；构建/运行/验收命令属于 `operating/`。

## 4. ver 账本

- `openwiki/ver.json` 是**唯一**的版本账本：`{ "<page-id>": "<6 hex>" }`，`page-id` 取自页面四行头的 `id`；
- ver = 页面文件的 git blob hash 前 6 位；**页面正文不写 ver 行**（写进正文就会自我指涉，永远算不出稳定值）；
- `bash scripts/wiki-ver.sh --check` 输出 `files=;changed=;missing_id=;dup_id=;missing_ver=;broken_links=;ok=`；`ok=1` 才允许提交；
- 改页面 → 必须重算账本（不带 `--check` 跑 `bash scripts/wiki-ver.sh` 即写盘），否则下一轮 `--check` 报 `changed>0`；
- 生成页（`index.md`）不入账本。

## 5. 一致性闭包（两条脚本，缺一不可）

| 脚本 | 管什么 | 输出 |
| --- | --- | --- |
| `scripts/wiki-ver.sh --check` | 账本与页面是否同步、页内相对链接是否解析、id 是否唯一 | `files/changed/missing_id/dup_id/missing_ver/broken_links/ok` |
| `scripts/wiki-consistency.sh` | 全库相对链接、`AGENTS.md` 行数预算、**claims 证据闭包**（证据文件存在且 sha256 与 claim 记录一致） | `links/broken/agents/claims/claims_bad/ok` |

代码侧的事件日志签名核验（旧 `--attest` 模式）随 egg / agent-seed 一起删除：本仓不再有 run 目录，签名语义只作为**产品行为**活在 `openwiki/operating/*` 的历史页里。

`claims_bad>0` 意味着「页面在说一件没有证据支撑的事」——这比链接断更严重，必须清零才能提交。

## 6. `AGENTS.md` 的行数预算

预算 **160 行**（`scripts/wiki-consistency.sh` 的 `AGENTS_BUDGET`）。理由：冷启动文本的每一行都在每次会话里被读一遍，膨胀的代价是复利；超预算时该做的是**下沉**（把细节挪进本库对应页），不是调高预算。

## 7. 维护义务

- **改代码即改契约**：同一个提交里，改 `crates/**` 的契约面就必须改对应 `contracts/**` 页并重算 ver；
- **删组件即删页**：删除 crate/脚本/示例时，同提交里删掉它的页面与 claims；
- **批结算前**：`bash scripts/wiki-ver.sh --check` 与 `bash scripts/wiki-consistency.sh` 双绿；
- **不留兼容层**：旧路径整段改写引用，不写「见旧位置」的转发页（转发页比死链更难发现）。

## 8. 呈报分层

| 层 | 说什么 |
| --- | --- |
| 结论 | 做完了什么、门禁绿不绿、下一节点是什么 |
| 证据 | 命令原文 + 关键输出行（含哈希、计数、耗时） |
| 偏差 | 与计划的差异、以及为什么 |
| 明细 | 文件清单、diff 统计——只在被问时展开 |

## 9. 维护责任

- **触发**：任何 openwiki 页的增删改、`AGENTS.md` 改动、ver 账本或 claims 结构变化、门禁脚本改动。
- **检查**：`bash scripts/wiki-ver.sh --check; bash scripts/wiki-consistency.sh`（双 `ok=1`）。
