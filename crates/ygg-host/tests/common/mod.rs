//! 两份用例共用的样本集：每个 `Message` 变体一份。
//!
//! 为什么放在公共模块：往返用例断言「字节稳定」，schema 用例断言「schema 认同一份字节」，
//! 两者必须**看同一批样本**，否则 schema 与 Rust 类型悄悄分叉时两边都绿。样本集只有一份，
//! 分叉就只能靠改这一份来发生，改的时候两个用例一起炸。

use ygg_host::proto::{
    Changed, Component, ComponentKind, ComponentValues, ErrorCode, Frame, Log, LogHeader, LogLevel,
    LogLine, Message, Op, ProtocolError, Snapshot, Stage, SystemInfo, Value, CODEC, PROTOCOL,
};

fn message_samples() -> Vec<Message> {
    vec![
        Message::Hello {
            protocol: PROTOCOL.to_owned(),
            codec: CODEC.to_owned(),
            pack_id: "ygg:draughts".to_owned(),
            pack_hash: "0".repeat(64),
        },
        Message::Ready {
            tick: 0,
            hash: 0xcbf2_9ce4_8422_2325,
            sealed: true,
            components: vec![Component {
                id: 3,
                name: "hp".to_owned(),
                kind: ComponentKind::Number,
            }],
            systems: vec![SystemInfo {
                name: "damage".to_owned(),
                stage: Stage::Rules,
            }],
        },
        Message::Open {
            bytes: "AAEC".to_owned(),
        },
        Message::Submit {
            ops: vec![
                Op::Spawn {
                    n: 2,
                    comps: vec![
                        ComponentValues {
                            cid: 1,
                            values: vec![Value::Int(10)],
                        },
                        ComponentValues {
                            cid: 2,
                            values: vec![Value::Str("pawn".to_owned())],
                        },
                    ],
                },
                Op::Despawn { ids: vec![7, 9] },
                Op::Set {
                    id: 7,
                    comps: vec![ComponentValues {
                        cid: 1,
                        values: vec![Value::Int(-3)],
                    }],
                },
                Op::Insert {
                    id: 7,
                    comps: vec![ComponentValues {
                        cid: 4,
                        values: vec![Value::Uint(11)],
                    }],
                },
                Op::Remove {
                    id: 7,
                    cids: vec![1, 2],
                },
            ],
        },
        Message::Step { ticks: 1, dt: 16 },
        Message::Events {
            tick: 1,
            hash: 0,
            log: vec![LogLine {
                level: LogLevel::Info,
                text: "moved".to_owned(),
            }],
            errors: vec![ProtocolError::new(ErrorCode::ERule, "hp 越界")],
        },
        Message::Delta {
            tick: 1,
            changed: vec![Changed {
                eid: 7,
                comps: vec![ComponentValues {
                    cid: 1,
                    values: vec![Value::Int(9)],
                }],
            }],
        },
        Message::Snapshot(Snapshot::Request {}),
        Message::Snapshot(Snapshot::Reply {
            tick: 1,
            hash: 0,
            bytes: "AAEC".to_owned(),
        }),
        Message::Replay {
            log: Log {
                header: LogHeader {
                    seed: 0x0123_4567_89ab_cdef,
                    start_tick: 0,
                },
                frames: vec![Frame {
                    tick: 1,
                    dt: 16,
                    ops: vec![Op::Despawn { ids: vec![7] }],
                    hash: 0,
                }],
            },
        },
        Message::ReplayResult {
            ok: true,
            first_bad_tick: None,
        },
        Message::ReplayResult {
            ok: false,
            first_bad_tick: Some(42),
        },
        Message::Error(ProtocolError::new(ErrorCode::EPack, "pack_hash 不符")),
    ]
}

/// 全部样本（每个 `Message` 变体至少一份；`replay_result` 两种形态各一份）。
pub fn samples() -> Vec<Message> {
    message_samples()
}
