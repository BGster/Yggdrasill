//! 用例 6-10：fuel/预算扩缩与稳定错误码映射面。
use super::*;

/// 死循环 Mod：VM 指令计量面耗尽（燃料杀循环负例经 facade 信封）。
/// `event.start` 无操作注册：燃料耗尽后的恢复步用（fail-closed 后未注册名
/// 不再有成功信封）。
const LOOP_MOD: &str = r#"
ygg.on("event.loop", function()
  while true do end
end)
ygg.on("event.start", function() end)
"#;

/// r13 自动扩缩击杀回归 Mod：fill 事件灌 1000 条 KV（推高自动扩缩预算到
/// 2M）；burn 事件跑固定迭代数计数循环（约几十万指令——地板 100k 杀不死、
/// 2M 扩缩预算放行）；loop 事件是纯死循环（任何有限预算下都被杀）。
const AUTOSCALE_MOD: &str = r#"
ygg.on("event.fill", function()
  for i = 1, 1000 do
    ygg.kv.set("probe." .. tostring(i), i)
  end
end)
ygg.on("event.burn", function()
  local acc = 0
  for i = 1, 150000 do
    acc = acc + i
  end
  ygg.kv.set("burned", acc)
end)
ygg.on("event.loop", function()
  while true do end
end)
-- 击杀步后的还原验证需要一次后续 dispatch（③）：注册一个 no-op 事件作探针
--（同 LOOP_MOD 先例：fail-closed 后未注册名会被拒）。
ygg.on("event.start", function() end)
"#;

/// 沙箱拒绝 Mod：handler 内 require（白名单拒绝桩，可读 "sandbox denied"）。
const DENIED_MOD: &str = r#"
ygg.on("event.denied", function()
  require("io")
end)
"#;

#[test]
fn fuel_exhaustion_maps_to_stable_code() {
    let mut kernel = KernelFacade::new(LOOP_MOD).expect("kernel construction");
    let reply = kernel.dispatch("event.loop", "0");
    assert_eq!(reply_error_code(&reply), "FUEL_EXHAUSTED");
    // 预算确为驱动常量（转换边界重置额）：错误后再次 dispatch 正常工作。
    let parsed = reply_ok(&kernel.dispatch("event.start", "0"));
    assert_eq!(parsed["event"], json!("event.start"));
    assert_eq!(DEFAULT_FUEL_BUDGET, 100_000);
}

#[test]
fn autoscaled_budget_admits_the_scaled_workload_but_still_kills_loops() {
    // r13 击杀回归（大状态）：三段式证明「预算变大 ≠ 保护变弱」——
    // ① 大状态放行：kv_len=1000 → 自动预算 2M，几十万指令的合法工作负载
    //    在地板下必死、在扩缩预算下完整提交（burned 标记入档）；
    // ② 死循环仍被杀：同一大状态上派发纯死循环 → FUEL_EXHAUSTED
    //    （顶格区间 [100k, 4M] 内击杀语义保留）；
    // ③ canonical 回滚：击杀步写过的 KV 整体还原（probe.* 保留、burned
    //    消失），savepoint 语义与旧契约一致。
    let mut kernel = KernelFacade::new(AUTOSCALE_MOD).expect("kernel construction");
    // ① fill 自身（10k 指令量级，远低于地板 100k）提交，推高 kv_len 到 1000+。
    reply_ok(&kernel.dispatch("event.fill", "0"));
    assert_eq!(saved_kv(&mut kernel)["probe.1000"], json!(1000));
    let probe_count = saved_kv(&mut kernel)
        .as_object()
        .map(|entries| entries.keys().filter(|k| k.starts_with("probe.")).count())
        .unwrap_or(0);
    assert_eq!(probe_count, 1000, "fill must leave exactly 1000 probe keys");
    // ①' 同一负载在地地板预算下必死（对照组在 floor_budget... 用例覆盖）；
    //     这里只证扩缩后放行：burned 标记入档。
    reply_ok(&kernel.dispatch("event.burn", "0"));
    assert_eq!(
        saved_kv(&mut kernel)["burned"],
        json!(150_000i64 * 150_001 / 2),
        "scaled budget must admit the bounded workload"
    );
    // ② 死循环在大状态上仍被杀（预算 2M 有限，hook 面照杀）。
    let reply = kernel.dispatch("event.loop", "0");
    assert_eq!(reply_error_code(&reply), "FUEL_EXHAUSTED");
    // ③ 击杀步的 KV 写（无）与状态整体还原：probe.* 完整保留、后续 dispatch 正常。
    let probe_count_after = saved_kv(&mut kernel)
        .as_object()
        .map(|entries| entries.keys().filter(|k| k.starts_with("probe.")).count())
        .unwrap_or(0);
    assert_eq!(
        probe_count_after, 1000,
        "kill must not disturb canonical state"
    );
    let parsed = reply_ok(&kernel.dispatch("event.start", "0"));
    assert_eq!(parsed["event"], json!("event.start"));
}

#[test]
fn floor_budget_still_kills_the_workload_that_autoscale_admits() {
    // r13 击杀回归（空局对照）：同一 burn 负载在 kv_len≈0（地板 100k）下
    // 必须 FUEL_EXHAUSTED + 整体回滚——证明 ① 放行确实来自扩缩（不是负载
    // 本来就在地板内）；② 空局/小局行为与旧契约零变化。
    let mut kernel = KernelFacade::new(AUTOSCALE_MOD).expect("kernel construction");
    let reply = kernel.dispatch("event.burn", "0");
    assert_eq!(reply_error_code(&reply), "FUEL_EXHAUSTED");
    assert_eq!(
        saved_kv(&mut kernel),
        json!({}),
        "floor-budget kill must roll back the attempted writes"
    );
    let state = reply_ok(&kernel.dispatch("state", ""));
    assert_eq!(state["kvRevision"], json!(0), "kv revision rolls back");
    // 击杀后驱动照常工作（转换边界重置额仍 = 地板）。
    reply_ok(&kernel.dispatch("event.fill", "0"));
    assert_eq!(saved_kv(&mut kernel)["probe.1000"], json!(1000));
}

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
