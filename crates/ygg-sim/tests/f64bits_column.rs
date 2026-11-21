//! 位串列（`AttrType::F64Bits` / `Column::F64Bits`）的确定性断言集（sim-kernel §3 I2）。
//!
//! 本文件覆盖计划 §7 的六条：规范化向量、全序、同 seed 位串逐值 hash、回放 + 篡改负例、
//! v1 幂等、scan 分母。C1 先落批量 API 基础面（spawn 帧路径 / wire tag / 类型错配拒），
//! C2 补存档 v2 性质，C3 补齐余下断言。

use ygg_sim::archetype::{Column, ValueRef};
use ygg_sim::canonical::FrameWriter;
use ygg_sim::component::{AttrType, ComponentDef};
use ygg_sim::error::codes;
use ygg_sim::sim::Sim;

/// `f64` 位串常量（tests 面不受 R1′ 限，直接写 hex 便于对账）。
const NAN_BITS: u64 = 0x7FF8_0000_0000_0000;

/// 造 `spawn_batch` 帧：`names` 是组件名序，`bit_columns` 与之一一对应（F64Bits 列）。
fn spawn_bits_frame(rows: usize, names: &[&str], bit_columns: &[Vec<u64>]) -> Vec<u8> {
    let mut writer = FrameWriter::new();
    writer.u32(rows as u32);
    writer.u16(names.len() as u16);
    for name in names {
        writer.u16(name.len() as u16);
        writer.raw(name.as_bytes());
    }
    for column in bit_columns {
        for bits in column {
            writer.u64(*bits);
        }
    }
    writer.finish()
}

/// 批量帧里的位串列必须 canonical（读边界 fail-closed）。
#[test]
fn batch_frame_rejects_non_canonical_bits() {
    let mut sim = Sim::new(7);
    sim.register("Gauge", ComponentDef::f64_bits()).unwrap();
    // 先落一帧 canonical（1.0），证明正路畅通。
    sim.spawn_batch(&spawn_bits_frame(
        1,
        &["Gauge"],
        &[vec![0x3FF0_0000_0000_0000]],
    ))
    .unwrap();
    // -0 位串（非 canonical）必拒
    let broken = spawn_bits_frame(1, &["Gauge"], &[vec![0x8000_0000_0000_0000]]);
    let err = sim.spawn_batch(&broken).unwrap_err();
    assert_eq!(err.code(), codes::FRAME, "-0 位串必须被读边界拒：{err}");

    // 带载荷的 NaN（非规范 quiet）同样拒
    let payload_nan = spawn_bits_frame(1, &["Gauge"], &[vec![0x7FF8_0000_0000_0042]]);
    let err = sim.spawn_batch(&payload_nan).unwrap_err();
    assert_eq!(err.code(), codes::FRAME, "非规范 NaN 载荷必须被拒：{err}");
}

/// 写入边界归一：`ValueRef::Bits` 携带原始位串，入列即 pack（±0→+0、NaN→NAN_BITS）。
#[test]
fn push_value_normalizes_at_the_boundary() {
    let gauge_at = 0u32; // 单组件注册表里的 id（下同，仅语义命名）
    let mut probe = Sim::new(11);
    let gauge = probe.register("Gauge", ComponentDef::f64_bits()).unwrap();
    assert_eq!(gauge, gauge_at);
    let rows = 3usize;
    let mut column = Column::F64Bits(Vec::new());
    for raw in [
        0x8000_0000_0000_0000u64,
        0x7FF0_0000_0000_0001,
        0xBFF0_0000_0000_0000,
    ] {
        column.push_value(&ValueRef::Bits(raw), gauge).unwrap();
    }
    let ids = probe
        .world_mut()
        .spawn_columns(rows, &[(gauge, column)], &[])
        .unwrap();
    assert_eq!(ids.len(), 3);
    let stored: Vec<u64> = (0..3)
        .map(
            |row| match probe.world().value_of(ids[row], gauge).unwrap() {
                ValueRef::Bits(bits) => bits,
                other => panic!("期望 Bits，得到 {other:?}"),
            },
        )
        .collect();
    // -0 → +0、sNaN → NAN_BITS、普通负数原样——写入即归一（sim-kernel I2）。
    assert_eq!(stored, vec![0, NAN_BITS, 0xBFF0_0000_0000_0000]);
}

/// 类型错配拒：Bits 值写进 I64 列（或 I64 值写进 F64Bits 列）即 `YGG-SIM-SCHEMA`。
#[test]
fn type_mismatch_is_rejected_as_schema_error() {
    let mut sim = Sim::new(3);
    let gauge = sim.register("Gauge", ComponentDef::f64_bits()).unwrap();
    let counter = sim.register("Counter", ComponentDef::number()).unwrap();
    let mut bits_col = Column::F64Bits(Vec::new());
    let err = bits_col.push_value(&ValueRef::I64(5), gauge).unwrap_err();
    assert_eq!(err.code(), codes::SCHEMA);
    let mut i64_col = Column::I64(Vec::new());
    let err = i64_col
        .push_value(&ValueRef::Bits(0x3FF0_0000_0000_0000), counter)
        .unwrap_err();
    assert_eq!(err.code(), codes::SCHEMA);
}

/// wire 词汇：`f64bits` label、tag 6、from_wire 往返。
#[test]
fn wire_vocabulary_round_trips() {
    let ty = AttrType::F64Bits;
    assert_eq!(ty.label(), "f64bits");
    assert_eq!(ty.wire_tag(), 6);
    assert_eq!(AttrType::from_wire(6, 0).unwrap(), ty);
    assert_eq!(ty, ComponentDef::f64_bits().ty);
    assert!(ty.has_column());
    // tag 字节越界照旧 fail-closed
    assert_eq!(AttrType::from_wire(7, 0).unwrap_err().code(), codes::FRAME);
}

/// 同 seed 同帧 spawn：位串逐值相等，state hash 相等（确定性最小样本，C3 扩到逐 tick）。
#[test]
fn same_seed_same_frame_same_hash() {
    let build = || -> Sim {
        let mut sim = Sim::new(20_261_121);
        sim.register("Gauge", ComponentDef::f64_bits()).unwrap();
        sim.spawn_batch(&spawn_bits_frame(
            4,
            &["Gauge"],
            &[vec![
                0x3FF0_0000_0000_0000,
                0xC008_0000_0000_0000, // -3.0
                0x7FF0_0000_0000_0000, // +Inf
                0x0000_0000_0000_0001, // 次正规
            ]],
        ))
        .unwrap();
        sim
    };
    let a = build();
    let b = build();
    let gauge_a = a.world().registry().id_of("Gauge").unwrap();
    for row in 0..4 {
        assert_eq!(
            a.world().value_of(1 + row as u64, gauge_a).unwrap(),
            b.world().value_of(1 + row as u64, gauge_a).unwrap()
        );
    }
    assert_eq!(a.hash(), b.hash());
}

// ─────────────────────────────────────────────────────────────────────────────
// §7 断言集（计划）：①规范化向量（pack 表驱动在 src/bits.rs 单测；本文件钉快照读边界）
// ②全序（bits.rs 单测钉私有面，本条走公共 API 复证）③同 seed 逐 tick hash ④回放+篡改
// 负例 ⑤v1 幂等（落在 tests/snapshot_roundtrip.rs，C2）⑥scan 分母+白名单独立断言。
// ─────────────────────────────────────────────────────────────────────────────

use std::cmp::Ordering;

use ygg_sim::archetype::Column as ArchColumn;
use ygg_sim::bits;
use ygg_sim::input::ReplayLog;
use ygg_sim::snapshot;

/// §7.1（读边界·快照路径）：绕过写边界把列值改成 -0 后，snapshot encode 不拦
/// （编码器不校验内存），但 decode 的 read_column 逐值 canonical 断言必拒。
#[test]
fn snapshot_read_rejects_non_canonical_bits() {
    let mut sim = Sim::new(7);
    sim.register("Gauge", ComponentDef::f64_bits()).unwrap();
    sim.spawn_batch(&spawn_bits_frame(
        2,
        &["Gauge"],
        &[vec![0x3FF0_0000_0000_0000, 0x4000_0000_0000_0000]],
    ))
    .unwrap();
    // 模拟内存损坏 / 伪造：直改列内存（写边界之外），把第二行改成 -0
    for archetype in sim.world_mut().archetypes_mut() {
        if let ArchColumn::F64Bits(values) = &mut archetype.cols[0] {
            values[1] = 0x8000_0000_0000_0000;
        }
    }
    let frame = sim.snapshot();
    let err = snapshot::decode(&frame).unwrap_err();
    assert_eq!(
        err.code(),
        codes::FRAME,
        "快照读边界必须拒非 canonical：{err}"
    );
    // 带 payload 的 NaN 同拒
    for archetype in sim.world_mut().archetypes_mut() {
        if let ArchColumn::F64Bits(values) = &mut archetype.cols[0] {
            values[1] = 0x7FF8_0000_0000_0042;
        }
    }
    let frame = sim.snapshot();
    let err = snapshot::decode(&frame).unwrap_err();
    assert_eq!(err.code(), codes::FRAME, "快照读边界必须拒 NaN 载荷：{err}");
}

/// §7.2 全序（公共 API 复证）：pack 之后的位串序与 `f64::total_cmp` 同序；
/// NaN 归一使「任意 NaN 载荷」收敛为单一键。
#[test]
fn order_key_agrees_with_total_cmp_via_public_api() {
    let values = [
        0.0f64,
        -0.0,
        1.0,
        -1.0,
        2.5,
        -3.5,
        5e-324,                 // 最小次正规
        -5e-324,                // 最小负次正规
        1.7976931348623157e308, // max
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
    ];
    let keys: Vec<u64> = values.iter().map(|value| bits::pack(*value)).collect();
    // 归一：±0 → 同键；任意 NaN → NAN_BITS
    assert_eq!(keys[0], keys[1], "±0 必须同键");
    assert_eq!(keys[11], bits::NAN_BITS);
    // 对照域 = 归一后的值域：键序 ⟺ 归一值上的 totalOrder（±0 折叠为 +0 是设计决策，
    // total_cmp 对 ±0 的有序性在归一域里不出现；NaN 收敛为 NAN_BITS 后同位串互比 Equal）。
    let canon: Vec<f64> = keys.iter().map(|key| bits::unpack(*key)).collect();
    let key_of: Vec<u64> = keys.iter().map(|key| bits::order_key(*key)).collect();
    for i in 0..keys.len() {
        assert!(bits::is_canonical(keys[i]));
        for j in 0..keys.len() {
            let key_order = key_of[i].cmp(&key_of[j]);
            let value_order = canon[i].total_cmp(&canon[j]);
            assert_eq!(
                key_order, value_order,
                "keys[{i}] vs keys[{j}] 与归一域 total_cmp 分叉"
            );
            if keys[i] != keys[j] {
                assert_ne!(key_order, Ordering::Equal);
            }
        }
    }
}

/// §7.3 同 seed 位串逐值：两 Sim 同帧 spawn → 逐值 bits 相等 → 同 tick 数 step →
/// 逐 tick hash 全等（系统做位串搬运 + rng 派生写入，rng 流也进 hash）。
fn bits_relay_world(seed: u64) -> Sim {
    use ygg_sim::rng::stream_name;

    let mut sim = Sim::new(seed);
    let src = sim.register("Src", ComponentDef::f64_bits()).unwrap();
    let dst = sim.register("Dst", ComponentDef::f64_bits()).unwrap();
    // 系统：每 tick 把 Src 的位串搬进 Dst（仅搬运不算术）；每 7 tick 用 rng 派生一个
    // 次正规位型全列写入 Src（尾数域掩码内任意 u64 都是 canonical 次正规）。
    sim.add_system("relay", move |ctx| {
        for archetype in ctx.store_mut().archetypes_mut() {
            let (Some(src_at), Some(dst_at)) = (archetype.col_pos(src), archetype.col_pos(dst))
            else {
                continue;
            };
            let rows = archetype.ids.len();
            for row in 0..rows {
                let carried = match archetype.cols[src_at].value_at(row) {
                    ValueRef::Bits(bits) => bits,
                    _ => continue,
                };
                if let ArchColumn::F64Bits(values) = &mut archetype.cols[dst_at] {
                    values[row] = carried;
                }
            }
        }
        if ctx.tick() % 7 == 0 {
            let draw = ctx.rng32(&stream_name("seed_bits", 0)) as u64;
            let seeded = draw & 0x000F_FFFF_FFFF_FFFF;
            for archetype in ctx.store_mut().archetypes_mut() {
                if let Some(src_at) = archetype.col_pos(src) {
                    if let ArchColumn::F64Bits(values) = &mut archetype.cols[src_at] {
                        for value in values.iter_mut() {
                            *value = seeded;
                        }
                    }
                }
            }
        }
        Ok(())
    });
    sim.spawn_batch(&spawn_bits_frame(
        3,
        &["Src", "Dst"],
        &[
            vec![
                0x3FF0_0000_0000_0000,
                0xC008_0000_0000_0000,
                0x7FF0_0000_0000_0000,
            ],
            vec![0, 0, 0],
        ],
    ))
    .unwrap();
    sim
}

#[test]
fn same_seed_bit_streams_hash_identically_per_tick() {
    const TICKS: usize = 30;
    let mut a = bits_relay_world(20_261_122);
    let mut b = bits_relay_world(20_261_122);
    // 逐值（spawn 后立刻对账）
    let src = a.world().registry().id_of("Src").unwrap();
    let dst = a.world().registry().id_of("Dst").unwrap();
    for row in 0..3u64 {
        assert_eq!(
            a.world().value_of(1 + row, src),
            b.world().value_of(1 + row, src)
        );
        assert_eq!(
            a.world().value_of(1 + row, dst),
            b.world().value_of(1 + row, dst)
        );
    }
    a.trace(true);
    for _ in 0..TICKS {
        a.step(ygg_sim::fixed::ONE / 60).unwrap();
        b.step(ygg_sim::fixed::ONE / 60).unwrap();
        assert_eq!(a.hash(), b.hash(), "同 seed 同帧必须逐 tick 同 hash");
    }
    // trace 与独立 hash 对账（不是「断言恒真」）
    let trace = a.trace_entries().unwrap();
    assert_eq!(trace.len(), TICKS);
    let mut expected = bits_relay_world(20_261_122);
    for (index, (tick, hash)) in trace.iter().enumerate() {
        expected.step(ygg_sim::fixed::ONE / 60).unwrap();
        assert_eq!(*tick, (index + 1) as u64);
        assert_eq!(*hash, expected.hash());
    }
}

/// §7.4 回放：含位串写入的系统 restore+replay 逐 tick hash 一致；
/// 篡改日志里 spawn 帧的一个位串（换成另一个 canonical 值并重算帧 digest）必失配。
#[test]
fn replay_with_bits_matches_and_tamper_fails() {
    const DT: i64 = ygg_sim::fixed::ONE / 60;
    let mut sim = bits_relay_world(20_261_123);
    for _ in 0..10 {
        sim.step(DT).unwrap();
    }
    let snap = sim.snapshot();
    sim.record(true);
    // 第 12 tick 再 spawn 一帧（结构操作进日志，回放重投）
    for tick in 11..=30 {
        if tick == 12 {
            sim.spawn_batch(&spawn_bits_frame(
                1,
                &["Src", "Dst"],
                &[vec![0x4000_0000_0000_0000], vec![0xBFF0_0000_0000_0000]],
            ))
            .unwrap();
        }
        sim.step(DT).unwrap();
    }
    let log = sim.take_log().expect("录制中的日志").encode();
    let final_hash = sim.hash();

    // 正路：restore + replay 逐 tick 一致
    let report = sim.replay(&snap, &log).unwrap();
    assert_eq!(report.ticks, 20);
    assert_eq!(report.matched, report.ticks);
    assert_eq!(report.first_mismatch, None);
    assert_eq!(
        report.final_hash, final_hash,
        "回放终点 hash 与原始运行一致"
    );

    // 负例：把日志里 spawn 记录的一个位串改成另一个 canonical 值（合法帧、不同状态）
    let mut tampered = ReplayLog::decode(&log).unwrap();
    'find: {
        for record in tampered.records_mut() {
            for op in record.ops.iter_mut() {
                // 批量帧自带 FrameWriter 头：magic 8 | fmt_version 2 | flags 2 | rows 4
                // | comp_n 2 | 名单（2B len + "Src" 3B + 2B len + "Dst" 3B）= 28 字节起
                // 是载荷。本帧 1 行 2 列 → 28..36 是 Src 首行位串。
                if op.frame.len() > 44 {
                    op.frame[28] ^= 0x08; // 0x4000_0000_0000_0000 → 0x4000_…_0008（同为 canonical 正数）
                    let payload_len = op.frame.len() - 8;
                    let digest = ygg_sim::hash::fnv1a64(&op.frame[..payload_len]);
                    op.frame[payload_len..].copy_from_slice(&digest.to_le_bytes());
                    break 'find;
                }
            }
        }
        panic!("日志里没有可篡改的 spawn 帧");
    }
    let tampered_bytes = tampered.encode();
    let report = sim.replay(&snap, &tampered_bytes).unwrap();
    assert_eq!(
        report.first_mismatch,
        Some(12),
        "篡改的 spawn 恰在 tick 12 重投，必须在该 tick 显形为 hash 失配"
    );
    // 末 tick hash 允许收敛（relay 系统在 tick 14 用 rng 派生值全列覆写 Src，
    // 篡改被合法冲刷）——负例的判据是「失配发生」，不是「终态不同」。
    let _ = final_hash;
}

/// §7.6 bits.rs 白名单面独立断言：算术运算符字符零命中（独立于 determinism_scan
/// 自身的实现，防「扫描器坏了但两边一起坏」）。
#[test]
fn bits_face_has_no_arithmetic_characters_independently() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("src/bits.rs");
    let text = std::fs::read_to_string(&path).expect("读 src/bits.rs");
    // 剥行注释（bits.rs 不用块注释；字符串内的 // 不存在，逐字符状态机兜底）
    let bytes = text.as_bytes();
    let mut stripped = String::with_capacity(text.len());
    let mut in_string = false;
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if byte == b'\\' && index + 1 < bytes.len() {
                index += 2;
                continue;
            }
            if byte == b'"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if byte == b'"' {
            in_string = true;
            index += 1;
            continue;
        }
        if byte == b'/' && index + 1 < bytes.len() && bytes[index + 1] == b'/' {
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            continue;
        }
        stripped.push(byte as char);
        index += 1;
    }
    // 字符串内容丢弃（错误消息文本不参与）；返回箭头占位（它含 '-' 但不是运算符）
    let no_strings: String = {
        let mut out = String::with_capacity(stripped.len());
        let mut inside = false;
        for ch in stripped.chars() {
            if inside {
                if ch == '"' {
                    inside = false;
                    out.push('"');
                }
                continue;
            }
            if ch == '"' {
                inside = true;
                out.push('"');
                continue;
            }
            out.push(ch);
        }
        out
    };
    let code = no_strings.replace("->", " ");
    let hits: Vec<char> = code
        .chars()
        .filter(|ch| matches!(ch, '+' | '-' | '*' | '/' | '%'))
        .collect();
    assert!(
        hits.is_empty(),
        "src/bits.rs 白名单面出现算术运算符字符 {hits:?}（sim-kernel I2 被破坏）"
    );
    // 分母：文件非空且确实扫到了代码体（防止「读错文件静默通过」）
    assert!(code.contains("pub fn pack"));
    assert!(code.contains("order_key"));
    println!("f64bits_column.bits_face bytes={} hits=0", code.len());
}
