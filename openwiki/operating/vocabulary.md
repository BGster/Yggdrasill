---
type: "Reference"
title: "术语表 — 一句话定义与易混点"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T07:10:30.723Z
sources:
  - id: openwiki-source-c0d5856e80be2560c5caa5c8
    resource: repo://crates/ygg-host/src/image.rs
  - id: openwiki-source-2525bb44114bad58534d56ee
    resource: repo://crates/ygg-host/src/proto.rs
  - id: openwiki-source-682e02a9e393e886de83488d
    resource: repo://crates/ygg-host/src/session.rs
  - id: openwiki-source-ce6aebf2c3323e209b2938c2
    resource: repo://crates/ygg-sandbox/src/host_api/world/install.rs
  - id: openwiki-source-3ce02e4845420a1c74760ca3
    resource: repo://crates/ygg-sandbox/src/modload.rs
  - id: openwiki-source-fef267a23f756637e121d083
    resource: repo://crates/ygg-sandbox/src/sandbox.rs
  - id: openwiki-source-bcfb11d1b18518491d65f25e
    resource: repo://crates/ygg-sim/src/archetype.rs
  - id: openwiki-source-8c02dd48d2aed1554766f57a
    resource: repo://crates/ygg-sim/src/lib.rs
  - id: openwiki-source-06a44f5a944119bb5b73eaf6
    resource: repo://crates/ygg-sim/src/sim.rs
  - id: openwiki-source-dfe0e9598ad418e75d354f01
    resource: repo://packs/example/golden.json
generated: { by: "pi", at: "2026-11-26T07:10:30.723Z" }
---

# 术语表 — 一句话定义与易混点

> **id** `operating/vocabulary` · **模块** 操作 · **触发** 读契约时遇到形状词、评审里两方用同一词指不同东西
> **读不到代价** `silent-drift`（同一个词在同一份设计里指两件事，讨论会在一小时内失去意义）

每条一句话，紧跟「易混点」——那里才是真正会吵起来的地方。

## 1. 世界与存储

| 词 | 定义 | 易混点 |
| --- | --- | --- |
| **世界（world）** | 内核持有的唯一权威状态：tick、archetype 集、RNG 流表、KV | 不是「场景」「关卡」——那些是包用世界搭出来的东西 |
| **实体（entity）** | 一个 `u64` id，指向某 archetype 里的一个槽位 | 不是对象、没有身份——id 复用时旧引用会静默指向新实体 |
| **组件（component）** | 一个有名字、有类型、固定整数宽度的列；id 是 `u32` | 不是「属性包」——一个实体的同类语义不要拆成两列（`moved` 不塞进 `pos`） |
| **kind** | 组件宽度声明：`i64` / `u64` / `u8` / `bytes` | `i64` 不是「任意整数」——超过 64 位直接拒绝；`bytes` 不参与数值比较 |
| **archetype** | 组件集合相同的一组实体，内部按列存 | 不是「表」也不是「类」——加/删组件会换 archetype，逐元素搬迁 |
| **槽位（slot / row）** | 某实体在其 archetype 列里的下标 | 与 eid 不同：eid 稳定，槽位随结构变更移动 |
| **列（column）** | 某组件在 archetype 内的连续存储 | 与「字段」不同：列是所有实体共享的一份内存 |

## 2. 时间与推进

| 词 | 定义 | 易混点 |
| --- | --- | --- |
| **tick** | 单调递增的整数步号，世界唯一的时间 | 不是秒——`dt` 才是时长 |
| **dt** | 一步的毫秒时长（`u32`），由宿主给出 | 内核不用它算物理；它只进哈希与 delta 帧 |
| **step** | 「推进 N 步」的整段操作：先 pre 阶段、再每 tick 的 rules/post、最后 delta | 不是「一帧渲染」——渲染频率由宿主定 |
| **seal** | 封盘：登记系统表与组件表，此后不可再注册 | 不是「启动」——`seal` 之后才开始 step，之前只会报 sealed |
| **pending** | 结构变更（spawn/despawn/换 archetype）的待办队列 | 不是「立即生效」——它在 tick 边界统一落地，保证中途读到的 id 一定有效 |
| **变更集（changed）** | 本帧真正被写过的 `(eid, cid)` 列表 | 不是「全量状态」——没写的列不出现在里面，宿主不能靠它推断其余列 |
| **回放（replay）** | 拿事件日志重跑，逐帧比对 hash | 不是「存档」——它不带状态，只带输入 |
| **快照（snapshot）** | 世界的完整二进制状态（含 tick、hash、RNG 流、archetype） | 不是「镜像」——快照是运行态，镜像是装载源 |

## 3. 规则面（Lua）

| 词 | 定义 | 易混点 |
| --- | --- | --- |
| **包（pack）** | 一个规则单元：`pack.json` + `src/*.lua` + 可选 `data/` | 不是「插件」——v1 没有多包共存与卸载 |
| **入口 chunk** | `pack.json` 的 `entry` 指向的 Lua 文件，返回值即声明面 | 不是 `main()`——它的返回值才是契约（`return { commands = … }`） |
| **命令面（command surface）** | 入口返回值里的 `commands` 表：名字 → 函数 | 不是「系统」——命令由宿主发起，系统由 tick 驱动 |
| **系统（system）** | 声明了 `stage/reads/writes` 的 tick 级函数 | 不是「回调」——`writes` 不诚实会在 `seal` 时被写写冲突拦下 |
| **stage** | `pre` / `rules` / `post` 三段 | 不是优先级数字——同段内顺序由注册序决定 |
| **指令预算（budget）** | 单次 Lua 入口最多执行的指令数，默认 200 万，按入口重置 | 不是「超时」——它按指令计数，不看墙钟 |
| **整定点（Q32.32）** | 低 32 位是小数部分的 `i64` | 不是浮点近似——比较与累加都精确，但乘除要自己移位 |
| **RNG 流（stream）** | 按名字派生的独立随机序列 | 不是「全局种子」——`math.random` 走 `math.random` 流，包自建的流各不相干 |

## 4. 协议与数据面

| 词 | 定义 | 易混点 |
| --- | --- | --- |
| **协议帧（frame）** | 一条 JSON 消息，`type` 字段区分 13 个 tag | 不是「一次调用」——`step` 会产出两条（`events` 与 `delta`） |
| **delta** | 只含本帧 `changed` 的最小更新 | 不是「events」——`events` 讲发生了什么，`delta` 讲哪里变了 |
| **会话（session）** | 宿主侧的连接状态：`Fresh` / `Ready` / `Closed` | 不是「世界」——同一世界可以换会话（`Open` 快照） |
| **镜像（image）** | 装载源：`modSource` + `manifest` + `schemas` + `resources` + `modules` + `chunkPaths` | 不是「打包产物」——它是可读 JSON，装完不进世界状态 |
| **pack_hash** | `manifest` 文本的 sha256，握手时对账 | 不是「版本号」——改了包数据但 manifest 没变，它不会变 |
| **载体（carrier）** | 绑定的实现形态：Rust / C ABI / WASM | 不是「平台」——三载体跑同一份 parity 脚本，字节必须一致 |

## 5. 构建与发布

| 词 | 定义 | 易混点 |
| --- | --- | --- |
| **装载序** | 资源表 → 沙箱 → 世界 → 注入 → 宿主面 → 资源面 → 标准库 → 模块 → 入口 | 不是「初始化脚本」——顺序写死在 `Loaded::build` 里，改序等于改契约 |
| **哈希输入面** | 参与 `state_hash` 的字节：tick、组件表、archetype 内容、RNG 流、结构 | 不是「文件摘要」——注释、日志、KV 都不在里面 |
| **金值（golden）** | 固定种子下逐 tick 的 hash 列表 | 不是「快照」——它只是一串数字，用来钉住确定性 |
| **零构建宿主** | 不依赖打包器的宿主（`hosts/h5`） | 不是「无构建」——内核自己仍要 `build-wasm.sh` 产 wasm |

## 6. 维护责任

- **触发**：新增/更名任一内核概念、契约页新增小节、评审中出现「同一个词两种用法」。
- **检查**：本页词条必须能在某一篇契约页里找到对应小节；新增词条要同时补进对应契约页的四行头或小节标题。
