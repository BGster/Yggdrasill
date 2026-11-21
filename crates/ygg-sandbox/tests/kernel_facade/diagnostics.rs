//! 用例 21-24：错误定位、warnings 警告面与 host 回调拒绝取证面。
use super::*;

/// 报错定位 Mod（C3）：handler 内显式 error——验证 chunk 命名 `@mod.lua`
/// 进入错误文本（mod 侧定位面）。
const BOOM_MOD: &str = r#"
ygg.on("event.boom", function()
  error("boom")
end)
"#;

/// D1 反例 Mod：handler 先 emit 再 choice——派生事件被语义丢弃，
/// 信封应携带 warnings 警告（含事件名与丢弃条数）。
/// `ygg.choice_selected` 无操作注册：respond 步所需（同 GATE_MOD 先例——
/// 用了 choice 的 Mod 自己注册 respond 事件；fail-closed 后静默成功不再存在）。
const CHOICE_DROPS_MOD: &str = r#"
ygg.on("event.go", function()
  ygg.emit("chapter.advance", "to_road")
  ygg.emit("chapter.advance", "to_cellar")
  ygg.choice("去哪儿？", { "地窖", "客房" })
end)
ygg.on("ygg.choice_selected", function() end)
"#;

/// D2 取证 Mod：host 回调拒绝（kv.set 非有限值）——mlua CallbackError
/// 链应携带回调点捕获的 traceback（mod.lua 调用点帧）。
const KV_NAN_MOD: &str = r#"
ygg.on("event.bad", function()
  ygg.kv.set("x", 0/0)
end)
"#;

#[test]
fn handler_error_message_carries_mod_chunk_name() {
    // C3：Mod chunk 命名 `@mod.lua`（与 stdlib `@stdlib/<name>` 同法）——
    // handler 内报错的错误文本必须携带 mod 侧 chunk 位置，排障定位不再
    // 只见 Lua 桥行号。注：Lua 错误格式化会剥掉文件型 chunk 名前缀的 `@`
    //（`@` 是「名字为文件」的约定记号，非名字本体），故文本形态为
    // `mod.lua:3: boom`。仍映射 VALIDATION（handler 错误既有口径不变）。
    let mut kernel = KernelFacade::new(BOOM_MOD).expect("kernel construction");
    let reply = kernel.dispatch("event.boom", "0");
    assert_eq!(reply_error_code(&reply), "VALIDATION");
    let parsed: Json = parse(&reply);
    let message = parsed["error"]["message"].as_str().expect("message");
    assert!(
        message.contains("mod.lua"),
        "error text must name the mod chunk: {reply}"
    );
    assert!(
        message.contains("boom"),
        "error text carries the message: {reply}"
    );
}

#[test]
fn choice_preceded_emits_surface_runtime_warning() {
    // D1：emit-then-choice 的丢弃不再静默——信封加性 warnings 键，一条
    // 警告合并本次 dispatch 的全部丢弃条数（2 条 emit → 一条警告计数 2）。
    let mut kernel = KernelFacade::new(CHOICE_DROPS_MOD).expect("kernel construction");
    let reply = kernel.dispatch("event.go", "0");
    let parsed = reply_ok(&reply);
    assert_eq!(
        parsed["warnings"],
        json!(["事件 `event.go` 的 handler 在 ygg.choice 之前产生了 2 条派生事件（ygg.emit），已按语义丢弃（choice 须为 handler 最后一个输出调用）"]),
        "exact deterministic warning text: {reply}"
    );
    assert_eq!(parsed["eventsDropped"], json!(2));
    assert_eq!(parsed["eventsDerived"], json!(2));
    // 丢弃面只在 emit：无 narrate 时叙述缓冲为空（语义面与改动前一致）。
    assert_eq!(parsed["narrative"], json!([]));
    assert!(parsed["pendingChoice"].is_object());
    // 警告不跨 dispatch 泄漏：respond 后的信封无 warnings 键。
    let parsed = reply_ok(&kernel.dispatch("ygg.choice_selected", "1"));
    assert!(
        parsed.get("warnings").is_none(),
        "warning must not leak to the next dispatch: {parsed}"
    );
}

#[test]
fn dispatch_without_pre_choice_emits_has_no_warnings_key() {
    // D1：无该模式的 dispatch 信封形状不变——warnings 键不存在（加性键
    // 只在非空时写入；GATE_MOD 的 choice 前无 emit）。
    let mut kernel = KernelFacade::new(GATE_MOD).expect("kernel construction");
    let parsed = reply_ok(&kernel.dispatch("event.start", "0"));
    assert!(
        parsed.get("warnings").is_none(),
        "absent pattern must not add warnings key: {parsed}"
    );
    // narrate-then-choice 也不产生警告：narrate 不在丢弃面（信封 narrative 可见）。
    const NARRATE_THEN_CHOICE_MOD: &str = r#"
ygg.on("event.scene", function()
  ygg.narrate("灯下的小酒馆")
  ygg.choice("点单吗？", { "是", "否" })
end)
"#;
    let mut kernel = KernelFacade::new(NARRATE_THEN_CHOICE_MOD).expect("kernel construction");
    let parsed = reply_ok(&kernel.dispatch("event.scene", "0"));
    assert!(
        parsed.get("warnings").is_none(),
        "narrate survives: {parsed}"
    );
    assert_eq!(parsed["narrative"], json!(["灯下的小酒馆"]));
}

#[test]
fn host_callback_rejection_carries_mod_call_site() {
    // D2：host 回调拒绝（kv.set 非有限值）的错误文本已含 mod 侧调用点——
    // mlua 在 Rust 回调返回错误时捕获当时 Lua 栈（CallbackError.traceback），
    // chunk 命名 @mod.lua（C3）使帧可读：`mod.lua:3`（kv.set 所在行）。
    // 无需穿透修复：测试固证既有行为。
    let mut kernel = KernelFacade::new(KV_NAN_MOD).expect("kernel construction");
    let reply = kernel.dispatch("event.bad", "0");
    assert_eq!(reply_error_code(&reply), "VALIDATION");
    let parsed: Json = parse(&reply);
    let message = parsed["error"]["message"].as_str().expect("message");
    assert!(
        message.contains("non-finite"),
        "bridge rejection text: {reply}"
    );
    assert!(
        message.contains("stack traceback") && message.contains("mod.lua:3"),
        "host-callback error must carry the mod call-site line: {reply}"
    );
    // entity.set_attribute 同法（拒绝点在第 4 行 → mod.lua:4）。
    const ENT_NAN_MOD: &str = r#"
ygg.on("event.bad", function()
  local id = ygg.entity.spawn()
  ygg.entity.set_attribute(id, "hp", 0/0)
end)
"#;
    let mut kernel = KernelFacade::new(ENT_NAN_MOD).expect("kernel construction");
    let reply = kernel.dispatch("event.bad", "0");
    let parsed: Json = parse(&reply);
    let message = parsed["error"]["message"].as_str().expect("message");
    assert!(
        message.contains("mod.lua:4"),
        "entity host-callback error must carry the mod call-site line: {reply}"
    );
}

// ---- dispatch 级 savepoint：一次 dispatch 要么全生效、要么全回滚 ----
//
