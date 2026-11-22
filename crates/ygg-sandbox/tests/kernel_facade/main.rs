//! S10 KernelFacade 线格式/信封集成测试（34 测）。
//!
//! 验证面：JSON 信封形状（A1 线格式）、choice 终结与 respond（B2
//! choice-as-state）、稳定错误码映射、存读档往返与统一 store 代决证据、
//! 外部事件日志（B1 回放前提）、确定性 canonical 字节、stdlib（narrative /
//! fsm）经 facade 全链路、load 回包携带 pendingChoice（A1）、
//! choice_selected 序号域内校验（A2）、fsm when 条件边（B1）与 initial
//! 落盘可见化（B2）、**dispatch 原子性（r1 savepoint：失败步整体回滚
//! entities + kv 与两个 revision，成功步提交；域校验门早退不触发回滚；
//! 旁路面 `log` 照记失败步）**。
//!
//! 断言口径备忘：KV 不在状态哈希的覆盖范围内（`src/hash.rs` 覆盖面声明），
//! 所以「失败步没改状态」的断言必须读 `save` 回执（`save.kv.entries`）或
//! 比较 save 字节，只看 `state_hash` 是不够的。
//!
//! 保留命令名约定（驱动面）：Mod 事件不得用 `save / load / state / log /
//! drain` 五个名字；`ygg.choice_selected` 为玩家选择的 respond 事件
//! （B2：选择 = 新外部事件），不受保留名限制。

use serde_json::{json, Value as Json};
use ygg_sandbox::driver::{KernelError, KernelFacade};
use ygg_sandbox::sandbox::ENGINE_VERSION;
/// 玩家选择门 Mod：入口事件挂起 choice；respond 事件按序号叙述选择结果。
const GATE_MOD: &str = r#"
ygg.on("event.start", function()
  ygg.choice("继续前行？", { "go", "stay" })
end)
ygg.on("ygg.choice_selected", function(index)
  ygg.narrate("picked=" .. tostring(index))
end)
"#;

/// 状态写入 Mod：装载期生成实体（hp=10）；hit 事件扣血并计数 KV——
/// handler 即席写经 host 桥直接落 canonical store（统一 store 代决）。
const STATE_MOD: &str = r#"
local id = ygg.entity.spawn()
ygg.entity.set_attribute(id, "hp", 10)
ygg.on("event.hit", function()
  local hp = ygg.entity.get_attribute(id, "hp")
  ygg.entity.set_attribute(id, "hp", hp - 1)
  ygg.kv.set("hits", (ygg.kv.get("hits") or 0) + 1)
end)
"#;

fn parse(reply: &str) -> Json {
    serde_json::from_str(reply).expect("reply must be valid JSON")
}

fn reply_ok(reply: &str) -> Json {
    let parsed = parse(reply);
    assert_eq!(parsed["ok"], json!(true), "expected ok reply: {reply}");
    parsed
}

fn reply_error_code(reply: &str) -> String {
    let parsed = parse(reply);
    assert_eq!(parsed["ok"], json!(false), "expected error reply: {reply}");
    parsed["error"]["code"]
        .as_str()
        .expect("error code")
        .to_owned()
}

/// 存档里的实体表（`save.entities.entities` = id 键控对象）。
fn saved_entities(kernel: &mut KernelFacade) -> Json {
    let envelope = reply_ok(&kernel.dispatch("save", ""));
    envelope["save"]["entities"]["entities"].clone()
}

/// 存档里的 KV 条目（`save.kv.entries`）。
fn saved_kv(kernel: &mut KernelFacade) -> Json {
    let envelope = reply_ok(&kernel.dispatch("save", ""));
    envelope["save"]["kv"]["entries"].clone()
}

mod diagnostics;
mod envelope;
mod error_mapping;
mod persistence;
mod rollback;
mod stdlib;
