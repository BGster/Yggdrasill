extends RefCounted
class_name YggSession
## 会话状态机（UI 无关，headless 下同样执行）。
##
## 语义镜像 `packages/client-web/src/session.ts`——两个表现层共用同一份内核
## 信封契约，行为必须一致：
## - 事件信封 → 叙事行按序追加 → D1 警告行 → 待决选择更新；
## - 结局叙事行（【结局 前缀）出现即收口（ended），追加「—— 冒险结束 ——」；
## - 错误信封 → 错误条目，**保留**原待决选择（玩家可重试，不丢选择面）；
## - 存档/读档/状态类回执 → 系统行（不重放叙事）。
##
## 内核信封的 `narrative` 只含**本次转换**产生的行（历史叙事不入档、由本层
## 累积），因此本类是叙事日志的唯一权威——存档时同日志一起落盘。

## 结局叙事行前缀（与 crates/ygg-sandbox/src/playtest.rs 同源同值）。
const ENDING_PREFIX := "【结局"
## 结局收口行。
const ENDING_FOOTER := "—— 冒险结束 ——"

## 日志条目（按序渲染的原子）：{kind, text}，kind ∈ narrative/warning/system/error。
var entries: Array[Dictionary] = []
## 待决选择：{prompt, options}（无选择时为空字典）。
var pending_choice: Dictionary = {}
## 是否存在待决选择。
var has_choice := false
## 结局行已出现（选择面收口）。
var ended := false
## 到达的结局标识（A/B/C…；未结局为空串）。
var ending_id := ""
## 首个事件信封已落账。
var started := false


## 事件信封落账（叙事 / 警告 / 待决选择 / 结局收口 / 错误）。
func apply_envelope(reply: Dictionary) -> void:
	if reply.is_empty():
		add_system("（信封无法解析：回包不是 JSON 对象）")
		return
	started = true
	if not bool(reply.get("ok", false)):
		var error: Dictionary = reply.get("error", {})
		var code := str(error.get("code", "UNKNOWN"))
		var message := str(error.get("message", ""))
		if message.is_empty():
			message = "（信封未携带错误详情）"
		entries.append({"kind": "error", "text": "%s：%s" % [code, message]})
		return
	var reached_ending := false
	for line in reply.get("narrative", []):
		var text := str(line)
		entries.append({"kind": "narrative", "text": text})
		if not reached_ending and is_ending_line(text):
			reached_ending = true
			ending_id = ending_id_of(text)
	# D1 警告行（创作侧人读提示，⚠ 前缀在渲染层加）。
	for warning in reply.get("warnings", []):
		entries.append({"kind": "warning", "text": str(warning)})
	if reached_ending:
		ended = true
		entries.append({"kind": "system", "text": ENDING_FOOTER})
		clear_choice()
		return
	var choice = reply.get("pendingChoice", null)
	if choice is Dictionary and not (choice as Dictionary).is_empty():
		set_choice(str(choice.get("prompt", "")), _string_list(choice.get("options", [])))
	else:
		clear_choice()


## 待决选择更新（读档后从绑定缓存面回填同用；options 可为 Array 或
## PackedStringArray——绑定 getter 返回后者）。
func set_choice(prompt: String, options: Variant) -> void:
	pending_choice = {"prompt": prompt, "options": _string_list(options)}
	has_choice = not pending_choice["options"].is_empty()


## 清空待决选择（结局 / 无选择）。
func clear_choice() -> void:
	pending_choice = {}
	has_choice = false


## 系统行（存档 / 读档 / 状态诊断等不推进剧情的回执）。
func add_system(text: String) -> void:
	entries.append({"kind": "system", "text": text})


## 重开：清空全部会话面。
func reset() -> void:
	entries.clear()
	pending_choice = {}
	has_choice = false
	ended = false
	ending_id = ""
	started = false


## 存档日志面（仅 {kind, text}——渲染层不依赖其它字段）。
func log_snapshot() -> Array:
	var items := []
	for entry in entries:
		items.append({"kind": str(entry.get("kind", "narrative")), "text": str(entry.get("text", ""))})
	return items


## 读档日志面：以存档日志替换当前日志（条目形态不符的项丢弃）。
func restore_log(items: Array, ended_flag: bool) -> void:
	entries.clear()
	ending_id = ""
	for item in items:
		if typeof(item) != TYPE_DICTIONARY:
			continue
		var entry: Dictionary = item
		var text := str(entry.get("text", ""))
		var kind := str(entry.get("kind", "narrative"))
		entries.append({"kind": kind, "text": text})
		if kind == "narrative" and is_ending_line(text) and ending_id.is_empty():
			ending_id = ending_id_of(text)
	ended = ended_flag or not ending_id.is_empty()
	started = true


## 叙事行是否为结局行（【结局 前缀；首尾空白不敏感）。
static func is_ending_line(text: String) -> bool:
	return text.strip_edges().begins_with(ENDING_PREFIX)


## 结局标识：前缀后的首个 ASCII 字母（缺失 → "?"）。
static func ending_id_of(text: String) -> String:
	var tail := text.strip_edges().trim_prefix(ENDING_PREFIX)
	if tail.is_empty():
		return "?"
	var head := tail.substr(0, 1)
	return head if head.to_upper() == head and head.to_lower() != head else "?"


func _string_list(source: Variant) -> Array:
	var out := []
	if source is Array or source is PackedStringArray:
		for item in source:
			out.append(str(item))
	return out
