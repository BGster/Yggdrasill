//! M1-C：Lua 世界面 `world.*`——组件声明、实体增删改查、系统声明与 `seal` 上浮。
//!
//! 与 `ygg.*` 并列注册（[`super::install`] 两张表同一次安装），但**只走列式内核面**：
//! 一切读写都落到 `ArchetypeStore` 的批式 API（`spawn_columns` / `insert_columns` /
//! `set_many` / `for_each_mut` / `collect_ids` / `value_of`）。不发明第二套存储，
//! 也不发明逐实体的存储路径——单实体的增删改查在本面里都是「一行批式调用」。
//!
//! ## 组件 kind → 内核列型
//!
//! **声明序即组件 id 序**（`world.component` 的第 n 次成功声明得到 id n-1，与
//! `Registry::register` 的「只增」语义逐字一致）。kind 只有四个：
//!
//! | kind | 内核列型 | Lua 值域 |
//! | --- | --- | --- |
//! | `i64` | `number`（`Column::I64`） | 整数（过 `YGG-SIM-FIXED` 定点闸门） |
//! | `u64` | `ref`（`Column::U64`） | `0 ..= i64::MAX`（Lua 5.4 整数是 `i64`，越界报错不回绕） |
//! | `u8` | `number`（`Column::I64`）+ 本面 `0..=255` 把关 | 整数 |
//! | `bytes` | `string`（`Column::Bytes`） | 字符串 |
//!
//! `u8` 落 `number` 列而不是内核的 `boolean`(`Column::U8`) 列：`RowSet` 只有
//! `fixed` / `u64v` / `bytes` 三个读取口和 `set_fixed` / `set_u64` 两个写回口，
//! 走 `boolean` 列会让该组件在查询面上**完全不可达**（内核缺口，已上报）。
//! 值域因此由本面把关：`0..=255`，越界即 `YGG-SIM-FIXED`。
//!
//! ## 决定性
//!
//! - **迭代序** = 内核 `for_each_mut` 的遍历序（桶下标升序 → 行号升序）= `collect_ids`
//!   的序。同种子、同操作序列的两次运行逐项相同（`tests/world_facade.rs` 的
//!   「两次运行序相同」用例就是这个契约的可执行形式）。
//! - **无浮点**：所有数值入口过同一道定点闸门，`math` 的结果（`0.5` 之类）过不了面，
//!   因此双精度不可能经本面进入世界值。
//! - **未挂载 / 重入都是可读错误**（码在首段）：不是 `nil`、不是 `false`、不是 panic。
//!   `nil` 与「世界是空的」不可分辨，静默降级会让 Mod 把「宿主没这个能力」读成
//!   「世界为空」并据此写下错误状态。
//!
//! ## 系统体与重入
//!
//! `world.system{...}` 的 `run` 收 `(dt, ctx)`：`dt` 是本 tick 步长（Q32.32 原始整数），
//! `ctx` 是与 `world.*` 同名同义的函数表，但走 `SystemCtx` 的能力面（`tick / dt /
//! each / each_mut / ids / rng / spawn / despawn / insert`）。系统体里再调 `world.*`
//! 一律 `YGG-WORLD-BUSY`：`step` 期间内核槽已被独占借用，**不是**可以绕过的限制。
//! 结构变更（`ctx.spawn` / `ctx.despawn` / `ctx.insert`）按内核语义延迟到 tick 边界。

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

use mlua::{Error, Function, Lua, MultiValue, Scope, Table, Value as LuaValue};
use ygg_sim::archetype::{Archetype, Column, ValueRef};
use ygg_sim::component::{AttrType, ComponentDef, ComponentId, EntityId, Registry};
use ygg_sim::error::SimError;
use ygg_sim::query::{RowSet, MAX_QUERY};
use ygg_sim::sim::Sim;
use ygg_sim::store::ArchetypeStore;
use ygg_sim::system::{Stage, SystemCtx, SystemDecl};

use super::marshal::{sim_fixed_arg, sim_unmounted};
use super::HostState;

/// 重入码（`world.*` 在内核槽已被独占时被调用；与 `YGG-SIM-UNMOUNTED` 同一处置）。
pub const WORLD_BUSY: &str = "YGG-WORLD-BUSY";

/// `world.log` 环形缓冲上限（超出丢最旧的一行：日志是诊断面，不是状态）。
pub const WORLD_LOG_CAP: usize = 256;

fn world_busy(what: &str) -> Error {
    Error::runtime(format!(
        "{WORLD_BUSY}: `{what}` 在系统执行期间被调用（内核槽已被 `step` 独占借用）；\
         系统体内请用 `run(dt, ctx)` 的 `ctx.*`"
    ))
}

fn world_unmounted(what: &str) -> Error {
    Error::runtime(sim_unmounted(what))
}

/// `SimError` → Lua 错误：**码在首段**（与 `ygg` 的 `sim.*` 同一口径，外部可 grep 可断言）。
fn sim_to_lua(e: SimError) -> Error {
    Error::runtime(e.to_string())
}

// ---------------------------------------------------------------------------
// 声明面：kind 表 + 日志环
// ---------------------------------------------------------------------------

/// 组件在 `world` 面上的列型（相对内核列型的**增量**只有两点：`u8` 的值域、
/// `bytes` 在 `each_mut` 上不可写）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    I64,
    U64,
    U8,
    Bytes,
    /// 零宽成员：只能由宿主直接 `Sim::register` 声明（本面不提供 `tag` kind），
    /// 在 `world` 面上「值」恒为 `true`。
    Tag,
}

impl Kind {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "i64" => Some(Kind::I64),
            "u64" => Some(Kind::U64),
            "u8" => Some(Kind::U8),
            "bytes" => Some(Kind::Bytes),
            _ => None,
        }
    }

    /// 内核列型（`world.component` 的落库型）。
    fn def(self) -> ComponentDef {
        match self {
            Kind::I64 | Kind::U8 => ComponentDef::number(),
            Kind::U64 => ComponentDef::reference(),
            Kind::Bytes => ComponentDef::string(),
            Kind::Tag => ComponentDef::tag(),
        }
    }

    /// 宿主直注册的组件（本面没记过声明）按内核列型反推。
    fn from_attr(ty: AttrType) -> Option<Self> {
        match ty {
            AttrType::Number { .. } => Some(Kind::I64),
            AttrType::Ref => Some(Kind::U64),
            AttrType::Str => Some(Kind::Bytes),
            AttrType::Tag => Some(Kind::Tag),
            AttrType::Bool | AttrType::F64Bits => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Kind::I64 => "i64",
            Kind::U64 => "u64",
            Kind::U8 => "u8",
            Kind::Bytes => "bytes",
            Kind::Tag => "tag",
        }
    }
}

/// `world` 面的宿主侧状态（挂在 [`HostState`] 上，与 `ygg.*` 共享同一次 install）。
///
/// `kinds` 只记 `world.component` 声明过的组件：`u8` 与 `i64` 都落 `number` 列，
/// 两者的区别（值域）只存在于声明里。宿主直注册的组件缺项时按注册表列型推导，
/// 因此这里**不是**第二份 schema——名字与 id 的唯一权威仍是内核 `Registry`。
#[derive(Default)]
pub struct WorldState {
    kinds: BTreeMap<ComponentId, Kind>,
    logs: VecDeque<String>,
}

impl WorldState {
    /// 组件 → 本面列型（声明优先，缺项按内核列型推导）。
    fn kind_of(
        &self,
        registry: &Registry,
        comp: ComponentId,
        name: &str,
    ) -> Result<Kind, SimError> {
        if let Some(kind) = self.kinds.get(&comp) {
            return Ok(*kind);
        }
        let ty = registry.type_of(comp)?;
        Kind::from_attr(ty).ok_or_else(|| {
            SimError::schema(format!(
                "组件 `{name}` 的内核列型是 {}，`world` 面只认 i64 / u64 / u8 / bytes",
                ty.label()
            ))
        })
    }

    /// 记一行日志（环形：满则丢最旧）。
    fn push_log(&mut self, line: String) {
        if self.logs.len() == WORLD_LOG_CAP {
            self.logs.pop_front();
        }
        self.logs.push_back(line);
    }

    /// 取走全部日志行（宿主 drain 口；顺序 = 追加序）。
    pub(super) fn take_logs(&mut self) -> Vec<String> {
        self.logs.drain(..).collect()
    }
}

/// 一次解析出的组件引用（名字 + id + 列型）：错误消息要名字，内核调用要 id。
struct Slot {
    name: String,
    comp: ComponentId,
    kind: Kind,
}

impl Slot {
    fn bit(&self) -> u128 {
        1u128 << self.comp
    }
}

fn slot_of(registry: &Registry, world: &WorldState, name: &str) -> Result<Slot, Error> {
    let comp = registry.id_of(name).map_err(sim_to_lua)?;
    let kind = world.kind_of(registry, comp, name).map_err(sim_to_lua)?;
    Ok(Slot {
        name: name.to_string(),
        comp,
        kind,
    })
}

/// 组件名表（Lua 字符串序列）→ 槽位表：保序、去重、长度 ≤ [`MAX_QUERY`]。
fn slots_of(
    registry: &Registry,
    world: &WorldState,
    names: &Table,
    what: &str,
) -> Result<Vec<Slot>, Error> {
    let mut slots: Vec<Slot> = Vec::new();
    let mut seen: u128 = 0;
    for item in names.sequence_values::<LuaValue>() {
        let name = match item? {
            LuaValue::String(s) => s.to_str()?.to_owned(),
            other => {
                return Err(sim_to_lua(SimError::schema(format!(
                    "`{what}` 的组件名表必须是字符串数组，遇到 {}",
                    other.type_name()
                ))))
            }
        };
        let slot = slot_of(registry, world, &name)?;
        if seen & slot.bit() != 0 {
            return Err(sim_to_lua(SimError::schema(format!(
                "`{what}` 的组件名表里 `{}` 出现了两次",
                slot.name
            ))));
        }
        seen |= slot.bit();
        slots.push(slot);
    }
    if slots.is_empty() {
        return Err(sim_to_lua(SimError::schema(format!(
            "`{what}` 的组件名表不得为空（空集不等于「遍地都是」）"
        ))));
    }
    if slots.len() > MAX_QUERY {
        return Err(sim_to_lua(SimError::schema(format!(
            "`{what}` 一次最多 {MAX_QUERY} 个组件，得到 {}",
            slots.len()
        ))));
    }
    Ok(slots)
}

// ---------------------------------------------------------------------------
// 值编解码（四个 kind 一一对应；浮点无入口）
// ---------------------------------------------------------------------------

/// Lua 整数 → `i64`（过定点闸门：非整数值的 `number` 一律 `YGG-SIM-FIXED`）。
fn int_of(slot: &Slot, value: &LuaValue) -> Result<i64, Error> {
    let raw = sim_fixed_arg(value.clone(), &slot.name)?;
    match slot.kind {
        Kind::U8 if !(0..=255).contains(&raw) => Err(sim_to_lua(SimError::fixed(format!(
            "组件 `{}` 是 u8（0..=255），得到 {raw}",
            slot.name
        )))),
        Kind::U64 if raw < 0 => Err(sim_to_lua(SimError::fixed(format!(
            "组件 `{}` 是 u64（0..=i64::MAX），得到 {raw}",
            slot.name
        )))),
        _ => Ok(raw),
    }
}

/// 单元格 Lua 值 → 一行的列载荷（`ValueRef` + 以它建列）。
fn column_of(slot: &Slot, value: &LuaValue) -> Result<Column, Error> {
    Ok(match slot.kind {
        Kind::I64 | Kind::U8 => {
            let raw = int_of(slot, value)?;
            let mut column = Column::I64(Vec::with_capacity(1));
            column
                .push_value(&ValueRef::I64(raw), slot.comp)
                .map_err(sim_to_lua)?;
            column
        }
        Kind::U64 => {
            let raw = int_of(slot, value)?;
            let raw = u64::try_from(raw).map_err(|_| {
                sim_to_lua(SimError::fixed(format!(
                    "组件 `{}` 是 u64，得到 {raw}",
                    slot.name
                )))
            })?;
            let mut column = Column::U64(Vec::with_capacity(1));
            column
                .push_value(&ValueRef::U64(raw), slot.comp)
                .map_err(sim_to_lua)?;
            column
        }
        Kind::Bytes => {
            let LuaValue::String(text) = value else {
                return Err(sim_to_lua(SimError::schema(format!(
                    "组件 `{}` 是 bytes（字符串），得到 {}",
                    slot.name,
                    value.type_name()
                ))));
            };
            let mut column = Column::Bytes {
                off: vec![0],
                buf: Vec::new(),
            };
            let bytes = text.as_bytes();
            column
                .push_value(&ValueRef::Bytes(&bytes), slot.comp)
                .map_err(sim_to_lua)?;
            column
        }
        Kind::Tag => {
            return Err(sim_to_lua(SimError::schema(format!(
                "组件 `{}` 是零宽 tag（没有列），不能带值；用 `{{ {} = true }}` 表示成员",
                slot.name, slot.name
            ))))
        }
    })
}

/// 实体 id → Lua 整数（越界即错：id 是 `u64`，Lua 5.4 整数是 `i64`）。
fn entity_to_lua(id: EntityId) -> Result<i64, Error> {
    i64::try_from(id)
        .map_err(|_| sim_to_lua(SimError::entity(format!("实体 id {id} 超出 Lua 整数范围"))))
}

/// Lua 值 → 实体 id（≥ 1 的整数；不静默取整）。
fn entity_of(value: LuaValue, what: &str) -> Result<EntityId, Error> {
    let raw = sim_fixed_arg(value, what)?;
    u64::try_from(raw)
        .ok()
        .filter(|id| *id != 0)
        .ok_or_else(|| {
            sim_to_lua(SimError::entity(format!(
                "`{what}` 必须是实体 id（≥ 1 的整数），得到 {raw}"
            )))
        })
}

/// 一列的一行 → Lua 值。
fn cell_to_lua(lua: &Lua, slot: &Slot, rs: &RowSet<'_>, k: u8) -> Result<LuaValue, SimError> {
    let missing = || {
        SimError::state(format!(
            "组件 `{}` 在本次遍历的桶里读不到值（列型与声明不符）",
            slot.name
        ))
    };
    Ok(match slot.kind {
        Kind::Tag => LuaValue::Boolean(true),
        Kind::I64 | Kind::U8 => LuaValue::Integer(rs.fixed(k).ok_or_else(missing)?),
        Kind::U64 => {
            let raw = rs.u64v(k).ok_or_else(missing)?;
            let raw = i64::try_from(raw).map_err(|_| {
                SimError::fixed(format!(
                    "组件 `{}` 的 u64 值 {raw} 超出 Lua 整数范围",
                    slot.name
                ))
            })?;
            LuaValue::Integer(raw)
        }
        Kind::Bytes => {
            let bytes = rs.bytes(k).ok_or_else(missing)?;
            LuaValue::String(
                lua.create_string(bytes)
                    .map_err(|e| SimError::state(format!("字符串进 Lua 失败：{e}")))?,
            )
        }
    })
}

/// 载荷表的落地形态：（有列组件的列（值待填）, 要挂的零宽成员）。
///
/// 列按**组件 id 升序**——载荷表的书写序不参与（Lua 表无序，序必须来自注册表）。
type Payload = (Vec<(ComponentId, Column)>, Vec<ComponentId>);

/// 载荷表 →（有列组件的列, 零宽成员）；列按**组件 id 升序**（确定序，载荷书写序不参与）。
fn payload_of(
    registry: &Registry,
    world: &WorldState,
    payload: &Table,
    what: &str,
    allow_tags: bool,
) -> Result<Payload, Error> {
    let mut entries: Vec<(ComponentId, Slot, LuaValue)> = Vec::new();
    for pair in payload.pairs::<LuaValue, LuaValue>() {
        let (key, value) = pair?;
        let name = match key {
            LuaValue::String(s) => s.to_str()?.to_owned(),
            other => {
                return Err(sim_to_lua(SimError::schema(format!(
                    "`{what}` 的载荷键必须是组件名（字符串），遇到 {}",
                    other.type_name()
                ))))
            }
        };
        let slot = slot_of(registry, world, &name)?;
        entries.push((slot.comp, slot, value));
    }
    if entries.is_empty() {
        return Err(sim_to_lua(SimError::schema(format!(
            "`{what}` 的载荷表不得为空"
        ))));
    }
    entries.sort_by_key(|(comp, ..)| *comp);

    let mut cols: Vec<(ComponentId, Column)> = Vec::with_capacity(entries.len());
    let mut tags: Vec<ComponentId> = Vec::new();
    for (comp, slot, value) in entries {
        if slot.kind == Kind::Tag {
            if !allow_tags {
                return Err(sim_to_lua(SimError::schema(format!(
                    "`{what}` 不能改零宽成员 `{}`（tag 没有列；用 world.insert / world.remove）",
                    slot.name
                ))));
            }
            if !matches!(value, LuaValue::Boolean(true)) {
                return Err(sim_to_lua(SimError::schema(format!(
                    "零宽成员 `{}` 的值必须是 `true`，得到 {}",
                    slot.name,
                    value.type_name()
                ))));
            }
            tags.push(comp);
            continue;
        }
        cols.push((comp, column_of(&slot, &value)?));
    }
    Ok((cols, tags))
}

// ---------------------------------------------------------------------------
// 内核槽访问（重入与未挂载都是可读错误，永不 panic）
// ---------------------------------------------------------------------------

fn sim_slot(state: &Rc<RefCell<HostState>>) -> Rc<RefCell<Option<Box<Sim>>>> {
    Rc::clone(&state.borrow().sim)
}

fn with_sim_mut<R>(
    state: &Rc<RefCell<HostState>>,
    what: &str,
    f: impl FnOnce(&mut Sim) -> Result<R, Error>,
) -> Result<R, Error> {
    let slot = sim_slot(state);
    let mut guard = slot.try_borrow_mut().map_err(|_| world_busy(what))?;
    let sim: &mut Sim = guard.as_mut().ok_or_else(|| world_unmounted(what))?;
    f(sim)
}

// ---------------------------------------------------------------------------
// 迭代：单行喂参 / 写回 / 错误透传
// ---------------------------------------------------------------------------

/// 迭代内 Lua 错误的透传槽：内核只认 `SimError`，Lua 错误先存这里、用哨兵中止内核迭代，
/// 出迭代后**原样**抛出（用户错误不被降级成 `YGG-SIM-STATE` 文本，码与消息都保住）。
const ABORT_SENTINEL: &str = "__world_iteration_aborted__";

#[derive(Default)]
struct Abort(Option<Error>);

impl Abort {
    fn trip(&mut self, e: Error) -> SimError {
        self.0 = Some(e);
        SimError::state(ABORT_SENTINEL)
    }

    fn take(&mut self) -> Option<Error> {
        self.0.take()
    }
}

/// 一行 → Lua 实参（`id, v1, v2, …`，按调用方声明序）并调用回调。
///
/// **性能契约**：每行的值作为**多个返回值**逐个进实参缓冲，**不建行表**——逐行
/// `lua.create_table()` 会给每一行分配一个 Lua 对象并把 GC 拖进内循环。
#[allow(clippy::too_many_arguments)]
fn call_row(
    lua: &Lua,
    slots: &[Slot],
    rs: &mut RowSet<'_>,
    f: &Function,
    mutate: bool,
    args: &mut MultiValue,
) -> Result<(), Error> {
    args.clear();
    args.push_back(LuaValue::Integer(entity_to_lua(rs.id())?));
    for (k, slot) in slots.iter().enumerate() {
        args.push_back(cell_to_lua(lua, slot, rs, k as u8).map_err(sim_to_lua)?);
    }
    if !mutate {
        f.call::<()>(&*args)?;
        return Ok(());
    }
    let returns: MultiValue = f.call(&*args)?;
    if returns.len() != slots.len() {
        return Err(sim_to_lua(SimError::schema(format!(
            "each_mut 的回调必须按声明序返回 {} 个值，得到 {}",
            slots.len(),
            returns.len()
        ))));
    }
    for (k, slot) in slots.iter().enumerate() {
        let Some(value) = returns.get(k) else {
            continue;
        };
        match slot.kind {
            Kind::I64 | Kind::U8 => {
                let raw = int_of(slot, value)?;
                rs.set_fixed(k as u8, raw).map_err(sim_to_lua)?;
            }
            Kind::U64 => {
                let raw = int_of(slot, value)?;
                let raw = u64::try_from(raw).map_err(|_| {
                    sim_to_lua(SimError::fixed(format!(
                        "组件 `{}` 是 u64，得到 {raw}",
                        slot.name
                    )))
                })?;
                rs.set_u64(k as u8, raw).map_err(sim_to_lua)?;
            }
            // `slots_of` + 入参只读检查已把这两种挡在迭代之外。
            Kind::Bytes | Kind::Tag => {
                return Err(sim_to_lua(SimError::schema(format!(
                    "组件 `{}` 不能经 each_mut 写回",
                    slot.name
                ))))
            }
        }
    }
    Ok(())
}

/// 跑一遍迭代：`run` 决定从哪个面取行（`ArchetypeStore::for_each_mut` 或
/// `SystemCtx::query_mut`），本函数只负责逐行喂参、写回与错误透传。
fn iterate<F>(lua: &Lua, slots: &[Slot], f: &Function, mutate: bool, run: F) -> Result<u64, Error>
where
    F: FnOnce(&mut dyn FnMut(&mut RowSet<'_>) -> Result<(), SimError>) -> Result<u64, SimError>,
{
    let mut abort = Abort::default();
    let mut args = MultiValue::with_capacity(slots.len() + 1);
    let mut body = |rs: &mut RowSet<'_>| -> Result<(), SimError> {
        match call_row(lua, slots, rs, f, mutate, &mut args) {
            Ok(()) => Ok(()),
            Err(e) => Err(abort.trip(e)),
        }
    };
    let visited = run(&mut body);
    if let Some(e) = abort.take() {
        // 用户错误优先于内核的哨兵错：透传原始码与消息。
        return Err(e);
    }
    visited.map_err(sim_to_lua)
}

/// `each_mut` 的入参只读检查：`bytes`（缺 `set_bytes`）与 tag（无列）不可能写回，
/// 在迭代**开始前**拒绝，避免写了一半才发现。
fn check_writable(slots: &[Slot], what: &str) -> Result<(), Error> {
    for slot in slots {
        if matches!(slot.kind, Kind::I64 | Kind::U64 | Kind::U8) {
            continue;
        }
        return Err(sim_to_lua(SimError::schema(format!(
            "`{what}` 不能写 `{}`（{}）：tag 没有列，bytes 列在内核查询面上没有写回口；\
             用 `world.set` 逐实体写",
            slot.name,
            slot.kind.label()
        ))));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 系统面：`run(dt, ctx)`
// ---------------------------------------------------------------------------

/// `ctx` 函数表：与 `world.*` 同名同义，但走 `SystemCtx` 的能力面。
///
/// 借用纪律：`SystemCtx` 由 `RefCell` 暂存，每个闭包进出各取一次 `try_borrow_mut`；
/// 借用失败（理论上不可达——同一时刻只有一个闭包在跑）给可读错误而不是 panic。
///
/// 生命周期：`'a` = `lua.scope(...)` 体这一小段（四个入参共用一个区间），
/// `'b` / `'c` = `SystemCtx` 自身的借用与内部生命周期（不变，与会话区间解耦）。
#[allow(clippy::needless_lifetimes)]
fn ctx_table<'a, 'env, 'b, 'c>(
    lua: &'a Lua,
    scope: &'a Scope<'a, 'env>,
    state: &'a Rc<RefCell<HostState>>,
    cell: &'a RefCell<&'b mut SystemCtx<'c>>,
) -> Result<Table, Error> {
    let table = lua.create_table()?;

    table.raw_set(
        "tick",
        scope.create_function(|_, ()| -> Result<i64, Error> {
            let guard = cell
                .try_borrow()
                .map_err(|_| Error::runtime(format!("{WORLD_BUSY}: ctx 上下文已被占用")))?;
            Ok(i64::try_from(guard.tick()).unwrap_or(i64::MAX))
        })?,
    )?;

    table.raw_set(
        "dt",
        scope.create_function(|_, ()| -> Result<i64, Error> {
            let guard = cell
                .try_borrow()
                .map_err(|_| Error::runtime(format!("{WORLD_BUSY}: ctx 上下文已被占用")))?;
            Ok(guard.dt())
        })?,
    )?;

    let what = "ctx.each";
    table.raw_set(
        "each",
        scope.create_function(
            move |lua, (names, f): (Table, Function)| -> Result<i64, Error> {
                ctx_each(lua, state, cell, names, f, false, what)
            },
        )?,
    )?;
    let what = "ctx.each_mut";
    table.raw_set(
        "each_mut",
        scope.create_function(
            move |lua, (names, f): (Table, Function)| -> Result<i64, Error> {
                ctx_each(lua, state, cell, names, f, true, what)
            },
        )?,
    )?;

    table.raw_set(
        "ids",
        scope.create_function(|lua, names: Table| -> Result<Table, Error> {
            let mut guard = cell
                .try_borrow_mut()
                .map_err(|_| Error::runtime(format!("{WORLD_BUSY}: ctx 上下文已被占用")))?;
            let ctx = &mut **guard;
            let slots = {
                let world = state.borrow();
                slots_of(ctx.registry(), &world.world, &names, "ctx.ids")?
            };
            let comps: Vec<ComponentId> = slots.iter().map(|s| s.comp).collect();
            // `SystemCtx` 不暴露 `collect_ids`：用遍历收 id，序与 `each` 逐项相同。
            let mut ids: Vec<EntityId> = Vec::new();
            ctx.query(&comps, |rs| ids.push(rs.id()));
            let out = lua.create_table_with_capacity(ids.len(), 0)?;
            for (i, id) in ids.iter().enumerate() {
                out.raw_set(i + 1, entity_to_lua(*id)?)?;
            }
            Ok(out)
        })?,
    )?;

    table.raw_set(
        "rng",
        scope.create_function(|_, stream: String| -> Result<i64, Error> {
            let mut guard = cell
                .try_borrow_mut()
                .map_err(|_| Error::runtime(format!("{WORLD_BUSY}: ctx 上下文已被占用")))?;
            Ok(i64::from(guard.rng32(&stream)))
        })?,
    )?;

    table.raw_set(
        "spawn",
        scope.create_function(|_, payload: Table| -> Result<(), Error> {
            let mut guard = cell
                .try_borrow_mut()
                .map_err(|_| Error::runtime(format!("{WORLD_BUSY}: ctx 上下文已被占用")))?;
            let ctx = &mut **guard;
            let (cols, tags) = {
                let world = state.borrow();
                payload_of(ctx.registry(), &world.world, &payload, "ctx.spawn", true)?
            };
            ctx.defer_spawn(1, cols, tags);
            Ok(())
        })?,
    )?;

    table.raw_set(
        "despawn",
        scope.create_function(|_, id: LuaValue| -> Result<(), Error> {
            let id = entity_of(id, "ctx.despawn")?;
            let mut guard = cell
                .try_borrow_mut()
                .map_err(|_| Error::runtime(format!("{WORLD_BUSY}: ctx 上下文已被占用")))?;
            guard.defer_despawn(vec![id]);
            Ok(())
        })?,
    )?;

    table.raw_set(
        "insert",
        scope.create_function(|_, (id, payload): (LuaValue, Table)| -> Result<(), Error> {
            let id = entity_of(id, "ctx.insert")?;
            let mut guard = cell
                .try_borrow_mut()
                .map_err(|_| Error::runtime(format!("{WORLD_BUSY}: ctx 上下文已被占用")))?;
            let ctx = &mut **guard;
            let (cols, tags) = {
                let world = state.borrow();
                payload_of(ctx.registry(), &world.world, &payload, "ctx.insert", true)?
            };
            ctx.defer_insert(vec![id], cols, tags);
            Ok(())
        })?,
    )?;

    Ok(table)
}

fn ctx_each(
    lua: &Lua,
    state: &Rc<RefCell<HostState>>,
    cell: &RefCell<&mut SystemCtx<'_>>,
    names: Table,
    f: Function,
    mutate: bool,
    what: &str,
) -> Result<i64, Error> {
    let mut guard = cell
        .try_borrow_mut()
        .map_err(|_| Error::runtime(format!("{WORLD_BUSY}: ctx 上下文已被占用")))?;
    let ctx = &mut **guard;
    let slots = {
        let world = state.borrow();
        slots_of(ctx.registry(), &world.world, &names, what)?
    };
    if mutate {
        check_writable(&slots, what)?;
    }
    let comps: Vec<ComponentId> = slots.iter().map(|s| s.comp).collect();
    let visited = iterate(lua, &slots, &f, mutate, |body| ctx.query_mut(&comps, body))?;
    Ok(i64::try_from(visited).unwrap_or(i64::MAX))
}

// ---------------------------------------------------------------------------
// 安装
// ---------------------------------------------------------------------------

/// 装 `world` 面（全局表 `world`；须在 `ygg.kv` 之后——`world.kv` 直接复用那张表）——
/// 一块表里 15 个键，逐个 `raw_set`。
pub(super) fn install_world(lua: &Lua, state: &Rc<RefCell<HostState>>) -> Result<(), Error> {
    let ygg: Table = lua.globals().raw_get("ygg")?;
    let world = lua.create_table()?;

    // ---- world.component{ name, kind } ----
    let st = Rc::clone(state);
    world.raw_set(
        "component",
        lua.create_function(move |_, spec: Table| -> Result<i64, Error> {
            let name: String = spec.raw_get("name")?;
            let kind_name: String = spec.raw_get("kind")?;
            let Some(kind) = Kind::parse(&kind_name) else {
                return Err(sim_to_lua(SimError::schema(format!(
                    "`world.component` 的 kind 必须是 i64|u64|u8|bytes，\
                     组件 `{name}` 得到 `{kind_name}`"
                ))));
            };
            let id = with_sim_mut(&st, "world.component", |sim| {
                if sim.world().registry().find(&name).is_some() {
                    return Err(sim_to_lua(SimError::schema(format!(
                        "组件 `{name}` 已声明过（world.component 每个组件只声明一次）"
                    ))));
                }
                sim.register(&name, kind.def()).map_err(sim_to_lua)
            })?;
            st.borrow_mut().world.kinds.insert(id, kind);
            Ok(i64::from(id))
        })?,
    )?;

    // ---- world.spawn{ ... } -> id ----
    let st = Rc::clone(state);
    world.raw_set(
        "spawn",
        lua.create_function(move |_, payload: Table| -> Result<i64, Error> {
            with_sim_mut(&st, "world.spawn", |sim| {
                let (cols, tags) = {
                    let world = st.borrow();
                    payload_of(
                        sim.world().registry(),
                        &world.world,
                        &payload,
                        "world.spawn",
                        true,
                    )?
                };
                let ids = sim
                    .world_mut()
                    .spawn_columns(1, &cols, &tags)
                    .map_err(sim_to_lua)?;
                ids.first()
                    .copied()
                    .ok_or_else(|| sim_to_lua(SimError::state("spawn 未返回实体 id")))
                    .and_then(entity_to_lua)
            })
        })?,
    )?;

    // ---- world.despawn(id) -> n ----
    let st = Rc::clone(state);
    world.raw_set(
        "despawn",
        lua.create_function(move |_, id: LuaValue| -> Result<i64, Error> {
            let id = entity_of(id, "world.despawn")?;
            with_sim_mut(&st, "world.despawn", |sim| {
                let removed = sim.world_mut().despawn_ids(&[id]).map_err(sim_to_lua)?;
                Ok(i64::try_from(removed).unwrap_or(i64::MAX))
            })
        })?,
    )?;

    // ---- world.insert(id, { ... }) -> n ----
    let st = Rc::clone(state);
    world.raw_set(
        "insert",
        lua.create_function(
            move |_, (id, payload): (LuaValue, Table)| -> Result<i64, Error> {
                let id = entity_of(id, "world.insert")?;
                with_sim_mut(&st, "world.insert", |sim| {
                    let (cols, tags) = {
                        let world = st.borrow();
                        payload_of(
                            sim.world().registry(),
                            &world.world,
                            &payload,
                            "world.insert",
                            true,
                        )?
                    };
                    let written = sim
                        .world_mut()
                        .insert_columns(&[id], &cols, &tags)
                        .map_err(sim_to_lua)?;
                    Ok(i64::try_from(written).unwrap_or(i64::MAX))
                })
            },
        )?,
    )?;

    // ---- world.set(id, { ... }) -> n ----
    let st = Rc::clone(state);
    world.raw_set(
        "set",
        lua.create_function(
            move |_, (id, payload): (LuaValue, Table)| -> Result<i64, Error> {
                let id = entity_of(id, "world.set")?;
                with_sim_mut(&st, "world.set", |sim| {
                    let (cols, tags) = {
                        let world = st.borrow();
                        payload_of(
                            sim.world().registry(),
                            &world.world,
                            &payload,
                            "world.set",
                            false,
                        )?
                    };
                    debug_assert!(tags.is_empty(), "world.set 已在 payload_of 里拒绝 tag");
                    let store = sim.world_mut();
                    for (comp, column) in &cols {
                        store.set_many(*comp, &[id], column).map_err(sim_to_lua)?;
                    }
                    Ok(i64::try_from(cols.len()).unwrap_or(i64::MAX))
                })
            },
        )?,
    )?;

    // ---- world.get(id, name) -> value | nil ----
    let st = Rc::clone(state);
    world.raw_set(
        "get",
        lua.create_function(
            move |lua, (id, name): (LuaValue, String)| -> Result<LuaValue, Error> {
                let id = entity_of(id, "world.get")?;
                with_sim_mut(&st, "world.get", |sim| {
                    let slot = {
                        let world = st.borrow();
                        slot_of(sim.world().registry(), &world.world, &name)?
                    };
                    let store = sim.world();
                    let (arch_idx, _) = store.locate(id).map_err(sim_to_lua)?;
                    let arch = &store.archetypes()[arch_idx as usize];
                    if arch.key & slot.bit() == 0 {
                        return Ok(LuaValue::Nil);
                    }
                    if slot.kind == Kind::Tag {
                        return Ok(LuaValue::Boolean(true));
                    }
                    let value = store.value_of(id, slot.comp).map_err(sim_to_lua)?;
                    Ok(match (slot.kind, value) {
                        (Kind::I64 | Kind::U8, ValueRef::I64(raw)) => LuaValue::Integer(raw),
                        (Kind::U64, ValueRef::U64(raw)) => {
                            let raw = i64::try_from(raw).map_err(|_| {
                                sim_to_lua(SimError::fixed(format!(
                                    "组件 `{}` 的 u64 值 {raw} 超出 Lua 整数范围",
                                    slot.name
                                )))
                            })?;
                            LuaValue::Integer(raw)
                        }
                        (Kind::Bytes, ValueRef::Bytes(bytes)) => {
                            LuaValue::String(lua.create_string(bytes)?)
                        }
                        (kind, value) => {
                            return Err(sim_to_lua(SimError::state(format!(
                                "组件 `{}` 声明为 {} 但与列里的值形状不符（{:?}）",
                                slot.name,
                                kind.label(),
                                value
                            ))))
                        }
                    })
                })
            },
        )?,
    )?;

    // ---- world.remove(id, { "hp" }) -> n ----
    //
    // 内核没有「摘列」API：`ArchetypeStore` 的 `by_key` / `entity_index` 是私有派生面，
    // 摘掉一列必须重建它们，而唯一的重建口是 `from_parts`。于是本函数的工作序是
    // 「克隆 → 拆件 → 搬行 → 重建 → 换回」，**代价 O(实体数)**（不是热路径 API）。
    // 克隆保证「拒绝不留半改」：任何一步失败，原 store 一个字节都没动。
    let st = Rc::clone(state);
    world.raw_set(
        "remove",
        lua.create_function(
            move |_, (id, names): (LuaValue, Table)| -> Result<i64, Error> {
                let id = entity_of(id, "world.remove")?;
                with_sim_mut(&st, "world.remove", |sim| {
                    let slots = {
                        let world = st.borrow();
                        slots_of(sim.world().registry(), &world.world, &names, "world.remove")?
                    };
                    remove_components(sim.world_mut(), id, &slots)
                })
            },
        )?,
    )?;

    // ---- world.each / world.each_mut / world.ids ----
    let st = Rc::clone(state);
    world.raw_set(
        "each",
        lua.create_function(
            move |lua, (names, f): (Table, Function)| -> Result<i64, Error> {
                top_each(lua, &st, &names, f, false)
            },
        )?,
    )?;
    let st = Rc::clone(state);
    world.raw_set(
        "each_mut",
        lua.create_function(
            move |lua, (names, f): (Table, Function)| -> Result<i64, Error> {
                top_each(lua, &st, &names, f, true)
            },
        )?,
    )?;
    let st = Rc::clone(state);
    world.raw_set(
        "ids",
        lua.create_function(move |lua, names: Table| -> Result<Table, Error> {
            with_sim_mut(&st, "world.ids", |sim| {
                let slots = {
                    let world = st.borrow();
                    slots_of(sim.world().registry(), &world.world, &names, "world.ids")?
                };
                let comps: Vec<ComponentId> = slots.iter().map(|s| s.comp).collect();
                let mut ids: Vec<EntityId> = Vec::new();
                sim.world().collect_ids(&comps, &mut ids);
                let out = lua.create_table_with_capacity(ids.len(), 0)?;
                for (i, id) in ids.iter().enumerate() {
                    out.raw_set(i + 1, entity_to_lua(*id)?)?;
                }
                Ok(out)
            })
        })?,
    )?;

    // ---- world.tick / world.rng ----
    let st = Rc::clone(state);
    world.raw_set(
        "tick",
        lua.create_function(move |_, ()| -> Result<i64, Error> {
            with_sim_mut(&st, "world.tick", |sim| {
                Ok(i64::try_from(sim.tick()).unwrap_or(i64::MAX))
            })
        })?,
    )?;
    let st = Rc::clone(state);
    world.raw_set(
        "rng",
        lua.create_function(move |_, stream: String| -> Result<i64, Error> {
            with_sim_mut(&st, "world.rng", |sim| Ok(i64::from(sim.rng32(&stream))))
        })?,
    )?;

    // ---- world.isqrt（定点平方根：定量距离比较的唯一入口）----
    //
    // 为什么在 world 面而不是 `math.sqrt`：定量规则必须留在 Q32.32 定点域
    // （D-B5）。内核里唯一可以接受的平方根是定点逐位法（`ygg_sim::fixed::isqrt`，
    // 只有移位/比较/减法，native/wasm 位一致），它的结果还是定点值，于是
    // 「比距离」与「算坐标」永远在同一个数域里。浮点 sqrt 不进 `math`。
    world.raw_set(
        "isqrt",
        lua.create_function(move |_, value: LuaValue| -> Result<i64, Error> {
            let x = sim_fixed_arg(value, "world.isqrt")?;
            ygg_sim::fixed::isqrt(x).map_err(sim_to_lua)
        })?,
    )?;

    // ---- world.log(fmt, ...) ----
    let log_format: Function = lua.load(LOG_FORMAT_SRC).eval()?;
    let log_tostring: Function = lua.globals().raw_get("tostring")?;
    let st = Rc::clone(state);
    world.raw_set(
        "log",
        lua.create_function(
            move |_, (fmt, rest): (LuaValue, MultiValue)| -> Result<(), Error> {
                let line = format_line(&log_format, &log_tostring, fmt, rest)?;
                st.borrow_mut().world.push_log(line);
                Ok(())
            },
        )?,
    )?;

    // ---- world.system{ ... } ----
    let st = Rc::clone(state);
    world.raw_set(
        "system",
        lua.create_function(move |lua, spec: Table| -> Result<(), Error> {
            let name: String = spec.raw_get("name")?;
            let stage_name: String = spec.raw_get("stage")?;
            let stage = match stage_name.as_str() {
                "pre" => Stage::Pre,
                "rules" => Stage::Rules,
                "post" => Stage::Post,
                other => {
                    return Err(sim_to_lua(SimError::schema(format!(
                        "系统 `{name}` 的 stage 必须是 pre|rules|post，得到 `{other}`"
                    ))))
                }
            };
            let run: Function = spec.raw_get("run")?;
            let reads = names_field(&spec, "reads")?;
            let writes = names_field(&spec, "writes")?;
            let after = names_field(&spec, "after")?;

            with_sim_mut(&st, "world.system", |sim| {
                let read_ids: Vec<ComponentId> = resolve_ids(sim, &reads, &st)?;
                let write_ids: Vec<ComponentId> = resolve_ids(sim, &writes, &st)?;
                let after_refs: Vec<&str> = after.iter().map(String::as_str).collect();
                let decl = SystemDecl::new(name.as_str())
                    .reads(&read_ids)
                    .writes(&write_ids)
                    .after(&after_refs);
                let lua_for_system = lua.clone();
                let st_for_system = Rc::clone(&st);
                let sys_name = name.clone();
                sim.add_system(decl, stage, move |ctx: &mut SystemCtx<'_>| {
                    run_system(&lua_for_system, &st_for_system, &run, &sys_name, ctx)
                })
                .map_err(sim_to_lua)
            })
        })?,
    )?;

    // ---- world.seal() ----
    let st = Rc::clone(state);
    world.raw_set(
        "seal",
        lua.create_function(move |_, ()| -> Result<(), Error> {
            with_sim_mut(&st, "world.seal", |sim| sim.seal().map_err(sim_to_lua))
        })?,
    )?;

    // ---- world.kv：与 `ygg.kv` **同一张表**（同一份实现，没有第二个 kv）----
    world.raw_set("kv", ygg.raw_get::<Table>("kv")?)?;

    lua.globals().raw_set("world", world)?;
    Ok(())
}

/// `world.system` 的可选字符串数组字段（缺项 = 空表）。
fn names_field(spec: &Table, key: &str) -> Result<Vec<String>, Error> {
    let Some(list) = spec.raw_get::<Option<Table>>(key)? else {
        return Ok(Vec::new());
    };
    let mut out: Vec<String> = Vec::new();
    for item in list.sequence_values::<LuaValue>() {
        match item? {
            LuaValue::String(s) => out.push(s.to_str()?.to_owned()),
            other => {
                return Err(sim_to_lua(SimError::schema(format!(
                    "系统声明的 `{key}` 必须是字符串数组，遇到 {}",
                    other.type_name()
                ))))
            }
        }
    }
    Ok(out)
}

fn resolve_ids(
    sim: &Sim,
    names: &[String],
    state: &Rc<RefCell<HostState>>,
) -> Result<Vec<ComponentId>, Error> {
    let world = state.borrow();
    let registry = sim.world().registry();
    names
        .iter()
        .map(|name| Ok(slot_of(registry, &world.world, name)?.comp))
        .collect()
}

/// 系统体：把 `SystemCtx` 暂存进 `RefCell`，为本次调用造一张 `ctx` 表并调 `run(dt, ctx)`。
/// `run(dt, ctx)` 的 `dt` 与 `ctx.dt()` 都是 **Q32.32 裸值**——与 `ygg` 的 `sim.step(dt)`
/// 的入参**同一单位**。世界面的组件值是 `i64` 纯整数（schema 不带 `frac_bits`），
/// 所以面内没有第二套换算；擅自按 `to_int` 截断会把 60Hz 的 tick（dt ≈ 2^25）全截成 0，
/// 而擅自右移 32 位又与「调用方传给 `step` 的那个数」差 2³² 倍——两个都是静默错。
/// 要整数秒就在作者侧写 `dt >> 32`（`ygg-sim::fixed::ONE = 1 << 32`）。
fn run_system(
    lua: &Lua,
    state: &Rc<RefCell<HostState>>,
    run: &Function,
    name: &str,
    ctx: &mut SystemCtx<'_>,
) -> Result<(), SimError> {
    let dt = ctx.dt();
    let cell = RefCell::new(ctx);
    let outcome = lua.scope(|scope| {
        let view = ctx_table(lua, scope, state, &cell)?;
        run.call::<()>((dt, view))
    });
    outcome.map_err(|e| SimError::state(format!("系统 `{name}` 执行失败：{e}")))
}

/// `world.each` / `world.each_mut` 的共同实现（返回值 = 访问的行数）。
fn top_each(
    lua: &Lua,
    state: &Rc<RefCell<HostState>>,
    names: &Table,
    f: Function,
    mutate: bool,
) -> Result<i64, Error> {
    let what = if mutate {
        "world.each_mut"
    } else {
        "world.each"
    };
    let slots = {
        let slot = sim_slot(state);
        let guard = slot.try_borrow().map_err(|_| world_busy(what))?;
        let sim = guard.as_ref().ok_or_else(|| world_unmounted(what))?;
        let world = state.borrow();
        slots_of(sim.world().registry(), &world.world, names, what)?
    };
    if mutate {
        check_writable(&slots, what)?;
    }
    let comps: Vec<ComponentId> = slots.iter().map(|s| s.comp).collect();
    let slot = sim_slot(state);
    let mut guard = slot.try_borrow_mut().map_err(|_| world_busy(what))?;
    let sim: &mut Sim = guard.as_mut().ok_or_else(|| world_unmounted(what))?;
    let visited = iterate(lua, &slots, &f, mutate, |body| {
        sim.world_mut().for_each_mut(&comps, body)
    })?;
    Ok(i64::try_from(visited).unwrap_or(i64::MAX))
}

/// 从实体上摘掉若干组件（`world.remove` 的实现；`O(实体数)`，非热路径）。
fn remove_components(
    store: &mut ArchetypeStore,
    id: EntityId,
    slots: &[Slot],
) -> Result<i64, Error> {
    // ① 先校验后改：实体在、每个组件都在场。
    let (arch_idx, row) = store.locate(id).map_err(sim_to_lua)?;
    let arch = &store.archetypes()[arch_idx as usize];
    for slot in slots {
        if arch.key & slot.bit() == 0 {
            return Err(sim_to_lua(SimError::schema(format!(
                "实体 {id} 没有组件 `{}`，不能移除",
                slot.name
            ))));
        }
    }
    let removed_bits: u128 = slots.iter().fold(0, |acc, slot| acc | slot.bit());

    // ② 拆件（克隆上操作：失败时原 store 一字未动）。
    let (registry, mut archetypes, next_id) = store.clone().into_parts();

    // ③ 建/找目标桶：位集严格收缩 ⇒ 目标桶键必与源桶不同。
    let (target_key, target_cols) = {
        let src = &archetypes[arch_idx as usize];
        if src.key & !removed_bits == src.key {
            return Err(sim_to_lua(SimError::state(
                "remove 的目标桶与源桶相同（位集应当严格收缩）",
            )));
        }
        let mut col_ids: Vec<ComponentId> = Vec::with_capacity(src.col_ids.len());
        let mut cols: Vec<Column> = Vec::with_capacity(src.col_ids.len());
        for (p, comp) in src.col_ids.iter().enumerate() {
            if removed_bits & (1u128 << comp) != 0 {
                continue;
            }
            col_ids.push(*comp);
            cols.push(src.cols[p].empty_like());
        }
        (src.key & !removed_bits, (col_ids, cols))
    };
    let target_idx = match archetypes.iter().position(|a| a.key == target_key) {
        Some(idx) => idx,
        None => {
            archetypes.push(Archetype::new(target_key, target_cols.0, target_cols.1));
            archetypes.len() - 1
        }
    };

    // ④ 搬行：目标桶只追加不删，源桶摘尾填洞（索引在⑤统一重建）。
    {
        let (src, dst) = split_two(&mut archetypes, arch_idx as usize, target_idx);
        for p in 0..src.col_ids.len() {
            let comp = src.col_ids[p];
            if removed_bits & (1u128 << comp) != 0 {
                continue;
            }
            let q = dst.col_pos(comp).ok_or_else(|| {
                sim_to_lua(SimError::state(format!(
                    "目标桶缺少组件 id {comp} 的列（remove 建桶失败）"
                )))
            })?;
            dst.cols[q]
                .push_from(&src.cols[p], row as usize, comp)
                .map_err(sim_to_lua)?;
        }
        dst.ids.push(id);
        src.swap_remove_row(row as usize).map_err(sim_to_lua)?;
    }

    // ⑤ 重建 `by_key` / `entity_index`（两者都是私有派生面，只有这条口）。
    *store = ArchetypeStore::from_parts(registry, archetypes, next_id).map_err(sim_to_lua)?;
    Ok(i64::try_from(slots.len()).unwrap_or(i64::MAX))
}

/// 从 `&mut [T]` 里同时取两个**不相交**的可变借用（无 `unsafe`）。
fn split_two<T>(items: &mut [T], a: usize, b: usize) -> (&mut T, &mut T) {
    debug_assert_ne!(a, b, "split_two 的两个下标必须不同");
    if a < b {
        let (left, right) = items.split_at_mut(b);
        (&mut left[a], &mut right[0])
    } else {
        let (left, right) = items.split_at_mut(a);
        (&mut right[0], &mut left[b])
    }
}

/// `world.log(fmt, ...)` 的排版器：**经字符串元表**调 `string.format`。
///
/// M2 起 `string` 表已是沙箱全局（[`crate::sandbox`] 的 `LIBRARY_WHITELIST`），
/// 但这个一次性编译的包装器留着：它走的是**字符串值元表**那条路（`("x"):format()`），
/// 于是排版能力与「库表是否露头」脱钩——以后收紧或放宽库表白名单不必再动日志面。
/// 排版语义与 `string.format` 同一实现，**不另造格式化器**。
const LOG_FORMAT_SRC: &str = "return function(fmt, ...) return tostring(fmt):format(...) end";

/// `world.log(fmt, ...)`：结果进环形缓冲（[`WORLD_LOG_CAP`]），由宿主经
/// [`super::HostApi::take_world_log`] 取走。**不进**叙述流、**不进**事件流：
/// 日志是诊断面，混进那两条流会让「输出 = 状态」的对账多出一类噪声。
///
/// 排版失败（格式串里有多余 `%`、`%d` 配了字符串等）**不抛给游戏回路**：诊断面
/// 不许打断确定序，退化成各实参 `tostring` 以空格拼接。
fn format_line(
    format: &Function,
    tostring_fn: &Function,
    fmt: LuaValue,
    rest: MultiValue,
) -> Result<String, Error> {
    let mut argv = MultiValue::with_capacity(rest.len() + 1);
    argv.push_back(fmt.clone());
    for value in &rest {
        argv.push_back(value.clone());
    }
    if let Ok(line) = format.call::<String>(argv) {
        return Ok(line);
    }
    let mut parts = Vec::with_capacity(rest.len() + 1);
    parts.push(tostring_fn.call::<String>(fmt)?);
    for value in rest {
        parts.push(tostring_fn.call::<String>(value)?);
    }
    Ok(parts.join(" "))
}
