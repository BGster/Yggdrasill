use super::*;

// ---- verify-suite ----

pub(crate) const VERIFY_FLAGS: &[&str] = &["--mode", "--repo"];

/// 门禁三件套（`AGENTS.md` §一·「门禁三件套」）——顺序即失败顺序。
pub(crate) fn verify_gates(mode: &str) -> Vec<(&'static str, Vec<&'static str>)> {
    if mode == "fast" {
        vec![(
            "cargo check --workspace --all-targets",
            vec!["check", "--workspace", "--all-targets"],
        )]
    } else {
        vec![
            ("cargo test --workspace", vec!["test", "--workspace"]),
            (
                "cargo test --example egg -p ygg-sandbox",
                vec!["test", "--example", "egg", "-p", "ygg-sandbox"],
            ),
            (
                "cargo test -p ygg-sandbox --test agent_seed_semantics",
                vec![
                    "test",
                    "-p",
                    "ygg-sandbox",
                    "--test",
                    "agent_seed_semantics",
                ],
            ),
        ]
    }
}

pub(crate) fn parse_custom_verify_commands(raw: &str) -> Vec<(String, String, Vec<String>)> {
    raw.split(';')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .filter_map(|segment| {
            let mut parts = segment.split_whitespace();
            let program = parts.next()?.to_owned();
            let args: Vec<String> = parts.map(ToOwned::to_owned).collect();
            Some((segment.to_owned(), program, args))
        })
        .collect()
}

pub(crate) fn verify_suite_fields() -> Result<Vec<(&'static str, String)>, OpFailure> {
    verify_suite_fields_with(&OpFlags::parse("verify-suite", VERIFY_FLAGS))
}

/// [`verify_suite_fields`] 的注入面（测试直接给旗标，不读进程 argv）。
pub(crate) fn verify_suite_fields_with(
    flags: &OpFlags,
) -> Result<Vec<(&'static str, String)>, OpFailure> {
    const OP: &str = "verify-suite";
    let mode = flags.get("--mode").unwrap_or("full").trim().to_owned();
    if mode != "fast" && mode != "full" {
        builtin_usage_error(
            OP,
            &format!("--mode 只接受 fast / full（当前是 `{mode}`）"),
            VERIFY_FLAGS,
        );
    }
    // 递归守卫：被 `cargo test` 牵着跑时不要再去抢构建锁。
    if std::env::var("YGG_VERIFY_SUITE").as_deref() == Ok("1") && std::env::var("CARGO").is_ok() {
        return Err(OpFailure::detail(
            "recursion",
            "YGG_VERIFY_SUITE=1 且处在 cargo 环境里，拒绝递归验证",
        ));
    }
    let repo = repo_root(flags.get("--repo")).map_err(|err| OpFailure::detail("repo", err))?;
    let custom = std::env::var("YGG_VERIFY_COMMANDS")
        .ok()
        .map(|raw| parse_custom_verify_commands(&raw))
        .unwrap_or_default();
    let commands: Vec<(String, String, Vec<String>)> = if !custom.is_empty() {
        custom
    } else {
        verify_gates(&mode)
            .into_iter()
            .map(|(label, args)| {
                (
                    label.to_owned(),
                    "cargo".to_owned(),
                    args.into_iter().map(ToOwned::to_owned).collect(),
                )
            })
            .collect()
    };
    let total = commands.len();
    for (index, (label, program, cmd_args)) in commands.iter().enumerate() {
        // `output()` 内部并发排空两个管道——不会因为管道塞满而死锁。
        let outcome = Command::new(program)
            .args(cmd_args)
            .current_dir(&repo)
            .stdin(Stdio::null())
            .output();
        match outcome {
            Ok(output) if output.status.success() => {}
            Ok(output) => {
                // 失败：stdout 保持空，stderr = 一行 header + 末 20 行 sanitized。
                eprintln!("gate={};{label}", index + 1);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let lines: Vec<&str> = stderr.lines().collect();
                let start = lines.len().saturating_sub(20);
                for line in &lines[start..] {
                    eprintln!("{}", wire_sanitize(line));
                }
                return Err(OpFailure::Reported);
            }
            Err(err) => return Err(OpFailure::detail("spawn", format!("{label}：{err}"))),
        }
    }
    // summary 数的是**门数**不是用例数（用例数逐工具链变，会让 wire 抖动）。
    Ok(vec![
        ("passed", "1".to_owned()),
        ("summary", format!("{total}/{total}")),
    ])
}
