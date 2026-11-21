use super::entity_decode::{entity_array, entity_value_arg};
use super::lua_type_name;
use super::{entity_frame, entity_id_error, entity_value};
use mlua::{Error, Table, Value as LuaValue};
use ygg_core::entity::{
    like_shape, normalize_f64, CompareOp, IsTest, Order, Predicate, Selector, Value,
};

/// `query(selector)` 的完整解码；返回即全部校验完毕（扫描前零副作用）。
///
/// 四个顶层键，多一个即 FRAME：`tag` / `where` / `select` / `order`。
pub fn entity_query_selector(selector: &Table) -> Result<Selector, Error> {
    let mut out = Selector::default();
    for pair in selector.clone().pairs::<LuaValue, LuaValue>() {
        let (key, value) = pair.map_err(|_| entity_frame("selector 的键值对不可读"))?;
        let name = match &key {
            LuaValue::String(text) => text
                .to_str()
                .map_err(|_| entity_frame("selector 键非 UTF-8"))?
                .to_string(),
            other => {
                return Err(entity_frame(format!(
                    "selector 的键必须是字符串，得到 {}",
                    lua_type_name(other)
                )))
            }
        };
        match name.as_str() {
            "tag" => match value {
                LuaValue::Nil => {}
                LuaValue::String(tag) => {
                    out.tag = Some(
                        tag.to_str()
                            .map_err(|_| entity_frame("`tag` 非 UTF-8"))?
                            .to_string(),
                    )
                }
                other => {
                    return Err(entity_frame(format!(
                        "`tag` 必须是字符串，得到 {}",
                        lua_type_name(&other)
                    )))
                }
            },
            "where" => match value {
                LuaValue::Nil => {}
                LuaValue::Table(map) => out.predicates = entity_predicate_map(&map)?,
                other => {
                    return Err(entity_frame(format!(
                        "`where` 必须是表，得到 {}",
                        lua_type_name(&other)
                    )))
                }
            },
            "select" => match value {
                LuaValue::Nil => {}
                LuaValue::Table(list) => out.select = entity_projection(&list)?,
                other => {
                    return Err(entity_frame(format!(
                        "`select` 必须是表，得到 {}",
                        lua_type_name(&other)
                    )))
                }
            },
            "order" => match value {
                LuaValue::Nil => {}
                LuaValue::String(text) => {
                    out.order = Some(entity_order(
                        &text
                            .to_str()
                            .map_err(|_| entity_frame("`order` 非 UTF-8"))?,
                    )?)
                }
                other => {
                    return Err(entity_frame(format!(
                        "`order` 必须是字符串，得到 {}",
                        lua_type_name(&other)
                    )))
                }
            },
            other => {
                return Err(entity_frame(format!(
                    "未知 selector 键 `{other}`（只有 tag / where / select / order）"
                )))
            }
        }
    }
    Ok(out)
}

/// `where` = 属性名 → 一个谓词（字符串态 / 结构态）或谓词数组（同键多条 = 隐式 AND）。
fn entity_predicate_map(map: &Table) -> Result<Vec<Predicate>, Error> {
    if map.raw_len() > 0 {
        // v1 的三元组数组形已退役：点名新形，防旧代码静默变成「无过滤」。
        return Err(entity_frame(
            "`where` 是**属性名 → 谓词**的映射（如 `{ hp = \"gte.10\" }`），不是三元组数组",
        ));
    }
    let mut out = Vec::new();
    for pair in map.clone().pairs::<LuaValue, LuaValue>() {
        let (key, value) = pair.map_err(|_| entity_frame("`where` 的键值对不可读"))?;
        let attr = match &key {
            LuaValue::String(text) => text
                .to_str()
                .map_err(|_| entity_frame("`where` 的属性名非 UTF-8"))?
                .to_string(),
            other => {
                return Err(entity_frame(format!(
                    "`where` 的键必须是字符串，得到 {}",
                    lua_type_name(other)
                )))
            }
        };
        match value {
            LuaValue::String(text) => {
                let text = text
                    .to_str()
                    .map_err(|_| entity_frame("谓词文本非 UTF-8"))?;
                out.push(text_predicate(&attr, &text)?);
            }
            LuaValue::Table(spec) => out.extend(entity_predicate_value(&attr, &spec)?),
            other => {
                return Err(entity_frame(format!(
                    "`{attr}` 的谓词必须是字符串或表，得到 {}",
                    lua_type_name(&other)
                )))
            }
        }
    }
    Ok(out)
}

/// 同键值的两态判别：**表里有 `op` 字符串键 ⇒ 结构态；否则视为谓词数组**
/// （数组不可能有 `op` 键 ⇒ 无歧义）。
fn entity_predicate_value(attr: &str, spec: &Table) -> Result<Vec<Predicate>, Error> {
    match spec.raw_get::<LuaValue>("op") {
        Ok(LuaValue::String(_)) => Ok(vec![structured_predicate(attr, spec)?]),
        Ok(LuaValue::Nil) => {
            let mut out = Vec::new();
            for item in entity_array(spec, attr)? {
                let LuaValue::String(text) = item else {
                    return Err(entity_frame(format!(
                        "`{attr}` 的谓词数组元素必须是字符串，得到 {}",
                        lua_type_name(&item)
                    )));
                };
                let text = text
                    .to_str()
                    .map_err(|_| entity_frame("谓词文本非 UTF-8"))?;
                out.push(text_predicate(attr, &text)?);
            }
            Ok(out)
        }
        Ok(other) => Err(entity_frame(format!(
            "`{attr}` 的 `op` 必须是字符串，得到 {}",
            lua_type_name(&other)
        ))),
        Err(_) => Err(entity_frame(format!("`{attr}` 的谓词表不可读"))),
    }
}

/// 字符串态一条谓词：`"op"` / `"op.operand"` / `"not.op.operand"`
/// （`op` = 首个 `.` 之前，其余 = 操作数，空余 = 空串）。
fn text_predicate(attr: &str, text: &str) -> Result<Predicate, Error> {
    let (negated, op_name, operand) = split_op_text(text);
    let operand = operand.unwrap_or("");
    let op = match op_name {
        "eq" => CompareOp::Eq(text_scalar(operand)?),
        "neq" => CompareOp::Neq(text_scalar(operand)?),
        "gt" => CompareOp::Gt(text_scalar(operand)?),
        "gte" => CompareOp::Gte(text_scalar(operand)?),
        "lt" => CompareOp::Lt(text_scalar(operand)?),
        "lte" => CompareOp::Lte(text_scalar(operand)?),
        "in" => text_in(operand)?,
        "like" | "ilike" => text_like(op_name, operand)?,
        "is" => CompareOp::Is(text_is(operand)?),
        other => {
            return Err(entity_frame(format!(
                "未知 op `{other}`（可选 eq/neq/gt/gte/lt/lte/in/like/ilike/is；`not.` 是前缀）"
            )))
        }
    };
    let predicate = Predicate::new(attr, op);
    Ok(if negated {
        predicate.negate()
    } else {
        predicate
    })
}

/// 结构态一条谓词：`{op = "gte", value = 10}`。`op` 可带 **`not.` 前缀**，但**不得**
/// 自带操作数（操作数只从 `value` 取）——两种来源同时存在就是歧义。
fn structured_predicate(attr: &str, spec: &Table) -> Result<Predicate, Error> {
    let mut op_text: Option<String> = None;
    let mut value: Option<LuaValue> = None;
    for pair in spec.clone().pairs::<LuaValue, LuaValue>() {
        let (key, item) = pair.map_err(|_| entity_frame("结构态谓词的键值对不可读"))?;
        let name = match &key {
            LuaValue::String(text) => text
                .to_str()
                .map_err(|_| entity_frame("结构态谓词键非 UTF-8"))?
                .to_string(),
            other => {
                return Err(entity_frame(format!(
                    "`{attr}` 的结构态谓词键必须是字符串，得到 {}",
                    lua_type_name(other)
                )))
            }
        };
        match name.as_str() {
            "op" => match item {
                LuaValue::String(text) => {
                    op_text = Some(
                        text.to_str()
                            .map_err(|_| entity_frame("`op` 非 UTF-8"))?
                            .to_string(),
                    )
                }
                other => {
                    return Err(entity_frame(format!(
                        "`{attr}` 的 `op` 必须是字符串，得到 {}",
                        lua_type_name(&other)
                    )))
                }
            },
            "value" => value = Some(item),
            other => {
                return Err(entity_frame(format!(
                    "结构态谓词只认 `op` / `value`，`{attr}` 多出 `{other}`"
                )))
            }
        }
    }
    let op_text = op_text.ok_or_else(|| entity_frame(format!("`{attr}` 的结构态谓词缺 `op`")))?;
    let (negated, op_name, inline) = split_op_text(&op_text);
    if inline.is_some() {
        return Err(entity_frame(format!(
            "结构态的 `op` 不能带操作数（操作数走 `value`），`{attr}` 得到 `{op_text}`"
        )));
    }
    let value = value.ok_or_else(|| entity_frame(format!("`{attr}` 的结构态谓词缺 `value`")))?;
    let op = match op_name {
        "eq" | "neq" | "gt" | "gte" | "lt" | "lte" => {
            let scalar = entity_value_arg(&value, attr)?;
            match op_name {
                "eq" => CompareOp::Eq(scalar),
                "neq" => CompareOp::Neq(scalar),
                "gt" => CompareOp::Gt(scalar),
                "gte" => CompareOp::Gte(scalar),
                "lt" => CompareOp::Lt(scalar),
                _ => CompareOp::Lte(scalar),
            }
        }
        "in" => {
            let LuaValue::Table(list) = value else {
                return Err(entity_frame(format!(
                    "`{attr}` 的 `in` 操作数必须是数组，得到 {}",
                    lua_type_name(&value)
                )));
            };
            let mut values = Vec::new();
            for item in entity_array(&list, attr)? {
                values.push(entity_value_arg(&item, attr)?);
            }
            CompareOp::In(values)
        }
        "like" | "ilike" => {
            let LuaValue::String(text) = value else {
                return Err(entity_frame(format!(
                    "`{attr}` 的 `{op_name}` 操作数必须是字符串，得到 {}",
                    lua_type_name(&value)
                )));
            };
            let text = text
                .to_str()
                .map_err(|_| entity_frame("`like` 操作数非 UTF-8"))?;
            text_like(op_name, &text)?
        }
        "is" => {
            let LuaValue::String(text) = value else {
                return Err(entity_frame(format!(
                    "`{attr}` 的 `is` 操作数必须是字符串，得到 {}",
                    lua_type_name(&value)
                )));
            };
            CompareOp::Is(text_is(
                &text
                    .to_str()
                    .map_err(|_| entity_frame("`is` 操作数非 UTF-8"))?,
            )?)
        }
        other => {
            return Err(entity_frame(format!(
                "未知 op `{other}`（可选 eq/neq/gt/gte/lt/lte/in/like/ilike/is；`not.` 是前缀）"
            )))
        }
    };
    let predicate = Predicate::new(attr, op);
    Ok(if negated {
        predicate.negate()
    } else {
        predicate
    })
}

/// `not.` 前缀 + `op[.operand]` 的切分（两态共用）。
fn split_op_text(text: &str) -> (bool, &str, Option<&str>) {
    let (negated, rest) = match text.strip_prefix("not.") {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    match rest.split_once('.') {
        Some((op, operand)) => (negated, op, Some(operand)),
        None => (negated, rest, None),
    }
}

/// 字符串态的标量解析（文档化、与 locale 无关的确定性规则）：
/// `-?[0-9]+(\.[0-9]+)?` → 数字；`true`/`false` → 布尔；其余 → 字符串；
/// `null` → **拒**（判空走 `is`）。
fn text_scalar(text: &str) -> Result<Value, Error> {
    if text == "null" {
        return Err(entity_frame("`null` 不是操作数（判空请用 `is.null`）"));
    }
    if text == "true" {
        return Ok(Value::Bool(true));
    }
    if text == "false" {
        return Ok(Value::Bool(false));
    }
    Ok(text_number(text).unwrap_or_else(|| Value::Str(text.to_owned())))
}

/// 纯数字形态 → `Int` / `Float`（带小数点才走浮点，与 Lua 的整浮身份直觉一致）。
fn text_number(text: &str) -> Option<Value> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    let (int_part, frac_part) = match digits.split_once('.') {
        Some((int_part, frac)) => (int_part, Some(frac)),
        None => (digits, None),
    };
    if int_part.is_empty() || !int_part.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if let Some(frac) = frac_part {
        if frac.is_empty() || !frac.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
    }
    match frac_part {
        // 超 i64 的整数字面量降为 Float（与 `Value` 反序列化 visitor 的取舍同源）
        None => Some(
            text.parse::<i64>()
                .map(Value::Int)
                .or_else(|_| text.parse::<f64>().map(Value::Float))
                .ok()
                .filter(|value| match value {
                    Value::Float(number) => number.is_finite(),
                    _ => true,
                })?,
        ),
        Some(_) => Some(Value::Float(
            text.parse::<f64>().ok().filter(|f| f.is_finite())?,
        )),
    }
}

/// 字符串态 `in.(a,b)`（空 `()` = 空数组 = 恒假）。
fn text_in(operand: &str) -> Result<CompareOp, Error> {
    let inner = operand
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
        .ok_or_else(|| {
            entity_frame(format!(
                "`in` 的字符串态必须是 `in.(a,b)`，得到 `{operand}`"
            ))
        })?;
    if inner.is_empty() {
        return Ok(CompareOp::In(Vec::new()));
    }
    let mut values = Vec::new();
    for item in inner.split(',') {
        if item.is_empty() {
            return Err(entity_frame("`in.(…)` 的元素不能为空"));
        }
        values.push(text_scalar(item)?);
    }
    Ok(CompareOp::In(values))
}

/// `like` / `ilike` 的形态门（四形态之外的写法是错，不是宽匹配）。
fn text_like(op_name: &str, pattern: &str) -> Result<CompareOp, Error> {
    like_shape(pattern).ok_or_else(|| {
        entity_frame(format!(
            "`{op_name}` 只接受四种形态：无 `*` / `P*` / `*S` / `*C*`，得到 `{pattern}`"
        ))
    })?;
    Ok(if op_name == "like" {
        CompareOp::Like(pattern.to_owned())
    } else {
        CompareOp::ILike(pattern.to_owned())
    })
}

/// `is` 的四种操作数。
fn text_is(operand: &str) -> Result<IsTest, Error> {
    IsTest::of(operand).ok_or_else(|| {
        entity_frame(format!(
            "`is` 的操作数只能是 `null` / `notnull` / `true` / `false`，得到 `{operand}`"
        ))
    })
}

/// `select`：非空具名出表；空数组 ≡ 缺省（裸 id 数组）。`"id"` 拒（行恒带 id）。
fn entity_projection(list: &Table) -> Result<Vec<String>, Error> {
    let mut out = Vec::new();
    for item in entity_array(list, "select")? {
        let LuaValue::String(text) = item else {
            return Err(entity_frame(format!(
                "`select` 的元素必须是字符串，得到 {}",
                lua_type_name(&item)
            )));
        };
        let name = text
            .to_str()
            .map_err(|_| entity_frame("`select` 元素非 UTF-8"))?
            .to_string();
        if name == "id" {
            return Err(entity_frame("`select` 不接受 `id`（行恒带 id）"));
        }
        out.push(name);
    }
    Ok(out)
}

/// `order = "field" / "field.asc" / "field.desc"`。
/// 属性名可含 `.`：**只有结尾恰为 `.asc` / `.desc` 才当方向**。
fn entity_order(text: &str) -> Result<Order, Error> {
    let (attr, descending) = if let Some(attr) = text.strip_suffix(".desc") {
        (attr, true)
    } else if let Some(attr) = text.strip_suffix(".asc") {
        (attr, false)
    } else {
        (text, false)
    };
    if attr.is_empty() {
        return Err(entity_frame(format!(
            "`order` 的属性名不能为空，得到 `{text}`"
        )));
    }
    Ok(Order {
        attr: attr.to_owned(),
        descending,
    })
}
