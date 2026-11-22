//! `frame-*` 族（S31 R2）：渲染面 core + Lua 五键——帧 digest KAT 钉值 /
//! 越界钳制 / blit 形状拒绝 / 脏区并集与复位 / PPM 头字节 / 录制行 frame
//! 字段加性 / Lua 五键过内核的录制→回放全等 / seal 与形状错误可读。
//!
//! KAT 钉值以 **python3 hashlib 预算法**独立复算钉死（规格：
//! `SHA256("ygg.rt.frame.v1|<w>x<h>|" ++ RGBA8 行主序顶行在前原始字节)`），
//! 与计划稿钉值截断（前 8/后 7 位）逐项比对一致。

use std::time::{Duration, Instant};

use ygg_sandbox::driver::KernelFacade;
use ygg_sandbox::frame::{frame_digest, write_ppm, FrameError, FrameFace, Rect, FRAME_MAX_EDGE};
use ygg_sandbox::rt::{record_frames, replay_records, RtConfig, RtHost};

const RED: u32 = 0xFF00_00FF;
const GREEN: u32 = 0x0000_00FF;

/// K1：2×2 全 clear 0x102030FF（黑蓝调不透明）。
const K1: &str = "e659996d4c7cbacdbcc8b4a7d7e3bb7cab4b9b30ddacb757f073552d4860e53a";
/// K2：4×3 黑底 + 中行红 span。
const K2: &str = "a74025228840224be1a06bb4bf09bfca19663da8361e42325b8459f93b6d042a";
/// K3：K2 后 blit 2×1 绿 @(1,1)。
const K3: &str = "496e8c2003f9bcc94b4c7ce6af747e12717f3dc8e0a7554686d9e6915479a66c";
/// K4：4×3 全红（span(−1,−1,9,9,红) ≡ clear(红)——钳制语义被钉）。
const K4: &str = "d6c3d184baeb1d22d98ae114c8bf78b35ab7f13e4978ffdb7667f8e612dfc45d";

fn open(w: i64, h: i64) -> FrameFace {
    let mut face = FrameFace::new();
    let handle = face.open(w, h).expect("open");
    assert_eq!(handle, 1, "v1 单面 handle 1 起");
    face
}

/// K1–K4 钉值逐字节复现 + K4 钳制等价（两组调用序同 digest）。
#[test]
fn frame_kat_pins_k1_to_k4_reproduce_byte_exact() {
    // K1：2×2 全 clear 0x102030FF。
    let mut face = open(2, 2);
    face.clear(0x1020_30FF).expect("clear");
    assert_eq!(face.present(0).expect("present").digest, K1);

    // K2：4×3 黑底 + 中行红 span。
    let mut face = open(4, 3);
    face.clear(0x0000_00FF).expect("clear");
    face.span(0, 1, 4, 1, RED).expect("span");
    assert_eq!(face.present(0).expect("present").digest, K2);

    // K3：K2 后 blit 2×1 绿 @(1,1)（源字节 RGBA8：绿 = 00 FF 00 FF）。
    let mut face = open(4, 3);
    face.clear(0x0000_00FF).expect("clear");
    face.span(0, 1, 4, 1, RED).expect("span");
    face.blit(1, 1, 2, &[0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF])
        .expect("blit");
    assert_eq!(face.present(0).expect("present").digest, K3);

    // K4：钳制等价——span(−1,−1,9,9,红) 与 clear(红) 同 digest（越界部分
    // 被静默裁掉后两组调用序落到同一缓冲字节）。
    let mut spanned = open(4, 3);
    spanned.span(-1, -1, 9, 9, RED).expect("span 越界钳制");
    let mut cleared = open(4, 3);
    cleared.clear(RED).expect("clear");
    assert_eq!(
        spanned.present(0).expect("present").digest,
        cleared.present(0).expect("present").digest
    );
    assert_eq!(spanned.present(0).expect("present").digest, K4);
}

/// 越界裁剪：半出界 span 只写入界部分（剩余矩形字节断言）。
#[test]
fn frame_span_clips_out_of_bounds_and_remaining_rect_bytes_asserted() {
    let mut face = open(4, 3);
    face.clear(0x0000_00FF).expect("clear");
    // span(−1, 0, 2, 1)：入界部分只有 col 0。
    face.span(-1, 0, 2, 1, RED).expect("span");
    let snap = face.snapshot().expect("snapshot");
    assert_eq!(snap.pixels[0], RED, "col 0 入界被写");
    assert_eq!(snap.pixels[1], 0x0000_00FF, "col 1 出界被裁");
    // 底两行未触碰。
    assert!(snap.pixels[8..].iter().all(|p| *p == 0x0000_00FF));
    // 完全出界 = 零面积 no-op（零错误）。
    face.span(4, 0, 2, 2, RED).expect("完全出界 no-op");
    face.span(0, 3, 1, 1, RED).expect("完全出界 no-op");
    // 超大 span 钳到全幅（全缓冲变绿，非 K4 红）。
    face.span(-100, -100, 1000, 1000, GREEN).expect("钳制");
    assert_eq!(
        face.present(0).expect("present").digest,
        frame_digest(4, 3, &vec![GREEN; 12]),
        "钳制后全幅绿"
    );
}

/// 形状拒绝面：负尺寸 / 零尺寸 open / 越界 open / 再开 / handle 不符 /
/// blit 长度不整除与空串——全 `YGG-RT-SHAPE` / `YGG-RT-OPENED` 可读错误。
#[test]
fn frame_shape_and_open_rejections_are_typed() {
    let mut face = open(4, 3);
    assert_eq!(
        face.span(0, 0, -1, 1, RED),
        Err(FrameError::Shape(
            "span 宽高必须非负，得到 w=-1,h=1".to_owned()
        ))
    );
    assert_eq!(
        face.span(0, 0, 1, -1, RED),
        Err(FrameError::Shape(
            "span 宽高必须非负，得到 w=1,h=-1".to_owned()
        ))
    );
    assert!(matches!(
        face.blit(0, 0, 2, &[0, 0, 0, 0, 0]),
        Err(FrameError::Shape(_))
    ));
    assert!(matches!(face.blit(0, 0, 2, &[]), Err(FrameError::Shape(_))));
    assert!(matches!(
        face.blit(0, 0, 0, &[0, 0, 0, 0]),
        Err(FrameError::Shape(_))
    ));
    // 再开 = Opened；越界尺寸 = Shape。
    assert_eq!(face.open(2, 2), Err(FrameError::Opened));
    assert!(matches!(face.open(0, 3), Err(FrameError::Shape(_))));
    assert!(matches!(
        face.open(1, FRAME_MAX_EDGE + 1),
        Err(FrameError::Shape(_))
    ));
    // 未 open 面上的一切写键 = Shape。
    let mut closed = FrameFace::new();
    assert!(matches!(closed.clear(RED), Err(FrameError::Shape(_))));
    assert!(matches!(closed.present(1), Err(FrameError::Shape(_))));
    assert!(!closed.is_open());
    assert_eq!(closed.handle(), 0, "未 open 无 handle");
}

/// 脏区：多写并集（最小外包矩形）→ present 读出后复位 → 再 present = 无写。
#[test]
fn frame_dirty_unions_writes_and_present_resets() {
    let mut face = open(8, 6);
    face.span(1, 2, 2, 1, RED).expect("span");
    face.span(4, 1, 2, 2, GREEN).expect("span");
    let info = face.present(7).expect("present");
    assert_eq!(
        info.dirty,
        Some(Rect {
            x: 1,
            y: 1,
            w: 5,
            h: 2
        }),
        "并集 = 最小外包矩形（y 1..3）"
    );
    assert_eq!(info.frame, 7, "present 回执带帧号");
    // present 后脏区复位：无写再 present = nil。
    assert_eq!(face.present(7).expect("present").dirty, None);
    // clear = 全幅脏区。
    face.clear(RED).expect("clear");
    assert_eq!(
        face.present(7).expect("present").dirty,
        Some(Rect {
            x: 0,
            y: 0,
            w: 8,
            h: 6
        })
    );
}

/// PPM 头字节钉值：`P6\n<w> <h>\n255\n` + RGB（弃 alpha）。
#[test]
fn frame_ppm_bytes_are_pinned() {
    let mut face = open(2, 1);
    face.span(0, 0, 1, 1, 0x1020_30FF).expect("span");
    let snap = face.snapshot().expect("snapshot");
    let dir = std::env::temp_dir().join(format!("ygg-frame-ppm-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("f000001.ppm");
    write_ppm(&path, &snap).expect("write");
    let bytes = std::fs::read(&path).expect("read");
    assert_eq!(&bytes[..11], b"P6\n2 1\n255\n", "P6 头逐字节");
    assert_eq!(bytes.len(), 11 + 2 * 3, "载荷 = w×h×3（弃 alpha）");
    assert_eq!(&bytes[11..14], &[0x10, 0x20, 0x30], "首像素 RGB");
    assert_eq!(
        &bytes[14..17],
        &[0x00, 0x00, 0x00],
        "次像素 RGB（open 缺省零）"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// 录制行 frame 字段：加性（有则写、无则整键缺席）、字典序落位、往返等价、
/// 类型不符可读拒绝。R1 形状（无 frame）行字节逐字节不变。
#[test]
fn frame_record_line_field_is_additive_and_byte_stable() {
    let base = ygg_sandbox::rt::FrameRecord {
        n: 1,
        dt_ms: 16,
        input: "k=-;p=-;r=-;m=0,0,0".to_owned(),
        state: "0".repeat(64),
        hash: "1".repeat(64),
        frame: None,
    };
    // R1 形状：无 frame 键（行字节与 R1 逐字节同口径）。
    let line = base.to_line().expect("to_line");
    assert!(!line.contains("frame"), "无 present 无 frame 键：{line}");
    assert_eq!(
        ygg_sandbox::rt::FrameRecord::from_line(&line).expect("from_line"),
        base
    );
    // frame 字段字典序：dt_ms < frame < hash。
    let mut with_frame = base.clone();
    with_frame.frame = Some(format!("4x3|{}", K4));
    let line = with_frame.to_line().expect("to_line");
    let (dt, fr, hash) = (
        line.find("\"dt_ms\"").expect("dt_ms"),
        line.find("\"frame\"").expect("frame"),
        line.find("\"hash\"").expect("hash"),
    );
    assert!(dt < fr && fr < hash, "frame 字段落位字典序：{line}");
    assert_eq!(
        ygg_sandbox::rt::FrameRecord::from_line(&line).expect("from_line"),
        with_frame
    );
    // frame 非字符串 = 可读拒绝（解析不静默吞）。
    let bad = line.replace(&format!("\"frame\":\"4x3|{K4}\""), "\"frame\":42");
    assert!(ygg_sandbox::rt::FrameRecord::from_line(&bad).is_err());
}

/// 五键 Lua 面 mod（8×6：黑底 + 中两行红 + 绿 blit @(3,2)）——装载期 open、
/// 运行期写、present 证据进 FrameRecord.frame；录制→回放逐帧全等（含 frame
/// 字段），且 Lua 画出的字节与 Rust 侧独立构造的期望 digest 一致。
const FRAME_LUA_MOD: &str = r#"
local h = ygg.rt.frame.open({w = 8, h = 6})
ygg.on("rt.tick", function(payload)
  ygg.rt.frame.clear(h, 0x000000FF)
  ygg.rt.frame.span(h, 0, 2, 8, 2, 0xFF0000FF)
  ygg.rt.frame.blit(h, 3, 2, 2, "\0\255\0\255\0\255\0\255")
  local p = ygg.rt.frame.present(h)
  ygg.kv.set("digest", p.digest)
  ygg.kv.set("frame_n", tostring(p.frame))
end)
"#;

#[test]
fn frame_lua_five_keys_record_replay_frame_field_equal() {
    // Lua 画出的缓冲字节 → 期望 digest（Rust 独立构造同像素）。
    let mut expect = open(8, 6);
    expect.clear(0x0000_00FF).expect("clear");
    expect.span(0, 2, 8, 2, RED).expect("span");
    expect
        .blit(3, 2, 2, &[0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF])
        .expect("blit");
    let expect_key = format!(
        "8x6|{}",
        frame_digest(8, 6, &expect.snapshot().expect("snapshot").pixels)
    );

    let records = {
        let mut kernel = KernelFacade::new(FRAME_LUA_MOD).expect("kernel");
        let mut host = RtHost::new(RtConfig::default());
        let mut sink = Vec::new();
        let done = record_frames(&mut kernel, &mut host, 4, &mut sink).expect("record 4");
        assert_eq!(done, 4);
        sink
    };
    // 每帧都有渲染证据，且与 Rust 期望逐字节一致；present 回执帧号进 KV。
    for (index, record) in records.iter().enumerate() {
        assert_eq!(
            record.frame.as_deref(),
            Some(expect_key.as_str()),
            "第 {} 帧渲染证据 = Lua 画面的 Rust 期望 digest",
            index + 1
        );
    }
    // 录制文件字节面：frame 字段逐帧在场（JSONL 往返保真）。
    let lines: Vec<String> = records
        .iter()
        .map(|r| r.to_line().expect("to_line"))
        .collect();
    assert!(lines.iter().all(|l| l.contains("\"frame\":\"8x6|")));

    // 回放：新内核同配置，链哈希 ∧ frame 字段逐帧全等。
    let mut kernel = KernelFacade::new(FRAME_LUA_MOD).expect("kernel");
    let mut host = RtHost::new(RtConfig::default());
    let result = replay_records(&mut kernel, &mut host, &records);
    assert!(result.ok(), "回放应全等：{result:?}");
    assert_eq!(result.frames_checked, 4);
}

/// 运行期禁令与形状错误经信封可读浮出：seal 后 open = `YGG-RT-SEALED`；
/// 未 open 写键 / 负宽 / blit 不整除 = `YGG-RT-SHAPE`。
#[test]
fn frame_lua_seal_and_shape_errors_surface_readable() {
    // seal 后 open（handler 内）。
    let late_open = r#"
ygg.on("boom", function()
  ygg.rt.frame.open({w = 2, h = 2})
end)
"#;
    let mut kernel = KernelFacade::new(late_open).expect("kernel");
    let reply = kernel.dispatch("boom", "1");
    let envelope: serde_json::Value = serde_json::from_str(&reply).expect("envelope");
    assert_eq!(envelope["ok"], serde_json::Value::Bool(false));
    assert!(
        envelope["error"]["message"]
            .as_str()
            .expect("message")
            .contains("YGG-RT-SEALED"),
        "seal 后 open 应报 SEALED：{reply}"
    );

    // 形状错误三连（面已 open 的 mod）。
    let shapes = format!(
        r#"
local h = ygg.rt.frame.open({{w = 4, h = 3}})
ygg.on("boom.neg", function() ygg.rt.frame.span(h, 0, 0, -1, 1, 0xFF0000FF) end)
ygg.on("boom.blit", function() ygg.rt.frame.blit(h, 0, 0, 2, "short") end)
ygg.on("boom.handle", function() ygg.rt.frame.clear(99, 0xFF0000FF) end)
"#
    );
    let mut kernel = KernelFacade::new(&shapes).expect("kernel");
    for event in ["boom.neg", "boom.blit", "boom.handle"] {
        let reply = kernel.dispatch(event, "1");
        let envelope: serde_json::Value = serde_json::from_str(&reply).expect("envelope");
        assert_eq!(envelope["ok"], serde_json::Value::Bool(false), "{event}");
        assert!(
            envelope["error"]["message"]
                .as_str()
                .expect("message")
                .contains("YGG-RT-SHAPE"),
            "{event} 应报 SHAPE：{reply}"
        );
    }

    // 未 open 的 mod 上写键。
    let never_open = r#"
ygg.on("boom", function() ygg.rt.frame.span(1, 0, 0, 1, 1, 0xFF0000FF) end)
"#;
    let mut kernel = KernelFacade::new(never_open).expect("kernel");
    let reply = kernel.dispatch("boom", "1");
    assert!(
        reply.contains("YGG-RT-SHAPE") && reply.contains("frame 未 open"),
        "未 open 写键应可读报错：{reply}"
    );
}

/// digest 独立锚：同字节同 digest、尺寸入前像（异形同字节不同 digest）。
#[test]
fn frame_digest_distinguishes_shape_and_is_stable() {
    let black_2x2 = vec![0x0000_00FF; 4];
    let black_1x4 = vec![0x0000_00FF; 4];
    assert_eq!(
        frame_digest(2, 2, &black_2x2),
        frame_digest(2, 2, &black_2x2)
    );
    assert_ne!(
        frame_digest(2, 2, &black_2x2),
        frame_digest(1, 4, &black_1x4),
        "同字节异形必须不同 digest（尺寸入前像）"
    );
}

/// 吞吐实测 fixture mod（§6 预算表数据源）：320×200 全帧 cast 同形——
/// 320 列 ×（天/墙/地 3 span）+ 1 blit + present。
const FRAME_THROUGHPUT_MOD: &str = r#"
local h = ygg.rt.frame.open({w = 320, h = 200})
ygg.on("rt.tick", function()
  ygg.rt.frame.clear(h, 0x182838FF)
  for col = 0, 319 do
    local wall = 40 + (col * 37) % 90
    local top = (200 - wall) // 2
    ygg.rt.frame.span(h, col, 0, 1, top, 0x0A1428FF)
    ygg.rt.frame.span(h, col, top, 1, wall, 0xB04A28FF)
    ygg.rt.frame.span(h, col, top + wall, 1, 200 - top - wall, 0x204030FF)
  end
  ygg.rt.frame.blit(h, 100, 90, 6,
    "\222\184\135\255\222\184\135\255\222\184\135\255\222\184\135\255\222\184\135\255\222\184\135\255")
  ygg.rt.frame.present(h)
end)
"#;

const SPRITE_TAN: [u8; 24] = [
    0xDE, 0xB8, 0x87, 0xFF, 0xDE, 0xB8, 0x87, 0xFF, 0xDE, 0xB8, 0x87, 0xFF, 0xDE, 0xB8, 0x87, 0xFF,
    0xDE, 0xB8, 0x87, 0xFF, 0xDE, 0xB8, 0x87, 0xFF,
];

/// 吞吐实测（rt-face.md §6 预算表数据源；CI 只设灾难下限，不卡阈值）。
/// 双口径（计划 §5）：① host 上界 = Rust 直调缓冲 API（无 Lua）320×200
/// 全帧 span 化（320×3 段）× 60 帧 → 像素/s；② 端到端代理 = fixture mod
/// 同形全帧 `record_frames` 60 帧 → fps_dispatch 口径（CLI 真数另由
/// examples/rt 实跑同 mod 同形录取）。
#[test]
fn frame_throughput_320x200_full_frame_measured() {
    // ① host 上界（无 Lua）。
    let mut face = FrameFace::new();
    face.open(320, 200).expect("open");
    let frames = 60u64;
    let started = Instant::now();
    for f in 0..frames {
        face.clear(0x1828_38FF).expect("clear");
        for col in 0..320i64 {
            let wall = 40 + (col * 37) % 90;
            let top = (200 - wall) / 2;
            face.span(col, 0, 1, top, 0x0A14_28FF).expect("sky");
            face.span(col, top, 1, wall, 0xB04A_28FF).expect("wall");
            face.span(col, top + wall, 1, 200 - top - wall, 0x2040_30FF)
                .expect("floor");
        }
        face.blit(100, 90, 6, &SPRITE_TAN).expect("blit");
        face.present(f + 1).expect("present");
    }
    let host_elapsed = started.elapsed();
    let px_per_s = 320usize * 200 * frames as usize;
    let host_kpx = px_per_s as f64 / 1000.0 / host_elapsed.as_secs_f64();
    println!(
        "throughput host-upper-bound: 60帧 320x200 全帧span化 = {host_elapsed:.1?} → {host_kpx:.0} 千像素/s"
    );
    // 灾难下限分模式（计划 500ms 口径 = release；debug 无内联慢一个量级，
    // 实测 ≈90fps 仍达标，只防灾难不卡阈值）。
    let floor = if cfg!(debug_assertions) { 2000 } else { 500 };
    assert!(
        host_elapsed < Duration::from_millis(floor),
        "host 上界灾难下限：60 帧 {host_elapsed:.1?} ≥ {floor}ms"
    );

    // ② 端到端代理（fixture mod，同形 320×200 全帧 + present 证据）。
    let mut kernel = KernelFacade::new(FRAME_THROUGHPUT_MOD).expect("kernel");
    let mut host = RtHost::new(RtConfig::default());
    let mut sink = Vec::new();
    let started = Instant::now();
    let done = record_frames(&mut kernel, &mut host, 60, &mut sink).expect("record 60");
    let e2e_elapsed = started.elapsed();
    let fps = done as f64 / e2e_elapsed.as_secs_f64();
    println!("throughput e2e-proxy: 60帧 fps_dispatch ≈ {fps:.1}");
    assert_eq!(done, 60);
    assert!(
        e2e_elapsed < Duration::from_millis(if cfg!(debug_assertions) { 2000 } else { 500 }),
        "e2e 灾难下限：60 帧 {e2e_elapsed:.1?}"
    );
    assert!(
        sink.iter().all(|r| r.frame.is_some()),
        "每帧都有 present 证据（全帧渲染路径真实发生）"
    );
}
