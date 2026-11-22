# Yggdrasill · rt 实时宿主面契约

> **id** host/rt-face · **模块** ② egg CLI 与 host · **触发** 改 `ygg.rt.*` / `crates/ygg-sandbox/src/rt.rs` / `examples/rt/` / 帧循环调度或录制回放语义之前
> **读不到代价** silent-drift（帧链哈希口径或规范串字节漂移 → 回放证据链失效，已录制的局全部不可回放）

> **定位**：常驻实时宿主（`examples/rt/`）+ `ygg.rt.*` Lua 面 + 帧循环/输入/录制回放的确定性契约。渲染面（`ygg.rt.frame`）与音频面（`ygg.rt.audio`）属 S31 计划 R2/R3 轮（Schedule: r-20261122-022804-06aa §5，验收标准已在该计划锁定——scheduled，不在本契约内展开）。
> **上游**：[docs/README.md](../../README.md) · **相关**：[cli-daemon.md](cli-daemon.md) · [serve.md](serve.md) · [sim-kernel.md](../kernel/sim-kernel.md)（ReplayLog 证伪先例）
> **落地**：S31-R1（commit `28cf93a`…`6692e17`，分支 `feature/s31-rt-r1`）

## 1. 宿主形态（D1）

常驻进程 `examples/rt/` 是继 playtest（多步推进）/ serve（独立进程）之后的第三个**非单发**宿主形态。egg 单发模式（`--event` 一次一派发、`--drive` 交互）原样并存，**不共享进程**；`game.tick` 保留为 dt=1 的退化帧（run 模式语义零变更）。

**与 AGENTS §1.4（单派发）的边界**：该规则 scope = `crates/ygg-sandbox/examples/egg.rs` 与 scripts 的 **agent 回合纪律**（一次真实动作一个回合、读全文回执再决策——约束的是 agent 决策流）。rt 宿主循 playtest 多步先例：它是**游戏帧循环**（每帧一次 `rt.tick` 派发，载荷=全整数规范串），不产出结算、不替代任何 coordination 事件链；计划 r-20261122-022804-06aa §7-R1 已裁决此边界并过门禁，本段即设计依据的落档处。主代理的 AGENTS.md 一句注记（scope 澄清）仍待同步——本契约为权威口径，AGENTS 注记是其投影。

## 2. 数据形状

### 2.1 `rt.tick` 载荷（单字符串过驱动标量门）

```text
n=<帧号>;dt=<毫秒>;<输入规范串>
输入规范串: k=<位串|->;p=<csv|->;r=<csv|->;m=<dx>,<dy>,<buttons>
```

- 全整数（帧号 1 起、dt 定步毫秒、键码 1..=64、鼠标位移可负、buttons 非负位掩码）。
- 例：`n=42;dt=17;k=100100001;p=4,9;r=1;m=0,-2,0`
- 构造单点收口：`rt::frame_payload(n, dt, input)`——录制与回放共用，任何第二处拼串都是缺陷。

### 2.2 输入规范串（`RtInput::canonical`）

| 段 | 形状 | 语义 |
| --- | --- | --- |
| `k=` | `0/1` 位串或 `-` | 帧末按住集；位 i = 键码 i；长度 = 本帧三向量最大键码；全空 = `-` |
| `p=` | 升序 csv 或 `-` | 本帧净按下沿 |
| `r=` | 升序 csv 或 `-` | 本帧净抬起沿 |
| `m=` | `dx,dy,buttons` | 帧内位移累加；buttons 后写胜 |

`RtInput::parse` 是 `canonical` 的**严格逆**：重建后再规范化必须逐字节等于输入（键序未升、段缺失、非整数一律拒绝）——录制文件手改不留活口。

### 2.3 帧记录（`FrameRecord`，JSONL 一行一帧）

```json
{"dt_ms":17,"hash":"<64hex>","input":"k=-;p=-;r=-;m=0,0,0","n":1,"state":"<64hex>","hash":"<64hex>"}
```

字段序 = 字典序（serde_json 默认 Map = BTreeMap，与驱动信封同口径）→ 同记录同字节。`state` = 每帧 `dispatch("save","")` 信封 `save` 字段重序列化字节的 SHA-256（覆盖实体+KV+pending_choice；`canonical_state_bytes` 只有实体面，KV 才是 mod 游戏状态的大头）。

## 3. 不变量

- **I1 定步 dt 零墙钟**：`dt_ms(n) = ⌊n·1000/hz⌋ − ⌊(n−1)·1000/hz⌋`——纯函数 of (n,hz)，零浮点；60Hz 时 16/17 交替、每 60 帧积分恰 1000ms。墙钟只决定一拍跑几帧（Pacer），从不决定 dt 值；墙钟时间戳只入回执元数据，永不入哈希。
- **I2 帧链哈希**：`H(n) = SHA256(H(n−1)|n|dt|规范串|全状态摘要)`；genesis = `SHA256("ygg.rt.v1|event=…|hz=…")`——调度参数入链，换 hz/换事件回放第 1 帧即红。
- **I3 输入单源**：输入只从宿主（`InputDevice` 调度/后续预览端口）进，**无 Lua 写入口**。Mod 伪造输入 = 破坏回放证据链，结构性缺席。
- **I4 帧是沿的粒度**：快照 = 帧初/帧末按住集的差集；帧内同键 down+up（或 up+down）净零，两侧都不出现；单事件内同键 down+up 属歧义（帧内序无定义），注入时点即拒绝。多事件按注入序应用。
- **I5 循环即停**：帧派发失败（VALIDATION / FUEL_EXHAUSTED / 信封错误）循环终止，无「跳帧续跑」语义；失败帧在 rt 面上不留残渣（帧号/逻辑钟回退）。
- **I6 录制即可回放**：截断录制（错误帧之前的部分）回放到同位同红；回放不产生录制副作用。

## 4. API 面（`ygg.rt.*`）

| 键 | 相位 | 签名与语义 |
| --- | --- | --- |
| `ygg.rt.config` | **仅装载期** | `config({hz=?, maxCatchup=?})`；seal 后原位替换为 `YGG-RT-SEALED` 报错桩（与 `ygg.on` 同一条运行期禁令）；形状闭集（未知键 `YGG-RT-SHAPE`）；hz∈1..=1000，maxCatchup∈1..=64 |
| `ygg.rt.loop.info` | 运行期只读 | `→ {mode, n, hz, maxCatchup, logicalMs}`；mode ∈ `run`/`rt`（缺省 run，rt 宿主逐帧置 rt） |
| `ygg.rt.input.keys` | 运行期只读 | `→ {held=位串, pressed={…}, released={…}}`（升序；held 位串与规范串 k= 段同源） |
| `ygg.rt.input.mouse` | 运行期只读 | `→ {dx, dy, buttons}` |
| `ygg.rt.input.time` | 运行期只读 | `→ 逻辑 ms`（逐帧 dt 累加，含本帧） |

配置合流优先级：**CLI > Mod 装载期声明 > 默认**（`examples/rt/` 合流点）。run 模式下 `ygg.rt.*` 照常注册（表始终在）：input 读到中性快照、loop.info 报 mode=run——单一代码路径，不静默降级。

### 4.1 `examples/rt` CLI

`<mod目录> [--frames N|--duration S] --record <路径> | --replay <路径>` + `--hz/--max-catchup/--event/--input/--boot/--fuel`。stdout 单 JSON 摘要（`ok/mode/frames/fpsDispatch/droppedFrames/hashHead/firstMismatch`）；退出码 0/1/2。`--frames` 不限速（吞吐口径）；`--duration` 按 hz 实时节拍；`--event` 可指到既有 run 模式事件（兼容桥，如 Saltline `game.tick`）；`--boot` 循环前单次派发启动事件。

## 5. 确定性契约（声明收窄）

只证「**同 seed + 同录制 → 逐帧同哈希链**」，不证「两次直播同哈希」（直播输入到达本质非确定）。内核 seed 是编译期常量（`inject::KERNEL_RNG_SEED`），不进录制文件。燃料耗尽是确定性事件（VM 指令计数确定）：录制与回放同位复现（`rt_fuel_exhaustion_is_deterministic_and_replay_reproduces_it`）。证伪语义 = first_mismatch（仿 ygg-sim ReplayLog）：篡改任一 dt / 键序 / 配置 → 恰在被篡帧红（`rt_replay_catches_tampered_*` 三例）。

## 6. 性能预算（实测，方法在列）

| 口径 | 数值 | 方法 |
| --- | --- | --- |
| **fps_dispatch（Saltline 同 mod 同事件桥）** | **155.4**（120 帧 game.tick 全回合推进，不限速，含 boot） | `YGG_PACKAGES_ROOT=… ./target/debug/examples/rt ~/WorkHub/Saltline-YGG --boot game.boot --fuel 4000000 --event game.tick --frames 120 --record …`（2026-11-22，dev 构建，M 系列） |
| 基线（每帧一进程） | 35.9（Saltline-YGG/docs/stats.md：60 次 `egg open` 1.67s） | 同文档 A5·P3 口径 |
| 10s 实时录制 | 600 帧 @ **59.9 fps**、零弃帧（hz=60 节拍合规） | 同上 `--duration 10 --record`；输入调度 7 事件 |
| 10s 录制回放 | 600/600 帧哈希链全等，3.9s | `--replay`（每帧含全状态 save 序列化+SHA-256） |
| 轻载 mod 吞吐上界 | 7858 fps（smoke mod 单帧仅 KV 累加） | `--frames 60` |

预算声明：每帧非 Lua 宿主开销 = 1 次 dispatch（转换+信封）+ 1 次 save 信封序列化 + 2 次 SHA-256（状态摘要+链推进）——60Hz 预算内余量 ≥ 2.5×（155.4/60）。R2 渲染面（span/blit）与 R3 音频面的预算增量在各自轮的实测节追加本表。

## 7. 设计依据

- **D1 宿主形态**：常驻 example 而非 egg 新模式——egg.rs 是宿主观察面（症状+复现上报，绝不 patch），playtest/serve 已立「逻辑在 src、example 薄壳」先例。
- **D2 输入=轮询快照而非回调**：(a) 确定性——每帧一条规范记录，无到达序竞争；(b) 燃料——每帧一次入 VM；(c) 与批式铁律同构。
- **D3 定步 dt**：dt=floor 差分序列使「dt 入载荷=入状态」与「实时」同时成立；整数 dt 使 mod 定点积分（`dt*ONE//1000`）无舍入漂移。
- **D4 双层证据**：帧链哈希（宿主级，输入/调度/全状态）∧ mod 级 canonical state（每帧 save 摘要入链）——篡改 dt/键序被链字节抓，篡改状态被摘要抓。
- **D5 单点收口**：`frame_payload` 唯一拼串点；`canonical/parse` 互逆且规范化自检——两处拼串或宽松解析都是回放失效的根因。
- **D6 不引 serde derive**：FrameRecord 五字段手写 JSON——`serde_json` 既有依赖内闭环，依赖白名单零新增。
- **D7 `--event` 兼容桥**：rt 宿主可驱动任何已注册事件（缺省 rt.tick）——run 模式 mod 无需改造即可进 rt 循环（Saltline 实测通道），R3 的 Saltline 改造点清单照计划执行。
- **D8 catch-up 弃账**：补帧封顶 max_catchup、超出弃账（含零头清空）——防螺旋死亡；弃帧数只入回执元数据。
