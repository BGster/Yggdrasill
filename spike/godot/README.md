# M2.5 Godot 底座可行性 spike

godot-rust（gdext）把 ygg-core/ygg-sandbox 原生链接进 Godot 4 的最小实证闭环。
**本目录不进 cargo workspace**（`spike/godot/rust/Cargo.toml` 用空 `[workspace]`
表隔离），不动既有 crates/packages/docs。

## 布局

```text
spike/godot/
├── rust/                    # gdext crate（独立 workspace）
│   ├── Cargo.toml           # godot 0.5.5 + api-4-7；path 依赖 ygg-sandbox
│   └── src/lib.rs           # YggKernel 绑定类 + logic 纯逻辑模块 + 原生测试
└── project/                 # 最小 Godot 4.7 项目
    ├── project.godot
    ├── main.tscn            # Status Label + Narrative RichTextLabel + Choices 按钮
    ├── main.gd              # UI 接线 + headless 自动走查
    └── bin/ygg_spike.gdextension
```

## 环境约定

- Godot 4.7.2-stable（macOS arm64 universal）：`~/tmp/godot/Godot.app`（仓库外）
- Rust 1.99.1；gdext crate `godot = 0.5.5`，feature `api-4-7`

## 构建

```bash
cd spike/godot/rust
cargo build                                   # 出 target/debug/libygg_godot_spike.dylib
mkdir -p ../project/bin
cp target/debug/libygg_godot_spike.dylib \
   ../project/bin/libygg_godot_spike.macos.template_debug.dylib
```

## 原生测试（第二证据面：不经 Godot 直调绑定逻辑）

```bash
cd spike/godot/rust
cargo test   # 装载 test-adventure → event.start → 叙事非空+选择 → 选 1 → 推进
```

## headless 走查（主证据面）

```bash
GODOT=~/tmp/godot/Godot.app/Contents/MacOS/Godot
REPO=$(cd ../../../.. && pwd)   # 本文件在 spike/godot/ 下 → REPO = worktree 根
$GODOT --headless --path spike/godot/project -- \
  --ygg-mod-dir="$REPO/packages/mods/test-adventure" --ygg-auto=1
```

预期 stdout：`[spike] load_mod ok` → `[spike] event.start ok narrative_lines=N options=[...]`
→ `[spike] choice 1 ok narrative_lines=M head=...` → `[spike] DONE ok=true`，退出码 0。

## 窗口运行（人工点选 / 截屏）

```bash
$GODOT --path spike/godot/project -- \
  --ygg-mod-dir="$REPO/packages/mods/test-adventure" --ygg-shot=/tmp/ygg_spike.png
```

点击选项按钮推进；`--ygg-shot` 在推进后自动截屏。
