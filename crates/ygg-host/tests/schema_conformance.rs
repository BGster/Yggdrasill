//! schema 与 Rust 类型的一致性用例。
//!
//! 计划稿 §3 第 1 步要求「schema 与 Rust 类型由测试保证同步」。同步的做法不是比对两份
//! 文本（那是没法维系的），而是**让两侧对同一批字节表态**：
//!
//! 1. 每个 `Message` 变体一份样本（[`common::samples`]，与往返用例同一份）→ 编码成线上
//!    字节 → 用 `packages/schema/protocol/ygg-1.schema.json` 校验，必须零违规；
//! 2. 每个负例都成对给出「坏样本 → 好样本」，坏样本被拒**且**好样本被收——否则「schema
//!    拒了一切」也能让拒收断言通过，测试就变成了自欺；
//! 3. 两份已知的不对称显式断言（`1.0`、超 `u64` 字面量）：JSON Schema 的数据模型只有一种
//!    数，看不见词法形态，这一格的防线只能在 crate 边界（见 README 不变式 1）。
//!
//! schema 里每个 `$ref` 都必须落到 `$defs` 上——悬空引用会让校验器在构造期报错，但
//! 「构造成功」不等于「引用都对」，所以单独走一遍。

use ygg_host::proto::{decode, encode, CODEC, PROTOCOL};
use ygg_schema::validate::ValidatorSet;

mod common;

use common::samples;

/// schema 的 `$id`（同时也是 `ValidatorSet` 的注册键）。
const SCHEMA_ID: &str = "https://yggdrasill.dev/schemas/protocol/ygg-1.json";
/// 与本 crate 同仓的 schema 文本：编译期嵌入，改 schema 就会重编这条用例。
const SCHEMA: &str = include_str!("../../../packages/schema/protocol/ygg-1.schema.json");

fn set() -> ValidatorSet {
    ValidatorSet::from_texts([(SCHEMA_ID, SCHEMA)]).expect("ygg/1 schema 必须构造成功")
}

/// 拒收断言：返回被拒的违规条数（0 会让断言失败，同时把「为什么拒」的原文带上）。
fn reject(set: &ValidatorSet, label: &str, text: &str) {
    let instance: serde_json::Value =
        serde_json::from_str(text).expect("坏样本本身必须是合法 JSON");
    let issues = set
        .issues(SCHEMA_ID, label, &instance)
        .expect("校验调用不该失败");
    assert!(
        !issues.is_empty(),
        "{label} 应被 schema 拒收，实际零违规：{text}"
    );
}

fn accept(set: &ValidatorSet, label: &str, text: &str) {
    let instance: serde_json::Value = serde_json::from_str(text).expect("样本必须是合法 JSON");
    let issues = set
        .issues(SCHEMA_ID, label, &instance)
        .expect("校验调用不该失败");
    assert!(
        issues.is_empty(),
        "{label} 应被 schema 收下，实际 {} 条违规：{}",
        issues.len(),
        issues
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    );
}

/// 线上字节（而不是内存里的结构）必须逐份通过 schema：挡的是「Rust 编得出、schema 不认」
/// 这类分叉，而那正是手写宿主会踩的坑。
#[test]
fn schema_accepts_every_variant_sample() {
    let set = set();
    for message in samples() {
        let text = encode(&message).expect("序列化应成功");
        accept(&set, "sample", &text);
    }
}

/// 样本集的覆盖面钉在计划稿 §1 消息表上：漏一个变体就得动这张表，动表的人会看见它。
#[test]
fn sample_set_covers_every_wire_tag() {
    let mut tags: Vec<String> = Vec::new();
    for message in samples() {
        let text = encode(&message).expect("序列化应成功");
        let value: serde_json::Value = serde_json::from_str(&text).expect("编码结果必须是 JSON");
        let tag = value["type"].as_str().expect("判别键必须是字符串");
        // `snapshot` 是双向同名消息，方向属于判别面的一部分。
        match value["direction"].as_str() {
            Some(direction) => tags.push(format!("{tag}/{direction}")),
            None => tags.push(tag.to_owned()),
        }
    }
    tags.sort();
    tags.dedup();
    assert_eq!(
        tags,
        [
            "delta",
            "error",
            "events",
            "hello",
            "open",
            "ready",
            "replay",
            "replay_result",
            "snapshot/reply",
            "snapshot/request",
            "step",
            "submit",
        ]
    );
}

/// 每个 `$ref` 都要落到 `$defs`；`hello.protocol` / `hello.codec` 的 `const` 与 crate 常量
/// 必须同值——这两格是版本面，改一处不改另一处就是协议分叉。
#[test]
fn every_ref_resolves_and_version_consts_match_the_crate() {
    let schema: serde_json::Value = serde_json::from_str(SCHEMA).expect("schema 文本必须是 JSON");
    let defs = schema["$defs"].as_object().expect("schema 必须有 $defs");

    let mut dangling: Vec<String> = Vec::new();
    let mut seen: usize = 0;
    walk_refs(&schema, &mut |reference| {
        seen += 1;
        if let Some(name) = reference.strip_prefix("#/$defs/") {
            if !defs.contains_key(name) {
                dangling.push(reference.to_owned());
            }
        }
    });
    assert!(seen > 0, "schema 里应至少有一条 $ref");
    assert!(dangling.is_empty(), "$defs 里找不到这些引用：{dangling:?}");

    let hello = &defs["hello"]["properties"];
    assert_eq!(hello["protocol"]["const"], PROTOCOL);
    assert_eq!(hello["codec"]["const"], CODEC);
}

/// 递归收集 `$ref`（只认字符串值，`$defs` 里允许嵌套引用）。
fn walk_refs(value: &serde_json::Value, visit: &mut impl FnMut(&str)) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, inner) in map {
                if key == "$ref" {
                    if let Some(reference) = inner.as_str() {
                        visit(reference);
                    }
                } else {
                    walk_refs(inner, visit);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for inner in items {
                walk_refs(inner, visit);
            }
        }
        _ => {}
    }
}

/// 坏样本/好样本成对：拒的是**那一个字段**，不是一个整体坏掉的样本。
#[test]
fn schema_and_rust_reject_the_same_shape() {
    let set = set();
    let cases: [(&str, &str, &str); 18] = [
        (
            "未知 type",
            r#"{"type":"teleport","to":3}"#,
            r#"{"type":"step","ticks":1,"dt":16}"#,
        ),
        (
            "step 多一个字段",
            r#"{"type":"step","ticks":1,"dt":16,"bogus":0}"#,
            r#"{"type":"step","ticks":1,"dt":16}"#,
        ),
        (
            "ticks 是小数",
            r#"{"type":"step","ticks":1.5,"dt":16}"#,
            r#"{"type":"step","ticks":1,"dt":16}"#,
        ),
        (
            "ticks 为负",
            r#"{"type":"step","ticks":-1,"dt":16}"#,
            r#"{"type":"step","ticks":1,"dt":16}"#,
        ),
        (
            "ticks 为 0",
            r#"{"type":"step","ticks":0,"dt":16}"#,
            r#"{"type":"step","ticks":1,"dt":16}"#,
        ),
        (
            "step 缺 dt",
            r#"{"type":"step","ticks":1}"#,
            r#"{"type":"step","ticks":1,"dt":16}"#,
        ),
        (
            "组件 kind 不在枚举内",
            r#"{"type":"ready","tick":0,"hash":0,"sealed":true,"components":[{"id":1,"name":"hp","kind":"float"}],"systems":[]}"#,
            r#"{"type":"ready","tick":0,"hash":0,"sealed":true,"components":[{"id":1,"name":"hp","kind":"number"}],"systems":[]}"#,
        ),
        (
            "错误码不在枚举内",
            r#"{"type":"error","code":"E_NOPE","msg":"x"}"#,
            r#"{"type":"error","code":"E_PROTOCOL","msg":"x"}"#,
        ),
        (
            "snapshot 请求带载荷",
            r#"{"type":"snapshot","direction":"request","bytes":"AA"}"#,
            r#"{"type":"snapshot","direction":"request"}"#,
        ),
        (
            "snapshot 缺 direction",
            r#"{"type":"snapshot","tick":0,"hash":0,"bytes":"AA"}"#,
            r#"{"type":"snapshot","direction":"reply","tick":0,"hash":0,"bytes":"AA"}"#,
        ),
        (
            "replay_result 的 first_bad_tick 是 null",
            r#"{"type":"replay_result","ok":true,"first_bad_tick":null}"#,
            r#"{"type":"replay_result","ok":true}"#,
        ),
        (
            "世界值带了两个变体键",
            r#"{"type":"submit","ops":[{"op":"set","id":1,"comps":[{"cid":1,"values":[{"int":1,"str":"x"}]}]}]}"#,
            r#"{"type":"submit","ops":[{"op":"set","id":1,"comps":[{"cid":1,"values":[{"int":1}]}]}]}"#,
        ),
        (
            "op 判别键不在枚举内",
            r#"{"type":"submit","ops":[{"op":"move","id":1}]}"#,
            r#"{"type":"submit","ops":[{"op":"despawn","ids":[1]}]}"#,
        ),
        (
            "spawn 的 n 为 0",
            r#"{"type":"submit","ops":[{"op":"spawn","n":0,"comps":[]}]}"#,
            r#"{"type":"submit","ops":[{"op":"spawn","n":1,"comps":[]}]}"#,
        ),
        (
            "eid 超出 u64",
            r#"{"type":"delta","tick":1,"changed":[{"eid":18446744073709551616,"comps":[]}]}"#,
            r#"{"type":"delta","tick":1,"changed":[{"eid":18446744073709551615,"comps":[]}]}"#,
        ),
        (
            "hello 说的是别的协议版本",
            r#"{"type":"hello","protocol":"ygg/2","codec":"json","pack_id":"ygg:demo","pack_hash":"0000000000000000000000000000000000000000000000000000000000000000"}"#,
            r#"{"type":"hello","protocol":"ygg/1","codec":"json","pack_id":"ygg:demo","pack_hash":"0000000000000000000000000000000000000000000000000000000000000000"}"#,
        ),
        (
            "hello 说的是别的编码",
            r#"{"type":"hello","protocol":"ygg/1","codec":"cbor","pack_id":"ygg:demo","pack_hash":"0000000000000000000000000000000000000000000000000000000000000000"}"#,
            r#"{"type":"hello","protocol":"ygg/1","codec":"json","pack_id":"ygg:demo","pack_hash":"0000000000000000000000000000000000000000000000000000000000000000"}"#,
        ),
        (
            "pack_hash 不是 64 位十六进制",
            r#"{"type":"hello","protocol":"ygg/1","codec":"json","pack_id":"ygg:demo","pack_hash":"abc"}"#,
            r#"{"type":"hello","protocol":"ygg/1","codec":"json","pack_id":"ygg:demo","pack_hash":"0000000000000000000000000000000000000000000000000000000000000000"}"#,
        ),
    ];

    for (label, broken, fixed) in cases {
        accept(&set, label, fixed);
        reject(&set, label, broken);
        // 同一批形状在 Rust 侧也必须被拒——「两侧拒同一批输入」才是同步的定义。
        assert!(
            decode(broken).is_err(),
            "{label} 在 Rust 侧竟被收下：{broken}"
        );
        assert!(
            decode(fixed).is_ok(),
            "{label} 的好样本在 Rust 侧竟被拒：{}",
            decode(fixed).unwrap_err()
        );
    }
}

/// 已知不对称之一：JSON Schema 的 `integer` 指「数值上无小数部分」，`1.0` 算整数；
/// `serde_json` 记得的是 f64 形态，所以 crate 边界能把它拒在门外。这一格只能由边界守。
#[test]
fn schema_cannot_tell_one_from_one_point_zero() {
    let set = set();
    // schema 收：它看到的是 Number(1.0)，数值上是整数。
    accept(
        &set,
        "1.0 被 schema 收下",
        r#"{"type":"step","ticks":1.0,"dt":16}"#,
    );
    // Rust 拒：`strict::u32` 按 `Number` 形态判定，f64 一律拒。
    assert!(
        decode(r#"{"type":"step","ticks":1.0,"dt":16}"#).is_err(),
        "crate 边界必须拒 1.0"
    );
}
