# 灯下谜径（test-adventure）——M0 无头测试冒险

> 本 Mod 的双重目的：① 作为 M0 内核的**手写全链路验收素材**（多分支 / 多结局 /
> 存读档 / 重放确定性）；② 作为**创作体验样本**——作者视角使用 `ygg.*` API 写
> 一个完整短冒险时遇到的每处摩擦，都已回执给内核侧（见文末「创作体验问题清单」）。

## 如何运行

本 Mod 无独立运行器：装载与全通关由测试套件承载，跑：

```bash
cargo test -p ygg-sandbox --test test_adventure
```

6 个测试覆盖三分支通关、中盘存读档续玩、双 facade 重放确定性、资源接线证据。
测试经 `include_str! / include_bytes!` 直接取用本目录文件，无需安装步骤。

## 结构

```text
packages/mods/test-adventure/
├── mod.lua                      Mod 本体（节点图 / 状态机 / 选择路由 / 结局）
├── manifest.json                资源清单（schemaVersion 1；sha256 + byteLength 寻址）
├── schemas/scene-texts.json     资源 schema（2020-12 锚，资源 id 分发校验用）
├── data/scenes.json             全部文案与数值调参（场景 / 提示 / 选项 / 节拍 / 结局 / 伤害值）
└── README.md                    本文件
```

## 节点图与分支

```text
event.start
   └─> node:prologue  序章（生成主角 hp=10、铜提灯实体；id 落 kv）
        └─> node:tavern  酒馆
             └─ choice1「带上提灯？」 ──(ygg.choice_selected)
                  ├─ 1 接过提灯 → 提灯实体打 item:lantern 标签 ─┐
                  └─ 2 轻装出发 ────────────────────────────────┤
                     （两路均：章节机 tavern→road，goto 岔路）   │
   node:crossroad  岔路                                        │
        └─ choice2「去哪儿？」 ──(ygg.choice_selected)          │
             ├─ 1 下地窖 → node:cellar_door（章节机 road→cellar）│
             │    ├─ 有 item:lantern → 【结局A】灯下秘藏 ←──────┘
             │    └─ 无 item:lantern → 扣资源数值 dark_fall_damage
             │                          → 【结局C】暗巷折返
             └─ 2 回客房 → node:inn（支线机 road→inn）
                  └─ 【结局B】平安一夜
```

## 结局一览

| 结局 | 叙事标识行 | kv `ending` | 达成条件 |
| --- | --- | --- | --- |
| A 灯下秘藏 | `【结局A】…` | `"A"` | 带灯（choice1=1）且下地窖（choice2=1） |
| B 平安一夜 | `【结局B】…` | `"B"` | choice2=2（回客房，与 choice1 无关） |
| C 暗巷折返 | `【结局C】…` | `"C"` | 无灯（choice1=2）且下地窖（choice2=1） |

内核特性覆盖对照：叙事流 `ygg.narrative.node/["goto"]`（8 节点）；多分支
2 个 `ygg.choice` 点 + `"ygg.choice_selected"` 索引路由（kv `stage` 区分待回应点）；
实体 `ygg.entity.spawn/set_attribute/add_tag/has_tag`（提灯标签门控地窖结局）；
状态机 2 台 `ygg.fsm.define`（见下）；资源 `ygg.data`（Mod 顶层快照，全部文案
不落 Mod 源）；多结局 3 个，各带叙事标识行 + kv 旗。

**状态机为何是两台**：`ygg.fsm.define` 一机一事件、转换无条件/payload 分派、
声明序先到先得——「road → cellar / road → inn」两条出边无法同机共存（永远先
命中声明在前的 cellar）。因此主线机 `chapter`（tavern→road→cellar，每状态单
出边）+ 客房支线机 `inn_route`（road→inn，独立事件名）。这是内核表达力的真实
边界，不是本 Mod 的随意设计。

## 改动 data/scenes.json 后必须重新寻址

manifest 以 sha256 + byteLength 内容寻址。改了 `data/scenes.json` 后需同步
`manifest.json` 的 `sha256` / `byteLength`（本包手工维护，无工具链——见清单）：

```bash
shasum -a 256 data/scenes.json   # 取哈希与字节数，回填 manifest.json
```

## 创作体验问题清单（回执内核侧）

以下均以「当前 API 照实实现、内核零改动」完成，问题逐条披露：

1. **`ygg.choice` 终结会丢弃同 handler 先入队的 FIFO 派生事件**。choice 以哨兵
   错误终止当前 Lua 执行，转换层 pending_choice 置位后 FIFO 余量全部计 dropped
   ——节点体内「先 emit 再 choice」的写法静默丢事件（本 Mod 初版即踩中：章节
   推进 emit 后紧跟 choice，状态机永远停在旧态）。作者无从得知「choice 必须是
   handler 内最后一个输出调用」，建议文档显式锁定该约束。
2. **`fsm.define` 无条件/payload 分派**：同一 from 的多条出边不可表达（声明序
   先到先得），范例式的「tavern → road → cellar/inn」单机不可写，只能拆机或
   降级为线性机 + kv 路由旗。是否值得给 transition 加 `when`/payload 匹配，
   建议设计层讨论。
3. **fsm 只在发生转移时写 kv**：未触发的状态机连 `initial` 都不落盘，键缺席
   （kv get = nil）。语义自洽但作者排查「为什么没有 fsm.xxx 键」时缺提示。
4. **实体句柄不能存 Lua local**：读档 = Mod chunk 重装载 + 状态覆写，顶层
   local 指向的 id 与覆写后 store 不再对应（而重装载还会重跑顶层 spawn 造成
   重复实体）。正确姿势（id 落 kv + spawn 加 kv 门）只能靠踩坑或读内核源码获
   知，Mod 作者文档必须显式写。
5. **manifest sha256/byteLength 手工维护无工具链**：改一个字节的文案就要重算
   哈希，漏改即 ContentMismatch。建议提供打包/寻址小工具（哪怕一个脚本）。
6. **错误定位文案偏内核视角**：handler 报错浮出时位置是
   `crates/ygg-sandbox/src/driver.rs:185:107`（Lua 桥行号），作者看到的不是
   自己 mod.lua 的行号，排障要靠猜。
7. **独立 Lua 工具链不认识 `ygg` 全局**：lua-language-server 对 mod.lua 报
   62 条 undefined-global `ygg`。建议随仓库提供一份 `ygg` 类型 stub / meta
   注释，作者 IDE 体验才成立。
8. **`goto` 保留字括号形态**（已知项，`ygg.narrative["goto"]`）文档已写明，
   实际书写仍易错，IDE 无提示。

## 参考

- [Mod 作者指南](../../../docs/mod-authoring.md)——以上全部踩坑的正式文档化
  （句柄模式 / choice 终结语义 / fsm 语义 / payload 规则 / pack 工具 /
  存读档语义 / choice_selected 域校验）；
- [lua-stubs](../../../lua-stubs/README.md)——`ygg` 全局 LuaLS 类型 stub
  （IDE 补全与 undefined-global 静默）。
