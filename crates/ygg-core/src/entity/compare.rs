use std::cmp::Ordering;

use super::value::Value;

// ============================================================================
// 通用实体查询引擎（S24；契约 `openwiki/contracts/packages/entity.md` §9.8）
// ============================================================================
//
// 语义住在 core（无 VM 可单测）：选择器、比较矩阵与序遍历都在这里，宿主侧只做文法
// 解码（`host_api::entity_query_selector`）与两阶段材质化。这样「跨族比较是假不是
// 错」「缺属性一律假」「`is` 管存在性」这几条裁断不会随宿主重构漂。

/// 比较运算（PostgREST 族裁剪版的**执行形态**）。
///
/// `not` 不是独立 op，它是 [`Predicate::negated`] 上的前缀修饰；`in` 在谓词层展开
/// （任一元素 `eq` 成立即真）。所以枚举里**没有算术臂**——浮点不可结合性无从发生
/// （确定性论证第一条，v1 §4）。
#[derive(Clone, Debug, PartialEq)]
pub enum CompareOp {
    Eq(Value),
    Neq(Value),
    Gt(Value),
    Gte(Value),
    Lt(Value),
    Lte(Value),
    /// 标量数组，任一 `eq` 成立即真；**空数组恒假**。
    In(Vec<Value>),
    /// `*` 唯一通配符、四形态（[`like_shape`]）；越界形态在解码期已拒。
    Like(String),
    /// `like` + **ASCII** 小写折叠（Unicode 折叠表是版本相关依赖，不做）。
    ILike(String),
    /// **存在性测试**：唯一不受「缺属性 ⇒ 假」管辖的 op。
    Is(IsTest),
}

/// `is` 的四种测试（`null` = 不在场、`notnull` = 在场、`true`/`false` = 在场且布尔相等）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IsTest {
    Null,
    NotNull,
    True,
    False,
}

impl CompareOp {
    /// 存储值在场时的求值（缺属性由 [`Predicate::matches`] 先判）。
    ///
    /// 「跨族 ⇒ 假」对**所有** op 成立（含 `neq`）：跨族比较不是错误（扫描中报错会
    /// 产出半截结果），故 `neq` 不是全关系——契约明写。
    pub fn matches(&self, stored: &Value) -> bool {
        match self {
            Self::Eq(v) => value_compare(stored, v) == Some(Ordering::Equal),
            Self::Neq(v) => value_compare(stored, v).is_some_and(|ord| ord != Ordering::Equal),
            Self::Gt(v) => value_compare(stored, v) == Some(Ordering::Greater),
            Self::Gte(v) => matches!(
                value_compare(stored, v),
                Some(Ordering::Greater | Ordering::Equal)
            ),
            Self::Lt(v) => value_compare(stored, v) == Some(Ordering::Less),
            Self::Lte(v) => matches!(
                value_compare(stored, v),
                Some(Ordering::Less | Ordering::Equal)
            ),
            Self::In(list) => list
                .iter()
                .any(|v| value_compare(stored, v) == Some(Ordering::Equal)),
            Self::Like(pattern) => match stored {
                Value::Str(text) => like_matches(text, pattern),
                _ => false,
            },
            Self::ILike(pattern) => match stored {
                Value::Str(text) => ilike_matches(text, pattern),
                _ => false,
            },
            Self::Is(test) => test.holds(Some(stored)),
        }
    }
}

/// `like` 的四种合法形态（v2 §4「越窄越好」：四形态各对应一条线性扫描原语，
/// 零正则、零回溯、零 ReDoS 面）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LikeShape {
    /// 无 `*`（≡ `eq`）。
    Exact,
    /// `P*`
    Prefix,
    /// `*S`
    Suffix,
    /// `*C*`
    Contains,
}

/// 判 `like` 形态；`None` = 越界形态（解码期按 `YGG-ENTITY-FRAME` 拒）。
///
/// 合法：无 `*` / `P*` / `*S` / `*C*`，且**字面段非空**——裸 `*` 与 `**` 不属于四形态
/// 中任何一种（字面段为空 = 「全匹配」，与笔误不可区分）。`*` 不可转义：要匹配字面
/// `*` 用 `eq`。
pub fn like_shape(pattern: &str) -> Option<LikeShape> {
    let stars = pattern.bytes().filter(|byte| *byte == b'*').count();
    match stars {
        0 => Some(LikeShape::Exact),
        1 if pattern.starts_with('*') => non_empty(&pattern[1..]).map(|()| LikeShape::Suffix),
        1 if pattern.ends_with('*') => {
            non_empty(&pattern[..pattern.len() - 1]).map(|()| LikeShape::Prefix)
        }
        2 if pattern.starts_with('*') && pattern.ends_with('*') => {
            non_empty(&pattern[1..pattern.len() - 1]).map(|()| LikeShape::Contains)
        }
        _ => None,
    }
}

fn non_empty(part: &str) -> Option<()> {
    if part.is_empty() {
        None
    } else {
        Some(())
    }
}

/// `like` 匹配（越界形态保守为假——解码期已拒，此处不 panic）。
pub fn like_matches(haystack: &str, pattern: &str) -> bool {
    match like_shape(pattern) {
        Some(LikeShape::Exact) => haystack == pattern,
        Some(LikeShape::Prefix) => haystack.starts_with(&pattern[..pattern.len() - 1]),
        Some(LikeShape::Suffix) => haystack.ends_with(&pattern[1..]),
        Some(LikeShape::Contains) => haystack.contains(&pattern[1..pattern.len() - 1]),
        None => false,
    }
}

/// `ilike`：两侧都做 ASCII 折叠后走同一套形态匹配（`*` 是 ASCII，形态不变）。
pub fn ilike_matches(haystack: &str, pattern: &str) -> bool {
    like_matches(
        &haystack.to_ascii_lowercase(),
        &pattern.to_ascii_lowercase(),
    )
}

/// 跨族比较的唯一判据（v1 §2.3 矩阵）：**同族**返回值序，**跨族**返回 `None`
/// （调用方按「假」处置）。
///
/// `Int × Float` 按 f64 提升值比较（`|i| > 2⁵³` 亦如此）；`NaN` 不可达（写入路径三处
/// 拒绝），故 `partial_cmp` 的 `None` 只会因跨族出现。不给 `Value` 实现 `PartialOrd`：
/// 跨族语义不该长成 `Ord`。
pub fn value_compare(a: &Value, b: &Value) -> Option<Ordering> {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => Some(x.cmp(y)),
        (Value::Int(x), Value::Float(y)) => (*x as f64).partial_cmp(y),
        (Value::Float(x), Value::Int(y)) => x.partial_cmp(&(*y as f64)),
        (Value::Float(x), Value::Float(y)) => x.partial_cmp(y),
        (Value::Bool(x), Value::Bool(y)) => Some(x.cmp(y)),
        (Value::Str(x), Value::Str(y)) => Some(x.cmp(y)),
        _ => None,
    }
}
