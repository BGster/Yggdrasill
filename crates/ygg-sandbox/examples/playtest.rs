//! 无头走查 CLI：任意 Mod 目录装载 + `event.start` 自动推进 + JSON 报告
//! （playtest 逻辑在 src/playtest.rs，本文件只是参数解析薄壳——与
//! pack.rs / adventure.rs 同为 example 形态惯例）。
//!
//! **stdout JSON 契约（管线↔harness 的全部耦合面，D-8/D-12；pipeline 侧
//! 解析此契约，禁止触达任何内核符号）**：
//!
//! ```json
//! {"ok":true,"mod":"<目录>","steps":3,"reached":"ending",
//!  "narrativeSample":["…"],"warnings":[],"eventsProcessed":7,"errors":[]}
//! ```
//!
//! - `ok` = 装载成功 ∧ 全程无错误信封 ∧ 序章叙事非空 ∧ reached ∈ {ending, choice}；
//! - `reached`：ending（叙事出现【结局行）/ choice（`--choices` 脚本耗尽
//!   停在选择点）/ limit（`--max-steps` 上限）/ deadend（无选择且未达结局）；
//! - 退出码：0 = ok；1 = !ok；2 = 用法错误。
//!
//! 运行：
//!
//! ```bash
//! cargo run -p ygg-sandbox --example playtest -- <mod目录> [--max-steps 64] [--choices 1,2,1]
//! ```

use std::path::PathBuf;
use std::process::exit;

use ygg_sandbox::playtest::{playtest, PlaytestOptions, DEFAULT_MAX_STEPS};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(dir) = args.next() else {
        eprintln!(
            "用法: cargo run -p ygg-sandbox --example playtest -- <mod目录> [--max-steps N] [--choices 1,2,1]"
        );
        exit(2);
    };
    let mut options = PlaytestOptions {
        max_steps: DEFAULT_MAX_STEPS,
        choices: Vec::new(),
    };
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--max-steps" => {
                let Some(value) = args.next().and_then(|v| v.parse::<usize>().ok()) else {
                    eprintln!("error: --max-steps 需要正整数");
                    exit(2);
                };
                options.max_steps = value;
            }
            "--choices" => {
                let Some(value) = args.next() else {
                    eprintln!("error: --choices 需要逗号分隔的序号（1 起）");
                    exit(2);
                };
                options.choices = value
                    .split(',')
                    .map(|part| part.trim().parse::<usize>())
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap_or_else(|_| {
                        eprintln!("error: --choices 格式非法");
                        exit(2);
                    });
            }
            other => {
                eprintln!("error: 未知参数 {other}");
                exit(2);
            }
        }
    }

    let report = playtest(&PathBuf::from(dir), &options);
    println!(
        "{}",
        serde_json::to_string_pretty(&report.to_json()).expect("report serialize")
    );
    exit(if report.ok { 0 } else { 1 });
}
