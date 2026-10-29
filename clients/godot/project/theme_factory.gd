extends RefCounted
## 面板视觉：暗暖色主题（配色 / 字号 / 圆角的唯一来源）。
##
## 用代码构建 `Theme` 而不是 .tres：配色与字号随面板一起演进，避免手写
## 资源文本与引擎序列化格式耦合；`build()` 返回的 Theme 由 main.gd 挂在
## 根 Control 上，整棵子树继承。

const COLOR_TEXT := Color("#e8ddd0")
const COLOR_TEXT_DIM := Color("#9a8b7c")
const COLOR_ACCENT := Color("#d9a253")
const COLOR_ACCENT_SOFT := Color("#3a2e24")
const COLOR_PANEL_FILL := Color("#231c18")
const COLOR_PANEL_BORDER := Color("#453629")
const COLOR_SEPARATOR := Color("#3a2e26")
const COLOR_ENDING := Color("#f2c46b")
const COLOR_WARNING := Color("#e0a33c")
const COLOR_ERROR := Color("#e0654b")
const COLOR_SYSTEM := Color("#8a97a5")


static func build() -> Theme:
	var theme := Theme.new()
	theme.default_font_size = 17

	# ── Label ────────────────────────────────────────────────────────────
	theme.set_color("font_color", "Label", COLOR_TEXT)

	# ── Button：选择按钮与工具栏按钮共用（内容高亮靠 hover/pressed 区分） ──
	theme.set_stylebox("normal", "Button", _box(COLOR_ACCENT_SOFT, COLOR_PANEL_BORDER, 8, 12))
	theme.set_stylebox("hover", "Button", _box(Color("#4a392a"), COLOR_ACCENT, 8, 12))
	theme.set_stylebox("pressed", "Button", _box(Color("#5a4530"), COLOR_ACCENT, 8, 12))
	theme.set_stylebox("focus", "Button", _box(Color("#00000000"), COLOR_ACCENT, 8, 12))
	theme.set_stylebox("disabled", "Button", _box(Color("#1e1916"), COLOR_SEPARATOR, 8, 12))
	theme.set_color("font_color", "Button", COLOR_TEXT)
	theme.set_color("font_hover_color", "Button", COLOR_ACCENT)
	theme.set_color("font_pressed_color", "Button", COLOR_ACCENT)
	theme.set_color("font_focus_color", "Button", COLOR_ACCENT)
	theme.set_color("font_disabled_color", "Button", COLOR_TEXT_DIM)
	theme.set_font_size("font_size", "Button", 18)

	# ── PanelContainer：叙事日志底板 ─────────────────────────────────────
	theme.set_stylebox("panel", "PanelContainer", _box(COLOR_PANEL_FILL, COLOR_PANEL_BORDER, 10, 0))

	# ── RichTextLabel：叙事正文（行距放宽，中文段落更易读） ───────────────
	theme.set_font_size("normal_font_size", "RichTextLabel", 18)
	theme.set_constant("line_separation", "RichTextLabel", 7)
	theme.set_color("default_color", "RichTextLabel", COLOR_TEXT)

	# ── HSeparator / ScrollBar 等零碎控件 ────────────────────────────────
	theme.set_stylebox("separator", "HSeparator", _line(COLOR_SEPARATOR))
	theme.set_constant("separation", "HSeparator", 1)
	return theme


## 圆角底板（bg 半透明可传 `#00000000` 表示只描边）。
static func _box(fill: Color, border: Color, radius: int, margin: int) -> StyleBoxFlat:
	var box := StyleBoxFlat.new()
	box.bg_color = fill
	box.border_color = border
	box.set_border_width_all(1)
	box.set_corner_radius_all(radius)
	box.set_content_margin_all(margin)
	box.anti_aliasing = true
	return box


## 1px 分隔线。
static func _line(color: Color) -> StyleBoxLine:
	var line := StyleBoxLine.new()
	line.color = color
	line.thickness = 1
	return line
