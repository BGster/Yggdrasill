extends Control
## Yggdrasill Godot 客户端主面板：叙事日志 + 选择面 + 存档栏 + 状态栏。
##
## 数据流（单向）：GDScript 只经 `YggKernel` 绑定的 JSON 信封与内核交互；
## 叙事日志是**客户端**状态（内核信封只带本次转换的行），由 `session.gd`
## 累积并在存档时随内核快照一起落盘。
##
## 运行参数（user args，须写在命令行 `--` 之后）：
##   --mod-dir=/abs/path     Mod 目录（缺省：环境变量 YGG_MOD_DIR → 仓库内 test-adventure）
##   --choices=1,1           脚本驱动的选择序列（headless 走查 / 自动通关）
##   --save-after=N          应用第 N 个选择后把存档写入 --save-file
##   --save-file=/abs/path   存档槽路径（工具栏「存档/读档」缺省也用它）
##   --load-file=/abs/path   启动即读档（替代 event.start）
##   --shots=/abs/prefix     每次渲染后截屏为 <prefix>-NN.png（窗口运行）
##   --auto-quit=1           脚本驱动的选择序列耗尽后自动退出（退出码 0 = 无错误）
##
## headless 通关走查：
##   Godot --headless --path clients/godot/project -- --choices=1,1 --auto-quit=1
## 窗口运行（人工点选）：
##   Godot --path clients/godot/project

const SESSION_SCRIPT := preload("res://session.gd")
const THEME_FACTORY := preload("res://theme_factory.gd")

## 默认存档槽（`user://` 落在 Godot 的 app_userdata 目录，不入仓库）。
const DEFAULT_SAVE_PATH := "user://ygg-save.json"

## 绑定节点。
var ygg: YggKernel
## 会话状态机（`session.gd` 实例）。
var session: YggSession

# ── 运行参数 ───────────────────────────────────────────────────────────
var _mod_dir := ""
var _choices: Array = []
var _save_file := ""
var _save_after := 0
var _load_file := ""
var _shots_prefix := ""
var _auto_quit := false

# ── 运行态 ─────────────────────────────────────────────────────────────
var _applied_choices := 0
## 脚本驱动队列的读指针（与 _applied_choices 分离：读档起点下队列从头开始）。
var _script_index := 0
var _shot_index := 0
var _failed := false

@onready var _save_button: Button = %SaveButton
@onready var _load_button: Button = %LoadButton
@onready var _state_button: Button = %StateButton
@onready var _restart_button: Button = %RestartButton
@onready var _narrative: RichTextLabel = %Narrative
@onready var _prompt: Label = %Prompt
@onready var _choices_box: VBoxContainer = %Choices
@onready var _status: Label = %Status


func _ready() -> void:
	theme = THEME_FACTORY.build()
	session = SESSION_SCRIPT.new()
	_parse_args()
	ygg = YggKernel.new()
	ygg.name = "YggKernel"
	add_child(ygg)
	_save_button.pressed.connect(_on_save_pressed)
	_load_button.pressed.connect(_on_load_pressed)
	_state_button.pressed.connect(_on_state_pressed)
	_restart_button.pressed.connect(_on_restart_pressed)
	if not _boot():
		return
	if _choices.is_empty():
		return
	await _run_script()


# ───────────────────────── 启动 / 参数 ─────────────────────────

func _parse_args() -> void:
	for arg in OS.get_cmdline_user_args():
		if arg.begins_with("--mod-dir="):
			_mod_dir = arg.get_slice("=", 1)
		elif arg.begins_with("--choices="):
			for item in arg.get_slice("=", 1).split(",", false):
				var index := int(item.strip_edges())
				if index >= 1:
					_choices.append(index)
		elif arg.begins_with("--save-after="):
			_save_after = int(arg.get_slice("=", 1))
		elif arg.begins_with("--save-file="):
			_save_file = arg.get_slice("=", 1)
		elif arg.begins_with("--load-file="):
			_load_file = arg.get_slice("=", 1)
		elif arg.begins_with("--shots="):
			_shots_prefix = arg.get_slice("=", 1)
		elif arg.begins_with("--auto-quit="):
			_auto_quit = arg.get_slice("=", 1) != "0"


func _boot() -> bool:
	if not _load_mod(_resolve_mod_dir()):
		return false
	if _load_file.is_empty():
		return _dispatch_event("event.start", "0")
	if not _load_from_file(_load_file):
		return false
	return true


## Mod 目录解析：`--mod-dir` → `YGG_MOD_DIR` → 仓库内 test-adventure。
func _resolve_mod_dir() -> String:
	if not _mod_dir.is_empty():
		return _mod_dir
	var from_env := OS.get_environment("YGG_MOD_DIR")
	if not from_env.is_empty():
		return from_env
	# res:// = <repo>/clients/godot/project/ → 三级回到仓库根。
	return ProjectSettings.globalize_path("res://").path_join("../../../packages/mods/test-adventure").simplify_path()


func _load_mod(dir_path: String) -> bool:
	var reply = _parse_json(ygg.load_mod(dir_path))
	if reply.is_empty() or not bool(reply.get("ok", false)):
		var reason := str(reply.get("error", "?")) if not reply.is_empty() else "回包无法解析"
		_fail("load_mod 失败：%s" % reason)
		return false
	_mod_dir = dir_path
	print("[godot] load_mod ok  dir=", dir_path)
	return true


# ───────────────────────── 内核交互（唯一入口） ─────────────────────────

## 一次事件派发 + 会话落账 + UI 刷新。返回是否成功。
func _dispatch_event(cmd: String, payload: String) -> bool:
	var raw := str(ygg.dispatch(cmd, payload))
	var reply = _parse_json(raw)
	if reply.is_empty():
		_fail("内核回包不是 JSON 对象：%s" % raw)
		return false
	session.apply_envelope(reply)
	var ok := bool(reply.get("ok", false))
	_refresh_ui()
	if not ok:
		var error: Dictionary = reply.get("error", {})
		print("[godot] error code=", str(error.get("code", "?")), "  message=", str(error.get("message", "")))
	return ok


## 玩家点击 / 脚本驱动共用入口。
func _choose(index: int) -> void:
	if not session.has_choice:
		return
	if not _dispatch_event("ygg.choice_selected", str(index)):
		return
	_applied_choices += 1
	_refresh_status()
	if _save_after > 0 and _applied_choices == _save_after:
		_save_to_file(_save_file if not _save_file.is_empty() else DEFAULT_SAVE_PATH)


# ───────────────────────── 存档 / 读档 / 状态 ─────────────────────────

func _save_to_file(path: String) -> bool:
	var kernel_text := str(ygg.save_snapshot())
	if kernel_text.is_empty():
		_fail("存档失败：%s" % str(ygg.last_error()))
		return false
	# 存档槽 = 内核快照 + 客户端叙事日志（内核不存历史叙事，见 session.gd）。
	# 快照段**按原文拼接**，不经 Godot 的 JSON 类型往返：Godot 把所有 JSON
	# 数字解析为 float（`6` → `6.0`），u64 字段会因此被内核拒（实证）。
	var slot := "{\n  \"kind\": \"ygg.godot.save\",\n  \"version\": 1,\n  \"steps\": %d,\n  \"ended\": %s,\n  \"kernel\": %s,\n  \"log\": %s\n}\n" % [
		_applied_choices,
		"true" if session.ended else "false",
		kernel_text,
		JSON.stringify(session.log_snapshot()),
	]
	var absolute := _absolute(path)
	var parent := absolute.get_base_dir()
	if not parent.is_empty():
		DirAccess.make_dir_recursive_absolute(parent)
	var file := FileAccess.open(absolute, FileAccess.WRITE)
	if file == null:
		_fail("存档写入失败（%s）：%s" % [absolute, error_string(FileAccess.get_open_error())])
		return false
	file.store_string(slot)
	file.close()
	var diagnosis = _parse_json(str(ygg.dispatch("state", "0")))
	session.add_system("已存档 → %s（revision %s · kvRevision %s · 叙事 %d 行）" % [
		absolute,
		_int_text(diagnosis.get("revision", "?")),
		_int_text(diagnosis.get("kvRevision", "?")),
		session.entries.size(),
	])
	_refresh_ui()
	print("[godot] save ok  file=", absolute, "  bytes=", FileAccess.get_file_as_bytes(absolute).size(),
			"  revision=", _int_text(diagnosis.get("revision", "?")),
			"  kvRevision=", _int_text(diagnosis.get("kvRevision", "?")),
			"  pending=", _pending_label())
	return true


func _load_from_file(path: String) -> bool:
	var absolute := _absolute(path)
	if not FileAccess.file_exists(absolute):
		_fail("存档不存在：%s" % absolute)
		return false
	var slot_text := FileAccess.get_file_as_string(absolute)
	var bundle = _parse_json(slot_text)
	if bundle.is_empty():
		_fail("存档无法解析：%s" % absolute)
		return false
	# 内核快照段交绑定在 Rust 侧提取（serde 保留整数/浮点区分，见 load_bundle 注释）。
	var reply = _parse_json(str(ygg.load_bundle(slot_text)))
	if reply.is_empty() or not bool(reply.get("ok", false)):
		var reason := str(reply.get("error", "?")) if not reply.is_empty() else "回包无法解析"
		_fail("读档被内核拒绝：%s" % reason)
		return false
	if not bool(reply.get("restored", false)):
		_fail("读档回包未标记 restored：%s" % JSON.stringify(reply))
		return false
	session.restore_log(bundle.get("log", []), bool(bundle.get("ended", false)))
	_applied_choices = int(bundle.get("steps", 0))
	# 待决选择取绑定缓存面（load_bundle 已从信封回填）——内核语义权威。
	session.set_choice(str(ygg.choice_prompt()), ygg.choice_options())
	if session.ended:
		session.clear_choice()
	session.add_system("（读档完成 · %s）" % absolute.get_file())
	_refresh_ui()
	var diagnosis = _parse_json(str(ygg.dispatch("state", "0")))
	print("[godot] load ok  file=", absolute,
			"  revision=", _int_text(diagnosis.get("revision", "?")),
			"  kvRevision=", _int_text(diagnosis.get("kvRevision", "?")),
			"  pending=", _pending_label(),
			"  log_lines=", session.entries.size())
	return true


func _on_save_pressed() -> void:
	_save_to_file(_save_file if not _save_file.is_empty() else DEFAULT_SAVE_PATH)


func _on_load_pressed() -> void:
	_load_from_file(_save_file if not _save_file.is_empty() else DEFAULT_SAVE_PATH)


func _on_state_pressed() -> void:
	var diagnosis = _parse_json(str(ygg.dispatch("state", "0")))
	if diagnosis.is_empty() or not bool(diagnosis.get("ok", false)):
		session.add_system("（状态查询失败）")
		_refresh_ui()
		return
	session.add_system("状态 engine=%s revision=%s kvRevision=%s 快照=%s B 待决=%s" % [
		str(diagnosis.get("engineVersion", "?")),
		_int_text(diagnosis.get("revision", "?")),
		_int_text(diagnosis.get("kvRevision", "?")),
		_int_text(diagnosis.get("snapshotBytes", "?")),
		_pending_label(),
	])
	_refresh_ui()


func _on_restart_pressed() -> void:
	session.reset()
	_applied_choices = 0
	_script_index = 0
	if not _load_mod(_mod_dir):
		return
	if not _dispatch_event("event.start", "0"):
		return
	print("[godot] restart ok")


# ───────────────────────── UI 渲染 ─────────────────────────

func _refresh_ui() -> void:
	_render_log()
	_render_choices()
	_refresh_status()


## 叙事日志整体重建（读档后需要整段回放；本切片日志量级足够小）。
func _render_log() -> void:
	_narrative.clear()
	for entry in session.entries:
		_render_entry(entry)
	_narrative.scroll_to_line(maxi(0, _narrative.get_line_count() - 1))


func _render_entry(entry: Dictionary) -> void:
	var text := str(entry.get("text", ""))
	match str(entry.get("kind", "narrative")):
		"narrative":
			if SESSION_SCRIPT.is_ending_line(text):
				_narrative.push_color(THEME_FACTORY.COLOR_ENDING)
				_narrative.push_bold()
				_narrative.add_text(text)
				_narrative.pop()
				_narrative.pop()
			else:
				_narrative.add_text(text)
		"warning":
			_narrative.push_color(THEME_FACTORY.COLOR_WARNING)
			_narrative.add_text("⚠ " + text)
			_narrative.pop()
		"error":
			_narrative.push_color(THEME_FACTORY.COLOR_ERROR)
			_narrative.add_text("✖ " + text)
			_narrative.pop()
		"system":
			_narrative.push_color(THEME_FACTORY.COLOR_SYSTEM)
			_narrative.push_italics()
			_narrative.add_text(text)
			_narrative.pop()
			_narrative.pop()
		_:
			_narrative.add_text(text)
	_narrative.newline()
	_narrative.newline()


func _render_choices() -> void:
	for child in _choices_box.get_children():
		_choices_box.remove_child(child)
		child.queue_free()
	if session.ended:
		_prompt.text = "—— 冒险结束 ——（工具栏「重开」可再走一遍）"
		return
	if not session.has_choice:
		_prompt.text = ""
		return
	_prompt.text = str(session.pending_choice.get("prompt", ""))
	var options: Array = session.pending_choice.get("options", [])
	for i in options.size():
		var button := Button.new()
		button.text = "%d. %s" % [i + 1, str(options[i])]
		button.alignment = HORIZONTAL_ALIGNMENT_LEFT
		button.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		button.pressed.connect(_choose.bind(i + 1))
		_choices_box.add_child(button)


func _refresh_status() -> void:
	var phase := "未开始"
	if session.ended:
		phase = "已结局"
		if not session.ending_id.is_empty():
			phase += " %s" % session.ending_id
	elif session.has_choice:
		phase = "待选择"
	elif session.started:
		phase = "推进中"
	var diagnosis = _parse_json(str(ygg.dispatch("state", "0")))
	var parts := PackedStringArray()
	parts.append("阶段：%s" % phase)
	parts.append("事件步数：%d" % _applied_choices)
	if not diagnosis.is_empty():
		parts.append("revision %s" % _int_text(diagnosis.get("revision", "?")))
		parts.append("kvRevision %s" % _int_text(diagnosis.get("kvRevision", "?")))
		parts.append("快照 %s B" % _int_text(diagnosis.get("snapshotBytes", "?")))
	parts.append("叙事 %d 行" % session.entries.size())
	parts.append("Mod：%s" % _mod_dir.get_file())
	_status.text = " · ".join(parts)


# ───────────────────────── 脚本驱动（headless 走查） ─────────────────────────

func _run_script() -> void:
	await get_tree().process_frame
	await _capture()
	while session.has_choice and _script_index < _choices.size():
		var index := int(_choices[_script_index])
		_script_index += 1
		_choose(index)
		await get_tree().process_frame
		await _capture()
	_finish()


func _finish() -> void:
	var last_line := ""
	for entry in session.entries:
		if str(entry.get("kind", "")) == "narrative":
			last_line = str(entry.get("text", ""))
	if _failed:
		return
	var ending_label := session.ending_id if not session.ending_id.is_empty() else "-"
	print("[godot] summary ok=true  ended=", session.ended,
			"  ending=", ending_label,
			"  steps=", _applied_choices,
			"  pending=", session.has_choice,
			"  log_lines=", session.entries.size())
	print("[godot] last_line=", last_line)
	if _auto_quit:
		get_tree().quit(1 if _has_error_entries() else 0)


func _has_error_entries() -> bool:
	for entry in session.entries:
		if str(entry.get("kind", "")) == "error":
			return true
	return false


func _capture() -> void:
	if _shots_prefix.is_empty() or DisplayServer.get_name() == "headless":
		return
	await RenderingServer.frame_post_draw
	var image := get_viewport().get_texture().get_image()
	var path := "%s-%02d.png" % [_shots_prefix, _shot_index]
	_shot_index += 1
	var saved := image.save_png(path)
	print("[godot] screenshot -> ", path, "  ok=", saved)


# ───────────────────────── 玩家输入 ─────────────────────────

func _unhandled_key_input(event: InputEvent) -> void:
	var key := event as InputEventKey
	if key == null or not key.pressed or key.echo:
		return
	if key.ctrl_pressed or key.meta_pressed or key.alt_pressed:
		return
	var index := key.keycode - KEY_1 + 1
	if index < 1 or index > session.pending_choice.get("options", []).size():
		return
	_choose(index)


# ───────────────────────── 辅助 ─────────────────────────

func _parse_json(raw: String) -> Dictionary:
	var value = JSON.parse_string(raw)
	if value is Dictionary:
		return value
	return {}


func _pending_label() -> String:
	if not session.has_choice:
		return "无"
	return str(session.pending_choice.get("prompt", ""))


## 数字文本：Godot 的 JSON 解析把所有数字给成 float，展示面统一去尾数。
func _int_text(value: Variant) -> String:
	if value is float or value is int:
		return str(int(value))
	return str(value)


func _absolute(path: String) -> String:
	return ProjectSettings.globalize_path(path)


func _fail(message: String) -> void:
	_failed = true
	push_error("[godot] " + message)
	print("[godot] FAIL ", message)
	get_tree().quit(1)
