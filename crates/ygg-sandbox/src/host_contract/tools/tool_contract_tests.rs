// ============================================================================
// §工具两层白名单的装载期校验（`tools` / `allowedTools`）
//
// 这些用例只碰纯 JSON：不建 Mod 目录、不 spawn 进程、不读环境——校验规则是
// 装载期门禁的全部事实面（运行期执行模型在 `examples/egg.rs` 的
// `tool_runner_tests` 里测）。
// ============================================================================

use super::*;
use serde_json::json;

/// 一条形状完全合法的 cli 工具（各用例只改自己关心的字段）。
fn cli_spec() -> Json {
    json!({
        "kind": "cli",
        "program": "git",
        "args": ["status", "--porcelain"],
        "timeoutMs": 120_000u64,
        "inlineWaitMs": 5_000u64,
        "maxOutputBytes": 65_536u64,
        "readOnly": true,
        "resultEvent": "tool.result",
    })
}

/// 一条形状完全合法的 http 工具。
fn http_spec() -> Json {
    json!({
        "kind": "http",
        "method": "POST",
        "url": "https://example.test/api",
        "headers": {"Authorization": "Bearer $ENV{TOKEN}"},
        "bodyTemplate": "{\"repo\":\"{{repo}}\"}",
        "timeoutMs": 30_000u64,
        "inlineWaitMs": 1_000u64,
        "maxOutputBytes": 4_096u64,
        "readOnly": true,
        "resultEvent": "tool.result",
    })
}

/// 只含一条声明的 `pack.json`（键名可变，用于断言错误文案里的 id）。
fn pack_one(id: &str, spec: Json) -> Json {
    let mut table = serde_json::Map::new();
    table.insert(id.to_owned(), spec);
    json!({ "tools": Json::Object(table) })
}

/// 解析一条声明（id 固定为 `probe`）。
fn parse_one(spec: Json) -> Result<ToolRegistry, String> {
    ToolRegistry::from_pack(&pack_one("probe", spec))
}

/// 合成配置路径（只用于错误文案，不读盘）。
fn config_source() -> &'static Path {
    Path::new("/tmp/synthetic-mod/data/config.json")
}

/// 放行清单的合成 `config.json`。
fn config_with(allowed: Json) -> Json {
    json!({ "allowedTools": allowed })
}

#[test]
fn tools_absent_is_backward_compatible() {
    let registry = ToolRegistry::from_pack(&json!({"agent": {}})).expect("缺段必须放行");
    assert!(registry.is_empty());
    assert_eq!(registry.len(), 0);
    let access = ToolAccess::from_config(&json!({}), config_source()).expect("缺段必须放行");
    assert!(access.is_empty());
    assert!(registry.denied(&access).is_empty());
    assert!(require_tool_allowlist(&registry, &access, config_source()).is_ok());
}

#[test]
fn tools_empty_object_is_backward_compatible() {
    let registry = ToolRegistry::from_pack(&json!({"tools": {}})).expect("空对象必须放行");
    assert!(registry.is_empty());
    assert!(registry.iter().next().is_none());
    let access =
        ToolAccess::from_config(&config_with(json!([])), config_source()).expect("空数组必须放行");
    assert!(access.is_empty());
    assert!(require_tool_allowlist(&registry, &access, config_source()).is_ok());
}

#[test]
fn tools_rejects_non_object() {
    let error = ToolRegistry::from_pack(&json!({"tools": []})).expect_err("数组必须被拒");
    assert!(error.contains(TOOLS_SHAPE), "{error}");
    assert!(error.contains(CONTRACT_DOC), "{error}");
    assert!(error.contains("必须是对象"), "{error}");
}

#[test]
fn tool_ids_must_be_lowercase_wire_safe() {
    let error = ToolRegistry::from_pack(&pack_one("Git Status!", cli_spec()))
        .expect_err("非法 id 必须被拒");
    assert!(error.contains("Git Status!"), "{error}");
    assert!(error.contains("[a-z0-9._-]"), "{error}");
    // 合法 id：小写字母 / 数字 / `.` `_` `-`
    assert!(ToolRegistry::from_pack(&pack_one("git.status_v2-1", cli_spec())).is_ok());
}

#[test]
fn cli_happy_path_roundtrips() {
    let registry = parse_one(cli_spec()).expect("合法 cli 声明必须通过");
    let spec = registry.get("probe").expect("按 id 取到声明");
    assert_eq!(spec.id, "probe");
    assert_eq!(spec.kind, ToolKind::Cli);
    assert_eq!(spec.program.as_deref(), Some("git"));
    assert_eq!(
        spec.args,
        vec!["status".to_owned(), "--porcelain".to_owned()]
    );
    assert_eq!(spec.method, None);
    assert_eq!(spec.url, None);
    assert!(spec.headers.is_empty());
    assert_eq!(spec.body_template, None);
    assert_eq!(spec.timeout_ms, 120_000);
    assert_eq!(spec.inline_wait_ms, 5_000);
    assert_eq!(spec.max_output_bytes, 65_536);
    assert!(spec.read_only);
    assert_eq!(spec.result_event, "tool.result");
    assert_eq!(registry.len(), 1);
}

#[test]
fn http_happy_path_roundtrips() {
    let registry = parse_one(http_spec()).expect("合法 http 声明必须通过");
    let spec = registry.get("probe").expect("按 id 取到声明");
    assert_eq!(spec.kind, ToolKind::Http);
    assert_eq!(spec.method.as_deref(), Some("POST"));
    assert_eq!(spec.url.as_deref(), Some("https://example.test/api"));
    assert_eq!(
        spec.headers,
        vec![("Authorization".to_owned(), "Bearer $ENV{TOKEN}".to_owned())]
    );
    assert_eq!(
        spec.body_template.as_deref(),
        Some("{\"repo\":\"{{repo}}\"}")
    );
    assert_eq!(spec.program, None);
    assert!(spec.args.is_empty());
}

#[test]
fn cli_rejects_shell_basenames() {
    for program in [
        "sh",
        "bash",
        "zsh",
        "dash",
        "fish",
        "ksh",
        "csh",
        "tcsh",
        "cmd",
        "cmd.exe",
        "pwsh",
        "/bin/sh",
        "tools/bash",
    ] {
        let mut spec = cli_spec();
        spec["program"] = json!(program);
        let error = parse_one(spec).expect_err(&format!("`{program}` 必须被拒"));
        assert!(
            error.contains("program"),
            "错误文案必须点名 program（{program}）：{error}"
        );
    }
}

#[test]
fn cli_rejects_absolute_and_traversal() {
    for program in ["/usr/bin/git", "../git", "a/../b", "a b", "git;rm", "$(x)"] {
        let mut spec = cli_spec();
        spec["program"] = json!(program);
        let error = parse_one(spec).expect_err(&format!("`{program}` 必须被拒"));
        assert!(error.contains("program"), "{error}");
    }
}

#[test]
fn cli_allows_relative_subpath() {
    let mut spec = cli_spec();
    spec["program"] = json!("./scripts/probe.sh");
    let registry = parse_one(spec).expect("有效相对路径必须放行（否则规则过紧）");
    assert_eq!(
        registry
            .get("probe")
            .and_then(|spec| spec.program.as_deref()),
        Some("./scripts/probe.sh")
    );
}

/// `@self`：**只此一处**合法（内置子命令的可移植入口），其余 `@` 形态照旧被拒。
#[test]
fn cli_accepts_self_program_and_rejects_look_alikes() {
    let mut spec = cli_spec();
    spec["program"] = json!(SELF_PROGRAM);
    let registry = parse_one(spec).expect("`@self` 必须放行（否则内置子命令不可携带）");
    assert_eq!(
        registry
            .get("probe")
            .and_then(|spec| spec.program.as_deref()),
        Some(SELF_PROGRAM)
    );

    // 长像的都不行：`@` 不在常规字符集里，只有精确的 `@self` 走特例。
    for program in ["@SELF", "@self/x", "x@self", "@self..", "@", "@self."] {
        let mut spec = cli_spec();
        spec["program"] = json!(program);
        let error = parse_one(spec).expect_err(&format!("`{program}` 必须被拒"));
        assert!(error.contains("program"), "{error}");
    }

    // 裸名 `egg` 本来就是合法形态（`execute_cli` 会把它解析到 current_exe）。
    let mut spec = cli_spec();
    spec["program"] = json!("egg");
    assert!(parse_one(spec).is_ok());
}

#[test]
fn cli_rejects_pipe_in_args() {
    for bad in ["a|b", "a\nb", "a\rb", "a\0b"] {
        let mut spec = cli_spec();
        spec["args"] = json!([bad]);
        let error = parse_one(spec).expect_err(&format!("`{bad:?}` 必须被拒"));
        assert!(error.contains("args[0]"), "{error}");
    }
}

#[test]
fn cli_rejects_bad_env_syntax() {
    for bad in ["$ENV{1BAD}", "$ENV{X", "$ENV{}", "{{unclosed"] {
        let mut spec = cli_spec();
        spec["args"] = json!([bad]);
        let error = parse_one(spec).expect_err(&format!("`{bad}` 必须被拒"));
        assert!(error.contains("args[0]"), "{error}");
    }
    // 合法形态必须放行
    let mut spec = cli_spec();
    spec["args"] = json!(["--token", "$ENV{GH_TOKEN}", "{{repo}}"]);
    assert!(parse_one(spec).is_ok());
}

#[test]
fn http_rejects_non_http_scheme() {
    for url in ["file:///etc/passwd", "ftp://x", "//x", "example.test/api"] {
        let mut spec = http_spec();
        spec["url"] = json!(url);
        let error = parse_one(spec).expect_err(&format!("`{url}` 必须被拒"));
        assert!(error.contains("url"), "{error}");
    }
}

#[test]
fn http_rejects_unknown_method() {
    for method in ["PATCH", "get", "HEAD", ""] {
        let mut spec = http_spec();
        spec["method"] = json!(method);
        let error = parse_one(spec).expect_err(&format!("`{method}` 必须被拒"));
        assert!(error.contains("method"), "{error}");
    }
}

#[test]
fn cross_kind_fields_rejected() {
    let mut cli = cli_spec();
    cli["url"] = json!("https://example.test/api");
    let error = parse_one(cli).expect_err("cli 带 url 必须被拒");
    assert!(error.contains("半声明"), "{error}");

    let mut http = http_spec();
    http["args"] = json!(["--verbose"]);
    let error = parse_one(http).expect_err("http 带 args 必须被拒");
    assert!(error.contains("半声明"), "{error}");

    let mut cli_program = cli_spec();
    cli_program["headers"] = json!({"X-A": "b"});
    assert!(parse_one(cli_program).is_err(), "cli 带 headers 必须被拒");
}

#[test]
fn missing_fields_are_listed_in_one_shot() {
    let mut spec = cli_spec();
    for field in ["program", "args", "timeoutMs", "resultEvent"] {
        spec.as_object_mut().expect("对象").remove(field);
    }
    let error = parse_one(spec).expect_err("缺字段必须被拒");
    for field in ["program", "args", "timeoutMs", "resultEvent"] {
        assert!(error.contains(field), "错误文案必须列出 `{field}`：{error}");
    }
    assert!(error.contains("必填"), "{error}");
}

#[test]
fn inline_wait_must_not_exceed_timeout() {
    let mut spec = cli_spec();
    spec["timeoutMs"] = json!(1_000u64);
    spec["inlineWaitMs"] = json!(5_000u64);
    let error = parse_one(spec).expect_err("inlineWaitMs > timeoutMs 必须被拒");
    assert!(error.contains("inlineWaitMs"), "{error}");
    assert!(error.contains("timeoutMs"), "{error}");
}

#[test]
fn mutating_tool_must_be_inline() {
    let mut spec = cli_spec();
    spec["readOnly"] = json!(false);
    spec["inlineWaitMs"] = json!(0u64);
    let error = parse_one(spec).expect_err("非只读工具不得脱体");
    assert!(error.contains("inlineWaitMs"), "{error}");
    assert!(error.contains("readOnly"), "{error}");
    // 只读工具脱体是合法的（inlineWaitMs = 0 = 恒走场景 2）
    let mut readonly = cli_spec();
    readonly["inlineWaitMs"] = json!(0u64);
    let registry = parse_one(readonly).expect("只读工具允许 inlineWaitMs = 0");
    assert_eq!(
        registry.get("probe").map(|spec| spec.inline_wait_ms),
        Some(0)
    );
}

#[test]
fn max_output_bytes_bounds() {
    for (value, ok) in [(0i64, false), (65_537, false), (65_536, true), (1, true)] {
        let mut spec = cli_spec();
        spec["maxOutputBytes"] = json!(value);
        assert_eq!(
            parse_one(spec).is_ok(),
            ok,
            "maxOutputBytes = {value} 的判定不符合预期"
        );
    }
}

#[test]
fn timeout_bounds() {
    for (value, ok) in [(0i64, false), (3_600_001, false), (3_600_000, true)] {
        let mut spec = cli_spec();
        spec["timeoutMs"] = json!(value);
        spec["inlineWaitMs"] = json!(0u64);
        assert_eq!(
            parse_one(spec).is_ok(),
            ok,
            "timeoutMs = {value} 的判定不符合预期"
        );
    }
}

#[test]
fn allowlist_absent_allows_nothing() {
    let access = ToolAccess::from_config(&json!({}), config_source()).expect("缺段必须放行");
    assert!(access.is_empty());
    assert!(!access.allows("git.status"));
    assert!(access.allowed().is_empty());
    // tools 也空 → 交叉校验通过（两层都缺 = 工具能力不可用，但那不是错误）
    assert!(require_tool_allowlist(&ToolRegistry::default(), &access, config_source()).is_ok());
}

#[test]
fn allowlist_missing_entry_fails_closed() {
    let mut table = serde_json::Map::new();
    table.insert("a".to_owned(), cli_spec());
    table.insert("b".to_owned(), cli_spec());
    let registry =
        ToolRegistry::from_pack(&json!({ "tools": Json::Object(table) })).expect("两条声明都合法");
    let access =
        ToolAccess::from_config(&config_with(json!(["a"])), config_source()).expect("放行清单合法");
    assert_eq!(registry.denied(&access), vec!["b".to_owned()]);
    let error = require_tool_allowlist(&registry, &access, config_source())
        .expect_err("未放行的声明必须拒绝启动");
    assert!(error.contains("`b`"), "必须列出未放行的 id：{error}");
    assert!(
        !error.contains("`a`（"),
        "已放行的 id 不该进缺失清单：{error}"
    );
    assert!(error.contains("allowedTools"), "{error}");
    assert!(error.contains(ALLOWED_TOOLS_SHAPE), "{error}");
    assert!(error.contains(TOOLS_SHAPE), "{error}");
}

#[test]
fn allowlist_extra_entry_is_allowed() {
    let registry = ToolRegistry::from_pack(&pack_one("a", cli_spec())).expect("声明合法");
    let access = ToolAccess::from_config(&config_with(json!(["a", "zzz"])), config_source())
        .expect("多放行不是错误");
    assert!(access.allows("zzz"));
    assert!(require_tool_allowlist(&registry, &access, config_source()).is_ok());
}

#[test]
fn allowlist_rejects_non_string_entries() {
    for bad in [json!([1]), json!([""]), json!(["   "]), json!([null])] {
        let error = ToolAccess::from_config(&config_with(bad.clone()), config_source())
            .expect_err(&format!("{bad} 必须被拒"));
        assert!(error.contains("第 1 项"), "{error}");
        assert!(error.contains(ALLOWED_TOOLS_SHAPE), "{error}");
    }
    // 非数组整段拒绝
    let error = ToolAccess::from_config(&json!({"allowedTools": "a"}), config_source())
        .expect_err("非数组必须被拒");
    assert!(error.contains("必须是字符串数组"), "{error}");
    // 重复项按集合语义允许
    let access = ToolAccess::from_config(&config_with(json!(["a", "a"])), config_source())
        .expect("重复项允许");
    assert!(access.allows("a"));
    assert_eq!(access.allowed().len(), 2);
}
