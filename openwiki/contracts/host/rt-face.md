---
type: contract
title: rt 实时宿主面契约
description: examples/rt 常驻帧循环、rt.tick 载荷与输入规范串、帧链哈希、渲染/音频旁路 digest、ygg.rt.* 面、录制回放确定性与性能预算。
tags: [host, rt, determinism, replay, render, audio, contract]
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-effd842294adb6d5ed141e6b
    resource: repo://crates/ygg-sandbox/src/audio.rs
  - id: openwiki-source-82696215c520660413b50065
    resource: repo://crates/ygg-sandbox/src/frame.rs
  - id: openwiki-source-dd7737091f3598fb3612807c
    resource: repo://crates/ygg-sandbox/src/rt.rs
  - id: openwiki-source-117ae9cbc7e5f686a7dd5370
    resource: repo://crates/ygg-sandbox/tests/frame_face.rs
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# rt 实时宿主面契约

> **id** host/rt-face · **模块** ② egg CLI 与 host · **触发** 改 `ygg.rt.*` / `crates/ygg-sandbox/src/rt.rs` / `examples/rt/` / 帧循环调度或录制回放语义之前
> **读不到代价** silent-drift（帧链哈希口径或规范串字节漂移 → 回放证据链失效，已录制的局全部不可回放）
> **replaces**：DESIGN §3.4（L440–481）（搬运基线 3a2536f）

**定位**：常驻实时宿主 `examples/rt/` + `ygg.rt.*` Lua 面 + 帧循环/输入/渲染面/音频面/录制回放的确定性契约。渲染面（`ygg.rt.frame` 五键 + PPM/SSE 双通道 + 帧 digest 旁路 KAT）与音频面（`ygg.rt.audio` 三键 + 帧末混音 + WAV 通道 + 混音 digest 旁路）均已落地。
<!-- openwiki: broken internal link [../README.md] file "../README.md" does not exist. Fix the href or restore the target, then delete this comment. -->
**上游**：[architecture/README.md](../../architecture/README.md) · **相关**：[cli-daemon.md](cli-daemon.md) · [serve.md](serve.md) · [sim-kernel.md](../kernel/sim-kernel.md)（ReplayLog 证伪先例）

## 1. 宿主形态

常驻进程 `examples/rt/` 是继 playtest（多步推进）/ serve（独立进程）之后的第三个**非单发**宿主形态。egg 单发模式（`--event` 一次一派发、`--drive` 交互）原样并存，**不共享进程**；`game.tick` 保留为 dt=1 的退化帧（run 模式语义零变更）。

**为什么这不算破坏单发纪律**：单派发规则的 scope 是 `crates/ygg-sandbox/examples/egg.rs` 与 scripts 的 **agent 回合纪律**（一次真实动作一个回合、读全文回执再决策——约束 agent 决策流）。rt 宿主循 playtest 多步先例：它是**游戏帧循环**（每帧一次 `rt.tick` 派发，载荷 = 全整数规范串），不产出结算、不替代任何 coordination 事件链。

## 2. 数据形状

### 2.1 `rt.tick` 载荷

```text
n=<帧号>;dt=<毫秒>;<输入规范串>
输入规范串: k=<位串|->;p=<csv|->;r=<csv|->;m=<dx>,<dy>,<buttons>
```

全整数（帧号 1 起、dt 定步毫秒、键码 1..=64、鼠标位移可负、buttons 非负位掩码）。例：`n=42;dt=17;k=100100001;p=4,9;r=1;m=0,-2,0`。构造单点收口 `rt::frame_payload(n, dt, input)`——录制与回放共用，任何第二处拼串都是缺陷。

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
{"audio":"<64hex>","dt_ms":17,"frame":"<w>x<h>|<64hex>","hash":"<64hex>","input":"k=-;p=-;r=-;m=0,0,0","n":1,"state":"<64hex>"}
```

字段序 = 字典序（serde_json 默认 Map = BTreeMap，与驱动信封同口径）→ 同记录同字节。`state` = 每帧 `dispatch("save","")` 信封 `save` 字段重序列化字节的 SHA-256（覆盖实体+KV+pending_choice；`canonical_state_bytes` 只有实体面，KV 才是 mod 游戏状态的大头）。可选字段 `frame`（本帧末次 present）与 `audio`（本帧有活动 voice）——无则**整键缺席**（R1 录制行字节零回退）。回放**严格对表**（录有实无 / 录无实有 / 同有不同值均 mismatch）。

### 2.4 渲染面

- **缓冲**：单面（`open` 装载期开，再开 `YGG-RT-OPENED`），`w,h ∈ 1..=4096`（≤64MiB 封顶）；像素 = `u32` 打包 `0xRRGGBBAA`（大端字节即 RGBA8 序），行主序、顶行在前；跨帧持久（不 clear = 上帧残留，仍确定）。
- **写入语义**：clear / span / blit 全部**直写覆盖零混色**；x/y/w/hh 逐参钳到缓冲界（越界静默裁掉、零面积 no-op——射线柱超屏是常态非错误）；负 w/hh、浮点参数、blit 长度非 w×4 正整数倍 → `YGG-RT-SHAPE`。
- **帧 digest 规范序**：`SHA256("ygg.rt.frame.v1|<w>x<h>|" ++ RGBA8 行主序顶行在前原始字节)`——尺寸入前像防异形同字节碰撞。KAT 钉值（python3 hashlib 独立复算，`tests/frame_face.rs` 逐字节钉）：2×2 全 `0x102030FF` = `e659996d…4860e53a`；4×3 黑底+中行红 = `a7402522…b6d042a`；+blit 2×1 绿@(1,1) = `496e8c20…9a66c`；4×3 全红（span(−1,−1,9,9)≡clear）= `d6c3d184…2dfc45d`。
- **脏区**：写并集最小外包矩形；present 读出后**复位**（无写 = nil）；clear = 全幅。
- **输出双通道**（读侧，零确定性影响）：`--ppm <目录>` 落 `f%06d.ppm`（P6 头 + RGB 弃 alpha）；`--preview <端口>` 内嵌预览（GET `/` canvas 页、GET `/stream` SSE 推 base64 RGBA、POST `/api/input` 输入回注——走 `InputDevice::at` 既有入口、**仅录制模式消费**）。

### 2.5 音频面

- **单混音面**（`open` 装载期开，再开 `YGG-RT-OPENED`），`rate ∈ 8000..=48000` 缺省 22050，**ch 固定 1**（mono 下 pan 无定义，ch=2+pan 同轮才有意义）。
- **voice**：`play(h, pcm, {gain})` 入队即刻播——pcm = s16 小端单声道串（0 < 样本数 ≤ rate×10s），gain 千分比 0..=2000 缺省 1000，**入队时烘焙**（`s·gain/1000` 截断后饱和钳到 i16，后改不追溯 = 确定性）；voice 整数 1 起不回收；并发 ≤ 64 超限 `YGG-RT-AUDIO-CAP`。`stop(h,v)` **帧内生效**（已混完/未知 = no-op，v1 不留墓碑）。
- **窗口样本数 = I1 镜像**：`samples(n) = ⌊logical_n·rate/1000⌋ − ⌊logical_{n−1}·rate/1000⌋`——纯整数零漂移；22050@60Hz 每 1000ms 恰 22050 样本（KAT 钉）。play→混入延迟 = **0 帧**（handler 内 play 当帧窗口即混出；混音序 = 帧序 = 录制序）。
- **混音 = 饱和整数加法**（voice = 同帧同起点时间对齐层，逐层整数和、钳到 `[-32768,32767]`、不做二次归一）；voice 跨帧续播。
- **混音 digest 规范序**：`SHA256("ygg.rt.audio.v1|<rate>x1|" ++ 本帧窗口 s16 小端字节)`。本帧无活动 voice → 窗口照写静音进整局流（WAV 时长不失真）、`FrameRecord.audio` **整键缺席**（镜像 frame 键规则）。
- **WAV 通道**（读侧）：`--audio <路径>` = 整局混音流单 WAV（44 字节 RIFF 头关停时定稿 + s16 小端交织）。未 open 音频面则不写文件。
- **live-out 裁决**：拒 cpal——零新增依赖红线 + 回调线程属墙钟域契约外不可钉验收 + digest 可验的 WAV 通道已覆盖目标；人耳抽验 = 事后 `afplay`（不进热循环）。

## 3. 不变量

- **I1 定步 dt 零墙钟**：`dt_ms(n) = ⌊n·1000/hz⌋ − ⌊(n−1)·1000/hz⌋`——纯函数 of (n,hz)，零浮点；60Hz 时 16/17 交替、每 60 帧积分恰 1000ms。墙钟只决定一拍跑几帧，从不决定 dt 值；墙钟时间戳只入回执元数据，永不入哈希。
- **I2 帧链哈希**：`H(n) = SHA256(H(n−1)|n|dt|规范串|全状态摘要)`；genesis = `SHA256("ygg.rt.v1|event=…|hz=…")`——调度参数入链，换 hz / 换事件回放第 1 帧即红。
- **I3 输入单源**：输入只从宿主（`InputDevice` 调度 / 预览端口）进，**无 Lua 写入口**——Mod 伪造输入 = 破坏回放证据链，结构性缺席。
- **I4 帧是沿的粒度**：快照 = 帧初/帧末按住集差集；帧内同键 down+up 净零，两侧都不出现；单事件内同键 down+up 属歧义（帧内序无定义），注入时点即拒绝。
- **I5 循环即停**：帧派发失败（VALIDATION / 信封错误）循环终止，无「跳帧续跑」；失败帧在 rt 面上不留残渣。
- **I6 录制即可回放**：截断录制（错误帧之前的部分）回放到同位同红；回放不产生录制副作用。
- **I7 渲染零混色直写**：clear/span/blit 全部直写覆盖；越界静默钳制；全整数参数（浮点 = `YGG-RT-SHAPE`）。混色属后续轮次裁决，最小面不预支。
- **I8 帧 digest 不入链**：`FrameRecord.frame` 是旁路证据线——链公式 I2 的输入元组零变化；回放对 frame 字段**严格对表**，与 I2 链哈希比对并列成双闸。失败帧不留半帧证据。
- **I9 混音 digest 不入链**：`FrameRecord.audio` 同为旁路证据线（I8 同构）；混音挂在 dispatch **成功后**（失败帧不留半帧混音证据）；音频面是宿主旁路状态，天然不入 `state_digest`。

## 4. API 面（`ygg.rt.*`）

| 键 | 相位 | 语义 |
| --- | --- | --- |
| `ygg.rt.config` | **仅装载期** | `config({hz=?, maxCatchup=?})`；seal 后原位替换为 `YGG-RT-SEALED` 报错桩；hz∈1..=1000，maxCatchup∈1..=64 |
| `ygg.rt.loop.info` | 运行期只读 | `→ {mode, n, hz, maxCatchup, logicalMs}`；mode ∈ `run`/`rt` |
| `ygg.rt.input.keys` | 运行期只读 | `→ {held=位串, pressed={…}, released={…}}`（升序；与规范串 k= 段同源） |
| `ygg.rt.input.mouse` | 运行期只读 | `→ {dx, dy, buttons}` |
| `ygg.rt.input.time` | 运行期只读 | `→ 逻辑 ms`（逐帧 dt 累加，含本帧） |
| `ygg.rt.frame.open` | **仅装载期** | `open({w,h}) → handle`（v1 单面，再开 `YGG-RT-OPENED`）；w,h∈1..=4096 |
| `ygg.rt.frame.clear` | 运行期写 | `clear(h, rgba)` 全缓冲填色；脏区 = 全幅 |
| `ygg.rt.frame.span` | 运行期写 | `span(h,x,y,w,hh,rgba)` 实心矩形；x/y 可负、越界钳制；负 w/hh = `YGG-RT-SHAPE` |
| `ygg.rt.frame.blit` | 运行期写 | `blit(h,x,y,w,bytes)` bytes = RGBA8 行串（len 必须 w×4 正整数倍） |
| `ygg.rt.frame.present` | 运行期写 | `present(h) → {digest, dirty, frame}`；同帧多次合法、末次为准 |
| `ygg.rt.audio.open` | **仅装载期** | `open({rate=?}) → handle`（v1 单混音面，再开 `YGG-RT-OPENED`）；ch 固定 1 |
| `ygg.rt.audio.play` | 运行期写 | `play(h, pcm, gain?) → voice`；并发 ≤64；play→混入 = 0 帧 |
| `ygg.rt.audio.stop` | 运行期写 | `stop(h, v)` 即刻静默（帧内生效）；未知 voice = no-op Ok |

配置合流优先级：**CLI > Mod 装载期声明 > 默认**。run 模式下 `ygg.rt.*` 照常注册（表始终在）：input 读到中性快照、`loop.info` 报 `mode=run`——单一代码路径，不静默降级。

**`examples/rt` CLI**：`<mod目录> [--frames N|--duration S] --record <路径> | --replay <路径>` + `--hz/--max-catchup/--event/--input/--boot/--ppm/--preview/--audio`（传 `--fuel` = usage error exit 2，燃料已退役）。stdout 单 JSON 摘要（`ok/mode/frames/fpsDispatch/droppedFrames/hashHead/firstMismatch`）；退出码 0/1/2。`--frames` 不限速（吞吐口径）、`--duration` 按 hz 实时节拍、`--event` 可指到既有 run 模式事件（兼容桥）。

**预览端口线程纪律**：accept 线程只收字节（mpsc 单向），帧快照由主循环发布（Mutex 槽），零 unsafe、零 Rc 过线程、零新增依赖。

## 5. 确定性契约（声明收窄）

只证「**同 seed + 同录制 → 逐帧同哈希链**」，不证「两次直播同哈希」（直播输入到达本质非确定）。内核 seed 是编译期常量（`inject::KERNEL_RNG_SEED`），不进录制文件。证伪语义 = `first_mismatch`（仿 ygg-sim ReplayLog）：篡改任一 dt / 键序 / 配置 → 恰在被篡帧红（`rt_replay_catches_tampered_*` 三例）。音频面同域：录制→回放链全等 ∧ audio 字段全等 ∧ 两次重放 WAV 字节全等。

## 6. 性能预算（实测，方法在列）

| 口径 | 数值 | 方法 |
| --- | --- | --- |
| fps_dispatch（Saltline 同 mod 同事件桥） | **155.4**（120 帧 `game.tick` 全回合，不限速，含 boot） | `examples/rt ~/WorkHub/Saltline-YGG --boot game.boot --event game.tick --frames 120 --record …`（dev 构建） |
| 基线（每帧一进程） | 35.9 | 同文档 A5·P3 口径 |
| 10s 实时录制 / 回放 | 600 帧 @ 59.9 fps 零弃帧；回放 600/600 链全等（3.9s） | `--duration 10 --record` → `--replay` |
| 轻载 mod 吞吐上界 | 7858 fps | `--frames 60`（smoke mod 单帧仅 KV 累加） |
| 渲染面 host 上界 | **52364 千像素/s**（60 帧 320×200 全帧 span 化 73.3ms） | `cargo test -p ygg-sandbox --release --test frame_face frame_throughput` |
| 渲染面端到端 fps_dispatch | **780.5–786.4**（release；dev 80.8–84.2 亦达标） | `examples/rt <320×200 fixture> --frames 60 --record` |
| 帧 digest 成本 | 全幅 256KB SHA-256 含于上界（1.2ms/帧全含） | 同上 |
| 混音成本 | 64 voice 满载 × 600 帧（dev）< 0.5ms/帧登记停阈值 | `cargo test -p ygg-sandbox --test audio_face audio_mix_latency` |
| Leg B 实弹一局 | 1800 帧（30s）：链全等 ∧ audio 字段全等 ∧ 两次重放 WAV 字节全等；30s 恰 661500 样本 | `examples/rt tests/fixtures/rt-audio-demo --frames 1800 --record --audio <wav> --input <jsonl>` |

预算声明：每帧非 Lua 宿主开销 = 1 次 dispatch（转换+信封）+ 1 次 save 信封序列化 + 2 次 SHA-256；渲染帧另加全幅 digest + 输出快照；音频帧另加帧末混音。余量 ≥ 13×（780/60）。CI 只设灾难下界（500ms/2000ms 等），不卡阈值——阈值变红属回归信号，改阈值不算修复。

## 7. 设计依据

- **D1 宿主形态**：常驻 example 而非 egg 新模式——`egg.rs` 是宿主观察面（症状+复现上报，绝不 patch），playtest/serve 已立「逻辑在 src、example 薄壳」先例。
- **D2 输入 = 轮询快照而非回调**：确定性（每帧一条规范记录、无到达序竞争）+ 每帧一次入 VM + 与批式铁律同构。
- **D3 定步 dt**：dt = floor 差分序列使「dt 入载荷 = 入状态」与「实时」同时成立；整数 dt 使 mod 定点积分无舍入漂移。
- **D4 双层证据**：帧链哈希（宿主级：输入/调度/全状态）∧ mod 级 canonical state（每帧 save 摘要入链）。
- **D5 单点收口**：`frame_payload` 唯一拼串点；`canonical/parse` 互逆且规范化自检——两处拼串或宽松解析都是回放失效的根因。
- **D6 不引 serde derive**：FrameRecord 手写 JSON，依赖白名单零新增。
- **D7 `--event` 兼容桥**：rt 可驱动任何已注册事件（缺省 `rt.tick`）——run 模式 mod 无需改造即可进 rt 循环。
- **D8 catch-up 弃账**：补帧封顶 `max_catchup`、超出弃账（含零头清空）——防螺旋死亡；弃帧数只入回执元数据。
- **D9 帧/混音 digest 不入链**：位图与音频证据走独立字段线，与 D4 双层证据并列；回放严格对表使「Mod 渲染/音频行为漂移」可证伪。
- **D10 预览端口内嵌而非挂 serve**：serve 契约 = 零内核不连运行进程；推帧数据源在 rt 进程，挂 serve 须先造 serve↔rt IPC 桥 = 破坏观察面架构。
- **D11 五键闭集零混色**：host 不持渲染语义（调色板/精灵语义留 Mod 侧）；混色属后续轮次裁决。
- **D12 三键最小面 + 拒 cpal**：见 §2.5；人耳在环需求若出现再启白名单论证。
- **D13 帧同步混音 + 静音窗口照写**：证据缺席 ≠ 时间缺席（`audio` 键整键缺席，WAV 流时长不失真）。

## 8. Saltline 改造点落地状态

host 侧八项**全部就绪**（① rt 输入映射 ② dt 定点积分 ③ cast→span+调色板 ④ sprites→RGBA ⑤ compose→present+hud ⑥ golden 双轨 ⑦ weather 不动 ⑧ 武器/天气挂 `audio.play`），Saltline 侧除 ⑦（本轮回归即验）外均待其自身改造。真 mod 实弹的结构性障碍如实声明：`Saltline-YGG/mod.lua:601` 解析链非 JSON 非数字 → index=1，unmodified 下不可选动作 → 射击不可达；unmodified 双腿 smoke 已钉（桥回归 + fixture 实弹）。
