use super::*;

// ── `ygg.entity.query`（S24 通用实体查询引擎）─────────────────────────────────

/// 出表两形态 + `count` 兜底 + `where`/`tag`/`select`/`order` 四处接线。
///
/// 形态规则（契约 `packages/entity.md` §9.8）：`select` 缺省 = **裸 id 数组**；
/// 非空 = 行表（行恒带 `id`，缺属性**不设键**）；`count` 恒在，空结果可判。
#[test]
fn entity_query_ids_and_rows_shapes() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.query", function()
            ygg.entity.spawn_batch({
                { attrs = { hp = 50, name = "orc" }, tags = { "enemy" } },
                { attrs = { hp = 5.5, name = "rat" }, tags = { "enemy" } },
                { attrs = { hp = 100, name = "goblin" }, tags = { "enemy", "boss" } },
                { attrs = { name = "rock" } },
            })
            local function flat(list)
                local text = ""
                for i = 1, #list do
                    text = text .. (i > 1 and "," or "") .. tostring(list[i])
                end
                return text
            end
            local all = ygg.entity.query({})
            ygg.narrate("ALL:" .. all.count .. ":" .. flat(all.ids))
            ygg.narrate("EMPTY:" .. tostring(ygg.entity.query({ tag = "nobody" }).count))
            local enemy = ygg.entity.query({ tag = "enemy" })
            ygg.narrate("TAG:" .. enemy.count .. ":" .. flat(enemy.ids))
            local young = ygg.entity.query({ where = { hp = "lte.50" } })
            ygg.narrate("YOUNG:" .. young.count .. ":" .. flat(young.ids))
            local rows = ygg.entity.query({ where = { hp = "gte.50" }, select = { "hp", "name" } })
            local text = ""
            for i = 1, #rows.rows do
                local row = rows.rows[i]
                text = text .. (i > 1 and ";" or "") .. tostring(row.id)
                    .. "/" .. tostring(row.hp) .. "/" .. tostring(row.name)
                    .. "/" .. tostring(row.missing)
            end
            ygg.narrate("ROWS:" .. rows.count .. ":" .. text)
            -- 空结果的具名形态：`rows` 为空表、`count` 仍在（形态由 selector 决定）
            local none = ygg.entity.query({ where = { hp = "gt.1000" }, select = { "hp" } })
            ygg.narrate("NOROWS:" .. tostring(none.count) .. ":" .. tostring(#none.rows))
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let result = rig.run_full(&mut registry, "evt.query", Value::Int(0));
    assert_eq!(
        result.narrative(),
        [
            "ALL:4:1,2,3,4".to_string(),
            "EMPTY:0".to_string(),
            "TAG:3:1,2,3".to_string(),
            "YOUNG:2:1,2".to_string(),
            "ROWS:2:1/50/orc/nil;3/100/goblin/nil".to_string(),
            "NOROWS:0:0".to_string(),
        ]
    );
}

/// G2（v2 §6）：字符串态的五种形态一次钉死——
/// `op` / `op.operand` / `in.(…)` / `not.op.operand` / `is.xxx`。
#[test]
fn entity_query_string_form_parses_five_shapes() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.shapes", function()
            ygg.entity.spawn_batch({
                { attrs = { hp = 10 } },
                { attrs = { hp = 20 } },
                { attrs = { hp = 30, note = "x" } },
                { attrs = { name = "bare" } },
            })
            local function flat(list)
                local text = ""
                for i = 1, #list do
                    text = text .. (i > 1 and "," or "") .. tostring(list[i])
                end
                return text
            end
            local function ids(selector)
                return flat(ygg.entity.query(selector).ids)
            end
            -- ① 无操作数：操作数是空串（`eq` 一个字符串）⇒ 数值属性上恒不命中
            ygg.narrate("A:" .. ids({ where = { hp = "eq" } }))
            -- ② op.operand
            ygg.narrate("B:" .. ids({ where = { hp = "gte.20" } }))
            -- ③ in.(…)（数字按形态解析；空 `()` = 恒假）
            ygg.narrate("C:" .. ids({ where = { hp = "in.(10,30)" } }))
            ygg.narrate("C0:" .. ids({ where = { hp = "in.()" } }))
            -- ④ not.op.operand：整谓词取反，但**缺属性仍假**
            ygg.narrate("D:" .. ids({ where = { hp = "not.lt.20" } }))
            -- ⑤ is.null / is.notnull：存在性
            ygg.narrate("E:" .. ids({ where = { note = "is.null" } }))
            ygg.narrate("F:" .. ids({ where = { note = "is.notnull" } }))
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let result = rig.run_full(&mut registry, "evt.shapes", Value::Int(0));
    assert_eq!(
        result.narrative(),
        [
            "A:".to_string(),
            "B:2,3".to_string(),
            "C:1,3".to_string(),
            "C0:".to_string(),
            // 20 与 30 命中；**缺 hp 的 4 不命中**（`not.` 不翻「缺属性 ⇒ 假」）
            "D:2,3".to_string(),
            "E:1,2,4".to_string(),
            "F:3".to_string(),
        ]
    );
}

/// G3（v2 §6）：文法拒绝面。**先校验后扫**：形状错 `YGG-ENTITY-FRAME`、
/// 值域错 `YGG-ENTITY-VALUE`，且空库上照样拒（错误不依赖扫到什么）。
#[test]
fn entity_query_grammar_rejects() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        local function first_line(err)
            local text = tostring(err)
            local cut = text:find("\n", 1, true)
            return cut and text:sub(1, cut - 1) or text
        end
        ygg.on("evt.grammar", function()
            local function code(label, fn)
                local ok, err = pcall(fn)
                ygg.narrate(label .. ":" .. (ok and "OK" or first_line(err)))
            end
            -- 库是空的：下面的拒绝全都是「扫描前」发生的
            code("array_where", function() ygg.entity.query({ where = { { "hp", "gte", 1 } } }) end)
            code("unknown_op", function() ygg.entity.query({ where = { hp = "between.1" } }) end)
            code("unknown_key", function() ygg.entity.query({ whre = { hp = "gte.1" } }) end)
            code("like_wide", function() ygg.entity.query({ where = { name = "like.a*b" } }) end)
            code("like_bare", function() ygg.entity.query({ where = { name = "like.*" } }) end)
            code("in_null", function() ygg.entity.query({ where = { hp = "in.(1,null)" } }) end)
            code("eq_null", function() ygg.entity.query({ where = { hp = "eq.null" } }) end)
            code("is_operand", function() ygg.entity.query({ where = { hp = "is.maybe" } }) end)
            code("non_string_key", function() ygg.entity.query({ where = { [1] = "eq.1" } }) end)
            code("select_id", function() ygg.entity.query({ select = { "id" } }) end)
            code("tag_type", function() ygg.entity.query({ tag = 5 }) end)
            code("order_type", function() ygg.entity.query({ order = {} }) end)
            code("structured_extra",
                function() ygg.entity.query({ where = { hp = { op = "gte", value = 1, x = 1 } } }) end)
            code("structured_inline",
                function() ygg.entity.query({ where = { hp = { op = "gte.1", value = 1 } } }) end)
            code("structured_missing_value",
                function() ygg.entity.query({ where = { hp = { op = "gte" } } }) end)
            code("nan_value",
                function() ygg.entity.query({ where = { hp = { op = "gte", value = 0 / 0 } } }) end)
            code("in_non_scalar",
                function() ygg.entity.query({ where = { hp = { op = "in", value = { { 1 } } } } }) end)
            -- 合法但**不该**报错的两条（防把门做成过宽）
            code("ok_empty_where", function() ygg.entity.query({ where = {} }) end)
            code("ok_no_operand_eq", function() ygg.entity.query({ where = { name = "eq." } }) end)
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let result = rig.run_full(&mut registry, "evt.grammar", Value::Int(0));
    let lines = result.narrative();
    let frame = [
        "array_where",
        "unknown_op",
        "unknown_key",
        "like_wide",
        "like_bare",
        "in_null",
        "eq_null",
        "is_operand",
        "non_string_key",
        "select_id",
        "tag_type",
        "order_type",
        "structured_extra",
        "structured_inline",
        "structured_missing_value",
    ];
    for (index, label) in frame.iter().enumerate() {
        assert!(
            lines[index].contains("YGG-ENTITY-FRAME"),
            "{label} 必须报 YGG-ENTITY-FRAME：{}",
            lines[index]
        );
    }
    assert!(
        lines[15].contains("YGG-ENTITY-VALUE"),
        "NaN 必须报 YGG-ENTITY-VALUE：{}",
        lines[15]
    );
    assert!(
        lines[16].contains("YGG-ENTITY-VALUE"),
        "in 的非标量元素必须报 YGG-ENTITY-VALUE：{}",
        lines[16]
    );
    assert_eq!(lines[17], "ok_empty_where:OK", "空 where = 无过滤，不是错");
    assert_eq!(
        lines[18], "ok_no_operand_eq:OK",
        "`eq.` = 空串操作数，不是错"
    );
}

/// I4（v2 §6）：`order` 的值同 ⇒ id 升序兜底（asc/desc 都不反转）、
/// 缺属性排最后（与方向无关）、`id` 伪属性可排。
#[test]
fn entity_query_order_tiebreak() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.order", function()
            ygg.entity.spawn_batch({
                { attrs = { hp = 7 } },
                { attrs = { hp = 7 } },
                { attrs = { hp = 3 } },
                { attrs = {} },
            })
            local function flat(list)
                local text = ""
                for i = 1, #list do
                    text = text .. (i > 1 and "," or "") .. tostring(list[i])
                end
                return text
            end
            ygg.narrate("PLAIN:" .. flat(ygg.entity.query({}).ids))
            ygg.narrate("ASC:" .. flat(ygg.entity.query({ order = "hp" }).ids))
            ygg.narrate("ASC2:" .. flat(ygg.entity.query({ order = "hp.asc" }).ids))
            ygg.narrate("DESC:" .. flat(ygg.entity.query({ order = "hp.desc" }).ids))
            ygg.narrate("IDDESC:" .. flat(ygg.entity.query({ order = "id.desc" }).ids))
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let result = rig.run_full(&mut registry, "evt.order", Value::Int(0));
    assert_eq!(
        result.narrative(),
        [
            "PLAIN:1,2,3,4".to_string(),
            // 3 < 7 = 7（id 兜底 1 < 2）< 缺属性
            "ASC:3,1,2,4".to_string(),
            "ASC2:3,1,2,4".to_string(),
            // desc 只反转值序：7 = 7 的 id 兜底仍升序，缺属性仍在最后
            "DESC:1,2,3,4".to_string(),
            "IDDESC:4,3,2,1".to_string(),
        ]
    );
}
