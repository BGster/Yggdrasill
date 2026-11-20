//! 基准：批式面的**每实体成本**与 mlua 穿越成本（`entity.md` §9 下沉边界的算术依据）。
//!
//! # 这个文件只在 `--release` 下有数字
//!
//! `cfg!(debug_assertions)` 为真时只打印 `bench_skipped=dev-profile` 并返回（与
//! `crates/ygg-sim/tests/perf_tick.rs` 同款）。理由不是「debug 慢一点」而是**口径**：
//! dev profile 的 Lua 调用与 `BTreeMap` 操作比 release 慢一个量级，拿它去算「50× 门槛」
//! 只会得到假数。字面三件套走 dev profile，所以跳过是让门禁保持全绿的设计约束。
//!
//! # 这张表要回答什么
//!
//! 下沉边界的判定是算术：`after ≤ before / 目标倍数`。`before` 由包侧
//! `PERF entity.paths_*` 给（`packages/libs/entity/test/pkg-entity-paths.lua`），
//! `after` 只能在这里量——因为它是**原生侧**的成本（列拷贝 / `BTreeMap` 插入 / 穿越）。
//! 三条被量的量：
//!
//! 1. **穿越成本**：`Lua::create_function` 注册的面被 Lua 逐个调用的每 call 纳秒。
//!    这条同时是 `sim-kernel.md` §0 铁律②「~µs」说法的复算依据。
//! 2. **`EntityStore` 批量写**：`spawn` + 8×`set_attribute` + 3×`add_tag` ×10⁴。
//!    它是 `ygg.entity.spawn_batch` 的 after 上限（批式面不改变单条语义，只少穿越）。
//! 3. **`EntityStore` 批量读**：`get` + 8×`attribute` ×10⁴，`attr_fetch` 的 after 上限。
//!
//! # 断言
//!
//! 只有两条：数字必须为正（否则没量到东西），以及**数量级护栏**（10⁴ 批量写 < 200 ms）。
//! 后者抓的是「不小心写成 O(n²)」这类缺陷，不是性能阈値——阈值属于 `sim-kernel` §5 与本包 §5。

use std::collections::HashMap;
use std::time::Instant;

use mlua::{Lua, Value as LuaValue};
use ygg_core::entity::{EntityStore, Value};

const ATTRS: [&str; 8] = [
    "hp", "max_hp", "attack", "defense", "speed", "level", "exp", "weight",
];
const TAGS: [&str; 3] = ["enemy", "boss", "undead"];
const ROWS: usize = 10_000;
const ROUNDS: usize = 3;

fn median3(mut xs: [f64; 3]) -> f64 {
    xs.sort_by(|a, b| a.partial_cmp(b).expect("样本必须是可比数"));
    xs[1]
}

/// 三轮取中位；每轮重建被测对象（避免状态累积污染样本）。
fn bench<F: FnMut()>(mut body: F) -> f64 {
    let mut samples = [0.0f64; ROUNDS];
    for sample in samples.iter_mut() {
        let began = Instant::now();
        body();
        *sample = began.elapsed().as_secs_f64() * 1e9;
    }
    median3(samples)
}

fn crossing_cost(lua: &Lua, label: &str, body: &str) {
    let mut samples = [0.0f64; ROUNDS];
    for sample in samples.iter_mut() {
        let began = Instant::now();
        lua.load(body)
            .set_name("@probe/crossing")
            .exec()
            .expect("穿越探针的 Lua 片段必须可执行");
        *sample = began.elapsed().as_nanos() as f64 / 200_000.0;
    }
    println!(
        "probe.crossing.{label}_ns={:.1} ns/call  (samples {:.1}/{:.1}/{:.1})",
        median3(samples),
        samples[0],
        samples[1],
        samples[2]
    );
}

fn store_bulk_spawn() -> f64 {
    bench(|| {
        let mut store = EntityStore::new();
        for row in 0..ROWS {
            let id = store.spawn().expect("id 空间足够 10⁴");
            for (i, name) in ATTRS.iter().enumerate() {
                store
                    .set_attribute(id, *name, Value::Int(row as i64 * 3 + i as i64))
                    .expect("属性写入不该失败");
            }
            for tag in TAGS {
                store.add_tag(id, tag).expect("标签写入不该失败");
            }
        }
        assert_eq!(store.len(), ROWS, "批量写必须真的造出 10⁴ 实体");
    })
}

fn store_bulk_read() -> f64 {
    let mut store = EntityStore::new();
    for row in 0..ROWS {
        let id = store.spawn().expect("id 空间足够 10⁴");
        for (i, name) in ATTRS.iter().enumerate() {
            store
                .set_attribute(id, *name, Value::Int(row as i64 * 3 + i as i64))
                .expect("属性写入不该失败");
        }
    }
    let ns = bench(|| {
        let mut sink = 0i64;
        for id in 1..=ROWS as u64 {
            let entity = store.get(id).expect("实体必须存在");
            for name in ATTRS {
                if let Some(Value::Int(v)) = entity.attribute(name) {
                    sink += *v;
                }
            }
        }
        assert!(sink > 0, "读要真的取到值（否则量的是空循环）");
    });
    // `store` 在这里只读借用：上面的闭包是 FnMut 但只持有不可变借用，需要显式延长生命周期
    assert_eq!(store.len(), ROWS);
    ns
}

#[test]
fn path_costs_are_measured() {
    if cfg!(debug_assertions) {
        println!("bench_skipped=dev-profile");
        return;
    }

    // ── ① 穿越成本 ──────────────────────────────────────────────────────────
    let lua = Lua::new();
    let ygg = lua.create_table().expect("建表");
    let f0 = lua.create_function(|_, ()| Ok(0i64)).expect("注册 0 参面");
    let f3 = lua
        .create_function(|_, (_a, _b, _c): (i64, i64, String)| Ok(0i64))
        .expect("注册 3 参面");
    let ft = lua
        .create_function(|lua, _id: i64| {
            let t = lua.create_table()?;
            t.raw_set("tick", 1i64)?;
            t.raw_set("hash", "0123456789abcdef")?;
            Ok(t)
        })
        .expect("注册返回表的面");
    let kv: std::rc::Rc<std::cell::RefCell<HashMap<String, LuaValue>>> =
        std::rc::Rc::new(std::cell::RefCell::new(HashMap::new()));
    let kv_rc = std::rc::Rc::clone(&kv);
    let kv_set = lua
        .create_function(move |_, (k, v): (String, LuaValue)| {
            kv_rc.borrow_mut().insert(k, v);
            Ok(())
        })
        .expect("注册 kv 写面");
    ygg.raw_set("f0", f0).expect("挂 f0");
    ygg.raw_set("f3", f3).expect("挂 f3");
    ygg.raw_set("ft", ft).expect("挂 ft");
    ygg.raw_set("kvset", kv_set).expect("挂 kvset");
    lua.globals().raw_set("ygg", &ygg).expect("挂 ygg");

    crossing_cost(
        &lua,
        "zero_arg",
        "local f = ygg.f0 for _ = 1, 200000 do f() end",
    );
    crossing_cost(
        &lua,
        "int_int_str",
        "local f = ygg.f3 local s = 'hp' for i = 1, 200000 do f(i, 7, s) end",
    );
    crossing_cost(
        &lua,
        "ret_table_2fields",
        "local f = ygg.ft for i = 1, 200000 do local t = f(i) end",
    );
    crossing_cost(
        &lua,
        "kv_set_str_value",
        "local f = ygg.kvset local s = 'k' for i = 1, 200000 do f(s, i) end",
    );

    // 「数组进出」便不便宜是 A 路（多键面）能否赢的**唯一**判据：
    // 若 n=8 的表参 + 表返回 ≈ 8 次穿越，多键面必然打平；显著更低才成立。
    let t8 = lua
        .create_function(|lua, items: mlua::Table| {
            let mut acc = 0i64;
            for index in 1..=items.raw_len() {
                acc += items.raw_get::<i64>(index)?;
            }
            let out = lua.create_table()?;
            for index in 1..=8 {
                out.raw_set(index, acc)?;
            }
            Ok(out)
        })
        .expect("注册表参 n=8 面");
    let multi = lua
        .create_function(|_, ()| Ok((1i64, 2i64, 3i64, 4i64, 5i64, 6i64, 7i64, 8i64)))
        .expect("注册多返回值面");
    ygg.raw_set("t8", t8).expect("挂 t8");
    ygg.raw_set("multi", multi).expect("挂 multi");
    crossing_cost(
        &lua,
        "table_arg8_ret_table8",
        "local f = ygg.t8 local t = {1,2,3,4,5,6,7,8} for _ = 1, 200000 do local out = f(t) end",
    );
    crossing_cost(
        &lua,
        "multi_ret8",
        "local f = ygg.multi for _ = 1, 200000 do local a,b,c,d,e,g,h,i = f() end",
    );
    crossing_cost(
        &lua,
        "scalar_ret8_calls",
        "local f = ygg.f0 for _ = 1, 1600000 do f() end",
    );

    // ── ② `EntityStore` 批量写 / 读（`ygg.entity` 批式面的 after 上限）──────
    let spawn_ns = store_bulk_spawn();
    let read_ns = store_bulk_read();
    println!(
        "probe.entitystore.bulk_spawn_10k_ms={:.3}  → {:.1} ns/entity",
        spawn_ns / 1e6,
        spawn_ns / ROWS as f64
    );
    println!(
        "probe.entitystore.bulk_read_10k_ms={:.3}  → {:.1} ns/entity",
        read_ns / 1e6,
        read_ns / ROWS as f64
    );

    // 数字必须为正 + 数量级护栏（抓 O(n²)，不是性能阈值）。
    assert!(spawn_ns > 0.0 && read_ns > 0.0, "探针必须量到东西");
    assert!(
        spawn_ns < 200e6,
        "10⁴ 批量写 {:.1} ms 超过 200 ms 数量级护栏（命中 O(n²) 或每实体分配）",
        spawn_ns / 1e6
    );
}
