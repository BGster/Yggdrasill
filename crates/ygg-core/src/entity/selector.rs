use super::compare::CompareOp;
use super::store::Entity;
use super::value::Value;

/// 查询取值：真属性借用；`id` 伪属性就地构造（`Value::Int`，零分配）。
///
/// `id` 是**保留伪属性**（v1 §2.1 修订 ③）：真属性名 `id` 因此在 `where` / `order`
/// 里不可寻址，换来 `query` 对 `scan(lo, hi, tag)` 的语义覆盖。超出 `i64` 的 id 视同
/// 不在场（Lua 整数也表达不了）。
pub enum QueryValue<'a> {
    Attr(&'a Value),
    Id(Value),
}

impl QueryValue<'_> {
    pub fn value(&self) -> &Value {
        match self {
            Self::Attr(value) => value,
            Self::Id(value) => value,
        }
    }
}

impl Entity {
    /// 查询取值（`where` / `order` 的同一出处）：`id` 伪属性 + 真属性。
    pub fn query_value(&self, attr: &str) -> Option<QueryValue<'_>> {
        if attr == "id" {
            i64::try_from(self.id)
                .ok()
                .map(|id| QueryValue::Id(Value::Int(id)))
        } else {
            self.attribute(attr).map(QueryValue::Attr)
        }
    }
}

/// 一条谓词：属性名 + 运算 + `not.` 前缀。
#[derive(Clone, Debug, PartialEq)]
pub struct Predicate {
    pub attr: String,
    pub op: CompareOp,
    pub negated: bool,
}

impl Predicate {
    pub fn new(attr: impl Into<String>, op: CompareOp) -> Self {
        Self {
            attr: attr.into(),
            op,
            negated: false,
        }
    }

    /// `not.` 前缀修饰：取**整个谓词**的反，但**不翻**「缺属性 ⇒ 假」（v2 §1.2）。
    pub fn negate(mut self) -> Self {
        self.negated = true;
        self
    }

    /// 求值：`is` 管存在性，其余 op 对缺属性一律假。
    pub fn matches(&self, entity: &Entity) -> bool {
        let found = entity.query_value(&self.attr);
        if let CompareOp::Is(test) = &self.op {
            return test.holds(found.as_ref().map(QueryValue::value)) != self.negated;
        }
        let Some(found) = found else {
            return false;
        };
        self.op.matches(found.value()) != self.negated
    }
}

/// `order`：属性名 + 方向（族拼写 `field.asc` / `field.desc`；缺省 `asc`）。
#[derive(Clone, Debug, PartialEq)]
pub struct Order {
    pub attr: String,
    pub descending: bool,
}

/// 查询选择器（`ygg.entity.query(selector)` 的解码结果）。
///
/// 四个字段对应四个顶层键（多一个即 `FRAME`：拼错 `whre` 是错误，不是无过滤）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selector {
    /// tag 过滤（最便宜，先于 `where`）。
    pub tag: Option<String>,
    /// 隐式 AND 的谓词表，**保持解码序**（重排会把「写的序」变成隐形性能旋钮）。
    pub predicates: Vec<Predicate>,
    /// 输出序（`None` = id 升序）。
    pub order: Option<Order>,
    /// 投影键（空 = 裸 id 数组）。
    pub select: Vec<String>,
}

impl Selector {
    /// 存活判定：tag 先判、谓词数组序短路。
    pub fn matches(&self, entity: &Entity) -> bool {
        if let Some(tag) = &self.tag {
            if !entity.has_tag(tag) {
                return false;
            }
        }
        self.predicates.iter().all(|pred| pred.matches(entity))
    }
}
