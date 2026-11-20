//! 门禁入口薄壳。
//!
//! Mod 的**行为**全部由 Lua 自检覆盖（`.ygg/mods/agent-seed/tests/*.lua`，
//! `cargo run -q --example selftest -- --mod .ygg/mods/agent-seed`）。本 target
//! 不重复任何一条行为断言——那是纯 B 纪律禁止的「Rust 兜底用例」。
//!
//! 它存在的唯一理由：宿主三套门禁的第三条被硬编码成
//! `cargo test -p ygg-sandbox --test agent_seed_semantics`（见
//! `crates/ygg-sandbox/examples/egg.rs` 的 `verify_gates`），而 egg.rs 不可改。
//! 目标被删掉的那一刻，那条门禁就变成「error: no test target named」。
//!
//! 所以这里只断言一件**元事实**：Lua 套件还在、还有足够多的用例。
//! 它防的是「套件被清空/路径写错」这类静默退化，不是 Mod 行为回归。

use std::path::PathBuf;

fn tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.ygg/mods/agent-seed/tests")
}

/// 套件文件与用例数不得缩水到「看起来跑了、其实什么都没跑」。
#[test]
fn lua_selftest_suite_is_present_and_non_empty() {
    let dir = tests_dir();
    let mut files = 0usize;
    let mut cases = 0usize;
    let mut names: Vec<String> = Vec::new();

    let entries =
        std::fs::read_dir(&dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()));
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|s| s.to_str()) != Some("lua") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_owned();
        if name == "harness.lua" {
            continue;
        }
        files += 1;
        names.push(name);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        // 统计顶层 `test(` 注册（行首 `test(` 或 `test(` 紧跟字符串）。
        cases += text.matches("\ntest(").count() + usize::from(text.starts_with("test("));
    }

    names.sort();
    assert!(
        files >= 9,
        "Lua 套件文件数异常（{files}）：{names:?} —— 套件被删了？路径写错了？"
    );
    assert!(
        cases >= 70,
        "Lua 用例数异常（{cases}）：套件还在但几乎空了，第三条门禁会变成空跑"
    );

    // harness 必须还在：没有它所有用例都跑不起来。
    assert!(
        dir.join("harness.lua").is_file(),
        "缺 harness.lua —— Lua 套件无法执行"
    );
}
