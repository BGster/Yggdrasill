//! 稳定错误码映射面（payload 标量门 / 沙箱拒绝）。
//! （原「用例 6-10」中的燃料三例已随 S34 燃料机制整体退役删除——
//! 耗尽语义不复存在，对应稳定错误码同步收缩。）
use super::*;

/// 沙箱拒绝 Mod：handler 内 require（白名单拒绝桩，可读 "sandbox denied"）。
const DENIED_MOD: &str = r#"
ygg.on("event.denied", function()
  require("io")
end)
"#;

#[test]
fn payload_validation_maps_to_stable_code() {
    let mut kernel = KernelFacade::new(GATE_MOD).expect("kernel construction");
    // 嵌套对象 → 标量门拒绝。
    let reply = kernel.dispatch("event.start", r#"{"deep":{"n":1}}"#);
    assert_eq!(reply_error_code(&reply), "VALIDATION");
    // 非法 JSON → 同码。
    let reply = kernel.dispatch("event.start", "{not json");
    assert_eq!(reply_error_code(&reply), "VALIDATION");
    // 非保留名 + 标量 payload 正常（boolean / string 亦合法标量）。
    assert_eq!(
        parse(&kernel.dispatch("event.start", "true"))["ok"],
        json!(true)
    );
}

#[test]
fn sandbox_denial_maps_to_stable_code() {
    let mut kernel = KernelFacade::new(DENIED_MOD).expect("kernel construction");
    let reply = kernel.dispatch("event.denied", "0");
    assert_eq!(reply_error_code(&reply), "SANDBOX_DENIED");
}
