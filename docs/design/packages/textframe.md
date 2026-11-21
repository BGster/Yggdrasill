# Yggdrasill · packages/textframe — 帧文本（字符网格 → 分层合成 → 叙事行）

> **id** packages/textframe · **模块** ⑤ packages · 通用能力层 · **触发** 改帧缓冲形状、分层语义或行装配口径之前
> **读不到代价** irreversible（行宽与尾空格口径漂一字节，已交付的 golden 帧与帧摘要全部失配；而「两边一起变」的 `cmp` 比对会静默假绿）

> **上游**：[../README.md](../README.md) · **相关**：[README.md](README.md)（模板与命名规则）· [../kernel/lua-api.md](../kernel/lua-api.md)（`narrate` 是单行接口 + 沙箱 globals 白名单）· [codec.md](codec.md)（帧摘要与留存走它的 `sha256`）

## 1. 边界（这个包不做什么）

- **不决定画什么**：地形 / 实体的字形映射、谁压谁（DCSS 的 `mon > item > player`）、HUD 文案全是**数据与规则**，归 mod。本包只认「一格一个字符」。
- **不做颜色 / 属性格**：宿主叙事面只吃一个字符串（`crates/ygg-sandbox/src/host_api.rs` 的 `move |_, line: String|`），颜色靠字形区分（Saltline `src/view/cast.lua` 的 6 档字形集）。属性格没有消费者。
- **不折行、不做宽字符**：四个复现的网格都是**定宽 + 截断/补齐**，没有一处在包里做词级折行；`wcwidth` / 全角双列 / 组合字符一律不做（§8 反方案一）。
- **不测 HUD / 状态行的宽度**：`stack` 只搬运行数组。Saltline 的 HUD 行含中文、DCSS 的状态行是 UTF-8 文本，它们**原样穿过**本包——包内一次也不数多字节字符。
- **不做摘要**：帧摘要归 [codec.md](codec.md) 的 `sha256`。第二份哈希就是分叉起点（同 [assetgen.md](assetgen.md) §1 的纪律）。
- **不落盘、不发事件、不写 KV**：产物是返回值。
- **不迭代 `pairs`**：顺序只来自数值下标（I2）。

## 2. 数据形状

**帧缓冲（本包唯一结构）**

| 字段 | 类型 | 必填 | 缺省 | 持久化位置 |
| --- | --- | --- | --- | --- |
| `w` | integer ≥ 1 | 是 | 无 | 返回值；列数 |
| `h` | integer ≥ 1 | 是 | 无 | 返回值；行数 |
| `cells` | 表（前缀稠密，可断） | 是 | `{}` | 返回值；**行主序 1 基**：`cells[(y-1)*w + x]` |

- 单元格只有三种取值：**单字节字符串**（`"\x20"`..`"\x7e"` 起的可见字符）、`nil`、`""`。
- **空白三态**（写侧严格、读侧宽容）：
  - `nil` 与 `""` = 空白且**透明**——`blit` 不覆盖、`to_lines` 出 `" "`；
  - `" "` = **实字空格**——`blit` 会覆盖。
  - 三处事实：Saltline `src/view/frame.lua` 的 `buf.cells[base + col] or " "`；CDDA `src/view/frame.lua` 的稀疏 `layer`（未写的下标即透明）；DCSS `src/view/screen.lua` 的 `if ch == "" then ch = unseen`。
- **行数组**：稠密 `array of string`。`to_lines` 产出 `h` 行，`stack` 拼接；**每行恰 `w` 个字符、不 trim 尾空格**（I6）。

**持久化位置**：全部**不落 KV**。帧要跨局留存，由调用方把行数组按 `"\n"` 连接后交给 [codec.md](codec.md)——包不表态、不缓存。

## 3. 不变量

- **I1 纯**：`frame` / `to_lines` / `stack` / `perf_keys` 只返回新表；`put` / `blit` 只写**传入的** `dst`（调用方交出所有权）。包内无 `ygg.*`，无 `math` / `table` / `string` / `os` / `io`——沙箱白名单里它们全是 `nil`（[../kernel/lua-api.md](../kernel/lua-api.md) §3.1）。
- **I2 确定性**：遍历只用数值 `for`；同一输入序列产出逐字符相同的行数组。禁 `pairs`（`next` 顺序是实现的自由）、禁时间、禁随机。
- **I3 空白三态**：见 §2。**两个方向都要有断言**——`nil` / `""` 透明（不覆盖），`" "` 覆盖。
- **I4 裁剪是全的**：`blit` 永不抛——`src` 越出 `dst` 的部分整块丢弃，负偏移同理。`put` 越界是 `-RANGE`：单格写越界是调用方 bug，块级叠加踩边不是。
- **I5 单元格 = 恰 1 字节**：多字节、空串、多字符、非字符串 → `-CELL`。清格用 `nil`（用 `""` 清格是对宽度的谎报）。
- **I6 行宽恒等**：`to_lines` 返回 `h` 行、每行 `#line == w`（ASCII ⇒ 字符数 == 字节数）、**不 trim 尾空格**。改这条等于改帧摘要——它是与 mod 侧 golden 的唯一接缝。
- **I7 顶层纯度**：`init.lua` 顶层只有 `local` / `require` / 函数与常量定义；`install()` 幂等、无参、不建模块级可变状态（同 [../mod/mod-package.md](../mod/mod-package.md) 的 `mod.pure-export`）。
- **I8 零依赖**：`pkg.json` 无 `deps`；`require` 只指向本包子模块。拼接器是本包内的一个 `local function`——不借 `pkg.assetgen.join`（那会凭空造一条反向依赖边，而「每包自带这几行」是 codec / quest / entity / dialogue / assetgen 的既定先例）。

## 4. API 面

| 函数 | 签名 | 返回 | 副作用类别 |
| --- | --- | --- | --- |
| `textframe.install` | `(deps?)` | 包表 | 写（装配期，幂等）。夹具 `PACKAGES[i].install()` 无条件调用，**必须导出** |
| `textframe.frame` | `(w, h)` | 帧缓冲 `{ w, h, cells = {} }` | 纯（新表）；`-PARAM` |
| `textframe.put` | `(buf, x, y, ch)` | `buf` | 写：单格；`ch = nil` 清格；`-RANGE` / `-CELL` / `-SHAPE` |
| `textframe.blit` | `(dst, src, ox, oy)` | `dst` | 写：`src` 的清格跳过、其余覆盖 `dst`（后写胜），越界裁剪 |
| `textframe.to_lines` | `(buf)` | `h` 行数组 | 纯（新表；每行恰 `w` 字符、不 trim）；行宽 ≠ `w` ⇒ `-CELL` |
| `textframe.stack` | `(blocks)` | 行数组 | 纯（块序 = 行序；`nil` 块跳过） |
| `textframe.perf_keys` | `()` | 9 个键名的稠密数组 | 纯（自省面：自测按它逐个打印，冒烟按它逐个核对——「九键齐备」因此是机检） |
| `textframe.ERRORS` | — | 4 键表 | 常量 |

`textframe.ERRORS` 恰好四键，全部形如 `YGG-PKG-TEXTFRAME-*`：`PARAM`（`w` / `h` 不是 ≥1 的整数）、`RANGE`（`put` 坐标越界）、`CELL`（单元格不是恰 1 字节）、`SHAPE`（传入的不是帧缓冲 / 行数组）。与 [entity.md](entity.md) 的 `YGG-PKG-ENTITY-*`、[assetgen.md](assetgen.md) 的五码同风格。

复杂度形状：`put` = O(1)；`blit` = O(`src` 的非清格数)；`to_lines` = O(`w·h`)（唯一一次全扫，省不掉）；`stack` = O(总行数)。

## 5. 性能预算

**口径声明（先读再读数字）**：数字产在 **selftest 面**（普通 Lua，全标准库）——沙箱里既没有 `os.clock` 也没有 `collectgarbage`，计时与内存只在测试侧可测。三次取中位，`print` 面即口径（同 [assetgen.md](assetgen.md) §5 的方法论）。**基准集固定**：`80×24` 世界区 + `6` 行 HUD = 30 行帧（= Saltline 的 `TUNING.world.view_w/view_h` + HUD 六行，与 r4 适配器同形的输入）。尺寸是定义的输入，不是可调旋钮。

**两个数字面（这是本表最容易读错的地方）**：门禁命令是 `cargo run -q --example selftest`（**debug**），生产跑的是 release（Saltline 的 `t-saltline-perf` 就是 release）。同一用例两面差约 3.7×。所以**绝对帽按慢的那面（debug）标定**，两列都记：只记 release 会让 debug 门禁每跑必红，只记 debug 又会让「10k 帧/秒」这句话失去意义。

| 指标 | 阈值 | debug（门禁面） | release（生产面） | 测量方法 / 打印键 |
| --- | --- | --- | --- | --- |
| 包加载耗时 | `< 10 ms` | **0.28 ms** | **0.16 ms** | 清 `package.loaded["pkg.textframe*"]` 后 `require` + `install`，5 次取中位 → `perf.textframe.load_ms` |
| 单格写 | `O(1)`，`< 5 µs` | **1.12 µs** | **0.31 µs** | 10 万次 `put` 计时（`put` 内无整表扫描，是结构性的 O(1)）→ `perf.textframe.put_ns` |
| 整帧 blit | `≤ 500 µs` | **115 µs** | **32 µs** | 一层 80×24 稀疏覆盖层（每 20 格一个字，同 Saltline `sprites.lua` 的实况）叠到世界区上，2000 次 × 3 中位 → `perf.textframe.blit_us` |
| 帧合成耗时 | `≤ 1000 µs` | **350 µs**（348–363） | **95–99 µs** | `to_lines` + `stack` 跑 2000 帧 × 3 次取中位；区间是跨次运行的中位波动 → `perf.textframe.compose_us` |
| 帧合成吞吐 | 给数字 | **2 850 帧/秒** | **10 300–10 500 帧/秒** | `1000 / compose_ms` → `perf.textframe.throughput_fps` |
| vs 手搓对照 | `≤ 2.0×` | **1.64×** | **1.30×** | 同面跑 naive `out = out .. ch` 版 → `perf.textframe.vs_naive` |
| 每帧常驻内存 | `≤ 8 KiB` | **3.04 KiB** | **3.04 KiB** | 100 帧常驻后**整收一次**再取 `collectgarbage("count")` 差 ÷ 100 → `perf.textframe.frame_kib`（S-24 改法，见 §8 末条） |
| 泄漏检测 | `≤ 1%` 且逐字符相等 | **0.62–0.64% / 1** | **0.62–0.64% / 1** | 100 轮同输入 `compose`：内存差占比 + 每轮与第 1 轮逐字符相等 → `perf.textframe.leak_gc_pct` / `perf.textframe.leak_100_equal` |

**对照（需求点名要的那个数字，如实记）**：本包在 `w = 80` 上**没有**赢过手搓的 `out = out .. ch`（debug 1.64× / release 1.30×）。这个宽度下二次项的常数还没长出来，而两两合并要多付一张 `parts` 表的存取。它的价值在**增长性**：`w` 翻倍时 naive 是 O(n²) 的四倍退化，两两合并是 O(n·log n)。所以阈值定 2×，抓的是「拼接路径退化」，不是「比手搓快」——后者在本尺寸下本来就不成立。

**一次被实测推翻的实现选择（留痕）**：`to_lines` 起初逐格断言 `type(ch) == "string" and #ch == 1`（I5 的读侧副本，1920 次/帧），release 面实测吃掉约 45% 的合成时间（132 µs → 去掉后 93 µs）。改成**每行拼完断言一次 `#line == w`**：24 次/帧，守的还是 I6——一格多字节必然让 `#line > w`；一格数字会被 `..` 拼成多个字节，同样被 `#line` 抓住。

> **实现纪律**：目标值在首个实现提交后就地改成实测值。本表已由 S6 首实现回写；C1 时记的 `参考值`（`/tmp` scratch、standalone Lua 5.5：77.3 µs/帧、3.4 KiB/帧）**偏乐观**——那份 scratch 没有 `stack` 的表分配，也不是 mlua 的 Lua 5.4。偏差如实登记于此，回归阈取实测的 2×。

## 6. 用点计划

| 复现项目 | 用点 | 该包提供什么 | 阶段 |
| --- | --- | --- | --- |
| **Saltline（单文件 3D FPS，回执文本帧）** | `src/view/frame.lua` 的 `compose(buf, hud_rows)`——唯一装配点，消费方：`mod.lua`、`tests/t-render.lua`、`tests/t-saltline-golden.lua`、`tests/t-saltline-perf.lua` | `to_lines`（世界区 → 24 行）+ `stack`（拼 6 行 HUD）。替换 r4 交付的 ≤40 行临时适配器 | **S6（实证替换）** |
| **CDDA（字符渲染，JSON 数据驱动）** | `src/view/frame.lua` 的 `map_lines`（24×24 地图 + 稀疏实体 overlay）与 `lines`（HUD 1 + 地图 24 + 日志 ≤6） | `put`（overlay 逐格写）+ `to_lines`（行装配）+ `stack`（三块拼接） | P3 后续 |
| **DCSS（字符界面，整屏重绘）** | `mod/src/view/screen.lua` 的 `render`（79×21 视口 + 状态 2 行 + 消息 1 行 = 24 行） | `put` × 优先级链（`mon > item > player` 三次叠加）+ `to_lines`（定宽 79）+ `stack`（状态/消息行） | P3 后续 |

**为什么是 3 个而不是 4 个（如实登记）**：PokeRogue 是图形化呈现（`viewer/` 只有 JS），Lua 侧只有逐行 `ygg.narrate` 与菜单编号行，**没有字符网格**——它不是本包的用点，不凑数。≥2 闸由 Saltline（实证）+ CDDA + DCSS（指认，各带 file:line）满足。

## 7. 自测与验收

- **用例名前缀**：`pkg.textframe.`；载体三个文件（同 codec / entity / quest 的先例——每个文件自带计数垫片，独立可跑）：`test/pkg-textframe.lua`（用例 1–7、14–15）、`test/pkg-textframe-compose.lua`（8–13、16–17）、`test/pkg-textframe-perf.lua`（18）。`--filter pkg-textframe` 同时命中三者。
- **进哪条命令的计数**：`cargo run -q --example selftest -p ygg-sandbox -- --mod packages/mods/pkg-selftest --packages packages/libs --filter pkg-textframe` 的 `cases passed=` = **18**；全量 = **191**（173 + 18）。
- **用例清单（18 条）**：
  1. `frame-blank`——`frame(4,3)` 三行四空格，`cells` 为空表；
  2. `put-index-1based`——`put(1,1)` → `cells[1]`，`put(4,3)` → `cells[12]`；
  3. `put-clear`——`put(...,nil)` 清格回空白；
  4. `put-range`——`x=0` / `x=w+1` / `y=0` / `y=h+1` 四向都 `-RANGE`；
  5. `put-cell`——`""` / `"ab"` / 多字节 / 非字符串 都 `-CELL`；
  6. `to-lines-width`——满格（`w` 行各写不同字符）时 `#line == w`、行序 = 行主序；
  7. `to-lines-blank`——`nil` 与 `""` 都出 `" "`；
  8. `blit-overwrite`——后写胜（覆盖 `dst` 已有字符）；
  9. `blit-transparent`——`nil` / `""` 不覆盖；`" "` 覆盖（正反两向）；
  10. `blit-clip`——四角负偏移 / 越界偏移只画重叠区，不抛；
  11. `blit-chain`——三层叠加顺序 = DCSS 优先级链（`terrain < item < mon < player`）；
  12. `stack-order`——`stack({a,b,c})` 行序 = 块序，`nil` 块跳过；
  13. `stack-empty`——`{}` / 全 `nil` → 空数组（不返回 `nil`）；
  14. `shape-errors`——`w=0` / `w=1.5` / `h="3"` / `cells` 非表 → `-PARAM` / `-SHAPE`；
  15. `exports-shape`——导出面恰好 8 键 + `ERRORS` 恰好 4 码；
  16. `oracle-saltline-frame`——把 r4 适配器的逻辑内联成 oracle，80×24 + 6 HUD 行 → 30 行，与 `to_lines`+`stack` 逐字符相等；
  17. `purity-determinism`——100 轮同输入同输出、无全局写、无 `package.loaded` 改动；
  18. `perf`——按 `perf_keys()` 逐个打印 §5 的九个键并断言阈值（含 `blit_us` 与 `vs_naive` 对照比）。
- **mod 侧载体冒烟不进计数**（file 级 `error` ⇒ `file FAIL` ⇒ `ok=0`）：`packages/mods/pkg-selftest/tests/pkg-textframe-smoke.lua` 钉「装载面 + 顶层纯度 + `ERRORS` 码集 + 导出面清单 + 九个 `perf.textframe.*` 键名 + 空白帧行宽不 trim」。
- **为什么必须有 mod 侧同名文件**：`--filter` 对 mod 的 `tests/*.lua` 与包的 `test/*.lua` **同时**过滤，mod 侧筛完为空即「没有可执行的测试文件」exit 1（[README.md](README.md) §5）。
- **禁止宽松断言**：只断「两次长度相同」不逐字符 `==`，视同没测 I6；只断「`blit` 不报错」不对最终字符，视同没测 I3；把 oracle 从被测算式里 import 进去（自证）视同没测。

## 8. 设计依据

**为什么这是包。** 四个复现里三处各写了一份「字符缓冲 → 行数组」，且形状逐字段相同：`{w, h, cells}` 行主序 + 稀疏覆盖 + 定宽行。每处自造一遍就是 Goal 反例 2 的「复现里自造基建」——不是复现写得差，是包没覆盖到位。而它**不能**进内核：整件事只需要整数下标与字符串拼接，没有 I/O、没有事件，落进 `crates/**` 只会让「改一格空白语义」变成重新编译。

**为什么 API 恰好这五个原语。** 每个都有 ≥2 处事实来源：`to_lines`（Saltline `compose` 的行循环 / CDDA `map_lines` 的 `util.join` / DCSS `pad_right(concat(parts))`）、`stack`（Saltline 的 HUD 追加 / CDDA 的 HUD+地图+日志）、`put`（CDDA `layer[i] = glyph` / DCSS `parts[sx] = ch` / Saltline `cells[at] = glyph`）、`blit`（CDDA 稀疏 overlay 的「后写胜」/ Saltline `sprites.render` 覆盖 `cast.render` 的底图）、`frame`（三处的 `{}` + `w/h` 初值）。少一个就要调用方自己写回循环，多一个就有人开始往里塞游戏语义。

**反方案一：收 `wrap` / `pad` / `wcwidth`。** 四个复现都是**定宽网格 + 截断补齐**：DCSS 的 `util.upad` 是「超宽截断、不足补空格」，Saltline 的 HUD 行宽是构建期固定词条，CDDA 的行由 `grid.size` 决定。词级折行在本仓**没有一个消费者**——写进包就是把「别人可能用得上」当成需求。至于宽字符：本包连 `string` 库都不可达，数不清码位，也就无从测显示宽度（这条约束顺手把「全角占两列」的问题挡在包外，由需要它的 mod 自己解决）。
**反方案二：收 `digest` / `canon`。** 帧的变更检测已经在用 [codec.md](codec.md) 的 `sha256`（Saltline `t-saltline-perf` 的 `last_frame_digest`）。再实现一份摘要 = 第二份指纹，两处不一致时无法判定谁对。
**反方案三：收颜色 / 属性格。** 宿主叙事面只接受一个字符串（`host_api.rs` 的 `move |_, line: String|`）；属性格写进包，四个复现里没有一处能把它送到屏上——**形状没有消费者就是负担**。要上色只能是宿主面变更，那是内核能力，不是本包。
**反方案四：`require("pkg.assetgen.join")` 复用拼接器。** 十几行的两两合并，换来一条 `textframe → assetgen` 依赖边与一次跨包契约耦合；而 codec / quest / entity / dialogue / assetgen 五处都已各自持有同一份（[assetgen.md](assetgen.md) §8 已裁过同一个问题）。复制这三行是更便宜的一侧。

**判据（①②③）。** ① 阻断——把「哪个字给哪个地形」或「谁压谁」写进包；在包里折行 / 算显示宽度 / 造第二份摘要；用 `pairs` 决定行序。② 症状——同一条帧在 mod 侧和解析侧宽度差一格，`cmp` 静默假绿；`t-saltline-golden` 的 10 帧漂移；沙箱里 `attempt to index a nil value (global 'table')`。③ 确认——本文件被 [../README.md](../README.md) 索引；`--filter pkg-textframe` 的 `cases passed=18`；§5 的五个 `perf.textframe.*` 键齐备。

**S-24 实测发现的口径缺陷（测量方法修正，非阈值移动）**：原口径是「`m0` → 建 100 帧 → 读 `count`」，差值里**混入了这 100 轮还没被回收的瞬时垃圾**，而 GC 什么时候跑取决于**进入本用例时的堆大小**——`pkg-entity` 的查询面（`ygg.entity.query` 的包面自测 + 两条 perf 行）让前序堆多出约 1 MB 之后，同一份 textframe 代码从 **4.65 KiB/帧** 报到 **13.73 KiB/帧**（阈值 8），跨文件耦合打红了门禁。处置是**把差值挪到一次整收之后**（`collectgarbage()` 再读 `count`）：本条要量的是「留下 100 帧」的**活**内存，整收后瞬时垃圾不计入。改后 debug/release 两面都是 **3.04 KiB/帧**（原 4.0–4.7 里有约 1.6 KiB 是垃圾），阈值不变、口径更严。
