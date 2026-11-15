# Yggdrasill · 剧本索引（context/scenes）
> **id** scenes/README · **幕** all · **触发** 想知道「某一步系统会说什么」时 / 查某个回执键的归属时
> **读不到代价** silent-confusion（把回执当规则读，或去 mod 里翻散文）
> **ver** 332fbf

> **定位**：**回执键的索引**，不是回执正文的副本。每一行回答三件事：这一刻系统会说什么（键名与命名空间）、它由谁在什么条件下发出、相关**政策**去哪读。
> **上游**：[../README.md](../README.md) · **相关**：[../../wiki-governance.md](../../wiki-governance.md) §4.1（三分：契约 / 知识 / 输出）
> **为什么没有逐字模板**：回执的**唯一副本**在 mod 的 `data/texts/*.json`（输出归 mod，见 wiki-governance §4.1）。docs 里再抄一份就是「两处手写」，违反 §4.2 单副本纪律——曾经存在的那份逐字表已于 2026-11-15 删除。

## 七个片段与它们的回执域

| 片段 | 域 | 典型键（命名空间） |
| --- | --- | --- |
| `core.json` | 幕级台词、prompt、菜单选项、结局、tuning | `scenes.*`（5 幕）/ `prompts.*` / `options.*` / `endings.*` |
| `beats-flow.json` | 版式、模式、章程与幕转场 | `run_begun` / `charter_*` / `play_*` / `*_common` / `rejected*` |
| `beats-requirements.json` | 需求九幕的受理、门禁、集成、终态 | `req_*` / `accept_*` / `integration_*` / `lane_*` |
| `beats-settlement.json` | 盘点四项、批结算、发布、退役 | `verify_*` / `settlement_*` / `release_*` / `review_*` |
| `beats-harness.json` | 派发 brief、配置面板、工具桥、子代理回执 | `subagent_*` / `config_*` / `tool_*` |
| `beats-map-guide.json` | 地图/清单查询与人类动作门 | `map_*` / `requirement_list*` / `guide_human_blocked` |
| `beats-lanes.json` | 泳道台账与探针 | `lane_*` / `worktree_*` |

## 每一刻该读哪份政策

| 这一刻 | 回执会说什么 | 政策读这里 |
| --- | --- | --- |
| 立局 / 修订章程 / 开工 | `[RUN BEGUN]`、`charter_*` | [../acts/charter.md](../acts/charter.md) |
| 协调：派发 / 受理 / 门禁 | `[DISPATCH]`、`req_gate_*`、`[REJECTED:*]` | [../acts/coordinate.md](../acts/coordinate.md) |
| 需求九幕的每一步 | `req_*`、`accept_*`、`integration_*` | [../req-acts/](../req-acts/)（intake / delivery / integration） |
| 盘点与批结算 | `verify_*`、`settlement_*`、`[BATCH n SETTLED]` | [../acts/settle.md](../acts/settle.md) |
| 发布声明 | `release_*` | [../acts/release.md](../acts/release.md) |
| 退役 | `[ENDING`、`endings.done` | [../acts/retired.md](../acts/retired.md) |
| 派发子代理 | `subagent_brief`、`[TOOL SETTLED]` | [../rituals/dispatch.md](../rituals/dispatch.md) |
| 读签未过 / 身份不符 | `[READ FIRST`、`read_gate_*` | [../rituals/read-gate.md](../rituals/read-gate.md) |
| 清理未完成 | `[SESSIONS CLEANUP PENDING]`、`[TODO …]` | [../rituals/cleanup.md](../rituals/cleanup.md) |
| 无 egg 环境降级 | `*_fallback_*` | [../rituals/fallback.md](../rituals/fallback.md) |

## 回执的三条硬约束

1. **协议前缀是契约**：`[READ FIRST` / `[BATCH` / `[ENDING` / `[REJECTED:` / `[BLOCKED:` / `[DISPATCH]` / `[REPLAY:` 由宿主与测试按字面识别（`pack.json` 的 `agent.rejectionPrefixes` 决定「无效果」判定）。改文案不得改这些前缀。
2. **回执是输出，不是知识**：出现「因为…所以以后要…」「有几种用法」这类句子即为漂移——那属于 docs，不属于 mod（§4.1）。
3. **要改文案**：直接改 `.ygg/mods/agent-seed/data/texts/*.json`，跑 `./scripts/sync-seed-mod.sh` + 三道门禁；**不要**在 docs 里维护第二份。
