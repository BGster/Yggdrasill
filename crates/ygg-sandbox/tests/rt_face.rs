//! `rt-*` 族（S31 R1）：实时宿主面 e2e——定步推进 / dt 折叠 / catch-up 上限 /
//! 输入快照规范序与折叠 / 录制→回放哈希链全等 / 篡改 dt 与键序负例红 /
//! 燃料预算交互钉死 / run 模式兼容同 mod 双证。
//!
//! 断言都以**录制文件与信封字节**为准（回放证据面），不是 Rust 侧中间变量。
//! 所有帧循环走无墙钟节拍的 [`record_frames`]（调度节拍另测
//! `rt_pacer_catchup_is_capped`）——确定性不依赖测试机速度。

use std::time::{Duration, Instant};

use serde_json::json;
use ygg_sandbox::driver::KernelFacade;
use ygg_sandbox::host_api::RtInput;
use ygg_sandbox::rt::{
    dt_for_frame, frame_payload, read_records, record_frames, replay_records, write_records,
    FrameRecord, InputDevice, InputEvent, Pacer, RtConfig, RtError, RtHost,
};

/// rt 测试 mod：rt.tick 每帧把 dt 记进 KV（状态随 dt 漂移 → 篡改 dt 双层
/// 被抓），pressed 数与鼠标位移一并入 KV；game.tick 是 run 模式退化帧
/// （每派发 +1）。两事件同 mod 注册 = run 模式兼容的同 mod 双证载体。
const RT_MOD: &str = r#"
ygg.on("rt.tick", function(payload)
  local n = tonumber(payload:match("^n=(%d+);"))
  local dt = tonumber(payload:match(";dt=(-?%d+);"))
  local keys = ygg.rt.input.keys()
  local mouse = ygg.rt.input.mouse()
  local total = tonumber(ygg.kv.get("sim_ms") or "0") + dt
  ygg.kv.set("sim_ms", tostring(total))
  ygg.kv.set("frame", tostring(n))
  ygg.kv.set("pressed_n", tostring(#keys.pressed))
  ygg.kv.set("mouse_dx", tostring(mouse.dx))
  ygg.narrate("frame " .. n .. " dt " .. dt .. " logical " .. ygg.rt.input.time())
end)
ygg.on("game.tick", function(payload)
  local total = tonumber(ygg.kv.get("sim_ms") or "0") + 1
  ygg.kv.set("sim_ms", tostring(total))
  ygg.narrate("run step " .. tostring(payload))
end)
"#;

/// 60Hz 下前 5 帧的定步 dt（floor 差分：16/17 交替，积分恰为实时）。
#[test]
fn rt_dt_sequence_is_floor_difference_and_integrates_to_real_time() {
    assert_eq!(
        (1..=5).map(|n| dt_for_frame(n, 60)).collect::<Vec<_>>(),
        vec![16, 17, 17, 16, 17],
        "60Hz 前五帧 16/17 交替"
    );
    // 任意 hz：一秒的帧积分 = 1000ms（无浮点、无墙钟入值）。
    for hz in [1u32, 30, 60, 120, 1000] {
        let total: i64 = (1..=u64::from(hz)).map(|n| dt_for_frame(n, hz)).sum();
        assert_eq!(total, 1000, "hz={hz} 一秒积分应为 1000ms");
    }
    assert_eq!(dt_for_frame(1, 1), 1000, "hz=1 退化：单帧 1000ms");
}

/// 输入快照：规范序（升序）+ 帧内折叠（净沿）+ 鼠标叠合 + 越界前移拒绝。
#[test]
fn rt_input_snapshot_folds_net_edges_in_canonical_order() {
    let mut device = InputDevice::default();
    // 同帧注入乱序键：down 5,2,1 + up 3（未按下）→ pressed=[1,2,5]（升序）、
    // released=[]（3 本来就没按下，非沿）。
    device
        .at(
            1,
            InputEvent {
                down: vec![5, 2],
                ..Default::default()
            },
        )
        .expect("schedule");
    device
        .at(
            1,
            InputEvent {
                down: vec![1],
                up: vec![3],
                ..Default::default()
            },
        )
        .expect("schedule");
    let snap = device.snapshot_for(1).expect("snapshot");
    assert_eq!(
        snap.pressed,
        vec![1, 2, 5],
        "pressed 规范序（注入乱序被排序）"
    );
    assert!(snap.released.is_empty(), "未按下键的 up 不是沿");
    assert_eq!(snap.held, vec![1, 2, 5]);
    // 帧内 down+up 同键 = 净零折叠：第 2 帧 down 7 + up 7 → 两侧都不出现。
    device
        .at(
            2,
            InputEvent {
                down: vec![7],
                ..Default::default()
            },
        )
        .expect("schedule");
    device
        .at(
            2,
            InputEvent {
                up: vec![7],
                ..Default::default()
            },
        )
        .expect("schedule");
    let snap2 = device.snapshot_for(2).expect("snapshot");
    assert!(
        !snap2.pressed.contains(&7) && !snap2.released.contains(&7),
        "帧内净零折叠（帧初/帧末差集）"
    );
    assert!(!snap2.held.contains(&7));
    // 帧内 up+down 同键（按住中抬起又按下）= 净零：两侧都不出现。同帧
    // 序歧义（单事件内同键 down+up）在注入时点即拒绝，这里用两次注入保序。
    assert!(
        matches!(
            device.at(
                3,
                InputEvent {
                    up: vec![5],
                    down: vec![5],
                    ..Default::default()
                }
            ),
            Err(RtError::Input(_))
        ),
        "单事件同键 down+up 歧义即拒"
    );
    device
        .at(
            3,
            InputEvent {
                up: vec![5],
                ..Default::default()
            },
        )
        .expect("schedule");
    device
        .at(
            3,
            InputEvent {
                down: vec![5],
                ..Default::default()
            },
        )
        .expect("schedule");
    let snap2b = device.snapshot_for(3).expect("snapshot");
    assert!(
        !snap2b.pressed.contains(&5) && !snap2b.released.contains(&5),
        "帧内 up+down 净零"
    );
    assert!(snap2b.held.contains(&5), "帧末仍按住");
    // 鼠标：位移累加、掩码后写胜。
    device
        .at(
            3,
            InputEvent {
                mouse_dx: 3,
                mouse_dy: -1,
                ..Default::default()
            },
        )
        .expect("schedule");
    device
        .at(
            3,
            InputEvent {
                mouse_dx: 2,
                mouse_buttons: Some(2),
                ..Default::default()
            },
        )
        .expect("schedule");
    let snap3 = device.snapshot_for(3).expect("snapshot");
    assert_eq!((snap3.mouse_dx, snap3.mouse_dy), (5, -1), "帧内位移累加");
    assert_eq!(snap3.mouse_buttons, 2, "掩码后写胜");
    // 越界键码在注入时点即拒绝（错误前移，不留到帧中）。
    assert!(matches!(
        device.at(
            4,
            InputEvent {
                down: vec![0],
                ..Default::default()
            }
        ),
        Err(RtError::Input(_))
    ));
    assert!(matches!(
        device.at(
            4,
            InputEvent {
                down: vec![65],
                ..Default::default()
            }
        ),
        Err(RtError::Input(_))
    ));
}

/// 规范串：parse 是 canonical 的严格逆；非规范序（键序未升）被拒——
/// 录制文件手改不留活口。
#[test]
fn rt_canonical_string_parse_is_strict_inverse() {
    let snap = RtInput {
        held: vec![1, 3, 4],
        pressed: vec![2, 5],
        released: vec![1],
        mouse_dx: -7,
        mouse_dy: 9,
        mouse_buttons: 3,
    };
    let text = snap.canonical();
    // 位串长度 = 三向量最大键码（这里 pressed 的 5）。
    assert_eq!(text, "k=10110;p=2,5;r=1;m=-7,9,3", "规范串字节逐字符断言");
    let back = RtInput::parse(&text).expect("round-trip");
    assert_eq!(back, snap);
    assert_eq!(back.canonical(), text);
    // 空快照 = 全 `-`。
    let neutral = RtInput::neutral();
    assert_eq!(neutral.canonical(), "k=-;p=-;r=-;m=0,0,0");
    // 非规范序 / 形状残缺：parse 拒绝（不静默吞）。
    assert!(RtInput::parse("k=-;p=5,2;r=-;m=0,0,0").is_err());
    assert!(RtInput::parse("k=-;p=-;r=-;m=0,0").is_err());
    assert!(RtInput::parse("k=-;p=-;m=0,0,0").is_err());
}

/// 录制 → 回放：同 mod 同录制，逐帧哈希链全等（10 帧 + 注入输入序列）。
#[test]
fn rt_record_then_replay_hash_chain_matches_frame_by_frame() {
    let records = {
        let mut kernel = KernelFacade::new(RT_MOD).expect("kernel");
        let mut host = RtHost::new(RtConfig::default());
        host.input_at(
            2,
            InputEvent {
                down: vec![4, 1],
                ..Default::default()
            },
        )
        .expect("schedule");
        // 同帧 1 重复 down：仍单沿（已按下的 down 不重复计沿）。
        host.input_at(
            2,
            InputEvent {
                down: vec![1, 9],
                ..Default::default()
            },
        )
        .expect("schedule");
        host.input_at(
            5,
            InputEvent {
                up: vec![4],
                mouse_dx: 6,
                ..Default::default()
            },
        )
        .expect("schedule");
        let mut sink = Vec::new();
        let done = record_frames(&mut kernel, &mut host, 10, &mut sink).expect("record 10");
        assert_eq!(done, 10);
        sink
    };
    // 录制字节面：帧号严格递增、dt 序列即定步公式、链头逐帧不同。
    for (index, record) in records.iter().enumerate() {
        assert_eq!(record.n, index as u64 + 1);
        assert_eq!(record.dt_ms, dt_for_frame(record.n, 60));
    }
    assert!(
        records.windows(2).all(|w| w[0].hash != w[1].hash),
        "逐帧链头互异"
    );
    // 输入面：第 2 帧 pressed 折叠为 [1,4,9]（乱序注入、1 重复 down 单沿），
    // held 位串 = 100100001（位 1/4/9）。
    assert_eq!(
        records[1].input, "k=100100001;p=1,4,9;r=-;m=0,0,0",
        "规范序+折叠"
    );
    assert_eq!(
        records[4].input, "k=100000001;p=-;r=4;m=6,0,0",
        "第 5 帧抬起+鼠标"
    );
    // 回放：新内核 + 同配置，逐帧比对。
    let mut kernel = KernelFacade::new(RT_MOD).expect("kernel");
    let mut host = RtHost::new(RtConfig::default());
    let result = replay_records(&mut kernel, &mut host, &records);
    assert!(result.ok(), "回放应全等：{result:?}");
    assert_eq!(result.frames_checked, 10);
    assert_eq!(
        host.chain_head(),
        records.last().expect("frames").hash,
        "链头相等"
    );
}

/// 录制文件 JSONL 往返：逐字段相等、行数守恒、缺字段行整文件可读拒绝。
#[test]
fn rt_record_file_roundtrip_and_tamper_visibility() {
    let records = {
        let mut kernel = KernelFacade::new(RT_MOD).expect("kernel");
        let mut host = RtHost::new(RtConfig::default());
        let mut sink = Vec::new();
        record_frames(&mut kernel, &mut host, 3, &mut sink).expect("record");
        sink
    };
    let dir = std::env::temp_dir().join(format!("ygg-rt-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmpdir");
    let path = dir.join("record.jsonl");
    write_records(&path, &records).expect("write");
    let round = read_records(&path).expect("read");
    assert_eq!(round, records, "JSONL 往返逐字段相等");
    let text = std::fs::read_to_string(&path).expect("file");
    assert_eq!(text.lines().count(), 3, "一行一帧");
    // 篡改文件：缺字段行 → 解析层可读拒绝（不静默跳，报错点名字段）。
    std::fs::write(&path, "{\"n\":1,\"dt_ms\":16}\n").expect("tamper");
    let err = read_records(&path).expect_err("缺字段必须拒");
    assert!(
        err.to_string().contains("字段"),
        "缺字段报错点名字段：{err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 负例一：篡改录制 dt（16→15）→ 回放 first_mismatch 恰在被篡帧。
#[test]
fn rt_replay_catches_tampered_dt_at_first_mismatch() {
    let mut records = {
        let mut kernel = KernelFacade::new(RT_MOD).expect("kernel");
        let mut host = RtHost::new(RtConfig::default());
        let mut sink = Vec::new();
        record_frames(&mut kernel, &mut host, 6, &mut sink).expect("record");
        sink
    };
    // 篡改第 4 帧 dt：16→15（payload 与链输入同时变——双层证据都变）。
    records[3].dt_ms = 15;
    let mut kernel = KernelFacade::new(RT_MOD).expect("kernel");
    let mut host = RtHost::new(RtConfig::default());
    let result = replay_records(&mut kernel, &mut host, &records);
    assert!(!result.ok());
    let mismatch = result.first_mismatch.expect("必须有 first_mismatch");
    assert_eq!(mismatch.n, 4, "first_mismatch = 被篡帧号（证伪语义）");
    assert_ne!(mismatch.expected, mismatch.actual);
    // 回放停止在首处失配：只核到第 4 帧。
    assert_eq!(result.frames_checked, 4);
}

/// 负例二：篡改录制键序（p=1,2 → p=2,1）→ 同帧红（规范序是证据面）。
#[test]
fn rt_replay_catches_tampered_key_order_at_first_mismatch() {
    let mut records = {
        let mut kernel = KernelFacade::new(RT_MOD).expect("kernel");
        let mut host = RtHost::new(RtConfig::default());
        host.input_at(
            3,
            InputEvent {
                down: vec![2, 1],
                ..Default::default()
            },
        )
        .expect("schedule");
        let mut sink = Vec::new();
        record_frames(&mut kernel, &mut host, 5, &mut sink).expect("record");
        sink
    };
    assert!(
        records[2].input.contains("p=1,2"),
        "录制里是规范序：{}",
        records[2].input
    );
    // 键序对调（同键集、异字节）：载荷重建变化 → 链哈希变化 → 该帧红。
    records[2].input = records[2].input.replace("p=1,2", "p=2,1");
    let mut kernel = KernelFacade::new(RT_MOD).expect("kernel");
    let mut host = RtHost::new(RtConfig::default());
    let result = replay_records(&mut kernel, &mut host, &records);
    let mismatch = result.first_mismatch.expect("键序篡改必红");
    assert_eq!(mismatch.n, 3, "first_mismatch = 被篡帧");
    assert_ne!(mismatch.expected, mismatch.actual);
}

/// run 模式兼容（同 mod 双证）：game.tick（dt=1 退化帧）与 rt.tick（定步
/// dt）并存——同一 mod 两事件各自可跑，rt 面在 run 态报 mode="run"。
#[test]
fn rt_run_mode_coexists_with_rt_mode_on_same_mod() {
    // ① run 模式证：单发派发 game.tick（egg --drive 同面）——dt=1 退化。
    let mut kernel = KernelFacade::new(RT_MOD).expect("kernel");
    let info_before = rt_info(&kernel);
    assert_eq!(
        info_before["mode"],
        json!("run"),
        "未进 rt 循环前 = run 模式"
    );
    assert_eq!(info_before["n"], json!(0));
    let _ = reply_ok(&kernel.dispatch("game.tick", "1"));
    let _ = reply_ok(&kernel.dispatch("game.tick", "2"));
    let save = reply_ok(&kernel.dispatch("save", ""));
    assert_eq!(
        save["save"]["kv"]["entries"]["sim_ms"],
        json!("2"),
        "退化帧每步 +1（mod 存字符串 KV）"
    );
    // run 态下 rt 面仍可读（中性快照，无输入）。
    let keys = run_mode_held(&kernel);
    assert_eq!(keys, json!(""), "run 态输入快照中性");
    // ② rt 模式证：同 mod 的 rt.tick 定步帧——3 帧 = 16+17+17 = 50ms。
    let mut host = RtHost::new(RtConfig::default());
    let mut sink = Vec::new();
    record_frames(&mut kernel, &mut host, 3, &mut sink).expect("rt frames");
    let save = reply_ok(&kernel.dispatch("save", ""));
    // run 的 2ms + rt 的 50ms 在同一状态上累计（并存，不互毁）。
    assert_eq!(save["save"]["kv"]["entries"]["sim_ms"], json!("52"));
    let info = rt_info(&kernel);
    assert_eq!(info["mode"], json!("rt"));
    assert_eq!(info["n"], json!(3));
    assert_eq!(info["logicalMs"], json!(50), "逻辑钟 = 逐帧 dt 累加");
}

/// 调度节拍：落后补帧 ≤ max_catchup，超出弃帧并计数（螺旋死亡防护）。
/// 合成时钟驱动——不依赖测试机速度；期望值按节拍器同一整数公式推导。
#[test]
fn rt_pacer_catchup_is_capped() {
    let t0 = Instant::now();
    let target = Duration::from_nanos(1_000_000_000 / 60);
    let due_of = |elapsed: Duration| {
        u32::try_from(elapsed.as_nanos() / target.as_nanos()).unwrap_or(u32::MAX)
    };
    let mut pacer = Pacer::new(60, 4);
    assert_eq!(pacer.tick(t0), 1, "首拍先跑 1 帧（起点无积压）");
    // 停 100ms：due 帧欠账被封顶到 4，超出弃掉并计数。
    let t1 = t0 + Duration::from_millis(100);
    let due1 = due_of(Duration::from_millis(100));
    assert!(due1 > 4, "100ms 欠账超过补帧上限（due={due1}）");
    assert_eq!(pacer.tick(t1), 4, "补帧上限 = max_catchup");
    assert_eq!(
        pacer.dropped_frames(),
        u64::from(due1 - 4),
        "弃帧 = due − cap"
    );
    // 正常节拍（一拍一帧）：不再弃。
    let t2 = t1 + target;
    assert_eq!(pacer.tick(t2), 1);
    assert_eq!(
        pacer.dropped_frames(),
        u64::from(due1 - 4),
        "正常节拍零弃帧"
    );
    // 长停 1s：仍只补 4、余者弃（elapsed 恰为 1s——t2 后无积压）。
    let t3 = t2 + Duration::from_secs(1);
    let due3 = due_of(Duration::from_secs(1));
    assert!(due3 > 4);
    assert_eq!(pacer.tick(t3), 4);
    assert_eq!(
        pacer.dropped_frames(),
        u64::from(due1 - 4) + u64::from(due3 - 4),
        "两次弃账累加"
    );
}

/// 未注册帧事件 → 第 1 帧即可读拒绝（VALIDATION 列出已注册事件集）。
#[test]
fn rt_unregistered_frame_event_fails_fast_with_registered_list() {
    let mut kernel = KernelFacade::new(RT_MOD).expect("kernel");
    let mut host = RtHost::new(RtConfig {
        event: "rt.frame".to_owned(),
        ..RtConfig::default()
    });
    let mut sink = Vec::new();
    let err = record_frames(&mut kernel, &mut host, 1, &mut sink).expect_err("未注册必拒");
    match err {
        RtError::Dispatch { n, code, message } => {
            assert_eq!(n, 1);
            assert_eq!(code, "VALIDATION");
            assert!(
                message.contains("rt.tick"),
                "可读文案列出已注册事件：{message}"
            );
        }
        other => panic!("期望派发拒绝，得到 {other:?}"),
    }
}

/// 配置面：装载期生效、seal 后禁改、未知键与越界拒绝、调度参数入链。
#[test]
fn rt_config_is_load_time_only_and_closed_shape() {
    let source = r#"
ygg.on("rt.tick", function() end)
ygg.rt.config({ hz = 30, maxCatchup = 2 })
local ok_cfg = pcall(function() ygg.rt.config({ hz = 5 }) end)
ygg.kv.set("load_cfg_ok", ok_cfg and "1" or "0")
ygg.on("game.tick", function()
  local ok, err = pcall(function() ygg.rt.config({ hz = 90 }) end)
  if ok then
    ygg.narrate("allowed")
  else
    ygg.narrate("sealed:" .. tostring(err))
  end
end)
"#;
    let mut kernel = KernelFacade::new(source).expect("kernel");
    // 装载期两次 config 都成功（30 → 5 覆盖）。
    let save = reply_ok(&kernel.dispatch("save", ""));
    assert_eq!(
        save["save"]["kv"]["entries"]["load_cfg_ok"],
        json!("1"),
        "装载期 config 可用"
    );
    // rt 引擎按 (hz=5) 跑：dt 全 200。
    let mut host = RtHost::new(RtConfig {
        hz: 5,
        ..RtConfig::default()
    });
    let mut sink = Vec::new();
    record_frames(&mut kernel, &mut host, 2, &mut sink).expect("frames at 5hz");
    assert_eq!(sink[0].dt_ms, 200, "hz=5 定步 dt=200");
    // seal 后（运行期 handler 内）禁改：YGG-RT-SEALED 可读拒绝。
    let reply = kernel.dispatch("game.tick", "1");
    assert!(reply.contains("YGG-RT-SEALED"), "seal 后禁改：{reply}");
    // 未知键 / 越界：装载期即可读拒绝（ModLoad 失败携带拒绝码）。
    let bad = match KernelFacade::new("ygg.rt.config({ hz = 0 })") {
        Err(err) => err.to_string(),
        Ok(_) => panic!("hz=0 越界必须拒绝"),
    };
    assert!(bad.contains("YGG-RT-SHAPE"), "{bad}");
    let bad2 = match KernelFacade::new("ygg.rt.config({ what = 1 })") {
        Err(err) => err.to_string(),
        Ok(_) => panic!("未知键必须拒绝"),
    };
    assert!(bad2.contains("YGG-RT-SHAPE"), "{bad2}");
    // 调度参数入链：同一录制换 hz 回放即红在第 1 帧。
    let records = {
        let mut kernel = KernelFacade::new(RT_MOD).expect("kernel");
        let mut host = RtHost::new(RtConfig::default());
        let mut sink = Vec::new();
        record_frames(&mut kernel, &mut host, 2, &mut sink).expect("record");
        sink
    };
    let mut kernel = KernelFacade::new(RT_MOD).expect("kernel");
    let mut host = RtHost::new(RtConfig {
        hz: 120,
        ..RtConfig::default()
    });
    let result = replay_records(&mut kernel, &mut host, &records);
    let mismatch = result.first_mismatch.expect("hz 换配置回放必红");
    assert_eq!(mismatch.n, 1, "调度参数入链：配置篡改第 1 帧即红");
}

/// `frame_payload` 单点收口：载荷字节 = 引擎派发字节（录制/回放一致的前提）。
#[test]
fn rt_payload_construction_is_single_point() {
    assert_eq!(
        frame_payload(7, 16, "k=-;p=1;r=-;m=0,0,0"),
        "n=7;dt=16;k=-;p=1;r=-;m=0,0,0"
    );
}

// ---- helpers ----

fn reply_ok(reply: &str) -> serde_json::Value {
    let parsed: serde_json::Value = serde_json::from_str(reply).expect("信封是 JSON");
    assert_eq!(parsed["ok"], json!(true), "期望成功信封：{reply}");
    parsed
}

/// loop.info 投影的 Rust 侧同源读取（测试面：同一 RtFrame）。
fn rt_info(kernel: &KernelFacade) -> serde_json::Value {
    let frame = kernel.rt_handle();
    let frame = frame.borrow();
    json!({
        "mode": frame.mode.as_str(),
        "n": frame.n,
        "hz": frame.hz,
        "logicalMs": frame.logical_ms,
    })
}

/// run 态 held 位串（中性 = 空串）。
fn run_mode_held(kernel: &KernelFacade) -> serde_json::Value {
    let frame = kernel.rt_handle();
    let frame = frame.borrow();
    if frame.input.held.is_empty() {
        json!("")
    } else {
        json!("?")
    }
}

/// 录制记录零值占位（防 unused 警告的显式消费面；FrameRecord 在多个
/// 用例间共享类型而无需完整构造）。
#[allow(dead_code)]
fn frame_record_shape(record: &FrameRecord) -> (u64, i64, &str, &str, &str) {
    (
        record.n,
        record.dt_ms,
        &record.input,
        &record.state,
        &record.hash,
    )
}
