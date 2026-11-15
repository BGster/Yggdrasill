# Yggdrasill · mod 文案契约（copy-contract）
> **id** copy/README · **幕** all（改文案前） · **触发** 任一 mod 文案键变更
> **读不到代价** silent-drift
> **ver** 000000

> **定位**：mod 状态行文键的知识权威索引。正文按现有 beats 消费域拆为七件；mod `data/texts/` 是运行时投影，改文案先改对应域文件再同步 mod。
> **上游**：[../README.md](../../README.md) · **相关**：[acts/](../acts/) · [../design/prompt-surface.md](../../design/prompt-surface.md)
> **拆分判据**：消费域，不按行数对半切；每个域文件直接对应一个 `data/texts` 片段，core 文件同时承载跨域指针壳与键账。

## 域文件与召回路径

| 消费域 | 权威文件 | 运行时片段 | 当前键数 |
| --- | --- | --- | --- |
| 回执版式 / 模式 / 章程与幕转场 | [flow.md](flow.md) | `beats-flow.json` | 63 |
| 配置 / 身份 / 工具桥 / tools 注册 | [harness.md](harness.md) | `beats-harness.json` | 69 |
| 泳道账本 / 池 / probe | [lanes.md](lanes.md) | `beats-lanes.json` | 36 |
| 地图 / 人机门禁 / 节点注记 | [map-guide.md](map-guide.md) | `beats-map-guide.json` | 38 |
| 需求 / 提案 / 门禁 / 修订 | [requirements.md](requirements.md) | `beats-requirements.json` | 144 |
| 盘点 / 批结算 / 发布 / tools 机证 | [settlement.md](settlement.md) | `beats-settlement.json` | 54 |
| options / tuning / endings / 指针壳与总账 | [core.md](core.md) | `core.json` | 36 + §H |

六个 beats 片段共 404 键，core 的 options/tuning/endings 共 36 键，合计 440 个唯一键。§H 的 23 处指针壳是子集，不额外计数；实测口径、owner、trigger 与检查见 [core §H](core.md#h-23-处散文键的指针壳句式mod-收缩后形态)。

## 通用读法与协议归属

- **句式模板**逐字对应 mod 键值（`{{变量}}` 原样保留）；mod 运行时 `fill(TEXTS.x, {vars})` 填充后输出。
- **协议归属**：`[REJECTED:ACTION]` / `[REJECTED:PAYLOAD]` 前缀由 pack `agent.rejectionPrefixes` 声明，宿主据此把该行计为 failed step；`[ENDING` = 终局行（`agent.endingPrefix`）；`[REPLAY` = 幂等重放提示（`agent.replayPrefix`）；`叙述` = 普通叙事行，无协议语义。
- **动态拼接键**：`<node>_common`（render.lua 按 `st.."_common"` 拼接）、`map_note_*`（render.lua 注记表按动作名引用）、`req_prop_{exec,actor,actor_session}_misplaced`（wire.lua 按段名查 beat）、`executor_bad_actor{,_session}`（wire.lua 按身份段查 beat）——键名由代码拼接，不可改。
- **23 处散文键**（`*_common`×6、`scenes`/`prompts` 及长策略键）的叙事正文在 [acts/](../acts/) / [rituals/dispatch.md](../rituals/dispatch.md) / [rituals/cleanup.md](../rituals/cleanup.md)；域表只收运行时句式，指针目标集中在 [core §H](core.md#h-23-处散文键的指针壳句式mod-收缩后形态)。

## 维护闭环

owner = 文案或键面修改者；trigger = 任一键值、键集、域归属或指针目标变化。先在对应域文件修改，再同步 mod JSON、manifest、[../design/prompt-surface.md](../../design/prompt-surface.md) 与 [../README.md](../../README.md)。`agent_seed_semantics` 逐域比较键集并检查指针文件存在，`docs-consistency.sh` 检查文档闭包；缺键、死链或只改总数而未改域表，均不得验收。
