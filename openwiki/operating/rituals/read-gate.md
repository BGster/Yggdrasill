---
type: "Reference"
title: "推进必签 — 签名该消费什么、窗口怎么算、拒签怎么说"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-c1b4b807feda7647ba638e19
    resource: repo://.ygg/mods/agent-seed/data/config.json
  - id: openwiki-source-20786f1cdc5649019aa1a3b9
    resource: repo://.ygg/mods/agent-seed/src/domain/gates.lua
  - id: openwiki-source-9706fc11c20facdfc630a8e9
    resource: repo://.ygg/mods/agent-seed/src/domain/readset.lua
  - id: openwiki-source-1c80d889f00dcfd1c85bbf4a
    resource: repo://.ygg/mods/agent-seed/src/domain/readsig.lua
  - id: openwiki-source-da9af83703d953f139bc0632
    resource: repo://scripts/gen-scene-readfirst.py
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# 推进必签 — 签名该消费什么、窗口怎么算、拒签怎么说

> **id** rituals/read-gate · **幕** all（任何推进动作前） · **触发** 执行 `requirement.dispatch` / `proposal.submit` / `gate.approve` / `coordinate.submit` 之前
> **读不到代价** hard-refusal（缺签 / 过期 / 跨幕 / 他人签署 → 门禁 fail-closed，主状态不动）
> **replaces**：`docs/context/rituals/read-gate.md`（搬运基线 `3a2536f`）
> **上游**：[../../governance.md](../../governance.md) · **相关**：[dispatch.md](dispatch.md) · [../acts/coordinate.md](../acts/coordinate.md)

本页只给**执行侧判定**；规则本体（幕进必读、推进必签的义务与例外）见 [governance.md](../../governance.md) §5。

## 1. 判定输入

三个事实：磁盘上的 `ver`、你上次**正文通读**的 `ver`、以及距上次通读的**派发距离**与是否发生**上下文丢失**（压缩 / 新会话 / 交接）。

| 情形 | 必做 | 合规的样子 |
| --- | --- | --- |
| 磁盘 `ver` ≠ 你上次读的 `ver` | **正文通读**，再签名 | 能说出这次改了什么 |
| `ver` 相同，距上次正文通读 ≤ `readGate.window`（缺省 80 次派发），无上下文丢失 | **版本核对**（确认磁盘 `ver` == 已签 `ver`）后签名 | 回执写 `was=<ver> now=<ver>`；这不是通读 |
| `ver` 相同，但超窗口，或上下文丢失 | **正文通读**，再签名 | 通读在本会话记录里 |

**版本核对永不延展窗口：只有正文通读会重置它。** `ver` 就是内容身份：把正文原样取哈希、取前 6 位十六进制（24 位前缀）——它是**变更探测器，不是完整性证明**。所以「文档可能已改」由版本核对覆盖；版本核对覆盖不了的是**你自己的记忆衰减**，那由窗口与上下文丢失触发测量。机制只检查 `ver` / 期次 / 窗口：签名声明「我签了」，永不声明「我读了」。

**`ver` 从哪来**：`openwiki/ver.json` 是唯一账本，由 `scripts/wiki-ver.sh` 生成（正文留空后 `git hash-object` 取前 6 位）；页面**不写** `ver` 行——一个 `ver` 只能有一个落点，写进页面就会有两个。

## 2. 签名

```text
<egg> open <run-root> --actor <who> --event config.set --payload '"read.<doc-id>=<ver>"'
```

`doc-id` = 该页的固定头 `id`（如 `acts/coordinate`；契约页用 `kernel/state-machine` 这种 `<目录>/<文件名>` 形态）。同一 `(动作, 文件)` 换人执行须由**执行此动作的人**重签。

## 3. 维度与拒台词

| 维度 | 触发 | 文案键 |
| --- | --- | --- |
| 覆盖 | 该幕必读文件没有本动作执行者的签名 | `read_gate_missing` |
| 新鲜 | 签的 `ver` ≠ 磁盘 `ver` | `read_gate_stale` |
| 遗忘 | 跨幕未重签，或超 `readGate.window` | `read_gate_forgotten` |
| 按人 | 签了但不是你签的 | `read_gate_other_actor` |

空签署人（旧存档）不参与「按人」维；人工菜单点击结构上无宿主章，降级为 any-signer 并在回执告明（否则 `guide` 模式会被锁死）。**amend 与两路打回推进 `rev` 时，签名在同一事务里被销光**，下一个推进动作报的是「覆盖」而不是「签名过期」——因此「签名早于新 rev」不是可达状态。

## 4. 不可读即停

路径不存在、文件被移动、正文与索引矛盾——一律**停机报告**，不得据记忆推进；不得用前几行或一次 `grep` 命中冒充「读过了」。

## 5. 判据

① 阻断——用版本核对冒充通读（`ver` 相同但仍超窗口）、用别人的签名推进自己的动作、页面里又写一行 `ver`（两个落点）、读不到路径却继续。② 症状——`read_gate_stale` 与 `read_gate_missing` 交替出现、回执里没有 `was= now=` 对、`openwiki/ver.json` 与页面正文不一致。③ 确认——`scripts/wiki-ver.sh --check` 报 `changed=0 missing=0 ok=1`、`config.set read.<id>=<ver>` 的 `ver` 与账本一致。

维护责任 = 改签名机制、窗口或 `ver` 账本的执行者；触发 = 门禁维度变化、`ver` 计算方式变化、页面改名；检查 = 本页四个文案键与 `src/handlers/**` 的判定一致，`wiki-ver.sh --check` 为绿。
