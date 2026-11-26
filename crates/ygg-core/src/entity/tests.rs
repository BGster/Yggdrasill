use std::cmp::Ordering;

use super::*;

#[test]
fn spawn_allocates_monotonic_ids_from_one() {
    let mut store = EntityStore::new();
    assert_eq!(store.spawn().unwrap(), 1);
    assert_eq!(store.spawn().unwrap(), 2);
    assert_eq!(store.spawn().unwrap(), 3);
    assert_eq!(store.next_entity_id(), 4);
}

#[test]
fn manual_default_starts_ids_at_one() {
    // 手写 Default 委托 new()：派生缺省会给出 next_id = 0（CONCERN 2 回归锚）。
    let mut store = EntityStore::default();
    assert_eq!(store.next_entity_id(), 1);
    assert_eq!(store.spawn().unwrap(), 1);
}

#[test]
fn insert_advances_next_id_and_rejects_duplicates() {
    let mut store = EntityStore::new();
    let mut e = Entity::new(10);
    assert!(e.add_tag("hero"));
    store.insert(e).unwrap();
    assert_eq!(store.next_entity_id(), 11);
    assert!(store.spawn().unwrap() >= 11);
    assert!(matches!(
        store.insert(Entity::new(10)),
        Err(StateError::DuplicateEntity(10))
    ));
}

#[test]
fn set_attribute_rejects_non_finite_with_context() {
    let mut store = EntityStore::new();
    let id = store.spawn().unwrap();
    let err = store
        .set_attribute(id, "hp", Value::Float(f64::NAN))
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("hp") && msg.contains("is non-finite"));
}

#[test]
fn set_attribute_rejects_infinity() {
    let mut store = EntityStore::new();
    let id = store.spawn().unwrap();
    assert!(matches!(
        store.set_attribute(id, "atk", Value::Float(f64::INFINITY)),
        Err(StateError::NonFiniteF64 { .. })
    ));
    assert!(matches!(
        store.set_attribute(id, "def", Value::Float(f64::NEG_INFINITY)),
        Err(StateError::NonFiniteF64 { .. })
    ));
}

#[test]
fn negative_zero_is_normalized_on_write() {
    let mut store = EntityStore::new();
    let id = store.spawn().unwrap();
    store.set_attribute(id, "bias", Value::Float(-0.0)).unwrap();
    // 断言对象 = 存储值本身（写入路径单点门），不依赖编码侧兜底。
    match store.get(id).unwrap().attribute("bias") {
        Some(Value::Float(f)) => {
            assert_eq!(*f, 0.0);
            assert!(f.is_sign_positive());
        }
        other => panic!("expected Float(0.0), got {other:?}"),
    }
}

#[test]
fn value_float_constructor_gates_nan() {
    assert!(Value::float(f64::NAN).is_none());
    assert!(matches!(Value::float(1.5), Some(Value::Float(_))));
}

#[test]
fn revision_bumps_exactly_on_actual_state_changes() {
    let mut store = EntityStore::new();
    assert_eq!(store.revision(), 0);
    let id = store.spawn().unwrap();
    assert_eq!(store.revision(), 1);
    store.set_attribute(id, "hp", Value::Int(1)).unwrap();
    assert_eq!(store.revision(), 2);
    store.set_attribute(id, "hp", Value::Int(1)).unwrap(); // 同值覆写：幂等不计
    assert_eq!(store.revision(), 2);
    assert!(store.add_tag(id, "hero").unwrap());
    assert_eq!(store.revision(), 3);
    assert!(!store.add_tag(id, "hero").unwrap()); // 幂等：不计变更
    assert_eq!(store.revision(), 3);
    assert!(store.remove_tag(id, "hero").unwrap());
    assert_eq!(store.revision(), 4);
    assert!(!store.remove_tag(id, "hero").unwrap()); // 幂等：不计变更
    assert_eq!(store.revision(), 4);
    let mut e = Entity::new(9);
    assert!(e.add_tag("npc"));
    store.insert(e).unwrap();
    assert_eq!(store.revision(), 5);
    store.set_attribute(9, "hp", Value::Int(2)).unwrap();
    assert_eq!(store.revision(), 6);
    assert!(store.remove_attribute(9, "hp").unwrap().is_some());
    assert_eq!(store.revision(), 7);
    assert!(store.remove_attribute(9, "hp").unwrap().is_none()); // 幂等
    assert_eq!(store.revision(), 7);
    assert!(store.remove(9).is_some());
    assert_eq!(store.revision(), 8);
    assert!(store.remove(9).is_none()); // 幂等
    assert_eq!(store.revision(), 8);
}

#[test]
fn remove_keeps_the_id_watermark_so_ids_are_never_reused() {
    // S9 锚：真删回收「本体」，但**不回收 id**——`remove` 只动实体表与 `revision`，
    // `next_id` 是独立水位。所以 I1（id 永不复用）与「本体是否永存」无关：
    // 宿主面 `ygg.entity.destroy` 正是建在这条性质上的（回收面不该破分配纪律）。
    let mut store = EntityStore::new();
    let first = store.spawn().unwrap();
    let doomed = store.spawn().unwrap();
    store.set_attribute(doomed, "hp", Value::Int(5)).unwrap();
    store.add_tag(doomed, "enemy").unwrap();
    let watermark = store.next_entity_id();
    let revision_before = store.revision();

    let removed = store.remove(doomed).expect("在场实体必须被移除");
    assert_eq!(
        removed.attribute("hp"),
        Some(&Value::Int(5)),
        "属性随记录一起返回（真删而不是丢引用）"
    );
    assert!(removed.has_tag("enemy"), "tag 也随记录一起回收");
    assert_eq!(store.next_entity_id(), watermark, "真删不得回收 id 水位");
    assert_eq!(store.revision(), revision_before + 1, "真删计一次变更");
    assert!(
        !store.contains(doomed),
        "真删后 contains = false（exists 的 store 层语义）"
    );
    assert!(store.get(doomed).is_none());
    assert!(
        store.remove(doomed).is_none(),
        "幂等：二次删除返回 None、不报错"
    );
    assert_eq!(
        store.revision(),
        revision_before + 1,
        "幂等删除不涨 revision"
    );

    let next = store.spawn().unwrap();
    assert_ne!(next, doomed, "I1：id 永不复用");
    assert!(
        next > doomed && next > first,
        "删后新 id 必须更大：{next} vs doomed={doomed}"
    );
}

#[test]
fn store_mutators_reject_missing_entity() {
    let mut store = EntityStore::new();
    assert!(matches!(
        store.set_attribute(7, "hp", Value::Int(1)),
        Err(StateError::NoSuchEntity(7))
    ));
    assert!(matches!(
        store.remove_attribute(7, "hp"),
        Err(StateError::NoSuchEntity(7))
    ));
    assert!(matches!(
        store.add_tag(7, "x"),
        Err(StateError::NoSuchEntity(7))
    ));
    assert!(matches!(
        store.remove_tag(7, "x"),
        Err(StateError::NoSuchEntity(7))
    ));
}

#[test]
fn tags_and_attributes_iterate_in_sorted_order() {
    let mut e = Entity::new(1);
    assert!(e.add_tag("zeta"));
    assert!(e.add_tag("alpha"));
    e.set_attribute("z", Value::Int(1)).unwrap();
    e.set_attribute("a", Value::Int(2)).unwrap();
    let names: Vec<_> = e.attributes().map(|(k, _)| k).collect();
    assert_eq!(names, ["a", "z"]);
    let tags: Vec<_> = e.tags().collect();
    assert_eq!(tags, ["alpha", "zeta"]);
    assert!(!e.remove_tag("nope"));
}

#[test]
fn value_round_trip_untagged_scalar_form() {
    // S2 amendment 2 回归锚：未标记标量形态两向往返（原派生 Deserialize
    // 为外部标记形态，与本类型未标记编码不对称，往返即断）。
    for v in [
        Value::Int(-3),
        Value::Int(42),
        Value::float(1.5).unwrap(),
        Value::Bool(true),
        Value::Str("feni".into()),
    ] {
        let bytes = serde_json::to_vec(&v).unwrap();
        let back: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v, back);
    }
    // 整数 token：正数走 visit_u64、负数走 visit_i64，均归一为 Int
    let pos: Value = serde_json::from_str("7").unwrap();
    assert_eq!(pos, Value::Int(7));
    let neg: Value = serde_json::from_str("-7").unwrap();
    assert_eq!(neg, Value::Int(-7));
    // 超 i64 范围的 u64 降级为 Float（文档化取舍：M0 标量场景可接受）
    let huge: Value = serde_json::from_str("18446744073709551615").unwrap();
    assert!(matches!(huge, Value::Float(_)));
}

#[test]
fn serde_round_trip_preserves_sorted_content() {
    let mut store = EntityStore::new();
    let id = store.spawn().unwrap();
    store.set_attribute(id, "hp", Value::Int(7)).unwrap();
    let bytes = serde_json::to_vec(&store).unwrap();
    let restored: EntityStore = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(store, restored);
    assert_eq!(restored.next_entity_id(), 2);
    // spawn(+1) + set_attribute(+1) = revision 2（全字段序列化含 revision）
    assert_eq!(restored.revision(), 2);
}

// ── 查询引擎（S24）─────────────────────────────────────────────────────

/// 建一个实体（属性袋 + tag 名单）。
fn seeded(id: EntityId, attrs: &[(&str, Value)], tags: &[&str]) -> Entity {
    let mut entity = Entity::new(id);
    for (name, value) in attrs {
        entity.set_attribute(*name, value.clone()).unwrap();
    }
    for tag in tags {
        entity.add_tag(*tag);
    }
    entity
}

/// 四族矩阵（v1 §2.3）：同族有值序，**跨族一律 `None`**（调用方按假处置）。
#[test]
fn value_compare_matrix_is_within_family_only() {
    let int = Value::Int(2);
    let float = Value::Float(2.0);
    let boolean = Value::Bool(true);
    let text = Value::Str("2".into());

    assert_eq!(value_compare(&int, &Value::Int(3)), Some(Ordering::Less));
    assert_eq!(
        value_compare(&float, &Value::Float(2.5)),
        Some(Ordering::Less)
    );
    assert_eq!(
        value_compare(&Value::Bool(false), &boolean),
        Some(Ordering::Less)
    );
    assert_eq!(
        value_compare(&Value::Str("a".into()), &Value::Str("b".into())),
        Some(Ordering::Less)
    );
    // 整浮同族 + 双向：2 == 2.0（f64 提升值比较）
    assert_eq!(value_compare(&int, &float), Some(Ordering::Equal));
    assert_eq!(value_compare(&float, &int), Some(Ordering::Equal));
    // 整浮身份仍不同：`eq 2` 命中 `2.0`，但 `eq "2"` 不命中任何数值
    assert!(CompareOp::Eq(Value::Int(2)).matches(&float));
    assert!(!CompareOp::Eq(Value::Str("2".into())).matches(&int));
    // 跨族四例
    for (a, b) in [
        (&int, &boolean),
        (&float, &text),
        (&boolean, &text),
        (&text, &int),
    ] {
        assert_eq!(value_compare(a, b), None, "{a:?} × {b:?} 必须跨族");
    }
}

/// G1（v2 §6）：10 个 op + `not` 前缀的**命中 / 未命中 / 跨族**三态。
/// 不做 11×4 穷举（语义只有一份 = [`CompareOp::matches`]）：这里钉「每条 op 的
/// 接线」与三类边界——跨族、空 `in`、`in` 的任一命中。
#[test]
fn where_op_semantics_cover_all_ops() {
    let mut store = EntityStore::new();
    let id = store.spawn().unwrap();
    store.set_attribute(id, "hp", Value::Int(50)).unwrap();
    store
        .set_attribute(id, "name", Value::Str("orc".into()))
        .unwrap();
    let entity = store.get(id).unwrap();

    let cases: Vec<(&str, &str, CompareOp, bool)> = vec![
        ("eq hit", "hp", CompareOp::Eq(Value::Int(50)), true),
        ("eq miss", "hp", CompareOp::Eq(Value::Int(51)), false),
        (
            "eq cross",
            "hp",
            CompareOp::Eq(Value::Str("50".into())),
            false,
        ),
        ("neq hit", "hp", CompareOp::Neq(Value::Int(51)), true),
        ("neq miss", "hp", CompareOp::Neq(Value::Int(50)), false),
        (
            "neq cross",
            "hp",
            CompareOp::Neq(Value::Str("50".into())),
            false,
        ),
        ("gt hit", "hp", CompareOp::Gt(Value::Int(49)), true),
        ("gt miss", "hp", CompareOp::Gt(Value::Int(50)), false),
        ("gte hit", "hp", CompareOp::Gte(Value::Float(50.0)), true),
        ("gte miss", "hp", CompareOp::Gte(Value::Int(51)), false),
        ("lt hit", "hp", CompareOp::Lt(Value::Int(51)), true),
        ("lt miss", "hp", CompareOp::Lt(Value::Int(50)), false),
        ("lte hit", "hp", CompareOp::Lte(Value::Int(50)), true),
        ("lte miss", "hp", CompareOp::Lte(Value::Float(49.5)), false),
        (
            "in hit",
            "hp",
            CompareOp::In(vec![Value::Int(1), Value::Int(50)]),
            true,
        ),
        ("in miss", "hp", CompareOp::In(vec![Value::Int(1)]), false),
        ("in empty", "hp", CompareOp::In(Vec::new()), false),
        ("like hit", "name", CompareOp::Like("or*".into()), true),
        ("like miss", "name", CompareOp::Like("rz*".into()), false),
        ("ilike hit", "name", CompareOp::ILike("OR*".into()), true),
        ("ilike miss", "name", CompareOp::ILike("rc*".into()), false),
        ("is notnull", "hp", CompareOp::Is(IsTest::NotNull), true),
        ("is null", "hp", CompareOp::Is(IsTest::Null), false),
    ];
    for (label, attr, op, expected) in cases {
        let predicate = Predicate::new(attr, op);
        assert_eq!(predicate.matches(entity), expected, "{label}");
        // `not.` 是整谓词取反：在场时逐条翻转
        assert_eq!(
            predicate.clone().negate().matches(entity),
            !expected,
            "not.{label}"
        );
    }
    // `is.true` / `is.false` = 在场且布尔相等（不是「真值性」）
    store.set_attribute(id, "boss", Value::Bool(true)).unwrap();
    store.set_attribute(id, "hp", Value::Int(1)).unwrap();
    let entity = store.get(id).unwrap();
    assert!(Predicate::new("boss", CompareOp::Is(IsTest::True)).matches(entity));
    assert!(!Predicate::new("hp", CompareOp::Is(IsTest::True)).matches(entity));
    assert!(!Predicate::new("hp", CompareOp::Is(IsTest::False)).matches(entity));
}

/// 缺属性：除 `is` 外一律假——**含 `neq`**（v1 §2.3 的裁断）
#[test]
fn missing_attribute_is_false_even_for_neq() {
    let store = EntityStore::new();
    let mut store = store;
    let id = store.spawn().unwrap();
    let entity = store.get(id).unwrap();
    assert!(!Predicate::new("hp", CompareOp::Eq(Value::Int(1))).matches(entity));
    assert!(!Predicate::new("hp", CompareOp::Neq(Value::Int(1))).matches(entity));
    assert!(!Predicate::new("hp", CompareOp::Gt(Value::Int(0))).matches(entity));
    assert!(!Predicate::new("hp", CompareOp::Lte(Value::Int(9))).matches(entity));
    assert!(!Predicate::new("hp", CompareOp::In(vec![Value::Int(1)])).matches(entity));
    assert!(!Predicate::new("hp", CompareOp::Like("*x*".into())).matches(entity));
    assert!(!Predicate::new("hp", CompareOp::ILike("*x*".into())).matches(entity));
}

/// G4（v2 §6）：`is` **不受**「缺属性 ⇒ 假」管辖
#[test]
fn is_null_and_notnull_are_presence() {
    let mut store = EntityStore::new();
    let present = store.spawn().unwrap();
    let absent = store.spawn().unwrap();
    store.set_attribute(present, "hp", Value::Int(0)).unwrap();
    assert!(Predicate::new("hp", CompareOp::Is(IsTest::Null)).matches(store.get(absent).unwrap()));
    assert!(
        !Predicate::new("hp", CompareOp::Is(IsTest::NotNull)).matches(store.get(absent).unwrap())
    );
    assert!(
        Predicate::new("hp", CompareOp::Is(IsTest::NotNull)).matches(store.get(present).unwrap())
    );
    assert!(!Predicate::new("hp", CompareOp::Is(IsTest::Null)).matches(store.get(present).unwrap()));
}

/// G5（v2 §6）：`not.lt.100` 对**缺属性**仍假（`not.` 不翻这条）
#[test]
fn not_prefix_keeps_absent_false() {
    let mut store = EntityStore::new();
    let id = store.spawn().unwrap();
    let entity = store.get(id).unwrap();
    assert!(!Predicate::new("hp", CompareOp::Lt(Value::Int(100)))
        .negate()
        .matches(entity));
    assert!(!Predicate::new("hp", CompareOp::Eq(Value::Int(1)))
        .negate()
        .matches(entity));
    // 在场时 `not.lt.100` ≡ `gte.100`
    store.set_attribute(id, "hp", Value::Int(100)).unwrap();
    let entity = store.get(id).unwrap();
    assert!(Predicate::new("hp", CompareOp::Lt(Value::Int(100)))
        .negate()
        .matches(entity));
}

/// `like` / `ilike` 的**四形态**与越界拒绝（v2 §4）
#[test]
fn like_shapes_are_exactly_four() {
    assert_eq!(like_shape("orc"), Some(LikeShape::Exact));
    assert_eq!(like_shape("orc*"), Some(LikeShape::Prefix));
    assert_eq!(like_shape("*orc"), Some(LikeShape::Suffix));
    assert_eq!(like_shape("*orc*"), Some(LikeShape::Contains));
    for bad in ["*", "**", "a*b", "a*b*", "*a*b", "a**"] {
        assert_eq!(like_shape(bad), None, "`{bad}` 必须是越界形态");
    }
    assert!(like_matches("goblin", "gob*"));
    assert!(like_matches("goblin", "*lin"));
    assert!(like_matches("goblin", "*obl*"));
    assert!(like_matches("goblin", "goblin"));
    assert!(!like_matches("goblin", "lin"));
    assert!(!like_matches("goblin", "goblins*"));
    // `ilike` 只做 ASCII 折叠（大小写不敏感仅限 ASCII）
    assert!(!like_matches("Goblin", "gob*"));
    assert!(ilike_matches("Goblin", "gob*"));
    assert!(ilike_matches("goblin", "GOBLIN"));
    assert!(!ilike_matches("goblin", "GOBLINS*"));
}

/// 确定性第二条：序遍历（乱序 `insert` 也按 id 升序出）
#[test]
fn select_orders_by_id_under_shuffled_insert() {
    let mut store = EntityStore::new();
    for id in [7u64, 1, 9, 3] {
        store.insert(Entity::new(id)).unwrap();
    }
    let selector = Selector::default();
    let ids: Vec<u64> = store
        .select(&selector)
        .into_iter()
        .map(|entity| entity.id)
        .collect();
    assert_eq!(ids, vec![1, 3, 7, 9]);
}

/// `id` 是保留伪属性：可过滤 id 区间，且**真属性名 `id` 因此不可寻址**
#[test]
fn id_pseudo_attribute_filters() {
    let mut store = EntityStore::new();
    for _ in 0..5 {
        store.spawn().unwrap();
    }
    // 真属性 `id` 存在也不参与查询（伪属性优先）
    store.set_attribute(1, "id", Value::Int(999)).unwrap();
    let mut query = |op: CompareOp| {
        let selector = Selector {
            predicates: vec![Predicate::new("id", op)],
            ..Selector::default()
        };
        store
            .select(&selector)
            .into_iter()
            .map(|entity| entity.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(query(CompareOp::Gte(Value::Int(3))), vec![3, 4, 5]);
    assert_eq!(query(CompareOp::Lt(Value::Int(3))), vec![1, 2]);
    assert_eq!(query(CompareOp::Eq(Value::Int(999))), Vec::<u64>::new());
    assert_eq!(query(CompareOp::Is(IsTest::Null)), Vec::<u64>::new());
    assert_eq!(
        query(CompareOp::In(vec![Value::Int(2), Value::Int(4)])),
        vec![2, 4]
    );
}

/// `order`：值同 ⇒ id 升序（asc/desc 都不反转）、缺属性排最后、跨族固定族序
#[test]
fn order_tiebreak_nulls_last_and_family_rank() {
    let mut store = EntityStore::new();
    store
        .insert(seeded(1, &[("hp", Value::Int(7))], &[]))
        .unwrap();
    store
        .insert(seeded(2, &[("hp", Value::Int(7))], &[]))
        .unwrap();
    store.insert(seeded(3, &[], &[])).unwrap();
    store
        .insert(seeded(4, &[("hp", Value::Int(2))], &[]))
        .unwrap();
    store
        .insert(seeded(5, &[("hp", Value::Str("x".into()))], &[]))
        .unwrap();

    let mut ordered = |descending: bool| {
        let selector = Selector {
            order: Some(Order {
                attr: "hp".into(),
                descending,
            }),
            ..Selector::default()
        };
        store
            .select(&selector)
            .into_iter()
            .map(|entity| entity.id)
            .collect::<Vec<_>>()
    };
    // 2 < 7 = 7（id 兜底 1 < 2）< "x"（族序 Num < Str）< 缺属性
    assert_eq!(ordered(false), vec![4, 1, 2, 5, 3]);
    // desc 只反转**值序**：族序随之反转，7 = 7 的 id 兜底仍是升序，缺属性仍在最后
    assert_eq!(ordered(true), vec![5, 1, 2, 4, 3]);
}

// ── 索引（S24 v2 §6 的 I1/I2/I3）───────────────────────────────────────

/// 一个选择器的 id 出表（两条路径的比较样板）。
fn ids_of(store: &mut EntityStore, selector: &Selector) -> Vec<EntityId> {
    store
        .select(selector)
        .into_iter()
        .map(|entity| entity.id)
        .collect()
}

fn one_predicate(op: CompareOp) -> Selector {
    Selector {
        predicates: vec![Predicate::new("hp", op)],
        ..Selector::default()
    }
}

/// 浮点段按**数值序**（位序化 u64），且段序 `Num < Float < Bool < Str`
#[test]
fn index_keys_follow_numeric_order() {
    let keys: Vec<IndexKey> = [-3.5, -2.0, -1e-9, 0.0, 1e-9, 1.0, 2.0]
        .iter()
        .map(|x| IndexKey::of(&Value::Float(*x)))
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted, "浮点段必须按数值序（位序化 u64）");
    assert!(IndexKey::Num(i64::MAX) < IndexKey::Float(0));
    assert!(IndexKey::Float(0) < IndexKey::Bool(false));
    assert!(IndexKey::Bool(true) < IndexKey::Str(String::new()));
}

/// I3（v2 §6）：乱序 `insert` 后桶内仍 id 升序
#[test]
fn index_buckets_are_id_ascending() {
    let mut store = EntityStore::new();
    for id in [7u64, 1, 9, 3] {
        store
            .insert(seeded(id, &[("hp", Value::Int(5))], &[]))
            .unwrap();
    }
    store.ensure_index("hp");
    assert_eq!(
        store.index_bucket("hp", &IndexKey::Num(5)),
        Some([1u64, 3, 7, 9].as_slice())
    );
}

/// I2（v2 §6）：**索引路径 ≡ 全扫路径**逐值逐序相等。
///
/// 两条路径的造法：给同一条选择器补一条必然成立的 `id >= 0` ⇒ 两个不同属性 ⇒
/// 按 v2 §3 强制回全扫；除谓词表之外两选择器逐字相同。
#[test]
fn index_path_equals_scan_path() {
    let mut store = EntityStore::new();
    store
        .insert(seeded(1, &[("hp", Value::Int(50))], &[]))
        .unwrap();
    store
        .insert(seeded(2, &[("hp", Value::Float(50.0))], &[]))
        .unwrap();
    store
        .insert(seeded(3, &[("hp", Value::Int(7))], &[]))
        .unwrap();
    store.insert(seeded(4, &[], &[])).unwrap();
    store
        .insert(seeded(5, &[("hp", Value::Str("50".into()))], &[]))
        .unwrap();

    let ops = vec![
        CompareOp::Eq(Value::Int(50)),
        CompareOp::Gte(Value::Int(7)),
        CompareOp::Lt(Value::Float(50.0)),
        CompareOp::Lte(Value::Int(50)),
        CompareOp::In(vec![Value::Int(7), Value::Str("50".into())]),
        CompareOp::In(Vec::new()),
    ];
    for op in ops {
        let indexed = one_predicate(op.clone());
        let mut scanned = indexed.clone();
        scanned
            .predicates
            .push(Predicate::new("id", CompareOp::Gte(Value::Int(0))));
        let via_index = ids_of(&mut store, &indexed);
        let via_scan = ids_of(&mut store, &scanned);
        assert_eq!(via_index, via_scan, "{op:?} 的两条路径必须逐值逐序相等");
    }
    // 索引路径真的建了索引（否则上面那组用例是空转）
    assert!(store.indices.contains_key("hp"));
    // 命中值的逐条核对（不是「两条路径一起错」）
    assert_eq!(
        ids_of(&mut store, &one_predicate(CompareOp::Eq(Value::Int(50)))),
        vec![1, 2]
    );
    assert_eq!(
        ids_of(&mut store, &one_predicate(CompareOp::Lte(Value::Int(50)))),
        vec![1, 2, 3]
    );
    assert_eq!(
        ids_of(
            &mut store,
            &one_predicate(CompareOp::Lt(Value::Float(50.0)))
        ),
        vec![3]
    );
}

/// I1（v2 §6）：随机写 200 次（spawn / insert / set / remove_attr / destroy）后，
/// **增量索引 ≡ 从零重建**。重建侧 = 克隆体（`Clone` 丢索引 ⇒ 首查即从零建）。
#[test]
fn index_incremental_equals_rebuild() {
    let mut store = EntityStore::new();
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..5 {
        store.spawn().unwrap();
    }
    let probe_selector = one_predicate(CompareOp::Gte(Value::Int(0)));
    // 先建索引：后续 200 次写全部走增量维护
    ids_of(&mut store, &probe_selector);

    let values = [
        Value::Int(1),
        Value::Int(50),
        Value::Float(2.5),
        Value::Float(-0.0),
        Value::Str("x".into()),
        Value::Bool(true),
    ];
    for step in 0..200 {
        let roll = next() % 5;
        let target = (next() % 10 + 1) as EntityId;
        let value = values[(next() % 6) as usize].clone();
        match roll {
            0 => {
                store.spawn().unwrap();
            }
            1 => {
                let _ = store.insert(seeded(target, &[("hp", value)], &[]));
            }
            2 => {
                let _ = store.set_attribute(target, "hp", value);
            }
            3 => {
                let _ = store.remove_attribute(target, "hp");
            }
            _ => {
                store.remove(target);
            }
        }
        // 读写交错：只写不读测不出「增量维护漏一处」的交错形状
        if step % 20 == 0 {
            ids_of(&mut store, &probe_selector);
        }
    }

    let ops = [
        CompareOp::Eq(Value::Int(50)),
        CompareOp::Gte(Value::Int(1)),
        CompareOp::Lt(Value::Float(2.5)),
        CompareOp::In(vec![Value::Int(1), Value::Str("x".into())]),
    ];
    let mut rebuilt = store.clone();
    assert!(rebuilt.indices.is_empty(), "克隆必须丢掉索引（重建侧前提）");
    for op in ops {
        let selector = one_predicate(op.clone());
        let incremental = ids_of(&mut store, &selector);
        let from_scratch = ids_of(&mut rebuilt, &selector);
        assert_eq!(incremental, from_scratch, "{op:?}：增量索引 ≡ 从零重建");
    }
}
