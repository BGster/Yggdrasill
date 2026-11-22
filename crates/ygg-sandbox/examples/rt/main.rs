//! `rt` —— 实时宿主常驻进程（S31 R1；契约 `docs/design/host/rt-face.md`）。
//!
//! 与 `playtest` / `serve` 同先例：逻辑在 `src/rt.rs`（引擎面），本文件只是
//! 参数解析 + 实时节拍的薄壳——与 egg 单派发模式**并存不共享进程**
//! （AGENTS §1.4 单派发规则的 scope 是 egg.rs/scripts 的 agent 回合纪律；
//! 边界论证见 rt-face.md「设计依据」）。
//!
//! # 用法
//!
//! ```bash
//! cargo run -p ygg-sandbox --example rt -- <mod目录> [旗标]
//!   --frames N          跑 N 帧后收口（不限速——fps_dispatch 吞吐口径）
//!   --duration S        实时跑 S 秒后收口（60Hz 节拍；录制 10s 用它）
//!   --hz N              覆盖帧率（缺省：Mod `ygg.rt.config` 声明，再缺省 60）
//!   --max-catchup N     覆盖补帧上限（缺省 4）
//!   --event <名>        每帧派发的事件名（缺省 rt.tick；可指到既有 run 模式
//!                       事件——run 模式兼容桥）
//!   --record <路径>     录制模式：逐帧记录写 JSONL（与 --replay 二选一）
//!   --replay <路径>     回放模式：读录制逐帧重派 + 哈希链比对（first_mismatch）
//!   --input <路径>      输入调度 JSONL：{"frame":N,"down":[..],"up":[..],
//!                       "mouse":[dx,dy,buttons?]}（录制模式可配）
//!   --boot <事件>       循环前单次派发的启动事件（如 Saltline 的 game.boot）
//!   --fuel N            显式燃料预算（写 YGG_FUEL_BUDGET：env 最优先语义不变）
//!   --ppm <目录>        帧落盘通道：present 过的帧写 <目录>/f%06d.ppm
//!                       （P6；headless/selftest/KAT 证据）
//!   --preview <端口>    内嵌预览端口（127.0.0.1:端口）：GET / 页面、
//!                       GET /stream SSE 推帧、POST /api/input 输入回注
//!                       （回注仅录制模式消费；回放确定性不受影响）
//! ```
//!
//! # stdout 契约（机器面；单 JSON 摘要）
//!
//! ```json
//! {"ok":true,"mode":"record","frames":600,"fpsDispatch":412.1,
//!  "droppedFrames":0,"hashHead":"<64hex>","record":"rt-record.jsonl"}
//! ```
//!
//! 回放模式把 `mode` 换成 `"replay"`，哈希链失配时
//! `firstMismatch:{"n":N,"expected":..,"actual":..}`。退出码：0 = ok；
//! 1 = 失败；2 = 用法错误。

use std::path::{Path, PathBuf};
use std::process::exit;
use std::time::{Duration, Instant};

use serde_json::{json, Value as Json};

use ygg_sandbox::driver::KernelFacade;
use ygg_sandbox::modload::ModBundle;
use ygg_sandbox::rt::{
    read_records, replay_records, write_records, FrameRecord, InputEvent, Pacer, ReplayResult,
    RtConfig, RtError, RtHost,
};

mod preview;

fn usage_error(message: &str) -> ! {
    eprintln!(
        "error: {message}\n\n用法: cargo run -p ygg-sandbox --example rt -- <mod目录> \
         [--frames N|--duration S] [--record <路径>|--replay <路径>] [--hz N] \
         [--max-catchup N] [--event <名>] [--input <路径>] [--fuel N] \
         [--ppm <目录>] [--preview <端口>]"
    );
    exit(2);
}

fn next_value(args: &mut impl Iterator<Item = String>, flag: &str) -> String {
    args.next()
        .unwrap_or_else(|| usage_error(&format!("{flag} 缺值")))
}

/// 输入调度 JSONL 一行的解析（`mouse` 可选二元/三元数组，buttons 省略 = 不变）。
fn input_event_from_json(value: &Json) -> Result<(u64, InputEvent), String> {
    let frame = value["frame"].as_u64().ok_or("缺 `frame` 或非非负整数")?;
    let parse_codes = |name: &str| -> Result<Vec<i64>, String> {
        match value[name].as_array() {
            None => Ok(Vec::new()),
            Some(list) => list
                .iter()
                .map(|v| v.as_i64().ok_or(format!("{name} 含非整数键码")))
                .collect(),
        }
    };
    let down = parse_codes("down")?;
    let up = parse_codes("up")?;
    let (dx, dy, buttons) = match value["mouse"].as_array() {
        None => (0, 0, None),
        Some(list) if list.len() == 2 => (
            list[0].as_i64().ok_or("mouse[0] 非整数")?,
            list[1].as_i64().ok_or("mouse[1] 非整数")?,
            None,
        ),
        Some(list) if list.len() == 3 => (
            list[0].as_i64().ok_or("mouse[0] 非整数")?,
            list[1].as_i64().ok_or("mouse[1] 非整数")?,
            Some(list[2].as_i64().ok_or("mouse[2] 非整数")?),
        ),
        Some(list) => {
            return Err(format!(
                "mouse 期望 [dx,dy] 或 [dx,dy,buttons]，得到 {} 项",
                list.len()
            ))
        }
    };
    Ok((
        frame,
        InputEvent {
            down,
            up,
            mouse_dx: dx,
            mouse_dy: dy,
            mouse_buttons: buttons,
        },
    ))
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(mod_dir) = args.next() else {
        usage_error("缺少 mod 目录");
    };
    let mut hz: Option<u32> = None;
    let mut max_catchup: Option<u32> = None;
    let mut frames: Option<u64> = None;
    let mut duration_ms: Option<u64> = None;
    let mut event: Option<String> = None;
    let mut record: Option<PathBuf> = None;
    let mut replay: Option<PathBuf> = None;
    let mut input: Option<PathBuf> = None;
    let mut boot: Option<String> = None;
    let mut fuel: Option<u64> = None;
    let mut ppm: Option<PathBuf> = None;
    let mut preview_port: Option<u16> = None;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--hz" => {
                let value = next_value(&mut args, "--hz");
                hz = Some(match value.parse::<u32>() {
                    Ok(v) if (1..=1000).contains(&v) => v,
                    _ => usage_error("--hz 需要 1..=1000 的整数"),
                });
            }
            "--max-catchup" => {
                let value = next_value(&mut args, "--max-catchup");
                max_catchup = Some(match value.parse::<u32>() {
                    Ok(v) if (1..=64).contains(&v) => v,
                    _ => usage_error("--max-catchup 需要 1..=64 的整数"),
                });
            }
            "--frames" => {
                let value = next_value(&mut args, "--frames");
                frames = Some(match value.parse::<u64>() {
                    Ok(v) if v > 0 => v,
                    _ => usage_error("--frames 需要正整数"),
                });
            }
            "--duration" => {
                let value = next_value(&mut args, "--duration");
                duration_ms = Some(match value.parse::<f64>() {
                    Ok(v) if v > 0.0 => (v * 1000.0) as u64,
                    _ => usage_error("--duration 需要正秒数"),
                });
            }
            "--event" => event = Some(next_value(&mut args, "--event")),
            "--record" => record = Some(PathBuf::from(next_value(&mut args, "--record"))),
            "--replay" => replay = Some(PathBuf::from(next_value(&mut args, "--replay"))),
            "--input" => input = Some(PathBuf::from(next_value(&mut args, "--input"))),
            "--boot" => boot = Some(next_value(&mut args, "--boot")),
            "--fuel" => {
                let value = next_value(&mut args, "--fuel");
                fuel = Some(match value.parse::<u64>() {
                    Ok(v) if v > 0 => v,
                    _ => usage_error("--fuel 需要正整数（VM 指令预算）"),
                });
            }
            "--ppm" => ppm = Some(PathBuf::from(next_value(&mut args, "--ppm"))),
            "--preview" => {
                let value = next_value(&mut args, "--preview");
                preview_port = Some(match value.parse::<u16>() {
                    Ok(v) if v > 0 => v,
                    _ => usage_error("--preview 需要 1..=65535 的端口"),
                });
            }
            other => usage_error(&format!("未知旗标 `{other}`")),
        }
    }
    if record.is_some() == replay.is_some() {
        usage_error("--record / --replay 恰好给一个");
    }
    // 帧数/时限门只约束录制模式；回放不需要节拍参数。
    if record.is_some() && frames.is_some() == duration_ms.is_some() {
        usage_error("录制模式需要 --frames / --duration 恰好给一个");
    }
    if let Some(budget) = fuel {
        // 显式预算：与 env 直设同语义（resolve_budget env 最优先）。
        std::env::set_var("YGG_FUEL_BUDGET", budget.to_string());
    }

    // ① 装载：ModBundle（与宿主实跑同一份语义）→ 内核。
    let bundle = match ModBundle::load(&PathBuf::from(&mod_dir)) {
        Ok(bundle) => bundle,
        Err(message) => {
            eprintln!("装载失败：{message}");
            exit(1);
        }
    };
    let mut kernel = match bundle.kernel() {
        Ok(kernel) => kernel,
        Err(err) => {
            eprintln!("内核构造失败：{err}");
            exit(1);
        }
    };

    // ①′输出通道：PPM 目录（缺则建）与预览端口（绑定失败即可读退出）。
    if let Some(dir) = &ppm {
        if let Err(err) = std::fs::create_dir_all(dir) {
            eprintln!("PPM 目录创建失败（{:?}）：{err}", dir);
            exit(1);
        }
    }
    let preview = match preview_port {
        Some(port) => match preview::spawn(port) {
            Ok(handle) => {
                eprintln!(
                    "预览端口：http://{}/（SSE /stream · 回注 /api/input）",
                    handle.addr
                );
                Some(handle)
            }
            Err(err) => {
                eprintln!("预览端口绑定失败（端口 {port}）：{err}");
                exit(1);
            }
        },
        None => None,
    };

    // ② 配置合流：CLI > Mod 装载期声明（ygg.rt.config）> 默认。
    let (face_hz, face_catchup) = {
        let frame = kernel.rt_handle();
        let frame = frame.borrow();
        (frame.hz, frame.max_catchup)
    };
    let cfg = RtConfig {
        hz: hz.unwrap_or(face_hz),
        max_catchup: max_catchup.unwrap_or(face_catchup),
        event: event.unwrap_or_else(|| "rt.tick".to_owned()),
    };

    // ③ 录制或回放（boot 事件在循环前单次派发——启动初始化不逐帧重做）。
    if let Some(boot_event) = boot {
        let reply = kernel.dispatch(&boot_event, "1");
        let envelope: Json = serde_json::from_str(&reply).unwrap_or(json!({"ok": false}));
        if envelope["ok"] != Json::Bool(true) {
            eprintln!(
                "启动事件 {boot_event} 失败：{}",
                envelope["error"]["message"].as_str().unwrap_or("未知")
            );
            exit(1);
        }
    }
    match (record, replay) {
        (Some(path), _) => run_record(
            &mut kernel,
            cfg,
            input,
            frames,
            duration_ms,
            &path,
            ppm.as_deref(),
            preview.as_ref(),
        ),
        (_, Some(path)) => run_replay(&mut kernel, cfg, &path, ppm.as_deref(), preview.as_ref()),
        _ => unreachable!("arg gate 保证二选一"),
    }
}

/// 帧输出双通道（读侧，零确定性影响）：present 过的帧 → PPM 落盘 + 预览发布。
fn emit_frame_outputs(
    kernel: &KernelFacade,
    n: u64,
    ppm: Option<&Path>,
    preview: Option<&preview::PreviewHandle>,
) {
    let Some(snap) = kernel.frame_handle().borrow().snapshot() else {
        return;
    };
    if let Some(dir) = ppm {
        let path = dir.join(format!("f{n:06}.ppm"));
        if let Err(err) = ygg_sandbox::frame::write_ppm(&path, &snap) {
            eprintln!("PPM 写盘失败（{}）：{err}", path.display());
        }
    }
    if let Some(pv) = preview {
        pv.publish(n, &snap);
    }
}

/// 一步共用体：排空预览回注（录制模式）→ 派发一帧 → present 过则出双通道。
/// 回注拒绝只警告不中断（尽力而为面；确定性录制不受影响）。
fn step_and_emit(
    host: &mut RtHost,
    kernel: &mut KernelFacade,
    preview: Option<&preview::PreviewHandle>,
    ppm: Option<&Path>,
) -> Result<FrameRecord, RtError> {
    if let Some(pv) = preview {
        for event in pv.drain_inputs() {
            let n = host.next_n();
            if let Err(err) = host.input_at(n, event) {
                eprintln!("预览回注被拒：{err}");
            }
        }
    }
    let record = host.step_record(kernel)?;
    if record.frame.is_some() {
        emit_frame_outputs(kernel, record.n, ppm, preview);
    }
    Ok(record)
}

/// 录制模式。`--frames N` = 不限速批跑（fps_dispatch 吞吐口径，直接对比
/// 35.9 进程派发基线）；`--duration S` = 60Hz 实时节拍（Pacer 承接欠账）。
/// 输出通道：present 过的帧 → PPM/预览（读侧）；预览输入回注每帧顶排空。
#[allow(clippy::too_many_arguments)]
fn run_record(
    kernel: &mut KernelFacade,
    cfg: RtConfig,
    input: Option<PathBuf>,
    frames: Option<u64>,
    duration_ms: Option<u64>,
    path: &Path,
    ppm: Option<&Path>,
    preview: Option<&preview::PreviewHandle>,
) -> ! {
    let mut host = RtHost::new(cfg.clone());
    if let Some(script) = input {
        if let Err(err) = load_input_script(&mut host, &script) {
            eprintln!("输入调度装载失败：{err}");
            exit(1);
        }
    }
    let started = Instant::now();
    let mut records: Vec<FrameRecord> = Vec::new();
    let mut failure: Option<RtError> = None;
    let dropped = match frames {
        Some(limit) => {
            // 不限速批跑：直跑 N 帧，fps = 派发吞吐（35.9 进程派发基线的
            // 直接可比口径——基线同样无节拍）。
            while host.next_n() <= limit {
                match step_and_emit(&mut host, kernel, preview, ppm) {
                    Ok(record) => records.push(record),
                    Err(err) => {
                        failure = Some(err);
                        break;
                    }
                }
            }
            0u64
        }
        None => {
            // 实时节拍：Pacer 承接欠账，落后补帧封顶，超出弃账计数。
            let mut pacer = Pacer::new(cfg.hz, cfg.max_catchup);
            let deadline = started + Duration::from_millis(duration_ms.unwrap_or(0));
            loop {
                let now = Instant::now();
                if now >= deadline {
                    break;
                }
                let due_frames = pacer.tick(now);
                for _ in 0..due_frames {
                    match step_and_emit(&mut host, kernel, preview, ppm) {
                        Ok(record) => records.push(record),
                        Err(err) => {
                            failure = Some(err);
                            break;
                        }
                    }
                }
                if failure.is_some() {
                    break;
                }
                // 帧周期余量 sleep（欠账由 Pacer 承接）。
                if let Some(rest) = pacer.target().checked_sub(now.elapsed()) {
                    std::thread::sleep(rest);
                }
            }
            pacer.dropped_frames()
        }
    };
    let wall = started.elapsed();
    if let Err(err) = write_records(path, &records) {
        eprintln!("录制写盘失败：{err}");
        exit(1);
    }
    let fps = records.len() as f64 / wall.as_secs_f64().max(f64::EPSILON);
    let mut summary = json!({
        "ok": failure.is_none(),
        "mode": "record",
        "frames": records.len(),
        "fpsDispatch": (fps * 10.0).round() / 10.0,
        "droppedFrames": dropped,
        "durationMs": wall.as_millis() as u64,
        "hashHead": host.chain_head(),
        "record": path.display().to_string(),
        "hz": cfg.hz,
        "event": cfg.event,
    });
    if let Some(err) = &failure {
        summary["error"] = json!(err.to_string());
    }
    println!("{summary}");
    exit(if failure.is_none() { 0 } else { 1 });
}

/// 回放模式：读录制逐帧重派 + 哈希链比对，first_mismatch 证伪语义。
/// 输出通道照常出帧（读侧）；预览回注不消费（回放确定性红线）。
fn run_replay(
    kernel: &mut KernelFacade,
    cfg: RtConfig,
    path: &Path,
    ppm: Option<&Path>,
    preview: Option<&preview::PreviewHandle>,
) -> ! {
    let records = match read_records(path) {
        Ok(records) => records,
        Err(err) => {
            eprintln!("{err}");
            exit(1);
        }
    };
    let started = Instant::now();
    let mut host = RtHost::new(cfg.clone());
    let ReplayResult {
        frames_checked,
        first_mismatch,
        dispatch_error,
    } = replay_records(kernel, &mut host, &records);
    // 回放无逐帧钩子（replay_records 收口在先）；把末帧快照补发给双通道
    // （录制末帧 present 过 = 回放同位也 present 过，快照一致）。
    if let Some(last) = records.last() {
        if last.frame.is_some() && first_mismatch.is_none() && dispatch_error.is_none() {
            emit_frame_outputs(kernel, last.n, ppm, preview);
        }
    }
    let wall = started.elapsed();
    let ok = first_mismatch.is_none() && dispatch_error.is_none();
    let mut summary = json!({
        "ok": ok,
        "mode": "replay",
        "framesChecked": frames_checked,
        "framesTotal": records.len(),
        "replayMs": wall.as_millis() as u64,
        "hashChain": if ok { "match" } else { "mismatch" },
        "record": path.display().to_string(),
    });
    if let Some(step) = first_mismatch {
        summary["firstMismatch"] = json!({
            "n": step.n,
            "expected": step.expected,
            "actual": step.actual,
        });
    }
    if let Some(err) = dispatch_error {
        summary["error"] = json!(err);
    }
    println!("{summary}");
    exit(if ok { 0 } else { 1 });
}

/// 输入调度 JSONL 装载（一行一事件；帧序无关——设备按帧消费）。
fn load_input_script(host: &mut RtHost, path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|err| format!("读取失败：{err}"))?;
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Json = serde_json::from_str(line)
            .map_err(|err| format!("第 {} 行不是合法 JSON：{err}", index + 1))?;
        let (frame, event) =
            input_event_from_json(&value).map_err(|err| format!("第 {} 行：{err}", index + 1))?;
        host.input_at(frame, event)
            .map_err(|err| format!("第 {} 行：{err}", index + 1))?;
    }
    Ok(())
}
