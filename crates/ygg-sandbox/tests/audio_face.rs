//! `audio-*` 族（S31 R3）：音频面 core + Lua 三键——窗口序列钉（I1 镜像）/
//! 饱和与烘焙 KAT / 22050 样本整局钉值 / 形状闭集与 SEALED / voice 上限与
//! 释放 / stop 帧内生效 / audio 键加性与缺席 / 回放严格对表三负例 /
//! R1/R2 行字节不回退 / 端到端 fire→WAV digest 钉值 / 混音成本实测。
//!
//! KAT 钉值以 **python3 hashlib 预算法**独立复算钉死（规格：
//! `SHA256("ygg.rt.audio.v1|22050x1|" ++ 本帧窗口 s16 小端字节)`；
//! 整局流与 WAV 全文件 digest 同法预算）。

use sha2::{Digest, Sha256};
use ygg_sandbox::audio::{
    audio_digest, wav_bytes, window_samples, AudioError, AudioFace, AUDIO_MAX_VOICES,
};
use ygg_sandbox::driver::KernelFacade;
use ygg_sandbox::rt::{record_frames, replay_records, FrameRecord, InputEvent, RtConfig, RtHost};

/// A1：帧 1 窗口（352 样本）含 voice [600,−600,1200,−1200]（gain 1000）。
const A1: &str = "03b13be12c883014727dbb5a3dcf8e0334206007af65132842f6cfb3989b9ebd";
/// A2：饱和钉——[32767]+[1] = 32767、[−32768]+[−1] = −32768（帧 1 窗口）。
const A2: &str = "2c7290cc3432bd0a7041a0299cb30239ecd227de0f3738e1c42b7fd81fa017af";
/// A3：烘焙钉——[1000]×1.5 = 1500、[20000]×2.0 = 40000 → 钳 32767（帧 1 窗口）。
const A3: &str = "7e8ff0e20be161470004d3c76d2b273e7836769bb127b1aa07eaaea4658d263d";
/// A4：22050 样本整局流钉（v(i)=i%7−3，60 帧 @60Hz×22050 = 恰 1 秒）。
const A4_SESSION: &str = "605c7efed87a70b618de55e04f907eee0c47908083e3ecf64767a9f98adee76f";
/// A5：A4 的 WAV 全文件钉（44 字节头 + 22050 样本）。
const A5_WAV: &str = "89536fd68b24fcb57e9bec6bdfbbcd20f28fa02a87348f00a7a03279f2692318";
/// A6：端到端 4 帧（fire@帧 2）整局流钉（1455 样本 = 352+375+375+353）。
const A6_SESSION: &str = "2da8c09480960bf9cf18fc32c7a08c9b209ada49dcb382be2886232542e72a9b";
/// A7：A6 的 WAV 全文件钉。
const A7_WAV: &str = "4072717722d3011662ce68709df1ffbe09ed9557135885aaf400b0a9b31b0424";
/// 端到端帧 2 窗口 digest（375 样本，fire 当帧混入 = play→混入 0 帧语义）。
const A8_FRAME2: &str = "d3284384387ba8ebe93b8114ded148da88bff4ba7ca07e813413b0eb16c0ea23";

fn s16le(samples: &[i16]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.to_le_bytes()).collect()
}

fn sha(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// 端到端 Lua mod：帧面 + 音频面装载期 open；rt.tick 内 pressed[32] → play。
/// SHOT = s16le [600,−600,1200,−1200]（Saltline sfx_rifle_shot amp=900 域内
/// 的整数钉串，波形参数只读引用）。play 返回 voice 记 KV（0 帧延迟证据）。
const RT_AUDIO_MOD: &str = r#"
local h = ygg.rt.frame.open({w = 8, h = 4})
local a = ygg.rt.audio.open({rate = 22050})
local SHOT = "\x58\x02\xa8\xfd\xb0\x04\x50\xfb"
ygg.on("rt.tick", function(payload)
  local n = tonumber(payload:match("^n=(%d+);"))
  local keys = ygg.rt.input.keys()
  ygg.rt.frame.clear(h, 0x000000FF)
  ygg.rt.frame.present(h)
  for _, k in ipairs(keys.pressed) do
    if k == 32 then
      local v = ygg.rt.audio.play(a, SHOT)
      ygg.kv.set("shot", tostring(n) .. ":" .. tostring(v))
    end
  end
end)
"#;

/// R1 形状 mod（无渲染/音频面）：行字节不回退的对照组。
const RT_PLAIN_MOD: &str = r#"
ygg.on("rt.tick", function()
  ygg.kv.set("beat", tostring((tonumber(ygg.kv.get("beat") or "0") + 1)))
end)
"#;

/// 4 帧录制（fire@帧 2：帧 2 down 键 32、帧 3 up）。
fn record_audio_e2e() -> Vec<FrameRecord> {
    let mut kernel = KernelFacade::new(RT_AUDIO_MOD).expect("kernel");
    let mut host = RtHost::new(RtConfig::default());
    host.input_at(
        2,
        InputEvent {
            down: vec![32],
            ..Default::default()
        },
    )
    .expect("schedule down");
    host.input_at(
        3,
        InputEvent {
            up: vec![32],
            ..Default::default()
        },
    )
    .expect("schedule up");
    let mut sink = Vec::new();
    let done = record_frames(&mut kernel, &mut host, 4, &mut sink).expect("record 4");
    assert_eq!(done, 4);
    sink
}

/// 窗口序列 = I1 镜像（纯函数钉）+ 60 帧积分恰 22050 样本 + A1 digest 钉。
#[test]
fn audio_window_sequence_is_i1_mirror_and_pins_22050_per_second() {
    // 前 10 帧窗口序列（60Hz×22050：352/375/375/353 循环，floor 差分）。
    let seq: Vec<i64> = (1..=10)
        .map(|n| window_samples(((n - 1) * 1000) / 60, (n * 1000) / 60, 22050))
        .collect();
    assert_eq!(
        seq,
        vec![352, 375, 375, 353, 375, 375, 352, 375, 375, 353],
        "窗口序列 = ⌊logical·rate/1000⌋ 差分"
    );
    // 1000ms 积分恰 22050（telescoping：⌊1000·22050/1000⌋ − ⌊0⌋）。
    assert_eq!(
        window_samples(0, 1000, 22050),
        22050,
        "每 1000ms 恰 22050 样本"
    );
    assert_eq!(window_samples(0, 1000, 8000), 8000, "rate=8000 下界同构");
    assert_eq!(window_samples(0, 1000, 48000), 48000, "rate=48000 上界同构");

    // AudioFace 60 帧整局流长度 = 22050 字节×2；单 voice 4 样本仅帧 1 活动
    // （帧 2..60 无 voice = 键缺席，静音照入整局流）。
    let mut face = AudioFace::new();
    assert_eq!(
        face.open(None).expect("open"),
        1,
        "缺省 rate=22050、handle 1 起"
    );
    assert_eq!(face.rate(), Some(22050));
    let pcm = s16le(&[600, -600, 1200, -1200]);
    assert_eq!(face.play(&pcm, 1000).expect("play"), 1);
    // 帧 1..60 逐帧混音（逻辑钟按定步公式累计）。
    let mut logical = 0i64;
    let mut frame1_digest = String::new();
    for n in 1..=60u64 {
        logical += ((n * 1000) / 60) as i64 - (((n - 1) * 1000) / 60) as i64;
        let mix = face.mix_frame(logical);
        if n == 1 {
            let mix = mix.expect("帧 1 有 voice 必有 digest");
            frame1_digest = mix.digest;
            assert_eq!(mix.rate, 22050);
        } else {
            assert!(mix.is_none(), "帧 {n} 无 voice = 键缺席");
        }
    }
    assert_eq!(frame1_digest, A1, "A1：帧 1 窗口 digest 钉值");
    assert_eq!(face.session().len(), 22050 * 2, "60 帧 = 恰 1 秒（零漂移）");
    assert_eq!(face.active_voices(), 0, "voice 混完即出队");
    // 规范序独立复算（同 SHA256 规格另一入口，防止 digest 拼串漂移）。
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"ygg.rt.audio.v1|22050x1|");
    bytes.extend_from_slice(&s16le(&[600, -600, 1200, -1200]));
    bytes.extend_from_slice(&vec![0u8; 348 * 2]);
    assert_eq!(sha(&bytes), A1);
    assert_eq!(audio_digest(22050, &bytes[A24..]), A1, "audio_digest 同钉");
}

/// A24 = "ygg.rt.audio.v1|22050x1|" 的字节长（前像长度钉，防拼串漂移）。
const A24: usize = 24;

/// A2/A3 钉值：饱和整数加法（assetgen sound.mix 先例：voice = 同帧同起点
/// 的时间对齐层）+ gain 入队烘焙。
#[test]
fn audio_kat_pins_saturation_and_gain_bake() {
    // A2：四个同帧 voice，样本槽内补零把两条边界分到 slot 0/1——
    // slot0 = 32767+1 饱和 32767（上界）、slot1 = −32768−1 饱和 −32768（下界）。
    let mut face = AudioFace::new();
    face.open(Some(22050)).expect("open");
    face.play(&s16le(&[32767, 0]), 1000).expect("v1");
    face.play(&s16le(&[1, 0]), 1000).expect("v2");
    face.play(&s16le(&[0, -32768]), 1000).expect("v3");
    face.play(&s16le(&[0, -1]), 1000).expect("v4");
    let mix = face.mix_frame(16).expect("digest");
    assert_eq!(mix.digest, A2, "A2：饱和边界 KAT");
    // A3：gain 烘焙（入队时）——[1000]×1500‰ = 1500；[20000]×2000‰ 烘焙钳 32767。
    // 补零分槽（voice 同帧同起点）：slot0 = 1500、slot1 = 40000 烘焙即钳 32767。
    let mut face = AudioFace::new();
    face.open(Some(22050)).expect("open");
    face.play(&s16le(&[1000, 0]), 1500).expect("v1");
    face.play(&s16le(&[0, 20000]), 2000).expect("v2");
    let mix = face.mix_frame(16).expect("digest");
    assert_eq!(mix.digest, A3, "A3：烘焙 KAT（40000 → 钳 32767）");
    assert!(
        face.session()[..4].contains(&0xDC),
        "s16le 0x05DC=1500 小端含 DC"
    );
}

/// 22050 样本整局流钉（A4）+ WAV 头与全文件钉（A5）——字节级恒等。
#[test]
fn audio_session_and_wav_pins_22050_samples() {
    let mut face = AudioFace::new();
    face.open(Some(22050)).expect("open");
    let pcm: Vec<u8> = s16le(&(0..22050).map(|i| ((i % 7) - 3) as i16).collect::<Vec<_>>());
    face.play(&pcm, 1000).expect("play 22050 样本");
    let mut logical = 0i64;
    for n in 1..=60u64 {
        logical += ((n * 1000) / 60) as i64 - (((n - 1) * 1000) / 60) as i64;
        face.mix_frame(logical).expect("digest");
    }
    assert_eq!(face.session().len(), 44100, "22050 样本 × 2 字节");
    assert_eq!(sha(face.session()), A4_SESSION, "A4：整局流钉值");
    let wav = wav_bytes(22050, face.session());
    // WAV 头 44 字节字段逐项钉（assetgen wav16 恒等先例：RIFF/pcm ch=1）。
    assert_eq!(&wav[..4], b"RIFF");
    assert_eq!(&wav[8..12], b"WAVE");
    assert_eq!(&wav[12..16], b"fmt ");
    assert_eq!(u32::from_le_bytes(wav[16..20].try_into().unwrap()), 16);
    assert_eq!(
        u16::from_le_bytes(wav[20..22].try_into().unwrap()),
        1,
        "PCM"
    );
    assert_eq!(
        u16::from_le_bytes(wav[22..24].try_into().unwrap()),
        1,
        "ch=1"
    );
    assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 22050);
    assert_eq!(
        u32::from_le_bytes(wav[28..32].try_into().unwrap()),
        44100,
        "byteRate"
    );
    assert_eq!(
        u16::from_le_bytes(wav[32..34].try_into().unwrap()),
        2,
        "blockAlign"
    );
    assert_eq!(
        u16::from_le_bytes(wav[34..36].try_into().unwrap()),
        16,
        "bits"
    );
    assert_eq!(&wav[36..40], b"data");
    assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 44100);
    assert_eq!(wav.len(), 44 + 44100);
    assert_eq!(sha(&wav), A5_WAV, "A5：WAV 全文件钉值");
}

/// open 形状闭集 + 单混音面 + Lua 面 SEALED 装载期限定（四条可读拒绝）。
#[test]
fn audio_open_is_closed_shape_single_face_and_seal_time_only() {
    // 缺省/边界：None → 22050；8000/48000 合法；7999/48001 拒绝。
    let mut face = AudioFace::new();
    face.open(None).expect("default");
    let mut face2 = AudioFace::new();
    face2.open(Some(8000)).expect("8000");
    assert_eq!(face2.rate(), Some(8000));
    face2.open(Some(48000)).expect_err("再开 = Opened");
    assert!(matches!(face2.open(Some(48000)), Err(AudioError::Opened)));
    let mut face3 = AudioFace::new();
    assert!(matches!(face3.open(Some(7999)), Err(AudioError::Shape(_))));
    assert!(matches!(face3.open(Some(48001)), Err(AudioError::Shape(_))));
    // 未 open 就 play = 可读拒绝（handle 校验同族）。
    assert!(matches!(
        face3.play(&s16le(&[1]), 1000),
        Err(AudioError::Shape(_))
    ));

    // Lua 面：未知键/浮点 rate 装载期即可读拒绝（ModLoad 失败携带拒绝码）。
    let bad = match KernelFacade::new("ygg.rt.audio.open({ what = 1 })") {
        Err(err) => err.to_string(),
        Ok(_) => panic!("未知键必须拒绝"),
    };
    assert!(bad.contains("YGG-RT-SHAPE"), "{bad}");
    let bad2 = match KernelFacade::new("ygg.rt.audio.open({ rate = 22050.5 })") {
        Err(err) => err.to_string(),
        Ok(_) => panic!("浮点 rate 必须拒绝"),
    };
    assert!(bad2.contains("YGG-RT-SHAPE"), "{bad2}");
    let bad3 = match KernelFacade::new("ygg.rt.audio.open({ rate = 22050 }) ygg.rt.audio.open({})")
    {
        Err(err) => err.to_string(),
        Ok(_) => panic!("再开必须拒绝"),
    };
    assert!(bad3.contains("YGG-RT-OPENED"), "{bad3}");
    // seal 后（运行期 handler 内）禁开：YGG-RT-SEALED 可读拒绝。
    let source = r#"
ygg.on("game.tick", function()
  local ok, err = pcall(function() ygg.rt.audio.open({ rate = 8000 }) end)
  if ok then
    ygg.narrate("allowed")
  else
    ygg.narrate("sealed:" .. tostring(err))
  end
end)
"#;
    let mut kernel = KernelFacade::new(source).expect("kernel");
    let reply = kernel.dispatch("game.tick", "1");
    assert!(reply.contains("YGG-RT-SEALED"), "seal 后禁开：{reply}");
}

/// play 形状闭集：空/奇数长/超 10s/浮点 pcm、gain 越界与浮点、voice 1 起。
#[test]
fn audio_play_shape_is_closed_set() {
    let mut face = AudioFace::new();
    face.open(Some(22050)).expect("open");
    assert!(
        matches!(face.play(&[], 1000), Err(AudioError::Shape(_))),
        "空 pcm 拒绝"
    );
    assert!(
        matches!(face.play(&[0x58], 1000), Err(AudioError::Shape(_))),
        "奇数字节拒绝"
    );
    let oversize = vec![0u8; 22050 * 2 * 10 + 2];
    assert!(
        matches!(face.play(&oversize, 1000), Err(AudioError::Shape(_))),
        "超 rate×10s 拒绝（220501 样本）"
    );
    let exactly_10s = vec![0u8; 22050 * 2 * 10];
    face.play(&exactly_10s, 1000)
        .expect("恰 10s 合法（独占 voice 槽）");
    face.stop(1);
    assert!(matches!(
        face.play(&s16le(&[1]), -1),
        Err(AudioError::Shape(_))
    ));
    assert!(matches!(
        face.play(&s16le(&[1]), 2001),
        Err(AudioError::Shape(_))
    ));
    assert!(
        matches!(face.play(&s16le(&[1]), 0), Ok(_)),
        "gain=0 = 静音 voice 合法"
    );
    // voice 整数 1 起、跨 play 递增。
    assert_eq!(face.play(&s16le(&[1]), 1000).expect("v"), 3, "1,2 已用");
    // Lua 面浮点 gain / 非串 pcm 可读拒绝（经内核 handler）。
    let source = r#"
local a = ygg.rt.audio.open({ rate = 22050 })
ygg.on("rt.tick", function()
  local ok1, err1 = pcall(ygg.rt.audio.play, a, "\x58\x02", 1000.5)
  ygg.kv.set("e1", ok1 and "ok" or tostring(err1))
  local ok2, err2 = pcall(ygg.rt.audio.play, a, 42)
  ygg.kv.set("e2", ok2 and "ok" or tostring(err2))
  local ok3, err3 = pcall(ygg.rt.audio.play, a, "\x58\x02\xa8\xfd")
  ygg.kv.set("v", ok3 and "played" or tostring(err3))
end)
"#;
    let mut kernel = KernelFacade::new(source).expect("kernel");
    let mut host = RtHost::new(RtConfig::default());
    let mut sink = Vec::new();
    record_frames(&mut kernel, &mut host, 1, &mut sink).expect("frame");
    let save: serde_json::Value = serde_json::from_str(&kernel.dispatch("save", "")).expect("save");
    let kv = &save["save"]["kv"]["entries"];
    assert!(
        kv["e1"].as_str().unwrap_or("").contains("YGG-RT-SHAPE"),
        "{kv:?}"
    );
    assert!(
        kv["e2"].as_str().unwrap_or("").contains("YGG-RT-SHAPE"),
        "{kv:?}"
    );
    assert_eq!(kv["v"], json_str("played"), "合法 play 通过");
}

fn json_str(s: &str) -> serde_json::Value {
    serde_json::Value::String(s.to_owned())
}

/// voice 并发上限 = 64：第 65 个 CAP；stop 释放一槽可再 play；混完全释放。
#[test]
fn audio_voice_cap_is_64_and_release_reopens() {
    let mut face = AudioFace::new();
    face.open(Some(22050)).expect("open");
    let pcm = s16le(&[100; 100]);
    for v in 1..=64i64 {
        assert_eq!(face.play(&pcm, 1000).expect("play"), v);
    }
    assert!(
        matches!(face.play(&pcm, 1000), Err(AudioError::Cap(_))),
        "第 65 个 CAP"
    );
    assert_eq!(face.active_voices(), AUDIO_MAX_VOICES);
    // stop 释放一槽 → 可再 play。
    face.stop(7);
    assert_eq!(face.play(&pcm, 1000).expect("释放后可 play"), 65);
    // Lua 面：pcall 捕获第 65 个 play 的 CAP（错误进 KV，handler 不炸）。
    let source = r#"
local a = ygg.rt.audio.open({ rate = 22050 })
local SHORT = "\x01\x00"
ygg.on("rt.tick", function()
  local first_err = nil
  for i = 1, 65 do
    local ok, err = pcall(ygg.rt.audio.play, a, SHORT)
    if not ok and first_err == nil then first_err = tostring(err) end
  end
  ygg.kv.set("cap_err", first_err or "none")
end)
"#;
    let mut kernel = KernelFacade::new(source).expect("kernel");
    let mut host = RtHost::new(RtConfig::default());
    let mut sink = Vec::new();
    record_frames(&mut kernel, &mut host, 1, &mut sink).expect("frame");
    let save: serde_json::Value = serde_json::from_str(&kernel.dispatch("save", "")).expect("save");
    let cap_err = save["save"]["kv"]["entries"]["cap_err"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert!(cap_err.contains("YGG-RT-AUDIO-CAP"), "{cap_err}");
    // 混 60 帧（100 样本 < 352 窗口）→ 全出队 → 再 play 64 个全过。
    face.mix_frame(16).expect("mix");
    assert_eq!(face.active_voices(), 0, "全部混完出队");
    for v in 66..=129i64 {
        assert_eq!(
            face.play(&pcm, 1000).expect("play"),
            v,
            "voice 号不回收不复用"
        );
    }
}

/// stop 帧内生效（stop 后本帧窗口无贡献 = audio 键缺席）+ 已混完 voice
/// stop = no-op Ok + 窗口照写静音（WAV 时长不失真）。
#[test]
fn audio_stop_is_frame_scoped_and_silence_window_still_appends() {
    let mut face = AudioFace::new();
    face.open(Some(22050)).expect("open");
    let pcm = s16le(&[1000, -1000]);
    let v = face.play(&pcm, 1000).expect("play");
    face.stop(v);
    assert!(face.mix_frame(16).is_none(), "stop 当帧无 voice = 键缺席");
    assert_eq!(face.session().len(), 352 * 2, "静音窗口照写（时长不失真）");
    assert_eq!(sha(face.session()), sha(&vec![0u8; 352 * 2]), "全零字节");
    face.stop(v); // 已出队再 stop = no-op
    face.stop(999); // 从未存在的 voice = no-op
                    // 未 stop 的对照：同 play 不 stop → 当帧有 digest。
    let mut face2 = AudioFace::new();
    face2.open(Some(22050)).expect("open");
    face2.play(&pcm, 1000).expect("play");
    assert!(face2.mix_frame(16).is_some(), "未 stop 当帧混入");
}

/// R1/R2 行字节不回退：audio 缺席行 = R1/R2 逐字节同口径；audio 键字典序
/// 落位（audio < dt_ms < frame < hash）；非字符串 audio 可读拒绝；
/// R1 形状 mod 录制→回放全绿且 audio 全缺席。
#[test]
fn audio_additive_key_r1_r2_lines_byte_stable() {
    let base = FrameRecord {
        n: 1,
        dt_ms: 16,
        input: "k=-;p=-;r=-;m=0,0,0".to_owned(),
        state: "0".repeat(64),
        hash: "1".repeat(64),
        frame: None,
        audio: None,
    };
    // R1/R2 形状：无 audio 键，行字节与既有口径逐字节一致。
    let line = base.to_line().expect("to_line");
    assert!(!line.contains("audio"), "无混音无 audio 键：{line}");
    assert_eq!(
        ygg_sandbox::rt::FrameRecord::from_line(&line).expect("from_line"),
        base
    );
    // audio 键落位字典序：audio < dt_ms < frame < hash。
    let mut with_both = base.clone();
    with_both.frame = Some("4x3|ab".to_owned());
    with_both.audio = Some("cd".to_owned());
    let line = with_both.to_line().expect("to_line");
    let (a, d, f, h) = (
        line.find("\"audio\"").expect("audio"),
        line.find("\"dt_ms\"").expect("dt_ms"),
        line.find("\"frame\"").expect("frame"),
        line.find("\"hash\"").expect("hash"),
    );
    assert!(a < d && d < f && f < h, "字典序落位：{line}");
    assert_eq!(
        ygg_sandbox::rt::FrameRecord::from_line(&line).expect("from_line"),
        with_both
    );
    // audio 非字符串 = 可读拒绝。
    let bad = line.replace("\"audio\":\"cd\"", "\"audio\":42");
    assert!(ygg_sandbox::rt::FrameRecord::from_line(&bad).is_err());
    // R1 形状 mod：录制→回放全绿，audio 全缺席（引擎挂点对无音频 mod 静默）。
    let records = {
        let mut kernel = KernelFacade::new(RT_PLAIN_MOD).expect("kernel");
        let mut host = RtHost::new(RtConfig::default());
        let mut sink = Vec::new();
        record_frames(&mut kernel, &mut host, 5, &mut sink).expect("record");
        sink
    };
    assert!(
        records.iter().all(|r| r.audio.is_none()),
        "无音频 mod audio 全缺席"
    );
    let mut kernel = KernelFacade::new(RT_PLAIN_MOD).expect("kernel");
    let mut host = RtHost::new(RtConfig::default());
    let result = replay_records(&mut kernel, &mut host, &records);
    assert!(result.ok(), "R1 形状回放不回退：{result:?}");
}

/// 端到端 fire→WAV（RT_AUDIO_MOD 4 帧，fire@帧 2）：audio 键加性与缺席、
/// play→混入 0 帧（digest 落当帧记录）、整局流与 WAV 钉值、回放两次会话
/// 字节全等。
#[test]
fn audio_end_to_end_fire_wav_pins_and_absence_rule() {
    let records = record_audio_e2e();
    // 缺席规则：帧 1/3/4 无 voice = audio 键缺席；帧 2（fire 当帧）有 digest。
    assert!(
        records[0].audio.is_none(),
        "帧 1 无 voice：{:?}",
        records[0].audio
    );
    assert_eq!(
        records[1].audio.as_deref(),
        Some(A8_FRAME2),
        "帧 2 fire 当帧混入"
    );
    assert!(
        records[2].audio.is_none() && records[3].audio.is_none(),
        "3/4 帧静默"
    );
    // play 返回 voice 记 KV（同帧完成 = 0 帧延迟语义的证据面）。
    {
        let mut kernel = KernelFacade::new(RT_AUDIO_MOD).expect("kernel");
        let mut host = RtHost::new(RtConfig::default());
        host.input_at(
            2,
            InputEvent {
                down: vec![32],
                ..Default::default()
            },
        )
        .expect("schedule");
        let mut sink = Vec::new();
        record_frames(&mut kernel, &mut host, 2, &mut sink).expect("record 2");
        let save: serde_json::Value =
            serde_json::from_str(&kernel.dispatch("save", "")).expect("save");
        assert_eq!(
            save["save"]["kv"]["entries"]["shot"],
            json_str("2:1"),
            "帧 2 play → voice 1"
        );
    }
    // 录制侧整局流：1455 样本（352+375+375+353），A6 钉值；WAV A7 钉值。
    let mut kernel = KernelFacade::new(RT_AUDIO_MOD).expect("kernel");
    let mut host = RtHost::new(RtConfig::default());
    host.input_at(
        2,
        InputEvent {
            down: vec![32],
            ..Default::default()
        },
    )
    .expect("schedule");
    host.input_at(
        3,
        InputEvent {
            up: vec![32],
            ..Default::default()
        },
    )
    .expect("schedule");
    let mut sink = Vec::new();
    record_frames(&mut kernel, &mut host, 4, &mut sink).expect("record 4");
    let face = kernel.audio_handle();
    let session = face.borrow().session().to_vec();
    assert_eq!(session.len(), 1455 * 2, "4 帧窗口样本数合计");
    assert_eq!(sha(&session), A6_SESSION, "A6：端到端整局流钉值");
    let wav = wav_bytes(22050, &session);
    assert_eq!(sha(&wav), A7_WAV, "A7：端到端 WAV 钉值");
    // 回放两次：链全等 + audio 字段全等 + 会话字节两次重放全等。
    let mut first: Option<Vec<u8>> = None;
    for _ in 0..2 {
        let mut kernel = KernelFacade::new(RT_AUDIO_MOD).expect("kernel");
        let mut host = RtHost::new(RtConfig::default());
        let result = replay_records(&mut kernel, &mut host, &records);
        assert!(result.ok(), "回放全等（含 audio 字段）：{result:?}");
        assert_eq!(result.frames_checked, 4);
        let face = kernel.audio_handle();
        let replayed = face.borrow().session().to_vec();
        assert_eq!(sha(&replayed), A6_SESSION, "回放会话流 = 录制会话流");
        if let Some(prev) = &first {
            assert_eq!(&replayed, prev, "WAV 字节两次重放全等");
        }
        first = Some(replayed);
    }
}

/// 回放严格对表三负例（I9）：录有实无 / 录无实有 / 同有不同值均恰在被篡
/// 帧 mismatch；阴性对照（未篡改）全绿。
#[test]
fn audio_replay_strict_table_three_negatives() {
    let records = record_audio_e2e();
    // 阴性对照：未篡改全绿。
    let run = |records: &[FrameRecord]| {
        let mut kernel = KernelFacade::new(RT_AUDIO_MOD).expect("kernel");
        let mut host = RtHost::new(RtConfig::default());
        replay_records(&mut kernel, &mut host, records)
    };
    assert!(run(&records).ok(), "阴性对照全绿");
    // 负例 1 录有实无：静音帧 3 注入 audio 键（回放侧无 voice）→ 帧 3 红。
    let mut injected = records.clone();
    injected[2].audio = Some("f".repeat(64));
    let step = run(&injected).first_mismatch.expect("录有实无必红");
    assert_eq!(step.n, 3, "恰在被篡帧红");
    assert_eq!(step.expected, "f".repeat(64), "expected = 录制的 audio 键");
    assert_eq!(step.actual, "-", "actual = 无 voice（键缺席）");
    // 负例 2 录无实有：抹掉帧 2 的 audio 键（回放侧 fire 混音）→ 帧 2 红。
    let mut stripped = records.clone();
    stripped[1].audio = None;
    let step = run(&stripped).first_mismatch.expect("录无实有必红");
    assert_eq!(step.n, 2, "恰在被篡帧红");
    assert_eq!(step.expected, "-", "expected = 键缺席");
    assert_eq!(step.actual, A8_FRAME2, "actual = 回放侧窗口 digest");
    // 负例 3 同有不同值：帧 2 digest 篡改一字 → 帧 2 红。
    let mut tampered = records.clone();
    let mut bad = A8_FRAME2.as_bytes().to_vec();
    bad[0] = if bad[0] == b'0' { b'1' } else { b'0' };
    tampered[1].audio = Some(String::from_utf8(bad).expect("hex"));
    let step = run(&tampered).first_mismatch.expect("篡改 digest 必红");
    assert_eq!(step.n, 2);
    assert_ne!(step.expected, step.actual);
}

/// 混音成本实测（登记停阈值 0.5ms/帧）：64 voice 满载 × 600 帧，逐帧
/// µs 打印；断言灾难下界（不卡阈值——CI 机器差异不误报）。
#[test]
fn audio_mix_latency_measured_and_under_budget() {
    let mut face = AudioFace::new();
    face.open(Some(22050)).expect("open");
    let pcm = vec![0x11u8; 22050 * 2]; // 1 秒 voice × 64 并发
    let started = std::time::Instant::now();
    let mut logical = 0i64;
    let mut played = 0u64;
    for n in 1..=600u64 {
        logical += ((n * 1000) / 60) as i64 - (((n - 1) * 1000) / 60) as i64;
        // 每 60 帧（1 秒）补满 64 并发（最坏情形：全程满载）。
        if (n - 1) % 60 == 0 {
            for _ in 0..64 {
                face.play(&pcm, 1000).expect("play");
                played += 1;
            }
        }
        face.mix_frame(logical).expect("mix");
    }
    let wall = started.elapsed();
    let per_frame_us = wall.as_secs_f64() * 1e6 / 600.0;
    println!(
        "混音成本：64 voice 满载 600 帧 = {wall:?}（{per_frame_us:.1} µs/帧；阈值 500 µs/帧）"
    );
    assert_eq!(face.session().len(), 22050 * 2 * 10, "10 秒整局流");
    assert_eq!(played, 640);
    assert!(
        per_frame_us < 500.0,
        "混音成本超登记停阈值（0.5ms/帧）：{per_frame_us:.1} µs/帧"
    );
}
