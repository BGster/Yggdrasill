use serde::{Deserialize, Serialize};

/// 实体 ID：内核/宿主分配的持久标识（`u64`，规范序 = 数值升序）。
pub type EntityId = u64;

/// 属性标量值。M0 收敛为数字 / 布尔 / 字符串（research §3 威胁 #9：
/// 可哈希状态禁表 / userdata 键；复合结构属资源面，走 `ygg.data`，S11）。
///
/// 序列化形态 = **未标记标量**（`Serialize` / `Deserialize` 均手写、两向对称）：
/// 派生 `Deserialize` 是外部标记形态（如 `{"Int":7}`），与本类型未标记标量
/// 编码（`7`）不对称，往返即断——故两个方向都手写（S2 amendment 2，2026-10-27）。
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
}

impl Value {
    /// 便利构造 `Float`：NaN 返回 `None`（写入路径的守门构造器）。
    pub fn float(x: f64) -> Option<Self> {
        if x.is_nan() {
            None
        } else {
            Some(Value::Float(x))
        }
    }
}

// `Serialize` 手写：`-0.0 → 0.0` 在编码侧兜底（写入路径 `set_attribute` 已先行
// 规范化，此处覆盖反序列化旁路构造出的非规范值——纵深防御，非唯一防线）。
// 非有限值不做规范化——`canonical_snapshot` 先行校验拒绝（非有限值是错误，
// 不是静默改写）。
impl Serialize for Value {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::Int(v) => v.serialize(serializer),
            Value::Float(v) => normalize_f64(*v).serialize(serializer),
            Value::Bool(v) => v.serialize(serializer),
            Value::Str(v) => v.serialize(serializer),
        }
    }
}

// `Deserialize` 手写（S2 amendment 2）：与手写 `Serialize` 对称的未标记标量
// 形态。`deserialize_any` + 标量 visitor：
// - 整数 token 正数走 `visit_u64`、负数走 `visit_i64`，均归一为 `Int`；
//   超 `i64` 范围的 `u64` 降级为 `Float`（f64 对该区间可表示但可能失精度——
//   M0 标量场景的文档化取舍）；
// - 浮点走 `visit_f64`：非有限值在反序列化旁路即拒绝（`Error::custom`——
//   反序列化发生在裸 `Value` 上、无实体上下文，带定位的 `NonFiniteF64`
//   错误在此不可得；`snapshot::validate_finite` 仍是权威层，本处为纵深防御）；
// - 布尔 / 字符串直通。
impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ScalarVisitor;
        impl<'de> serde::de::Visitor<'de> for ScalarVisitor {
            type Value = Value;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an integer, float, boolean or string scalar")
            }

            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Value, E> {
                Ok(Value::Int(v))
            }

            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Value, E> {
                Ok(match i64::try_from(v) {
                    Ok(i) => Value::Int(i),
                    Err(_) => Value::Float(v as f64),
                })
            }

            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Value, E> {
                if v.is_finite() {
                    Ok(Value::Float(v))
                } else {
                    Err(serde::de::Error::custom("non-finite float in stored value"))
                }
            }

            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Value, E> {
                Ok(Value::Bool(v))
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Value, E> {
                Ok(Value::Str(v.to_owned()))
            }
        }
        deserializer.deserialize_any(ScalarVisitor)
    }
}

/// `-0.0 → 0.0` 浮点规范化唯一入口（native / wasm 位模式统一）。
pub fn normalize_f64(x: f64) -> f64 {
    if x == 0.0 {
        0.0
    } else {
        x
    }
}
