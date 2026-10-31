//! 交互式 CLI 游玩入口：《灯下谜径》（packages/mods/test-adventure）终端直玩。
//!
//! **绑定型文件**：本文件绑定 `packages/mods/test-adventure` 的 agent 契约（事件名 / 结局前缀硬编码在字面量里）；换 Mod 需同步修改。通用工具（examples/agent.rs、src/playtest.rs）不在此列——它们从 pack.json 读契约。
//!
//! std-only 示例（不新增依赖）：装载真实资源包 → 自动派发 `event.start`
//! → 待决选择驱动的主循环（数字选项 / q 退出 / s 存档 / l 读档 / i 状态）。
//!
//! 运行：
//!
//! ```bash
//! cargo run --example adventure
//! ```
//!
//! 存档写入当前工作目录 `adventure-save.json`（信封 `save` 字段原样落盘，
//! 可直接作为 `load` payload 回读）。实现约束：stdin 只锁一次、单循环
//! 状态机推进（无递归——递归持锁重入会自锁死）。

use std::io::{self, BufRead, Write};

use serde_json::{json, Value as Json};
use ygg_sandbox::driver::KernelFacade;

// 资源包五件套（include 路径相对本文件：crates/ygg-sandbox/examples/ →
// 仓库根的 packages/mods/test-adventure/；与 tests/test_adventure.rs 同形）。
const MOD_SOURCE: &str = include_str!("../../../packages/mods/test-adventure/mod.lua");
const MANIFEST_BYTES: &[u8] = include_bytes!("../../../packages/mods/test-adventure/manifest.json");
const SCENE_SCHEMA: &str =
    include_str!("../../../packages/mods/test-adventure/schemas/scene-texts.json");
const SCENE_DATA: &[u8] = include_bytes!("../../../packages/mods/test-adventure/data/scenes.json");

const SCENE_SCHEMA_ID: &str = "https://yggdrasill.dev/schemas/v1/scene-texts.json";
const SCENE_DATA_PATH: &str = "data/scenes.json";
/// 存档落盘文件名（相对当前工作目录）。
const SAVE_PATH: &str = "adventure-save.json";
/// 结局叙事行前缀（出现即收口退出）。
const ENDING_PREFIX: &str = "【结局";

fn main() {
    // 资源包三件套（schemas / resources 形参与 with_resources 签名一致）。
    let schemas = vec![(SCENE_SCHEMA_ID.to_owned(), SCENE_SCHEMA.to_owned())];
    let resources = vec![(SCENE_DATA_PATH.to_owned(), SCENE_DATA.to_vec())];

    let mut kernel =
        match KernelFacade::with_resources(MOD_SOURCE, MANIFEST_BYTES, &schemas, &resources) {
            Ok(kernel) => kernel,
            Err(err) => {
                eprintln!("[错误] 内核装载失败: {err}");
                std::process::exit(1);
            }
        };

    println!("== 灯下谜径 · 终端直玩 ==");
    println!("命令：数字=选择 / s=存档 / l=读档 / i=状态 / q=退出");
    println!();

    run(&mut kernel);
}

/// 主循环（单 stdin 锁 + 状态机）：打印信封 → 读输入 → 派发 → 再打印……
/// 结局叙事行（【结局 前缀）出现即打印收口并退出。
fn run(kernel: &mut KernelFacade) {
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();

    // 首屏：自动派发序章。
    let mut envelope: Json = parse_envelope(&kernel.dispatch("event.start", "0"));
    // 只有刚派发过的信封才重放叙事（s/i 这类不推进剧情的命令不重复打印）。
    let mut fresh = true;

    loop {
        // ── 打印当次信封 ──────────────────────────────────────────────
        if envelope["ok"] != Json::Bool(true) {
            print_error(&envelope);
        } else if fresh {
            if let Some(lines_out) = envelope["narrative"].as_array() {
                for line in lines_out {
                    if let Some(line) = line.as_str() {
                        println!("  {line}");
                    }
                }
            }
            // D1 运行期警告：choice 前置 emit 被语义丢弃等创作侧提示。
            if let Some(warnings) = envelope["warnings"].as_array() {
                for warning in warnings {
                    if let Some(warning) = warning.as_str() {
                        println!("⚠ {warning}");
                    }
                }
            }
        }
        fresh = false;

        // 结局收口：叙事中出现【结局 前缀即冒险结束。
        let reached_ending = envelope["narrative"]
            .as_array()
            .map(|ls| {
                ls.iter()
                    .filter_map(|v| v.as_str())
                    .any(|l| l.starts_with(ENDING_PREFIX))
            })
            .unwrap_or(false);
        if reached_ending {
            println!();
            println!("—— 冒险结束 ——");
            return;
        }

        // ── 提示面 ────────────────────────────────────────────────────
        let pending = envelope["pendingChoice"].as_object().cloned();
        match &pending {
            Some(choice) => present_choice(choice),
            None => prompt_when_no_choice(),
        }

        // ── 读一行输入（EOF = 干净退出）────────────────────────────────
        let line = match lines.next() {
            Some(Ok(line)) => line,
            Some(Err(err)) => {
                eprintln!("[错误] 读取输入失败: {err}");
                std::process::exit(1);
            }
            None => {
                println!();
                println!("（输入结束，退出。）");
                return;
            }
        };
        let input = line.trim();

        // ── 命令路由 ─────────────────────────────────────────────────
        envelope = match input {
            "q" | "quit" | "exit" => {
                println!("再见。");
                return;
            }
            "s" => {
                let reply = parse_envelope(&kernel.dispatch("save", ""));
                if reply["ok"] == Json::Bool(true) {
                    persist_save(&reply["save"]);
                } else {
                    print_error(&reply);
                }
                continue; // 存档不推进剧情：重新打印提示。
            }
            "l" => {
                let payload = read_save_file();
                let reply = parse_envelope(&kernel.dispatch("load", &payload));
                if reply["ok"] == Json::Bool(true) {
                    // load 信封只带 restored，不带 pendingChoice——补发 state
                    // 取回待决选择面，让提示接上读档前的进度。
                    let state = parse_envelope(&kernel.dispatch("state", ""));
                    let mut merged = reply;
                    merged["narrative"] = json!(["（读档完成，进度已恢复。）"]);
                    merged["pendingChoice"] = state["pendingChoice"].clone();
                    fresh = true;
                    merged
                } else {
                    reply
                }
            }
            "i" => {
                let reply = parse_envelope(&kernel.dispatch("state", ""));
                if reply["ok"] == Json::Bool(true) {
                    print_state(&reply);
                } else {
                    print_error(&reply);
                }
                continue; // 状态查询不推进剧情。
            }
            _ => match input.parse::<i64>() {
                // 数字 → 待决选择的索引路由（ygg.choice_selected 标量 payload）。
                Ok(n) if n >= 1 => {
                    fresh = true;
                    parse_envelope(&kernel.dispatch("ygg.choice_selected", &n.to_string()))
                }
                _ => {
                    println!("（无法识别的输入：{input:?}。数字=选择 / s / l / i / q）");
                    continue;
                }
            },
        };
    }
}

/// 信封解析（内部错误即退出——驱动信封永不非法）。
fn parse_envelope(reply: &str) -> Json {
    match serde_json::from_str(reply) {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("[错误] 信封不是合法 JSON: {err}");
            std::process::exit(1);
        }
    }
}

/// 错误信封人读输出（码 + 文案，无色，前缀标识）。
fn print_error(envelope: &Json) {
    let code = envelope["error"]["code"].as_str().unwrap_or("UNKNOWN");
    let message = envelope["error"]["message"].as_str().unwrap_or("");
    println!("[错误 {code}] {message}");
}

/// 打印待决选择的提示语与编号选项。
fn present_choice(choice: &serde_json::Map<String, Json>) {
    println!();
    if let Some(prompt) = choice["prompt"].as_str() {
        println!("? {prompt}");
    }
    if let Some(options) = choice["options"].as_array() {
        for (i, option) in options.iter().enumerate() {
            let text = option.as_str().unwrap_or("");
            println!("  {}. {text}", i + 1);
        }
    }
    print!("> ");
    let _ = io::stdout().flush();
}

/// 无待决选择时的提示（冒险是选择驱动：等待命令或存读档）。
fn prompt_when_no_choice() {
    println!();
    println!("（无待决选择。命令：s=存档 / l=读档 / i=状态 / q=退出）");
    print!("> ");
    let _ = io::stdout().flush();
}

/// `save`：信封内嵌 save 对象即存档本体——落盘 adventure-save.json。
fn persist_save(save: &Json) {
    match std::fs::write(SAVE_PATH, save.to_string()) {
        Ok(()) => println!("已存档 → {SAVE_PATH}"),
        Err(err) => println!("[错误] 写入 {SAVE_PATH} 失败: {err}"),
    }
}

/// `load` payload：磁盘存档原样回读；缺失时给空串（内核回 VALIDATION
/// 错误信封，主循环人读呈现——不静默假装读档成功）。
fn read_save_file() -> String {
    match std::fs::read_to_string(SAVE_PATH) {
        Ok(text) => text,
        Err(_) => {
            println!("（无存档文件 {SAVE_PATH}，先按 s 存档。）");
            String::new()
        }
    }
}

/// `state`：诊断面人读输出（引擎版本 / revision / 快照字节数 / 待决选择）。
fn print_state(envelope: &Json) {
    let pending = if envelope["pendingChoice"].is_null() {
        json!(null)
    } else {
        envelope["pendingChoice"]["prompt"].clone()
    };
    println!(
        "engine={} revision={} kvRevision={} snapshotBytes={} pendingChoice={}",
        envelope["engineVersion"],
        envelope["revision"],
        envelope["kvRevision"],
        envelope["snapshotBytes"],
        pending,
    );
}
