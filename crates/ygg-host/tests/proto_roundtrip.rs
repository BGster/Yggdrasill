//! `ygg/1` 信封的往返与拒收用例。
//!
//! 三条断言面（计划稿 §1 消息表 + §0 决策 3）：
//! 1. **每个变体往返逐字节稳定**：`encode → decode → encode` 两次编码相等。字段序不要求
//!    与手写一致，但要求**稳定**——否则「同一份状态两次序列化给出两份字节」，归档与
//!    日志对账都失去意义。
//! 2. **未知 `type` / 未知字段即 `E_PROTOCOL`**：跨语言手写宿主的错字必须在第一次往返
//!    炸出来，不能静默读成缺省值。
//! 3. **浮点一律拒收**（含 `1.0`）：`{"type":"step","ticks":1.5,"dt":16}` 必须以
//!    [`ErrorCode::EProtocol`] 被拒，且错误文本里带上原文。

use ygg_host::proto::{ErrorCode, Message, Snapshot};

mod common;

use common::samples;

fn round_trip(message: &Message) -> String {
    let first = ygg_host::proto::encode(message).expect("序列化应成功");
    let decoded = ygg_host::proto::decode(&first).expect("自己编的字节必须自己能解");
    let second = ygg_host::proto::encode(&decoded).expect("再序列化应成功");
    assert_eq!(first, second, "往返后字节必须稳定：{first}");
    first
}

#[test]
fn every_variant_round_trips_byte_identically() {
    for message in samples() {
        round_trip(&message);
    }
}

#[test]
fn error_code_wire_labels_match_as_str() {
    for code in [
        ErrorCode::EProtocol,
        ErrorCode::EPack,
        ErrorCode::EBudget,
        ErrorCode::ERule,
        ErrorCode::EState,
    ] {
        let wire = serde_json::to_string(&code).expect("错误码序列化应成功");
        assert_eq!(
            wire,
            format!("\"{}\"", code.as_str()),
            "as_str 与 derive 的线上字面量必须逐字一致"
        );
    }
}

#[test]
fn discriminant_shapes_are_what_the_plan_table_says() {
    // 判别键与方向：`type` 在外层，`snapshot` 的请求/回执靠 `direction`。
    assert_eq!(
        round_trip(&Message::Snapshot(Snapshot::Request {})),
        r#"{"type":"snapshot","direction":"request"}"#
    );
    // `replay` 的回执是独立标签（两个变体不能共用一个 tag）。
    assert_eq!(
        round_trip(&Message::ReplayResult {
            ok: true,
            first_bad_tick: None
        }),
        r#"{"type":"replay_result","ok":true}"#
    );
    // 值的外部标签：一个键既是判别式又是载荷。
    assert_eq!(
        round_trip(&Message::Step { ticks: 1, dt: 16 }),
        r#"{"type":"step","ticks":1,"dt":16}"#
    );
}

#[test]
fn unknown_type_is_e_protocol() {
    let err = ygg_host::proto::decode(r#"{"type":"teleport","to":3}"#).unwrap_err();
    assert_eq!(err.code, ErrorCode::EProtocol, "未知 type：{err}");
    assert!(
        err.msg.contains("teleport"),
        "错误里要带上对端发来的 type：{err}"
    );
}

#[test]
fn unknown_field_is_e_protocol() {
    // 少一个 s：`tick` 而不是 `ticks`。缺字段与多字段都必须炸，不能读成缺省。
    let err = ygg_host::proto::decode(r#"{"type":"step","tick":1,"dt":16}"#).unwrap_err();
    assert_eq!(err.code, ErrorCode::EProtocol, "缺字段：{err}");

    let err =
        ygg_host::proto::decode(r#"{"type":"step","ticks":1,"dt":16,"bogus":0}"#).unwrap_err();
    assert_eq!(err.code, ErrorCode::EProtocol, "多字段：{err}");
    assert!(err.msg.contains("bogus"), "错误里要带上出错字段：{err}");

    // `submit` 里的操作对象同样拒未知字段（错字藏在数组里也要炸）。
    let err =
        ygg_host::proto::decode(r#"{"type":"submit","ops":[{"op":"despawn","id":[1],"ids":[1]}]}"#)
            .unwrap_err();
    assert_eq!(err.code, ErrorCode::EProtocol, "操作层未知字段：{err}");
}

#[test]
fn float_is_rejected_with_exact_code() {
    let err = ygg_host::proto::decode(r#"{"type":"step","ticks":1.5,"dt":16}"#).unwrap_err();
    assert_eq!(err.code, ErrorCode::EProtocol, "浮点 tick 数：{err}");
    assert!(err.msg.contains("1.5"), "错误里要带上收到的小数：{err}");

    // 1.0 同样是浮点：`serde_json` 记的是 f64 形态，不能截断成 1。
    let err = ygg_host::proto::decode(r#"{"type":"step","ticks":1.0,"dt":16}"#).unwrap_err();
    assert_eq!(err.code, ErrorCode::EProtocol, "整值浮点也不收：{err}");
}

#[test]
fn float_inside_a_world_value_is_rejected() {
    let err = ygg_host::proto::decode(
        r#"{"type":"submit","ops":[{"op":"set","id":7,"comps":[{"cid":1,"values":[{"int":1.5}]}]}]}"#,
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::EProtocol, "世界值里的浮点：{err}");
    assert!(err.msg.contains("1.5"), "错误里要带上原文：{err}");
}

#[test]
fn unsigned_fields_reject_negative_and_overflow() {
    let err = ygg_host::proto::decode(r#"{"type":"step","ticks":-1,"dt":16}"#).unwrap_err();
    assert_eq!(err.code, ErrorCode::EProtocol, "负数进 u32：{err}");

    let err = ygg_host::proto::decode(r#"{"type":"step","ticks":4294967296,"dt":16}"#).unwrap_err();
    assert_eq!(err.code, ErrorCode::EProtocol, "越界 u32：{err}");
    assert!(err.msg.contains("u32"), "错误里要说明是哪一面越界：{err}");
}

#[test]
fn zero_step_or_zero_spawn_is_rejected() {
    // 「推进 0 步」「生成 0 个实体」都是无效指令而不是空操作：收下它等于允许一条
    // 什么都不做但占一个往返的消息，而 schema 的 `minimum: 1` 已经判了它不合法。
    let err = ygg_host::proto::decode(r#"{"type":"step","ticks":0,"dt":16}"#).unwrap_err();
    assert_eq!(err.code, ErrorCode::EProtocol, "ticks=0：{err}");

    let err =
        ygg_host::proto::decode(r#"{"type":"submit","ops":[{"op":"spawn","n":0,"comps":[]}]}"#)
            .unwrap_err();
    assert_eq!(err.code, ErrorCode::EProtocol, "spawn n=0：{err}");

    // 而 `dt=0` 是合法的：零步长的一 tick 仍然是一 tick（暂停用），语义不同。
    assert!(ygg_host::proto::decode(r#"{"type":"step","ticks":1,"dt":0}"#).is_ok());
}

#[test]
fn snapshot_direction_selects_the_payload() {
    // 回执缺字段 → 拒（不是「回执可以没有存档」）。
    let err = ygg_host::proto::decode(r#"{"type":"snapshot","direction":"reply"}"#).unwrap_err();
    assert_eq!(err.code, ErrorCode::EProtocol, "回执缺载荷：{err}");

    // 请求带载荷 → 拒（schema 侧同一条 `additionalProperties:false`）。
    let err = ygg_host::proto::decode(r#"{"type":"snapshot","direction":"request","bytes":"AA"}"#)
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::EProtocol, "请求不该带载荷：{err}");

    let err = ygg_host::proto::decode(r#"{"type":"snapshot"}"#).unwrap_err();
    assert_eq!(err.code, ErrorCode::EProtocol, "没有 direction：{err}");
}

#[test]
fn explicit_null_is_not_a_default() {
    // `first_bad_tick` 缺省的含义是「字段不在」。显式 `null` 是另一件事，收下它等于给了
    // 两条表达缺省的路径——而 schema 那一侧只有 `integer`，两侧必须拒同一批输入。
    let err =
        ygg_host::proto::decode(r#"{"type":"replay_result","ok":true,"first_bad_tick":null}"#)
            .unwrap_err();
    assert_eq!(err.code, ErrorCode::EProtocol, "null 不是缺省：{err}");

    // 字段不在则正常缺省，且回写时也不会长出 `null`。
    let ok = ygg_host::proto::decode(r#"{"type":"replay_result","ok":true}"#).expect("缺省应可解");
    assert_eq!(
        ygg_host::proto::encode(&ok).expect("序列化应成功"),
        r#"{"type":"replay_result","ok":true}"#
    );
}

#[test]
fn handshake_versions_are_judged_at_decode() {
    // 版本/编码面在解码处判：v1 的 crate 读不懂 v2 的信封，先读进来再比字符串会把
    // 「读不懂」误报成「内容不对」。摘要**格式**同属这一面；摘要**内容**不符才是 E_PACK。
    let hello = |protocol: &str, codec: &str, hash: &str| {
        format!(
            r#"{{"type":"hello","protocol":"{protocol}","codec":"{codec}","pack_id":"ygg:demo","pack_hash":"{hash}"}}"#
        )
    };
    let hash = "0".repeat(64);

    let err = ygg_host::proto::decode(&hello("ygg/2", "json", &hash)).unwrap_err();
    assert_eq!(err.code, ErrorCode::EProtocol, "版本不符：{err}");
    assert!(err.msg.contains("ygg/2"), "错误里要带上对端版本：{err}");

    let err = ygg_host::proto::decode(&hello("ygg/1", "cbor", &hash)).unwrap_err();
    assert_eq!(err.code, ErrorCode::EProtocol, "编码不符：{err}");

    let err = ygg_host::proto::decode(&hello("ygg/1", "json", "abc")).unwrap_err();
    assert_eq!(err.code, ErrorCode::EProtocol, "摘要格式不合：{err}");

    assert!(ygg_host::proto::decode(&hello("ygg/1", "json", &hash)).is_ok());
}
