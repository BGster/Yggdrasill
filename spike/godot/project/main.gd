extends Control

## M2.5 spike UI 接线：叙事显示 + 选项按钮列 + headless 自动走查。
##
## 运行参数（user args，须在 `--` 之后）：
##   --ygg-mod-dir=/abs/path   Mod 目录（缺省回退环境变量 YGG_MOD_DIR）
##   --ygg-auto=N              脚本自动点选第 N 项（headless 走查用；0 = 纯手动）
##   --ygg-shot=/abs/path.png  推进后截屏保存（窗口运行用；headless 无渲染跳过）
##
## headless 走查示例（证据面：stdout 日志 + 退出码）：
##   godot --headless --path spike/godot/project -- \
##     --ygg-mod-dir=$PWD/packages/mods/test-adventure --ygg-auto=1

var ygg: YggKernel
var mod_dir := ""
var auto_pick := 0
var shot_path := ""

@onready var status_label: Label = %Status
@onready var narrative_label: RichTextLabel = %Narrative
@onready var choices_box: VBoxContainer = %Choices


func _ready() -> void:
	ygg = YggKernel.new()
	add_child(ygg)
	for arg in OS.get_cmdline_user_args():
		if arg.begins_with("--ygg-mod-dir="):
			mod_dir = arg.get_slice("=", 1)
		elif arg.begins_with("--ygg-auto="):
			auto_pick = int(arg.get_slice("=", 1))
		elif arg.begins_with("--ygg-shot="):
			shot_path = arg.get_slice("=", 1)
	if mod_dir.is_empty():
		mod_dir = OS.get_environment("YGG_MOD_DIR")
	if mod_dir.is_empty():
		_fail("未提供 Mod 目录（--ygg-mod-dir= 或环境变量 YGG_MOD_DIR）")
		return
	var load_raw := ygg.load_mod(mod_dir)
	var load_reply: Variant = JSON.parse_string(load_raw)
	if load_reply == null or not bool(load_reply.get("ok", false)):
		_fail("load_mod 失败：%s" % load_raw)
		return
	print("[spike] load_mod ok  dir=", mod_dir)
	var start_raw := ygg.dispatch("event.start", "0")
	var start: Variant = JSON.parse_string(start_raw)
	if start == null or not bool(start.get("ok", false)):
		_fail("event.start 失败：%s" % start_raw)
		return
	_render()
	print("[spike] event.start ok  narrative_lines=", ygg.narrative_lines().size(),
			"  options=", ygg.choice_options())
	if auto_pick >= 1 and not ygg.choice_options().is_empty():
		_choose(auto_pick)


## 用 getter 缓存面刷新 UI（headless 下同样执行，仅渲染不可见）。
func _render() -> void:
	var lines := ygg.narrative_lines()
	narrative_label.text = "\n".join(lines)
	var prompt := ygg.choice_prompt()
	var options := ygg.choice_options()
	if prompt.is_empty():
		status_label.text = "（无待决选择）"
		return
	status_label.text = prompt
	for child in choices_box.get_children():
		child.queue_free()
	for i in options.size():
		var button := Button.new()
		button.text = "%d. %s" % [i + 1, options[i]]
		button.pressed.connect(_on_choice.bind(i + 1))
		choices_box.add_child(button)


## 玩家点击路径（窗口运行）与脚本自动路径（headless）共用同一入口。
func _on_choice(index: int) -> void:
	_choose(index)


func _choose(index: int) -> void:
	var reply_raw := ygg.dispatch("ygg.choice_selected", str(index))
	var reply: Variant = JSON.parse_string(reply_raw)
	if reply == null or not bool(reply.get("ok", false)):
		_fail("ygg.choice_selected(%d) 失败：%s" % [index, reply_raw])
		return
	_render()
	var lines := ygg.narrative_lines()
	var head := str(lines[0]).left(48) if lines.size() > 0 else ""
	print("[spike] choice ", index, " ok  narrative_lines=", lines.size(), "  head=", head)
	if not shot_path.is_empty():
		# 等一帧真实绘制后再抓（同帧抓会拿到未合成的黑帧）。
		await RenderingServer.frame_post_draw
		var image := get_viewport().get_texture().get_image()
		image.save_png(shot_path)
		print("[spike] screenshot -> ", shot_path)
	if auto_pick >= 1:
		print("[spike] DONE ok=true")
		get_tree().quit(0)


func _fail(message: String) -> void:
	push_error("[spike] " + message)
	print("[spike] FAIL ", message)
	get_tree().quit(1)
