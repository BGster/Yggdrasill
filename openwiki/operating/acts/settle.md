---
type: "Reference"
title: "盘点幕 · settle — 验绿、登记演进、清会话、清待办"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-c1b4b807feda7647ba638e19
    resource: repo://.ygg/mods/agent-seed/data/config.json
  - id: openwiki-source-654f4afa4081b245acf7b633
    resource: repo://.ygg/mods/agent-seed/src/domain/wire/evolution.lua
  - id: openwiki-source-90c75edb710a356b1957d6e9
    resource: repo://.ygg/mods/agent-seed/src/domain/wire/settlement.lua
  - id: openwiki-source-d5edd97ca7e6335c0cd02c35
    resource: repo://.ygg/mods/agent-seed/src/handlers/operate/review.lua
  - id: openwiki-source-b640c22b435c5b19de7f36dd
    resource: repo://.ygg/mods/agent-seed/src/handlers/system/config.lua
  - id: openwiki-source-502ebdd47f61d6de349bbd12
    resource: repo://crates/ygg-sandbox/src/driver/tests.rs
  - id: openwiki-source-422bb41f0e2d55ebad3aceea
    resource: repo://crates/ygg-sandbox/src/host_contract/agent.rs
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# 盘点幕 · settle — 验绿、登记演进、清会话、清待办

> **id** acts/settle · **幕** settle · **触发** `coordinate.submit` 成功（进入 settle 相位）
> **读不到代价** hard-refusal（载荷形状错、`c=`/`cls=` 计数不自洽被拒）
> **replaces**：`docs/context/acts/settle.md`（搬运基线 `3a2536f`）
> **上游**：[coordinate.md](coordinate.md) · **相关**：[release.md](release.md) · [../rituals/cleanup.md](../rituals/cleanup.md) · [../../contracts/kernel/state-surface.md](../../contracts/kernel/state-surface.md)

**场景台词（知识权威）**：任务「{{title}}」：{{count}} 条需求（{{done}} 条已终态）｜require_all_green {{gates}}。四项齐备（验证、演进登记、会话清理、待办清理）自动进发布幕。

## 1. 四项，顺序固定

**入口**：`coordinate.submit` 成功（`verify.passed` 复位为 false，本批 settlement 键清空）。

| 序 | 动作 | 断言 |
| --- | --- | --- |
| ① | `verify.submit` | 记全绿；`all_green=false` 且 `gates.require_all_green=true` 时打回协调幕返工 |
| ② | `settlement.evolution` | 登记演进，`summary` trim 后必填 |
| ③ | `settlement.sessions` | 会话清理声明 |
| ④ | `settlement.todo` | 待办清理声明 |

**出口**：四项齐备 → 发布幕（`[SETTLE READY]`，自动转场，不花事件）。③④ 可并入 ② 的可选段一次事务完成；独立事件形态仍保留。

## 2. 载荷语法与拒绝面

```
settlement.evolution <artifact>|<summary>[|c=<kept>/<screened>[|cls=<kind>:<n>,...]][|sessions=<moved>/<remaining>/<destination>][|todo=<remaining>/<verified_by>]
```

- `summary` trim 后必填；`c=0/0` 时 artifact 可空。
- `screened>0` 时 **artifact 与 cls 必填**，且六类计数与 `kept`/`screened` **自洽**。
- guide 可预登记 `config.set review.evolution.artifact|summary|candidates|classes=<value>`：逐键查形状，登记时查跨字段完整性。
- `review.note` **只收** `lesson|score`；旧 `ev` / `c` / `cls` 段一律拒。它先跑 `gates.settlement_problem()`，把本批键**单源归档**到 `review.evolution.*`，再批结算或退役。

## 3. 演进筛选（登记前逐项完成）

- **长期优先级**：换任务 / 换仓库之后这件事是否仍然出现？只在当前任务成立的约束不入册。
- **既有未用规则必须选唯一根因**（六类）：call chain / discoverability / contract mismatch / deliberately unused / genuinely no scenario / redundant。
- **每项独立评审**：工件含 `file:line`、可运行命令及**真实输出**、语义依据、根因类别与结论；排序 = 实际场景 → 未满足的流程步骤 → 代码与复现 → 根因 → 长期收益 → 改动面与验证。
- **不入册**：单任务约束、临时工具 / 探针约定、同任务内已解决项。

**登记证明什么、不证明什么**：`settlement.evolution` 只证明 summary 非空、路径形状、计数与类别自洽。沙箱读不到工件与子会话，所以**声明绝不证明建议质量或评审独立性**——报告不得越界声称。

## 4. 不得新增第二载体

演进流程**永不**新增第二个决策 / 执行入口、外部检查脚本或宿主改动。唯一登记动作是 `settlement.evolution`；唯一裁决与执行归属仍是 `AGENTS.md`（判据面）与 Mod 资源（`src/**` 门禁逻辑、`data/*.json` 提示面）两处。想「写个脚本帮你守纪律」，只会在同一个地方被绕过；想加第三个入口，等于建第二套规范面。

## 5. 宿主收口与批边界

- `agent.summary.nextEndsRun` 仅在 settle 首项为 `review.note` 且 `run.retire` 已武装时为 true；无预登记时默认空 artifact + 中性 summary + `c=0/0`。
- 宿主随后进入 closing（禁新写、最后 drain）；**未武装的批结算不得预关闭**。
- 常驻局在 `batchDrained` 且 inbox 无未消费行时安全暂停 exit 0——**不在空批上连发 `coordinate.submit`**。

**批边界即重启点（升级纪律）**：升级（Mod 演进或宿主二进制）**只在批结算之后**落地，随后开**新的 run-id**；旧 run 目录**永不迁移、永不 adopt**——装载失败即开新局。批边界之所以是天然重启点，是因为它已是一个自洽收口：本批需求全部终态、四项齐备、发布已声明。

## 6. 判据

① 阻断——批中间替换 Mod / 宿主后继续推进同一个局；写迁移或兼容代码去读旧 run 目录；在 `require_all_green=true` 时用 `all_green=false` 蒙混。② 症状——同一 `run-id` 下出现两个 Mod 版本的产物、为「让旧局能开」降低装载校验、`c=` 与 `cls=` 相加不等于 `kept+screened`。③ 确认——新局是新的 `.ygg/runs/<id>` 且 `run.json` 的 `store` / `engineVersion` 与当前二进制一致；`review.note` 回执的 `lesson|score` 不带旧段。

维护责任 = 改 `src/handlers/operate/review.lua`、`src/domain/gates.lua` 或本幕正文的执行者；触发 = 载荷语法、筛选六类或升级纪律变更；检查 = `agent_seed_semantics` 的结算与维护用例 + 本页 `c=`/`cls=` 自洽规则与代码一致。
