use super::*;

// ── `ygg.entity.*` 批式面（P-K A1；铁律②「数组进出、单次调用承载整批」）──────────

/// 键集是**闭集**：六个单条面 + 五个批式面 + 两个控制面（`exists` / `destroy`，S9）
/// + 一个查询面（`query`，S24），多一个即红（防退化成逐实体数据 getter；控制面是单标量、
/// 不搬数据，查询面是一次跨界承载整批）。
#[test]
fn entity_face_keys_are_exactly_the_fourteen_documented_keys() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.keys", function()
            local out = ""
            for key in pairs(ygg.entity) do
                out = out .. key .. ","
            end
            ygg.narrate("KEYS:" .. out)
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let result = rig.run_full(&mut registry, "evt.keys", Value::Int(0));
    let line = result.narrative()[0]
        .strip_prefix("KEYS:")
        .expect("narrate 必须以 KEYS: 开头");
    let mut seen: Vec<&str> = line.split(',').filter(|s| !s.is_empty()).collect();
    seen.sort_unstable();
    let mut expected = vec![
        "add_tag",
        "attr_apply",
        "attr_fetch",
        "destroy",
        "exists",
        "get_attribute",
        "has_tag",
        "query",
        "remove_tag",
        "scan",
        "set_attribute",
        "spawn",
        "spawn_batch",
        "tag_batch",
    ];
    expected.sort_unstable();
    assert_eq!(
        seen, expected,
        "ygg.entity 的键集必须恰为这 14 个：{seen:?}"
    );
}

/// 批式面的行为：造 3 行（属性 + tag）、列式取回、同值不写、整/浮身份保留、
/// tag 过滤扫描、批量去 tag——一条用例把五个面的「值域/顺序/幂等」都钉住。
#[test]
fn entity_batch_face_round_trip_preserves_values_and_identity() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.batch", function()
            local made = ygg.entity.spawn_batch({
                { attrs = { hp = 10, title = "a" }, tags = { "enemy", "boss" } },
                { attrs = { hp = 2.5 }, tags = { "enemy" } },
                { attrs = {} },
            })
            ygg.narrate("SPAWNED:" .. made.spawned .. "|" .. made.next_id
                .. "|" .. made.ids[1] .. "," .. made.ids[3])

            local got = ygg.entity.attr_fetch({
                { id = made.ids[1], keys = { "hp", "title", "missing" } },
                { id = made.ids[2], keys = { "hp" } },
            })
            ygg.narrate("FETCH:" .. tostring(got.values[1].hp) .. "|"
                .. tostring(got.values[1].title) .. "|"
                .. tostring(got.values[1].missing) .. "|"
                .. tostring(got.values[2].hp))
            ygg.narrate("TAGS1:" .. #got.tags[1] .. ":" .. got.tags[1][1]
                .. "," .. got.tags[1][2])

            -- 同值写：applied 计 1、changed 计 0（不写、不发事件的上游依据）
            local same = ygg.entity.attr_apply({ { id = made.ids[1], attrs = { hp = 10 } } })
            -- 2.5 → 2：整/浮身份变了，值也确实变了
            local ident = ygg.entity.attr_apply({ { id = made.ids[2], attrs = { hp = 2 } } })
            ygg.narrate("APPLY:" .. same.applied .. "/" .. same.changed
                .. "|" .. ident.applied .. "/" .. ident.changed)

            local scanned = ygg.entity.scan(1, 10, "enemy")
            ygg.narrate("SCAN:" .. #scanned.ids .. ":" .. scanned.ids[1]
                .. "," .. scanned.ids[2])

            local removed = ygg.entity.tag_batch({
                { id = made.ids[1], tag = "boss", add = false },
            })
            local again = ygg.entity.tag_batch({
                { id = made.ids[1], tag = "boss", add = false },
            })
            ygg.narrate("TAGBATCH:" .. removed .. "|" .. again)
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let result = rig.run_full(&mut registry, "evt.batch", Value::Int(0));
    assert_eq!(
        result.narrative(),
        [
            "SPAWNED:3|4|1,3".to_string(),
            "FETCH:10|a|nil|2.5".to_string(),
            "TAGS1:2:boss,enemy".to_string(),
            "APPLY:1/0|1/1".to_string(),
            "SCAN:2:1,2".to_string(),
            "TAGBATCH:1|0".to_string(),
        ]
    );

    // 宿主侧直读：值真的落到 `EntityStore` 上，且 `2` 是 Int、`2.5` 曾是 Float。
    let entities = rig.handles.entities.borrow();
    let first = entities.get(1).expect("实体 1 必须在库里");
    assert_eq!(first.attribute("hp"), Some(&Value::Int(10)));
    assert_eq!(first.attribute("title"), Some(&Value::Str("a".to_string())));
    assert!(!first.has_tag("boss"), "tag_batch 必须真的把它摘掉");
    assert!(first.has_tag("enemy"));
    let second = entities.get(2).expect("实体 2 必须在库里");
    assert_eq!(
        second.attribute("hp"),
        Some(&Value::Int(2)),
        "2.5 → 2 必须是 Int"
    );
}

/// 三族错误码：形状（FRAME）/ 值域（VALUE）/ 未知 id（ID），且**整批零副作用**。
#[test]
fn entity_batch_face_rejects_before_mutating() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        local function first_line(err)
            local text = tostring(err)
            local cut = text:find("\n", 1, true)
            return cut and text:sub(1, cut - 1) or text
        end
        ygg.on("evt.reject", function()
            -- 非表入参由 mlua 的签名转换先拒（消息不同）；这里测「表里的元素形状错」
            local _, frame = pcall(function() return ygg.entity.spawn_batch({ "nope" }) end)
            ygg.narrate("FRAME:" .. first_line(frame))

            local made = ygg.entity.spawn_batch({ { attrs = { hp = 1 } } })
            local _, nan = pcall(function()
                return ygg.entity.attr_apply({ { id = made.ids[1], attrs = { hp = 0 / 0 } } })
            end)
            ygg.narrate("VALUE:" .. first_line(nan))

            local _, missing = pcall(function()
                return ygg.entity.attr_apply({ { id = 999, attrs = { hp = 1 } } })
            end)
            ygg.narrate("ID:" .. first_line(missing))

            -- 先校验后改：坏批之后的 hp 仍是 1
            local after = ygg.entity.attr_fetch({ { id = made.ids[1], keys = { "hp" } } })
            ygg.narrate("AFTER:" .. tostring(after.values[1].hp))
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let result = rig.run_full(&mut registry, "evt.reject", Value::Int(0));
    let lines = result.narrative();
    // mlua 会在 Rust error 前面加一层 `runtime error: `，所以按**包含**断言码。
    assert!(
        lines[0].contains("YGG-ENTITY-FRAME"),
        "形状错必须报 YGG-ENTITY-FRAME：{}",
        lines[0]
    );
    assert!(
        lines[1].contains("YGG-ENTITY-VALUE"),
        "NaN 必须报 YGG-ENTITY-VALUE：{}",
        lines[1]
    );
    assert!(
        lines[2].contains("YGG-ENTITY-ID"),
        "未知 id 必须报 YGG-ENTITY-ID：{}",
        lines[2]
    );
    assert_eq!(lines[3], "AFTER:1", "被拒的批不得留下半截写入");
}

// ── `ygg.entity.exists` / `ygg.entity.destroy`（S9 控制面两键）──────────────────

/// 存在性是内核真源的读面，真删是它的写面：两键合起来把「死亡」表达成
/// **记录不存在**（而不是墓碑位遮蔽），且删除幂等、缺失 id 不报错。
///
/// 钉住的四件事：① `exists` 真/假/从未存在；② `destroy` 首次 true、二次 false、
/// 未知 id false（不抛错）；③ 真删之后宿主直读 store：属性 / tag / 记录一起没了；
/// ④ I1——删后 `spawn` 拿到更大的 id（水位不动）。
#[test]
fn entity_lifecycle_exists_and_destroy_are_the_liveness_pair() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.life", function()
            local id = ygg.entity.spawn()
            ygg.entity.set_attribute(id, "hp", 7)
            ygg.entity.add_tag(id, "enemy")
            ygg.narrate("ALIVE:" .. tostring(ygg.entity.exists(id)))
            ygg.narrate("GHOST:" .. tostring(ygg.entity.exists(999)))

            local first = ygg.entity.destroy(id)
            local second = ygg.entity.destroy(id)
            local missing = ygg.entity.destroy(999)
            ygg.narrate("DESTROY:" .. tostring(first) .. ","
                .. tostring(second) .. "," .. tostring(missing))

            ygg.narrate("AFTER:" .. tostring(ygg.entity.exists(id)))
            ygg.narrate("HP:" .. tostring(ygg.entity.get_attribute(id, "hp")))
            ygg.narrate("TAG:" .. tostring(ygg.entity.has_tag(id, "enemy")))

            local fresh = ygg.entity.spawn()
            ygg.narrate("FRESH:" .. tostring(fresh) .. ">?old")
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let result = rig.run_full(&mut registry, "evt.life", Value::Int(0));
    assert_eq!(
        result.narrative(),
        [
            "ALIVE:true".to_string(),
            "GHOST:false".to_string(),
            "DESTROY:true,false,false".to_string(),
            "AFTER:false".to_string(),
            "HP:nil".to_string(),
            "TAG:false".to_string(),
            "FRESH:2>?old".to_string(),
        ]
    );

    // 宿主侧直读：真删就是真删——记录不在表里，属性与 tag 随记录一起回收。
    let entities = rig.handles.entities.borrow();
    assert_eq!(entities.len(), 1, "只剩删后新分配的那一条：{entities:?}");
    assert!(!entities.contains(1), "实体 1 必须整条消失");
    assert!(entities.contains(2), "删后 spawn 拿到的必须是新 id 2");
    assert_eq!(
        entities.next_entity_id(),
        3,
        "水位不动也不复用（I1）：remove 不碰 next_id"
    );
}
