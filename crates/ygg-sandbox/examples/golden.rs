//! 参考包金值向量生成器（M3.4b）：为 `<pack目录>` 跑固定种子与步数，产出逐 tick 状态哈希序列。
//!
//! 用于把 native（`crates/ygg-sandbox/tests/example_pack.rs`）与 wasm（`scripts/wasm-lane.mjs`）
//! 钉到同一份金值上：两侧都从同一份种子与步数出发、逐 tick 比较 state hash，确保双运行时
//! 在同一份规则下产出完全一致的历史。
//!
//! 用法：
//!
//! ```bash
//! cargo run -q --example golden -p ygg-sandbox -- <pack目录> [--seed <u64>] [--ticks <u32>] [--dt <u32>] [--check <文件>] [--final-only]
//! ```
//!
//! - 缺省参数：`--seed 0x5eed`（24301）、`--ticks 50`、`--dt 1`（与 `tests/example_pack.rs`
//!   的基线一致，十六进制可带 `0x` 前缀或写十进制）；
//! - 缺省 `--check`：按 2 空格缩进 pretty JSON 打印到 stdout（末尾补换行），形态：
//!   ```json
//!   {
//!     "seed": 24301,
//!     "ticks": 50,
//!     "dt": 1,
//!     "hashes": ["...", "..."]
//!   }
//!   ```
//!   其中 `hashes` 含 `ticks` 个元素，第 `i` 项是推进到第 `i + 1` tick 后的十进制 `u64`
//!   state hash（以字符串存放，避免 JS `Number` 超过 `2^53 - 1` 时丢精度）；
//! - `--check <文件>`：读现有金值文件，按文件中声明的 `seed` / `ticks` / `dt`（若命令行未显式覆盖）
//!   重算后逐字节比对；不一致则打印具体差异并非 0 退出。
//! - `--final-only`：不打印金值 JSON，只打一行 `final_hash=<十进制> tick=<N> elapsed_ms=<x>`
//!   （`elapsed_ms` 只计逐 tick 推进，不含装载与 fork），与
//!   `node scripts/wasm-lane.mjs --ticks <N>` 的末行同形——长跑双端矩阵
//!   （`scripts/persist-matrix.sh`）靠这一行取数。JSON 的键按字母序输出
//!   （`hashes` 在前），用 `tail` 取末条 hash 会随键序静默错位。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::exit;
use std::time::Instant;

use serde_json::{json, Value as Json};
use ygg_sandbox::modload::Loaded;

/// 默认种子：与 `crates/ygg-sandbox/tests/example_pack.rs` 的 `0x5eed` 一致。
const DEFAULT_SEED: u64 = 0x5eed;
/// 默认推进 tick 数：与 `crates/ygg-sandbox/tests/example_pack.rs` 的 50 tick 一致。
const DEFAULT_TICKS: u32 = 50;
/// 默认单步 `dt`：与 `tests/example_pack.rs` 的 `step(_, 1)` 一致。
const DEFAULT_DT: u32 = 1;

fn main() {
    let mut dir_arg: Option<PathBuf> = None;
    let mut seed_arg: Option<u64> = None;
    let mut ticks_arg: Option<u32> = None;
    let mut dt_arg: Option<u32> = None;
    let mut check_arg: Option<PathBuf> = None;
    let mut final_only = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seed" => {
                let raw = args
                    .next()
                    .unwrap_or_else(|| fail("--seed 需要一个整数参数（十进制或 0x 十六进制）"));
                seed_arg = Some(parse_u64(&raw));
            }
            "--ticks" => {
                let raw = args
                    .next()
                    .unwrap_or_else(|| fail("--ticks 需要一个正整数参数"));
                ticks_arg = Some(parse_u32(&raw, "--ticks"));
            }
            "--dt" => {
                let raw = args
                    .next()
                    .unwrap_or_else(|| fail("--dt 需要一个正整数参数"));
                dt_arg = Some(parse_u32(&raw, "--dt"));
            }
            "--check" => {
                let raw = args
                    .next()
                    .unwrap_or_else(|| fail("--check 需要一个金值 JSON 文件路径"));
                check_arg = Some(PathBuf::from(raw));
            }
            "--final-only" => final_only = true,
            "-h" | "--help" => {
                print_usage();
                exit(0);
            }
            other if other.starts_with("--") => fail(format!("未知参数：{other}")),
            other => {
                if dir_arg.is_some() {
                    fail(format!("只接受一个 <pack目录> 参数，多出了：{other}"));
                }
                dir_arg = Some(PathBuf::from(other));
            }
        }
    }

    let Some(dir) = dir_arg else {
        print_usage();
        exit(1);
    };
    if !dir.is_dir() {
        fail(format!("目录不存在或不可读：{}", dir.display()));
    }

    let seed = seed_arg.unwrap_or(DEFAULT_SEED);
    let ticks = ticks_arg.unwrap_or(DEFAULT_TICKS);
    let dt = dt_arg.unwrap_or(DEFAULT_DT);
    if ticks == 0 {
        fail("--ticks 必须 ≥ 1");
    }
    if dt == 0 {
        fail("--dt 必须 ≥ 1");
    }
    if final_only && check_arg.is_some() {
        fail("--final-only 与 --check 互斥（一个是打印、一个是比对）");
    }

    let (hashes, elapsed_ms) = generate_hashes(&dir, seed, ticks, dt);
    if final_only {
        let Some(last) = hashes.last() else {
            fail("--ticks ≥ 1 却没有算出任何 hash");
        };
        println!("final_hash={last} tick={ticks} elapsed_ms={elapsed_ms}");
        exit(0);
    }
    let rendered = render_golden(seed, ticks, &hashes);
    match check_arg {
        None => print!("{rendered}"),
        Some(path) => check_file(&path, &rendered),
    }
}

/// 装载包、按 `seed` 重建并逐 tick 推进，返回逐 tick 哈希与推进耗时（毫秒）。
fn generate_hashes(dir: &Path, seed: u64, ticks: u32, dt: u32) -> (Vec<String>, u128) {
    let base = Loaded::from_dir(dir)
        .unwrap_or_else(|err| fail(format!("装载 {} 失败：{err}", dir.display())));
    let mut world = base
        .fork_with_seed(seed)
        .unwrap_or_else(|err| fail(format!("按种子 {seed} 重建失败：{err}")));

    let started = Instant::now();
    let mut hashes: Vec<String> = Vec::with_capacity(ticks as usize);
    for step in 1..=ticks {
        let report = world
            .step(1, dt)
            .unwrap_or_else(|err| fail(format!("第 {step} tick 步进失败：{err}")));
        hashes.push(report.hash.to_string());
    }
    let elapsed_ms = started.elapsed().as_millis();

    (hashes, elapsed_ms)
}

/// 序列化为 2 空格缩进 JSON（含尾换行）。
fn render_golden(seed: u64, ticks: u32, hashes: &[String]) -> String {
    let doc = json!({
        "seed": seed,
        "ticks": ticks,
        "hashes": hashes,
    });
    let mut text = serde_json::to_string_pretty(&doc)
        .unwrap_or_else(|err| fail(format!("序列化金值 JSON 失败：{err}")));
    text.push('\n');
    text
}

/// `--check`：与现有金值文件逐字节比对；如不等，先指出具体哪一 tick 分叉。
fn check_file(path: &Path, expected: &str) {
    let actual = fs::read_to_string(path)
        .unwrap_or_else(|err| fail(format!("无法读取金值文件 {}：{err}", path.display())));
    if actual == expected {
        let parsed: Json = serde_json::from_str(&actual).expect("已匹配现算的合法 JSON");
        let seed = parsed["seed"].as_u64().unwrap_or_default();
        let ticks = parsed["ticks"].as_u64().unwrap_or_default();
        println!("ok: {} (seed={seed}, ticks={ticks})", path.display());
        return;
    }

    // 尝试定位具体分叉的 tick，方便快速排查。
    if let (Ok(exp_json), Ok(act_json)) = (
        serde_json::from_str::<Json>(expected),
        serde_json::from_str::<Json>(&actual),
    ) {
        if let (Some(exp_arr), Some(act_arr)) = (
            exp_json.get("hashes").and_then(Json::as_array),
            act_json.get("hashes").and_then(Json::as_array),
        ) {
            for (idx, (e, a)) in exp_arr.iter().zip(act_arr.iter()).enumerate() {
                if e != a {
                    fail(format!(
                        "金值不匹配（{}）：tick {} 期望 {e}，文件里是 {a}",
                        path.display(),
                        idx + 1
                    ));
                }
            }
        }
    }

    fail(format!(
        "金值文件已漂移：{} 与现算输出逐字节不一致（请重跑生成器刷新）",
        path.display()
    ));
}

fn parse_u64(raw: &str) -> u64 {
    let parsed = if let Some(hex) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16)
    } else {
        raw.parse::<u64>()
    };
    parsed.unwrap_or_else(|err| fail(format!("--seed 解析失败（{raw:?}）：{err}")))
}

fn parse_u32(raw: &str, flag: &str) -> u32 {
    raw.parse::<u32>()
        .unwrap_or_else(|err| fail(format!("{flag} 解析失败（{raw:?}）：{err}")))
}

fn print_usage() {
    eprintln!(
        "用法: cargo run -q --example golden -p ygg-sandbox -- <pack目录> [--seed <u64>] [--ticks <u32>] [--dt <u32>] [--check <文件>] [--final-only]"
    );
    eprintln!("  缺省：--seed 0x5eed --ticks 50 --dt 1；向 stdout 打印 pretty JSON");
    eprintln!("  --check <文件>：与现有金值文件逐字节比对，不一致则非 0 退出");
    eprintln!(
        "  --final-only：只打印 final_hash=<十进制> tick=<N> elapsed_ms=<x>（长跑矩阵取数用）"
    );
}

fn fail(msg: impl AsRef<str>) -> ! {
    eprintln!("错误: {}", msg.as_ref());
    exit(1);
}
