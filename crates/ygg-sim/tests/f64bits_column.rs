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
