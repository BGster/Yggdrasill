//! 快照编解码：自描述、名称键控、尾 digest 的二进制列帧（sim-kernel §2）。
//!
//! # 帧布局（逐字节，改这里就要升 [`crate::canonical::FMT_VERSION`]）
//!
//! ```text
//! magic       8B   "YGGSIM\0\x01"
//! fmt_version 2B   LE = 1
//! flags       2B   LE = 0
//! tick        8B   LE
//! next_entity 8B   LE
//! comp_n      4B   LE
//!   × comp_n: name_len 2B | name UTF-8 | type 1B(1..5) | frac_bits 1B
//!            （按 name 字节序升序；下面的 archetype 段按**这个表的序**引用组件）
//! res_n       4B   LE            （键升序）
//!   × res_n: key 8B | value 8B
//! arch_n      4B   LE            （按「升序组件名串联」的字节序升序）
//!   × arch_n: comp_cnt 2B | comp_idx 2B×comp_cnt | row_n 4B
//!            | ids: u64×row_n  | 每列载荷（顺序 = comp_idx 顺序）:
//!                 I64/U64: 8B×row_n     U8: 1B×row_n     Bytes: (len 4B | bytes)×row_n
//! digest      8B   LE  = fnv1a64(前面全部字节) = state hash
//! ```
//!
//! # 三条硬性质（`tests/snapshot_roundtrip.rs` 逐条钉住）
//!
//! 1. **字节稳定**：同态两次 `encode` 逐字节相同（无 HashMap、无时间、无指针）。
//! 2. **往返幂等**：`decode(b).encode() == b`；`state_hash(b)` 与 decode 后的重编一致。
//! 3. **名称键控 ⇒ id 无关**：以两种不同顺序注册同一组件集，内部 id 不同，但快照字节与
//!    hash **完全相同**——因为组件表按名称排序，archetype 段按名称引用。这条同时证明
//!    「跨进程一致性不依赖注册顺序」。
//!
//! # 为什么自描述（组件名表 + 类型）而不是「编译期 schema」
//!
//! 存档要能被**另一个进程**读（跨进程 digest 是 I1 的唯一硬证据），而内部 id 与注册序
//! 是进程局部的。把名字与类型写进帧，读取方就不需要事先知道对方的注册顺序——少一条
//! 「两边必须按同一顺序注册」的隐式契约。代价是每帧多几百字节，10⁵ 实体的帧里可忽略。

use std::collections::BTreeMap;

use crate::archetype::{Archetype, Column};
use crate::canonical::{FrameReader, FrameWriter};
use crate::component::{AttrType, CompSet, ComponentDef, EntityId, Registry};
use crate::error::{SimError, SimResult};
use crate::store::ArchetypeStore;

/// 一次解码的结果（`Sim` 的 `restore` 把它写回自己的字段）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotState {
    /// tick 计数。
    pub tick: u64,
    /// 资源表（`KV<u64>`：键升序）。
    pub resources: BTreeMap<u64, u64>,
    /// 世界（注册表已按**名称升序**重建）。
    pub store: ArchetypeStore,
}

/// 编码一帧快照（含尾 digest）。
pub fn encode(tick: u64, resources: &BTreeMap<u64, u64>, store: &ArchetypeStore) -> Vec<u8> {
    let registry = store.registry();
    // 组件表的规范序：名称字节序升序。
    let mut table: Vec<(u32, &str, AttrType)> = Vec::with_capacity(registry.len());
    for id in 0..registry.len() as u32 {
        table.push((
            id,
            registry.name_of(id).unwrap_or(""),
            registry.type_of(id).unwrap_or(AttrType::Tag),
        ));
    }
    table.sort_by(|a, b| a.1.as_bytes().cmp(b.1.as_bytes()));

    let mut writer = FrameWriter::new();
    writer.u64(tick);
    writer.u64(store.next_id());
    writer.u32(table.len() as u32);
    for (_, name, ty) in &table {
        writer.u16(name.len() as u16);
        writer.raw(name.as_bytes());
        writer.u8(ty.wire_tag());
        writer.u8(match ty {
            AttrType::Number { frac_bits } => *frac_bits,
            _ => 0,
        });
    }

    writer.u32(resources.len() as u32);
    for (key, value) in resources {
        writer.u64(*key);
        writer.u64(*value);
    }

    // canonical index：内部 id → 组件表下标（名称序）。
    let mut canon_of: Vec<u16> = vec![0; registry.len()];
    for (canon_idx, (internal, _, _)) in table.iter().enumerate() {
        canon_of[*internal as usize] = canon_idx as u16;
    }

    // archetype 段的规范序：按「升序组件名串联」比较；等价于按组件的 canonical 索引序比较。
    let mut segments: Vec<(Vec<u16>, &Archetype)> = store
        .archetypes()
        .iter()
        .map(|arch| {
            let mut idx: Vec<u16> = registry
                .ids_of_set(arch.key)
                .into_iter()
                .map(|id| canon_of[id as usize])
                .collect();
            idx.sort_unstable();
            (idx, arch)
        })
        .collect();
    segments.sort_by(|a, b| compare_name_runs(&table, &a.0, &b.0));

    writer.u32(segments.len() as u32);
    for (idx, arch) in &segments {
        writer.u16(idx.len() as u16);
        for canon_idx in idx {
            writer.u16(*canon_idx);
        }
        writer.u32(arch.ids.len() as u32);
        for id in &arch.ids {
            writer.u64(*id);
        }
        // 列载荷按 canonical 索引序写出（与上面 comp_idx 列表同序）。
        for canon_idx in idx {
            let comp = table[*canon_idx as usize].0;
            let pos = match arch.col_pos(comp) {
                Some(pos) => pos,
                None => {
                    // 纯 tag 桶：该组件没有列，跳过（tag 只出现在 comp_idx 列表里）。
                    continue;
                }
            };
            write_column(&mut writer, &arch.cols[pos]);
        }
    }
    writer.finish()
}

/// 按「组件名序列」的字典序比较两个 archetype（长度短的在前，逐项按名字节序）。
fn compare_name_runs(table: &[(u32, &str, AttrType)], a: &[u16], b: &[u16]) -> std::cmp::Ordering {
    for (x, y) in a.iter().zip(b.iter()) {
        let order = table[*x as usize]
            .1
            .as_bytes()
            .cmp(table[*y as usize].1.as_bytes());
        if order != std::cmp::Ordering::Equal {
            return order;
        }
    }
    a.len().cmp(&b.len())
}

/// 单列写出（列编码的三条分支与 `AttrType` 的三种有列类型一一对应）。
fn write_column(writer: &mut FrameWriter, column: &Column) {
    match column {
        Column::I64(values) => {
            for value in values {
                writer.i64(*value);
            }
        }
        Column::U64(values) => {
            for value in values {
                writer.u64(*value);
            }
        }
        Column::U8(values) => {
            for value in values {
                writer.u8(*value);
            }
        }
        Column::Bytes { off, buf } => {
            for row in 0..off.len().saturating_sub(1) {
                let (start, end) = (off[row] as usize, off[row + 1] as usize);
                // 行内字节用 u32 前缀（与 `FrameWriter::bytes` 同形）；越界不可能，
                // 因为 `off` 由 push 路径单调增长维持。
                let len = (end - start) as u32;
                writer.u32(len);
                writer.raw(&buf[start..end]);
            }
        }
    }
}

/// 解码一帧快照（先全量校验，任何不符都 fail-closed）。
pub fn decode(bytes: &[u8]) -> SimResult<SnapshotState> {
    let mut reader = FrameReader::open(bytes)?;
    let tick = reader.u64()?;
    let next_id = reader.u64()?;

    // ① 组件表（名称升序；重复名即拒）。
    let comp_n = reader.u32()? as usize;
    let mut registry = Registry::new();
    let mut types: Vec<AttrType> = Vec::with_capacity(comp_n);
    let mut names: Vec<String> = Vec::with_capacity(comp_n);
    for _ in 0..comp_n {
        let name_len = reader.u16()? as usize;
        let raw = reader.raw(name_len)?;
        let name = std::str::from_utf8(raw)
            .map_err(|err| SimError::frame(format!("组件名不是合法 UTF-8：{err}")))?
            .to_string();
        let tag = reader.u8()?;
        let frac_bits = reader.u8()?;
        let ty = AttrType::from_wire(tag, frac_bits)?;
        registry.register(&name, ComponentDef { ty })?;
        // 表必须严格升序（否则「两种注册顺序字节相同」这条性质会失效）
        if let Some(prev) = names.last() {
            if prev.as_bytes() >= name.as_bytes() {
                return Err(SimError::frame(format!(
                    "组件表未按名称字节序严格升序：`{prev}` 之后是 `{name}`"
                )));
            }
        }
        names.push(name);
        types.push(ty);
    }

    // ② 资源表（键严格升序）。
    let res_n = reader.u32()? as usize;
    let mut resources = BTreeMap::new();
    let mut prev_key: Option<u64> = None;
    for _ in 0..res_n {
        let key = reader.u64()?;
        let value = reader.u64()?;
        if let Some(prev) = prev_key {
            if prev >= key {
                return Err(SimError::frame(format!(
                    "资源键未按升序：{prev} 之后是 {key}"
                )));
            }
        }
        prev_key = Some(key);
        resources.insert(key, value);
    }

    // ③ archetype 段。
    let arch_n = reader.u32()? as usize;
    let mut archetypes: Vec<Archetype> = Vec::with_capacity(arch_n);
    for _ in 0..arch_n {
        let comp_cnt = reader.u16()? as usize;
        let mut ids_in_arch: Vec<u32> = Vec::with_capacity(comp_cnt);
        let mut key: CompSet = 0;
        for _ in 0..comp_cnt {
            let canon_idx = reader.u16()? as usize;
            if canon_idx >= comp_n {
                return Err(SimError::frame(format!(
                    "archetype 引用了不存在的组件表下标 {canon_idx}（表长 {comp_n}）"
                )));
            }
            let internal = registry.id_of(&names[canon_idx])?;
            if key & (1u128 << internal) != 0 {
                return Err(SimError::frame(format!(
                    "archetype 的组件列表里 `{}` 出现两次",
                    names[canon_idx]
                )));
            }
            key |= 1u128 << internal;
            ids_in_arch.push(internal);
        }
        let row_n = reader.u32()? as usize;
        let mut ids: Vec<EntityId> = Vec::with_capacity(row_n);
        for _ in 0..row_n {
            ids.push(reader.u64()?);
        }
        // 列：只为「有列」的组件读载荷（全表序与 comp_idx 序一致）。
        let mut cols: Vec<Column> = Vec::new();
        let mut col_ids: Vec<u32> = Vec::new();
        for comp in &ids_in_arch {
            let ty = types[canon_index(&names, &registry, *comp)?];
            let Some(mut column) = empty_column(ty) else {
                continue;
            };
            read_column(&mut reader, &mut column, row_n, *comp)?;
            cols.push(column);
            col_ids.push(*comp);
        }
        archetypes.push(Archetype {
            key,
            ids,
            cols,
            col_ids,
        });
    }

    reader.finish()?;
    // ④ 用 store 的 from_parts 重建桶索引与实体索引（它同时做行数/重复 id/越界校验）。
    let store = ArchetypeStore::from_parts(registry, archetypes, next_id)?;
    Ok(SnapshotState {
        tick,
        resources,
        store,
    })
}

/// 组件 id → 组件表下标（解码侧的小工具）。
fn canon_index(names: &[String], registry: &Registry, comp: u32) -> SimResult<usize> {
    let name = registry.name_of(comp)?;
    names
        .iter()
        .position(|candidate| candidate == name)
        .ok_or_else(|| SimError::frame(format!("组件 `{name}` 不在组件表里")))
}

/// 类型 → 空列（`Tag` 没有列 ⇒ `None`）。
fn empty_column(ty: AttrType) -> Option<Column> {
    match ty {
        AttrType::Number { .. } => Some(Column::I64(Vec::new())),
        AttrType::Ref => Some(Column::U64(Vec::new())),
        AttrType::Bool => Some(Column::U8(Vec::new())),
        AttrType::Str => Some(Column::Bytes {
            off: vec![0],
            buf: Vec::new(),
        }),
        AttrType::Tag => None,
    }
}

/// 读一列的 `row_n` 行载荷（类型不符 / boolean 越域 / 长度越界一律拒）。
fn read_column(
    reader: &mut FrameReader<'_>,
    column: &mut Column,
    row_n: usize,
    comp: u32,
) -> SimResult<()> {
    match column {
        Column::I64(values) => {
            values.reserve(row_n);
            for _ in 0..row_n {
                values.push(reader.i64()?);
            }
        }
        Column::U64(values) => {
            values.reserve(row_n);
            for _ in 0..row_n {
                values.push(reader.u64()?);
            }
        }
        Column::U8(values) => {
            values.reserve(row_n);
            for _ in 0..row_n {
                let byte = reader.u8()?;
                if byte > 1 {
                    return Err(SimError::frame(format!(
                        "组件 id {comp} 的 boolean 列出现非 0/1 值：{byte}"
                    )));
                }
                values.push(byte);
            }
        }
        Column::Bytes { off, buf } => {
            for _ in 0..row_n {
                let bytes = reader.bytes()?;
                buf.extend_from_slice(bytes);
                let end = u32::try_from(buf.len())
                    .map_err(|_| SimError::frame(format!("组件 id {comp} 的字符串列超出 4 GiB")))?;
                off.push(end);
            }
        }
    }
    Ok(())
}

/// 从原始字节算 state hash（快照尾 8 字节；见 [`crate::hash::state_hash`]）。
///
/// 这个薄封装存在的理由：让「帧 → hash」在调用点只有一个名字，避免一半代码调
/// `state_hash`、另一半调 `fnv1a64(&b[..len-8])`——两份写法迟早分叉。
pub fn hash_of(bytes: &[u8]) -> u64 {
    crate::hash::state_hash(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::hash::fnv1a64;

    /// 空世界的最小帧也要稳定：两次编码逐字节相同，且能被读回。
    #[test]
    fn empty_world_round_trips() {
        let store = ArchetypeStore::new();
        let resources = BTreeMap::new();
        let bytes = encode(0, &resources, &store);
        assert_eq!(bytes, encode(0, &resources, &store));
        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded.tick, 0);
        assert!(decoded.store.registry().is_empty());
        assert_eq!(decoded.store.entity_count(), 0);
        assert_eq!(fnv1a64(b""), 0xCBF2_9CE4_8422_2325);
    }
}
