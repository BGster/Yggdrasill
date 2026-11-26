use std::cmp::Ordering;

use super::compare::{value_compare, CompareOp};
use super::selector::{QueryValue, Selector};
use super::store::Entity;
use super::value::{EntityId, Value};
use super::EntityStore;

impl EntityStore {
    /// 查询（S24）：`selector` 决定存活集与输出序。
    ///
    /// 序：无 `order` ⇒ **id 升序**（`BTreeMap` 序遍历）；有 `order` ⇒ `(值, id)` 升序
    /// （`desc` 只反转值序，**id 兜底不反转**；缺属性排最后）。
    ///
    /// **`&mut self` 的理由**：可走索引的谓词会惰性首建该属性的索引（v2 §2.2）；v1 §3.1
    /// 写的 `&self` 在带索引的形态下不可实现（无内部可变性，而把索引放宿主侧已被 v2
    /// 否证）。候选集出来以后不再变——索引路径与全扫路径都逐条跑
    /// [`Selector::matches`]，索引只是剪枝。
    pub fn select(&mut self, selector: &Selector) -> Vec<&Entity> {
        let candidates = self.index_candidates(selector);
        let mut rows: Vec<&Entity> = match &candidates {
            Some(ids) => ids
                .iter()
                .filter_map(|id| self.entities.get(id))
                .filter(|entity| selector.matches(entity))
                .collect(),
            None => self
                .entities
                .values()
                .filter(|entity| selector.matches(entity))
                .collect(),
        };
        self.apply_order(&mut rows, selector);
        rows
    }

    /// `order` 排序（键预取一次，比较只读键：k 行 = k 次属性查 + 一次排序）。
    fn apply_order<'a>(&self, rows: &mut Vec<&'a Entity>, selector: &Selector) {
        let Some(order) = &selector.order else {
            return;
        };
        let mut keyed: Vec<(Option<QueryValue<'a>>, &'a Entity)> = rows
            .iter()
            .map(|entity| (entity.query_value(&order.attr), *entity))
            .collect();
        keyed.sort_unstable_by(|(left, a), (right, b)| {
            compare_order_keys(left, right, order.descending).then(a.id.cmp(&b.id))
        });
        *rows = keyed.into_iter().map(|(_, entity)| entity).collect();
    }
}

/// 排序的**值族**（v2 §5：三族，整浮同族）：`Num < Bool < Str`。
pub fn sort_family(value: &Value) -> u8 {
    match value {
        Value::Int(_) | Value::Float(_) => 0,
        Value::Bool(_) => 1,
        Value::Str(_) => 2,
    }
}

/// 跨族全序（`order` 用）：同族按值序，跨族按 [`sort_family`]。
/// 同族数值恒有 `Some`（`NaN` 不可达），`unwrap_or(Equal)` 只是形式兜底。
pub fn value_sort_cmp(a: &Value, b: &Value) -> Ordering {
    match sort_family(a).cmp(&sort_family(b)) {
        Ordering::Equal => match (a, b) {
            (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
            (Value::Str(x), Value::Str(y)) => x.cmp(y),
            _ => value_compare(a, b).unwrap_or(Ordering::Equal),
        },
        other => other,
    }
}

/// `(值, id)` 序的比较：缺属性排最后（nulls last，与方向无关），值序按方向反转，
/// **id 兜底永不反转**（v2 §5）。
fn compare_order_keys(
    left: &Option<QueryValue<'_>>,
    right: &Option<QueryValue<'_>>,
    descending: bool,
) -> Ordering {
    match (left, right) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => {
            let ord = value_sort_cmp(x.value(), y.value());
            if descending {
                ord.reverse()
            } else {
                ord
            }
        }
    }
}

// ── 属性索引（S24 v2 §2：惰性首建 + 写点增量维护）─────────────────────────
//
// 选型与已否证的两条路（v2 §2.2 逐条论证过，此处只留结论）：
// ① 不「标脏 + 下次重建」——重建 10⁴ 实体 ≈ 0.3 ms，**比不建索引的全扫（0.15 ms）
//    还贵 2×**，写多读少的循环里是净负；
// ② 不放宿主侧（`Rc<RefCell<QueryIndex>>`）——那样失效即重建回到①，而增量补丁要在
//    宿主 6 个写键上各调一次，漏一处 = 静默错答。放进 `EntityStore` 是唯一能让
//    「每个写点必须维护」成为结构约束的位置（五处补丁与 `revision += 1` 同行）。

/// 索引键：四种标量族各占一段，族序即变体声明序（`Num < Float < Bool < Str`）。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum IndexKey {
    Num(i64),
    /// 浮点段的键是**位序化 `u64`**（升序 u64 序 = 数值序）：零新依赖、逐位确定。
    Float(u64),
    Bool(bool),
    Str(String),
}

impl IndexKey {
    pub fn of(value: &Value) -> Self {
        match value {
            Value::Int(number) => Self::Num(*number),
            Value::Float(number) => Self::Float(Self::float_bits(*number)),
            Value::Bool(flag) => Self::Bool(*flag),
            Value::Str(text) => Self::Str(text.clone()),
        }
    }

    /// f64 → 单调 u64（负数取反、非负数翻符号位）。`NaN` 不可达（写入路径三处已拒）。
    fn float_bits(x: f64) -> u64 {
        let bits = x.to_bits();
        if bits >> 63 == 1 {
            !bits
        } else {
            bits | (1 << 63)
        }
    }

    /// [`IndexKey::float_bits`] 的逆。
    fn float_of(bits: u64) -> f64 {
        f64::from_bits(if bits >> 63 == 1 {
            bits ^ (1 << 63)
        } else {
            !bits
        })
    }
}

/// 比较谓词要的序关系集合（`eq` / `gt` / `gte` / `lt` / `lte` 各一条）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Want {
    Eq,
    Gt,
    Gte,
    Lt,
    Lte,
}

impl Want {
    fn holds(self, ord: Ordering) -> bool {
        match self {
            Self::Eq => ord == Ordering::Equal,
            Self::Gt => ord == Ordering::Greater,
            Self::Gte => ord != Ordering::Less,
            Self::Lt => ord == Ordering::Less,
            Self::Lte => ord != Ordering::Greater,
        }
    }
}

/// 一条谓词在索引上的候选区间（四段上的**并集**；AND = 两两求交）。
///
/// 它是**剪枝结构**，不是权威：候选集出表后仍逐条跑 [`Selector::matches`]。所以区间
/// 只须是超集，而 I2 差分用例钉住「索引路径 ≡ 全扫路径」逐值逐序相等。
#[derive(Clone, Debug, Default, PartialEq)]
struct Probe {
    num: Vec<(i64, i64)>,
    float: Vec<(u64, u64)>,
    boolean: Vec<bool>,
    text: Vec<String>,
}

/// 数值段的两个半域：整数段（闭区间）与浮点位序段（闭区间）。
type NumericSpans = (Option<(i64, i64)>, Option<(u64, u64)>);

impl Probe {
    /// 并（`in` 的元素之间）。
    fn union(&self, other: &Self) -> Self {
        let mut out = self.clone();
        out.num.extend_from_slice(&other.num);
        out.float.extend_from_slice(&other.float);
        out.boolean.extend_from_slice(&other.boolean);
        out.text.extend_from_slice(&other.text);
        out.num.sort_unstable();
        out.float.sort_unstable();
        out.boolean.sort_unstable();
        out.text.sort_unstable();
        out
    }

    /// 交（同一属性上的多条谓词 = 隐式 AND；`gte`+`lte` 在这里收成一个区间）。
    fn intersect(&self, other: &Self) -> Self {
        Self {
            num: intersect_spans(&self.num, &other.num),
            float: intersect_spans(&self.float, &other.float),
            boolean: self
                .boolean
                .iter()
                .copied()
                .filter(|flag| other.boolean.contains(flag))
                .collect(),
            text: self
                .text
                .iter()
                .filter(|text| other.text.contains(text))
                .cloned()
                .collect(),
        }
    }

    /// 数值段区间：`hit` 吃「该段的存储值」，`want` 非 `eq` 时真值区必是半线。
    ///
    /// 浮点段的二分域必须裁到 **`[bits(-∞), bits(+∞)]`**：位序化把 NaN 模式推到全域两端
    /// （`Float(u64::MAX)` 是 NaN 位型），全 u64 域上真值区的单调查性不成立。写入路径
    /// 本就拒非有限值 ⇒ 可达键全在这个闭区间内。
    fn numeric_spans(value: &Value, want: Want) -> NumericSpans {
        let hit = |stored: &Value| value_compare(stored, value).is_some_and(|ord| want.holds(ord));
        let num = monotone_span(i64::MIN as i128, i64::MAX as i128, |raw| {
            hit(&Value::Int(raw as i64))
        })
        .map(|(lo, hi)| (lo as i64, hi as i64));
        let float = monotone_span(
            IndexKey::float_bits(f64::NEG_INFINITY) as i128,
            IndexKey::float_bits(f64::INFINITY) as i128,
            |raw| hit(&Value::Float(IndexKey::float_of(raw as u64))),
        )
        .map(|(lo, hi)| (lo as u64, hi as u64));
        (num, float)
    }
}

/// 两个闭区间列表的交（两两求交，丢空）。列表长度 = `in` 的元素数，量级极小。
fn intersect_spans<T: Copy + Ord>(left: &[(T, T)], right: &[(T, T)]) -> Vec<(T, T)> {
    let mut out = Vec::new();
    for (lo_a, hi_a) in left {
        for (lo_b, hi_b) in right {
            let lo = (*lo_a).max(*lo_b);
            let hi = (*hi_a).min(*hi_b);
            if lo <= hi {
                out.push((lo, hi));
            }
        }
    }
    out
}

/// 单调真值区的二分（`hit` 必须单调 = 真值区是**前/后缀**）。
/// `None` = 全假；`Some((真区下界, 上界))`。
///
/// 只在索引路径的**探测建立期**跑（每次 ≤ 126 轮），不在扫描热路径上。
fn monotone_span(lo0: i128, hi0: i128, hit: impl Fn(i128) -> bool) -> Option<(i128, i128)> {
    if hit(hi0) {
        if hit(lo0) {
            return Some((lo0, hi0));
        }
        let (mut lo, mut hi) = (lo0, hi0);
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            if hit(mid) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        return Some((hi, hi0));
    }
    if !hit(lo0) {
        return None;
    }
    let (mut lo, mut hi) = (lo0, hi0);
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if hit(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some((lo0, lo))
}

/// 一条谓词的可索引探测；`None` = 不可索引（`neq` / `like` / `ilike` / `is`、`not.`、
/// 以及 Str/Bool 上的**序**比较——补集与存在性无法从桶枚举，v2 §3 回全扫）。
fn probe_of(op: &CompareOp) -> Option<Probe> {
    let (value, want) = match op {
        CompareOp::Eq(value) => (value, Want::Eq),
        CompareOp::Gt(value) => (value, Want::Gt),
        CompareOp::Gte(value) => (value, Want::Gte),
        CompareOp::Lt(value) => (value, Want::Lt),
        CompareOp::Lte(value) => (value, Want::Lte),
        CompareOp::In(values) => return in_probe(values),
        CompareOp::Neq(_) | CompareOp::Like(_) | CompareOp::ILike(_) | CompareOp::Is(_) => {
            return None
        }
    };
    let mut probe = Probe::default();
    match value {
        Value::Int(_) | Value::Float(_) if want == Want::Eq => {
            // `eq` 是**点**（Int 段最多两处：整/浮同族的舍入原像），不做二分：
            // 单调二分要求真值区是半线，点不是。
            probe.num = int_preimage(value);
            probe.float = float_preimage(value);
        }
        Value::Int(_) | Value::Float(_) => {
            let (num, float) = Probe::numeric_spans(value, want);
            probe.num.extend(num);
            probe.float.extend(float);
        }
        Value::Bool(flag) => {
            if want != Want::Eq {
                return None;
            }
            probe.boolean.push(*flag);
        }
        Value::Str(text) => {
            if want != Want::Eq {
                return None;
            }
            probe.text.push(text.clone());
        }
    }
    Some(probe)
}

/// `in`：逐元素 `eq` 的并；**空数组的探测是空集**（与「恒假」一致）。
fn in_probe(values: &[Value]) -> Option<Probe> {
    let mut probe: Option<Probe> = None;
    for value in values {
        let one = probe_of(&CompareOp::Eq(value.clone()))?;
        probe = Some(match probe {
            None => one,
            Some(acc) => acc.union(&one),
        });
    }
    Some(probe.unwrap_or_default())
}

/// Int 段上「与 `value` 数值相等」的点集。
fn int_preimage(value: &Value) -> Vec<(i64, i64)> {
    match value {
        Value::Int(number) => vec![(*number, *number)],
        Value::Float(number) => {
            // `(i as f64) == f` 的原像是**舍入原像**（|f| ≥ 2⁵³ 时可能有两个整数），
            // 候选只需 base-1/base/base+1（`as` 饱和，故用 saturating 运算）。
            let base = *number as i64;
            let mut out: Vec<(i64, i64)> = Vec::new();
            for candidate in [base.saturating_sub(1), base, base.saturating_add(1)] {
                if (candidate as f64) == *number && !out.contains(&(candidate, candidate)) {
                    out.push((candidate, candidate));
                }
            }
            out
        }
        _ => Vec::new(),
    }
}

/// Float 段上「与 `value` 数值相等」的点集（整浮同族：`Int(i)` 命中 `f = i as f64`）。
fn float_preimage(value: &Value) -> Vec<(u64, u64)> {
    let number = match value {
        Value::Int(number) => *number as f64,
        Value::Float(number) => *number,
        _ => return Vec::new(),
    };
    let bits = IndexKey::float_bits(number);
    vec![(bits, bits)]
}

impl EntityStore {
    /// 走索引判定（v2 §3 一句话可背）：**所有谓词落在同一属性、每条都可索引、且无
    /// `not.`** ⇒ 候选 id 集（升序去重）；其余一律 `None` = 全扫。
    ///
    /// 空 `where` 也回全扫（那就是列活物，v1 原路径）。`id` 伪属性走
    /// `entities.range()`——`BTreeMap` 本身就是 id 索引，零构建。
    fn index_candidates(&mut self, selector: &Selector) -> Option<Vec<EntityId>> {
        let first = selector.predicates.first()?;
        if selector
            .predicates
            .iter()
            .any(|predicate| predicate.attr != first.attr || predicate.negated)
        {
            return None;
        }
        let mut probe: Option<Probe> = None;
        for predicate in &selector.predicates {
            let one = probe_of(&predicate.op)?;
            probe = Some(match probe {
                None => one,
                Some(acc) => acc.intersect(&one),
            });
        }
        let probe = probe?;
        if first.attr == "id" {
            return Some(self.id_candidates(&probe));
        }
        self.ensure_index(&first.attr);
        Some(self.collect_candidates(&first.attr, &probe))
    }

    /// 从索引取候选 id（升序、去重）。
    fn collect_candidates(&self, attr: &str, probe: &Probe) -> Vec<EntityId> {
        let Some(index) = self.indices.get(attr) else {
            return Vec::new();
        };
        let mut out: Vec<EntityId> = Vec::new();
        for (lo, hi) in &probe.num {
            for (_, ids) in index.buckets.range(IndexKey::Num(*lo)..=IndexKey::Num(*hi)) {
                out.extend_from_slice(ids);
            }
        }
        for (lo, hi) in &probe.float {
            for (_, ids) in index
                .buckets
                .range(IndexKey::Float(*lo)..=IndexKey::Float(*hi))
            {
                out.extend_from_slice(ids);
            }
        }
        for flag in &probe.boolean {
            if let Some(ids) = index.buckets.get(&IndexKey::Bool(*flag)) {
                out.extend_from_slice(ids);
            }
        }
        for text in &probe.text {
            if let Some(ids) = index.buckets.get(&IndexKey::Str(text.clone())) {
                out.extend_from_slice(ids);
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// `id` 上的谓词：直接走 `BTreeMap` 键区间（零索引构建）。
    /// 超出 `i64` 的 id 在 `where` 里恒假（[`QueryValue`] 的同一裁断）。
    fn id_candidates(&self, probe: &Probe) -> Vec<EntityId> {
        let mut out: Vec<EntityId> = Vec::new();
        for (lo, hi) in &probe.num {
            let lo = (*lo).max(1);
            if *hi < lo {
                continue;
            }
            let (Ok(lo), Ok(hi)) = (u64::try_from(lo), u64::try_from(*hi)) else {
                continue;
            };
            out.extend(self.entities.range(lo..=hi).map(|(id, _)| *id));
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}
