# Yggdrasill Godot 客户端

M2.5 Godot 底座正式工程：把 `crates/ygg-core` + `crates/ygg-sandbox`（内核与 Lua
沙箱）经 gdext **原生链接**进 Godot 4，跑通《灯下谜径》完整可玩垂直切片（开局 →
选择 → 结局，含存档/读档）。

`spike/godot/` 是可行性实证的历史档案（**不进 cargo workspace，也不再演进**）；
本目录是其成果的正式化：命名/结构去 spike 化、UI 补齐、走查与存读档纳入门禁。

## 布局

```text
clients/godot/
├── rust/                         # gdext 绑定 crate（独立 workspace）
│   ├── Cargo.toml                # godot 0.5.5 + api-4-7；path 依赖 ../../../crates/ygg-sandbox
│   └── src/
│       ├── lib.rs                # YggKernel 绑定类（装载/派发/存读档/getter）
│       └── logic.rs              # 纯 Rust 逻辑面（Mod 装载、信封拆解、存档槽拆分）+ 原生测试
└── project/                      # Godot 工程（可玩切片本体）
    ├── project.godot
    ├── main.tscn                 # 面板布局：标题栏 / 叙事区 / 选择区 / 状态栏
    ├── main.gd                   # UI 接线 + 脚本驱动走查 + 存读档槽
    ├── session.gd                # 会话状态机（叙事累积 / 结局收口；镜像 client-web/session.ts）
    ├── theme_factory.gd          # 暗暖色主题（配色/字号唯一来源，代码构建 Theme）
    └── bin/ygg_godot.gdextension # 绑定描述（dylib 产物不入 git）
```

## 环境约定

- Godot **4.7.2-stable**（macOS arm64），仓库外路径 `~/tmp/godot/Godot.app`；
  CLI 即 `~/tmp/godot/Godot.app/Contents/MacOS/Godot`。
- Rust **1.99.1**；gdext `godot = 0.5.5`，feature `api-4-7`（与工程
  `config/features` 的 4.7 同档；升级 Godot 需同步改 feature）。
- 内核是 **path 依赖**（`../../../crates/ygg-sandbox`）：`crates/` 只读引用，
  本客户端不改内核一个字节。

## 构建

```bash
cd clients/godot/rust
cargo build
mkdir -p ../project/bin
cp target/debug/libygg_godot.dylib ../project/bin/libygg_godot.macos.template_debug.dylib
```

### 首次构建 / 清理后必须先跑一次编辑器导入扫描

```bash
~/tmp/godot/Godot.app/Contents/MacOS/Godot --headless --editor --path clients/godot/project --quit
```

原因：gdext 扩展类的**全局类注册**与 `class_name` 全局类都需要工程导入缓存
（`.godot/extension_list.cfg`、`.godot/global_script_class_cache.cfg`）才可见。
缓存缺失时直接 `--path` 运行会报
`Parse Error: Could not find type "YggKernel"`（`YggSession` 同理）。
`.godot/` 不入 git（见 `.gitignore`），所以**新克隆的仓库第一次运行前必须做这一步**。

## 运行

```bash
GODOT=~/tmp/godot/Godot.app/Contents/MacOS/Godot
MOD=<仓库根>/packages/mods/test-adventure

# ① headless 通关走查（每步选择由 --choices 驱动）
"$GODOT" --headless --path clients/godot/project -- \
  --mod-dir="$MOD" --choices=1,1 --auto-quit=1

# ② 窗口运行（人工点选；键 1-9 也可选；工具栏：存档 / 读档 / 状态 / 重开）
"$GODOT" --path clients/godot/project -- --mod-dir="$MOD"

# ③ 逐帧截屏（窗口运行；产出 <prefix>-NN.png）
"$GODOT" --path clients/godot/project -- --choices=1,1 --shots=/tmp/ygg-run --auto-quit=1
```

### 自动化调用纪律（防挂死）

所有脚本化 Godot 调用**必须带看门狗**（GDScript 解析错误会让进程无输出挂死，
auto-quit 依赖脚本成功执行）：

```bash
"$GODOT" --headless --path clients/godot/project -- --choices=1,1 --auto-quit=1 &
GPID=$!; sleep 40
kill -0 $GPID 2>/dev/null && kill -9 $GPID   # 超时即杀，先查日志再重试
```

### 参数表（须写在命令行 `--` 之后）

| 参数 | 含义 |
| --- | --- |
| `--mod-dir=/abs/path` | Mod 目录（缺省：`YGG_MOD_DIR` → 仓库内 `packages/mods/test-adventure`） |
| `--choices=1,1` | 脚本驱动的选择序列（队列语义：从当前待决点起依次消费） |
| `--save-after=N` | 应用第 N 个选择后自动存档到 `--save-file` |
| `--save-file=/abs/path` | 存档槽路径（工具栏「存档/读档」缺省也用它；默认 `user://ygg-save.json`） |
| `--load-file=/abs/path` | 启动即读档（替代 `event.start`） |
| `--shots=/abs/prefix` | 每次渲染后截屏（headless 自动跳过） |
| `--auto-quit=1` | 选择序列耗尽后退出（退出码 0 = 无错误条目） |

**看门狗纪律**：Godot 进程可能挂死（窗口/渲染初始化），批处理里一律带超时：

```bash
run_godot() { "$GODOT" "$@" > /tmp/godot.log 2>&1 & local p=$!; \
  ( sleep 60; kill -0 $p 2>/dev/null && kill -9 $p ) & local w=$!; \
  wait $p; local e=$?; kill $w 2>/dev/null; return $e; }
```

## 存档槽格式

存档 = **内核快照 + 客户端叙事日志**（内核不存历史叙事，日志是表现层状态）：

```json
{
  "kind": "ygg.godot.save",
  "version": 1,
  "steps": 1,
  "ended": false,
  "kernel": { "engine_version": "0.1.0", "entities": { }, "kv": { }, "pending_choice": { } },
  "log": [ { "kind": "narrative", "text": "序章·雨夜：…" } ]
}
```

两条硬约束（都有实证）：

1. **`kernel` 段按原文拼接，不经 Godot 的 JSON 类型往返**。Godot 的
   `JSON.parse_string` 把所有 JSON 数字解析成 **float**，`6` 会变成 `6.0`；存档面
   有 u64 字段（`revision` / `next_id` / `format_version`），喂回内核会被拒：
   `load rejected: invalid type: floating point 1.0, expected u64`。
   读档时槽文本整体交给绑定 `load_bundle()`，由 **serde** 在 Rust 侧分离
   `kernel` 段（serde 保留 int/float 区分），日志段仍由 GDScript 解析（只含字符串）。
2. **待决选择随内核快照往返**（`pending_choice`），读档后无需重放事件即可恢复
   选择面；叙事日志由槽内 `log` 段恢复（读档后追加一行系统回执）。

## 门禁

```bash
# ① 绑定编译 + 原生测试（不经 Godot：装载 → event.start → 选择 → 结局 A → 存读档指纹一致）
cd clients/godot/rust && cargo build && cargo test

# ② headless 通关走查（结局 A + 退出码 0）
"$GODOT" --headless --path clients/godot/project -- --mod-dir="$MOD" --choices=1,1 --auto-quit=1

# ③ 存读档：中途存档 → 新进程读档 → 状态指纹一致 → 继续通关
"$GODOT" --headless --path clients/godot/project -- --mod-dir="$MOD" --choices=1 --save-after=1 --save-file=/tmp/slot.json --auto-quit=1
"$GODOT" --headless --path clients/godot/project -- --mod-dir="$MOD" --load-file=/tmp/slot.json --choices=1 --auto-quit=1

# ④ 主线零影响（本目录是独立 workspace，不进主 workspace 依赖图）
cd <仓库根> && cargo test --workspace
```

## 与 client-web 的关系

两个表现层共用同一份**内核信封契约**（`dispatch` JSON 信封 + 结局 `【结局` 前缀）：

| 面 | client-web（`packages/client-web`） | 本客户端 |
| --- | --- | --- |
| 内核形态 | wasm 绑定（wasm-bindgen） | 原生链接（gdext） |
| 会话语义 | `session.ts` | `session.gd`（语义镜像） |
| 存档载体 | `localStorage["ygg-save"]` | 存档槽 JSON 文件（`kernel` + `log`） |

行为口径一致：错误信封**保留**待决选择（可重试）、结局行出现即收口并追加
「—— 冒险结束 ——」、存档/状态类回执只加系统行不重放叙事。
