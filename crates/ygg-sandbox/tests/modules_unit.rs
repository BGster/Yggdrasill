//! r1 解析面单测（**不构造任何 Lua 状态**）：名字规范化、路径双射、装载栈、
//! 模块表上限。
//!
//! 这些用例就是「解析面零 Lua 依赖」的证据面——后续 pack / lint 复用同一份
//! 判定时，口径由本文件钉住。

use ygg_sandbox::modules::{
    name_of_path, normalize, path_of, LoadStack, ModuleError, ModuleSet, MAX_CODE_BYTES,
    MAX_MODULES, MAX_NAME_LEN, MAX_SEGMENT_LEN,
};

/// 拒绝用例：(输入, 期望的错误变体判定)。
type RejectCase = (&'static str, fn(&ModuleError) -> bool);

#[test]
fn normalize_accepts_dotted_identifier_names() {
    for name in ["a", "src", "src.util", "a.b.c", "_x1.y_2", "A0_Z"] {
        assert_eq!(normalize(name).as_deref(), Ok(name), "rename of {name}");
    }
    // 幂等：规范名再过一次不变。
    for name in ["a", "src.util", "a.b.c"] {
        let once = normalize(name).expect("normalize");
        assert_eq!(normalize(&once), Ok(once.clone()), "idempotent: {name}");
    }
}

#[test]
fn normalize_rejects_path_like_and_malformed_names() {
    let cases: &[RejectCase] = &[
        ("", |e| matches!(e, ModuleError::EmptyName)),
        ("src..util", |e| {
            matches!(e, ModuleError::EmptySegment { .. })
        }),
        (".src", |e| matches!(e, ModuleError::EmptySegment { .. })),
        ("src.", |e| matches!(e, ModuleError::EmptySegment { .. })),
        ("src/util", |e| {
            matches!(e, ModuleError::InvalidChar { ch: '/', .. })
        }),
        ("src util", |e| {
            matches!(e, ModuleError::InvalidChar { ch: ' ', .. })
        }),
        ("src-util", |e| {
            matches!(e, ModuleError::InvalidChar { ch: '-', .. })
        }),
        ("src\\util", |e| matches!(e, ModuleError::Backslash { .. })),
        ("/etc/passwd", |e| matches!(e, ModuleError::Absolute { .. })),
        ("C:src", |e| matches!(e, ModuleError::Absolute { .. })),
        ("src\0util", |e| matches!(e, ModuleError::NulChar)),
    ];
    for (raw, matches_variant) in cases {
        let err = normalize(raw).expect_err(&format!("{raw:?} must be rejected"));
        assert!(
            matches_variant(&err),
            "{raw:?} rejected with the wrong reason: {err:?}"
        );
    }
}

#[test]
fn normalize_rejects_over_long_segments_and_names() {
    let long_segment = "a".repeat(MAX_SEGMENT_LEN + 1);
    let err = normalize(&long_segment).expect_err("segment too long");
    assert!(matches!(err, ModuleError::SegmentTooLong { .. }), "{err:?}");

    // 段长正好等于上限：合法。
    assert!(normalize(&"a".repeat(MAX_SEGMENT_LEN)).is_ok());

    // 段全合法但整体超长：NameTooLong（分隔符也计入）。构造正好 256 字节。
    let name = [
        "a".repeat(MAX_SEGMENT_LEN - 1),
        "a".repeat(MAX_SEGMENT_LEN),
        "a".repeat(MAX_SEGMENT_LEN),
        "a".repeat(62),
    ]
    .join(".");
    assert_eq!(name.len(), MAX_NAME_LEN, "constructed at the limit");
    assert!(normalize(&name).is_ok(), "boundary name is legal");

    let too_long = format!("{name}.a");
    let err = normalize(&too_long).expect_err("name too long");
    assert!(matches!(err, ModuleError::NameTooLong { .. }), "{err:?}");
}

#[test]
fn path_and_name_are_bijective() {
    for name in [
        "a",
        "src.util",
        "a.b.c",
        "_x1.y_2",
        "src.deeply.nested.leaf",
    ] {
        let path = path_of(name).expect("path_of");
        assert_eq!(
            name_of_path(&path).as_deref(),
            Ok(name),
            "name_of_path(path_of({name})) must round-trip"
        );
    }
    assert_eq!(path_of("src.util").as_deref(), Ok("src/util.lua"));
    assert_eq!(name_of_path("src/util.lua").as_deref(), Ok("src.util"));
    assert_eq!(name_of_path("a.lua").as_deref(), Ok("a"));
}

#[test]
fn name_of_path_rejects_non_canonical_paths() {
    let cases: &[RejectCase] = &[
        ("", |e| matches!(e, ModuleError::EmptyName)),
        ("/abs/src.lua", |e| {
            matches!(e, ModuleError::Absolute { .. })
        }),
        ("C:src.lua", |e| matches!(e, ModuleError::Absolute { .. })),
        ("src\\util.lua", |e| {
            matches!(e, ModuleError::Backslash { .. })
        }),
        ("src/util", |e| matches!(e, ModuleError::NotLuaFile { .. })),
        ("src/util.txt", |e| {
            matches!(e, ModuleError::NotLuaFile { .. })
        }),
        ("src/util.lua.lua", |e| {
            matches!(e, ModuleError::AmbiguousSegment { .. })
        }),
        // 段内 `.`：`src/a.b.lua` 与 `src/a/b.lua` 会撞到同一个点分名。
        ("src/a.b.lua", |e| {
            matches!(e, ModuleError::AmbiguousSegment { .. })
        }),
        // `..` / `.` 段因此也被同一规则挡下（无路径穿越面）。
        ("src/../util.lua", |e| {
            matches!(e, ModuleError::AmbiguousSegment { .. })
        }),
        ("./src/util.lua", |e| {
            matches!(e, ModuleError::AmbiguousSegment { .. })
        }),
        ("src//util.lua", |e| {
            matches!(e, ModuleError::EmptySegment { .. })
        }),
        ("src/util.lua\0", |e| matches!(e, ModuleError::NulChar)),
        ("src/ut il.lua", |e| {
            matches!(e, ModuleError::InvalidChar { ch: ' ', .. })
        }),
    ];
    for (raw, matches_variant) in cases {
        let err = name_of_path(raw).expect_err(&format!("{raw:?} must be rejected"));
        assert!(
            matches_variant(&err),
            "{raw:?} rejected with the wrong reason: {err:?}"
        );
    }

    // 段超长与整体超长同样拒绝（双射只在合法域内成立）。
    let long = format!("{}.lua", "a".repeat(MAX_SEGMENT_LEN + 1));
    assert!(matches!(
        name_of_path(&long),
        Err(ModuleError::SegmentTooLong { .. })
    ));
}

#[test]
fn load_stack_detects_cycles_and_reports_the_chain() {
    let mut stack = LoadStack::new();
    stack.push("src.a").expect("a");
    stack.push("src.b").expect("b");

    let err = stack.push("src.a").expect_err("cycle");
    assert_eq!(
        err,
        ModuleError::Circular {
            chain: "src.a -> src.b -> src.a".to_owned()
        }
    );
    // 自环：只报自身一跳。
    let mut solo = LoadStack::new();
    solo.push("x").expect("x");
    assert_eq!(
        solo.push("x").expect_err("self cycle"),
        ModuleError::Circular {
            chain: "x -> x".to_owned()
        }
    );
    // 拒绝不改动栈态（链上帧仍在，请求未被加入）。
    assert_eq!(stack.frames(), ["src.a".to_owned(), "src.b".to_owned()]);
    assert_eq!(stack.depth(), 2);

    // 出栈后可重新进入（缓存未命中→重行装载的正常路径）。
    stack.push("src.c").expect("c");
    stack.pop();
    stack.pop();
    stack.push("src.b").expect("re-entry after pop");
}

#[test]
fn load_stack_depth_limit_is_explicit() {
    let mut stack = LoadStack::new();
    for i in 0..ygg_sandbox::modules::MAX_DEPTH {
        stack.push(&format!("m{i}")).expect("within depth limit");
    }
    let err = stack.push("one-too-deep").expect_err("depth limit");
    match &err {
        ModuleError::DepthExceeded { chain, limit } => {
            assert_eq!(*limit, ygg_sandbox::modules::MAX_DEPTH);
            // 链条中段省略（可读优先），首尾保留。
            assert!(chain.starts_with("m0 -> m1 -> m2 -> …"), "chain: {chain}");
            assert!(
                chain.ends_with("one-too-deep"),
                "chain ends at the pending module: {chain}"
            );
        }
        other => panic!("expected DepthExceeded, got {other:?}"),
    }
    assert_eq!(stack.depth(), ygg_sandbox::modules::MAX_DEPTH);
}

#[test]
fn module_set_validation_is_registration_time() {
    let mut set = ModuleSet::new();
    set.insert("src.util", b"return 1".to_vec())
        .expect("insert");
    // key 是规范名（点分）。
    assert!(set.get("src.util").is_some());
    assert!(set.get("src/util").is_none(), "path form is not a key");
    assert_eq!(set.names().collect::<Vec<_>>(), ["src.util"]);

    // 同名重复 = 作者侧歧义，不静默取后者。
    assert_eq!(
        set.insert("src.util", b"return 2".to_vec())
            .expect_err("dup"),
        ModuleError::Duplicate {
            name: "src.util".to_owned()
        }
    );
    // 非法名在登记期即拒绝。
    assert!(matches!(
        set.insert("src/util", b"x".to_vec()),
        Err(ModuleError::InvalidChar { .. })
    ));
    // 单模块源码上限。
    assert!(matches!(
        set.insert("big", vec![0u8; MAX_CODE_BYTES + 1]),
        Err(ModuleError::CodeTooLarge { .. })
    ));
    // 模块数上限。
    let mut many = ModuleSet::new();
    for i in 0..MAX_MODULES {
        many.insert(&format!("m{i}"), b"return true".to_vec())
            .expect("within module limit");
    }
    assert_eq!(many.len(), MAX_MODULES);
    assert!(matches!(
        many.insert("one_more", b"return true".to_vec()),
        Err(ModuleError::TooManyModules { .. })
    ));
}
