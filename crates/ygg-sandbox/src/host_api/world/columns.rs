use std::collections::{BTreeMap, VecDeque};

use mlua::{Error, Table, Value as LuaValue};
use ygg_sim::component::{AttrType, ComponentDef, ComponentId, Registry};
use ygg_sim::error::SimError;
use ygg_sim::query::MAX_QUERY;

use super::values::sim_to_lua;
use super::WORLD_LOG_CAP;

// ---------------------------------------------------------------------------
// 声明面：kind 表 + 日志环
// ---------------------------------------------------------------------------

/// 组件在 `world` 面上的列型（相对内核列型的**增量**只有两点：`u8` 的值域、
/// `bytes` 在 `each_mut` 上不可写）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    I64,
    U64,
    U8,
    Bytes,
    /// 零宽成员：只能由宿主直接 `Sim::register` 声明（本面不提供 `tag` kind），
    /// 在 `world` 面上「值」恒为 `true`。
    Tag,
}

impl Kind {
    pub(super) fn parse(text: &str) -> Option<Self> {
        match text {
            "i64" => Some(Kind::I64),
            "u64" => Some(Kind::U64),
            "u8" => Some(Kind::U8),
            "bytes" => Some(Kind::Bytes),
            _ => None,
        }
    }

    /// 内核列型（`world.component` 的落库型）。
    pub(super) fn def(self) -> ComponentDef {
        match self {
            Kind::I64 | Kind::U8 => ComponentDef::number(),
            Kind::U64 => ComponentDef::reference(),
            Kind::Bytes => ComponentDef::string(),
            Kind::Tag => ComponentDef::tag(),
        }
    }

    /// 宿主直注册的组件（本面没记过声明）按内核列型反推。
    pub(super) fn from_attr(ty: AttrType) -> Option<Self> {
        match ty {
            AttrType::Number { .. } => Some(Kind::I64),
            AttrType::Ref => Some(Kind::U64),
            AttrType::Str => Some(Kind::Bytes),
            AttrType::Tag => Some(Kind::Tag),
            AttrType::Bool | AttrType::F64Bits => None,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Kind::I64 => "i64",
            Kind::U64 => "u64",
            Kind::U8 => "u8",
            Kind::Bytes => "bytes",
            Kind::Tag => "tag",
        }
    }
}

/// `world` 面的宿主侧状态（挂在 [`HostState`](crate::host_api::HostState) 上，与 `ygg.*` 共享同一次 install）。
///
/// `kinds` 只记 `world.component` 声明过的组件：`u8` 与 `i64` 都落 `number` 列，
/// 两者的区别（值域）只存在于声明里。宿主直注册的组件缺项时按注册表列型推导，
/// 因此这里**不是**第二份 schema——名字与 id 的唯一权威仍是内核 `Registry`。
#[derive(Default)]
pub struct WorldState {
    pub(super) kinds: BTreeMap<ComponentId, Kind>,
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
    pub(super) fn push_log(&mut self, line: String) {
        if self.logs.len() == WORLD_LOG_CAP {
            self.logs.pop_front();
        }
        self.logs.push_back(line);
    }

    /// 取走全部日志行（宿主 drain 口；顺序 = 追加序）。
    pub(crate) fn take_logs(&mut self) -> Vec<String> {
        self.logs.drain(..).collect()
    }
}

/// 一次解析出的组件引用（名字 + id + 列型）：错误消息要名字，内核调用要 id。
pub(super) struct Slot {
    pub(super) name: String,
    pub(super) comp: ComponentId,
    pub(super) kind: Kind,
}

impl Slot {
    pub(super) fn bit(&self) -> u128 {
        1u128 << self.comp
    }
}

pub(super) fn slot_of(registry: &Registry, world: &WorldState, name: &str) -> Result<Slot, Error> {
    let comp = registry.id_of(name).map_err(sim_to_lua)?;
    let kind = world.kind_of(registry, comp, name).map_err(sim_to_lua)?;
    Ok(Slot {
        name: name.to_string(),
        comp,
        kind,
    })
}

/// 组件名表（Lua 字符串序列）→ 槽位表：保序、去重、长度 ≤ [`MAX_QUERY`]。
pub(super) fn slots_of(
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
