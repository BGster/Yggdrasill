//! 回放篡改必须**定位到具体 tick**（ADR-S3-6）。
//!
//! `replay_result.first_bad_tick` 是宿主唯一的定位依据：有它就是「第 N 帧起不对劲，
//! 从那里重下发」，没有它 `ok:false` 只能让宿主从第 0 帧开始瞎猜。所以这里不只断言
//! `ok:false`，每条用例都钉死帧号。
//!
//! ## 录制开关在哪
//!
//! 内核录制器是 `Sim::record(on)`（`crates/ygg-sim/src/sim.rs:429`），默认**关**：
//! `recorder: Option<ReplayLog>` 的初值是 `None`（同文件 106 / 129 行），只有需要落
//! `ReplayLog` 的调用方才显式打开（`crates/ygg-sim/tests/replay_tick_hash.rs` 等）。
//! `ygg-sandbox::Loaded` 与 `Session` 都没有转开它的入口（`Loaded::sim()` 只给只读
//! 引用），宿主侧拿不到内核录下来的帧。
//!
//! 于是这条用例自己攒 `Log`：ops 走 `submit`（批量帧在调用时立即生效），`hash` 取自
//! 随后 `step{ticks:1}` 的 `events.hash`，`Frame.tick` 用**步进前**的时钟——内核录制器
//! `push_tick` 收的就是步进前的 tick（首帧是 0 不是 1），与
//! `tests/session.rs::replay_detects_a_tampered_frame` 同一口径。宿主用「快照 + 自己的
//! 输入记录」能重建的正是这份等价日志。

use ygg_host::{
    Changed, ComponentValues, Frame, Log, LogHeader, Message, Op, Session, Value, CODEC, PROTOCOL,
};
use ygg_sandbox::modload::{Loaded, ModBundle};

/// 组件 id：包里第一个注册的组件是 `hp`。
const HP: u32 = 0;

/// 一个系统 + 一个整数组件 + 封盘（与 `tests/session.rs` 同一份最小包）。
const PACK: &str = r#"
world.component{ name = "hp", kind = "i64" }
world.system{
  name = "damage",
  stage = "rules",
  reads = { "hp" },
  writes = { "hp" },
  run = function(dt, ctx)
    ctx.each_mut({ "hp" }, function(id, hp) return hp - 1 end)
  end,
}
world.seal()
"#;

/// 日志总帧数（第 40 帧是篡改点，前后各有余量，避免「最后一帧」掩盖定位错误）。
const FRAMES: usize = 61;

/// 篡改点：第 40 帧（tick 40）。
const TAMPER_AT: usize = 40;

fn loaded() -> Loaded {
    Loaded::from_bundle(&ModBundle::in_memory(PACK)).expect("内存包应装载成功")
}

fn hello(pack_hash: &str) -> Message {
    Message::Hello {
        protocol: PROTOCOL.to_string(),
        codec: CODEC.to_string(),
        pack_id: "replay-tamper-test".to_string(),
        pack_hash: pack_hash.to_string(),
        seed: None,
    }
}

/// 已握手会话。
fn session() -> Session {
    let loaded = loaded();
    let pack_hash = loaded.manifest_hash().to_string();
    let mut session = Session::new(loaded);
    let reply = session.handle(hello(&pack_hash));
    assert!(
        matches!(reply, Message::Ready { .. }),
        "握手应得 ready，得到 {reply:?}"
    );
    session
}

fn hp(values: &[i64]) -> ComponentValues {
    ComponentValues {
        cid: HP,
        values: values.iter().map(|v| Value::Int(*v)).collect(),
    }
}

fn spawn(count: u32, values: &[i64]) -> Op {
    Op::Spawn {
        n: count,
        comps: vec![hp(values)],
    }
}

fn events_hash(frames: &[Message]) -> u64 {
    for frame in frames {
        if let Message::Events { hash, .. } = frame {
            return *hash;
        }
    }
    panic!("step 必须答 events 帧，得到 {frames:?}");
}

fn delta(frames: &[Message]) -> &Vec<Changed> {
    for frame in frames {
        if let Message::Delta { changed, .. } = frame {
            return changed;
        }
    }
    panic!("step 必须答 delta 帧，得到 {frames:?}");
}

fn kernel_seed(session: &Session) -> u64 {
    session
        .loaded()
        .sim()
        .as_ref()
        .map(|sim| sim.seed())
        .expect("内核已挂载")
}

/// 逐帧录一份日志：第 0 帧 `spawn`，之后每帧一条恒值的 `set`（hp = 5），再步进一 tick。
///
/// 恒值是为了让「篡改写入值」只需换一个常量：hp 每帧被 set 成 5、被系统减成 4，帧与帧
/// 之间不累积，于是第 40 帧被改成 50 时后面的帧只是「算不出来」，不是「算成别的样子」。
fn recorded_log() -> (Session, Log) {
    let mut session = session();
    let seed = kernel_seed(&session);
    let mut frames = Vec::with_capacity(FRAMES);
    // 实体 id 只有 `delta` 回执里才有（`spawn` 的回执不带 id）。
    let mut id = None;
    for index in 0..FRAMES {
        let ops = if index == 0 {
            vec![spawn(1, &[10])]
        } else {
            vec![Op::Set {
                id: id.expect("第 0 帧的 spawn 必须先落地"),
                comps: vec![hp(&[5])],
            }]
        };
        session.handle(Message::Submit { ops: ops.clone() });
        let reply = session.handle_all(Message::Step { ticks: 1, dt: 1 });
        if index == 0 {
            id = Some(delta(&reply).first().expect("spawn 必须留下增量").eid);
        }
        frames.push(Frame {
            tick: index as u64,
            dt: 1,
            ops,
            hash: events_hash(&reply),
        });
    }
    let log = Log {
        header: LogHeader {
            seed,
            start_tick: 0,
        },
        frames,
    };
    (session, log)
}

fn replay(session: &mut Session, log: Log) -> (bool, Option<u64>) {
    match session.handle(Message::Replay { log }) {
        Message::ReplayResult { ok, first_bad_tick } => (ok, first_bad_tick),
        other => panic!("replay 应得 replay_result，得到 {other:?}"),
    }
}

#[test]
fn untampered_log_replays_clean() {
    // 先证日志本身是干净的：后面三条「定位到第 N 帧」才有意义。
    let (mut live, log) = recorded_log();
    assert_eq!(log.frames.len(), FRAMES);
    assert_eq!(replay(&mut live, log), (true, None));
}

#[test]
fn tampered_write_value_is_located_at_its_frame() {
    let (mut live, mut log) = recorded_log();
    // 只改写入值（hp 5 → 50）：帧结构、id、帧数都不动，检出的唯一依据只能是复算 hash。
    let ops = &mut log.frames[TAMPER_AT].ops;
    match ops.first_mut() {
        Some(Op::Set { comps, .. }) => *comps = vec![hp(&[50])],
        other => panic!("第 {TAMPER_AT} 帧应当是一条 set，得到 {other:?}"),
    }
    assert_eq!(
        replay(&mut live, log),
        (false, Some(TAMPER_AT as u64)),
        "写入值被改的那一帧就是第一处复算不上的帧"
    );
}

#[test]
fn tampered_hash_is_located_at_its_frame() {
    let (mut live, mut log) = recorded_log();
    // 故意换一帧（末帧，不是上面那条的 40）：两条用例报出不同的帧号，才能排除
    // 「`first_bad_tick` 恒等于某个常量」这种恒真断言。
    let at = FRAMES - 1;
    log.frames[at].hash = log.frames[at].hash.wrapping_add(1);
    assert_eq!(
        replay(&mut live, log),
        (false, Some(at as u64)),
        "hash 被改的那一帧就是第一处不符的帧"
    );
}

#[test]
fn truncated_log_is_a_valid_prefix() {
    let (mut live, mut log) = recorded_log();
    log.frames.truncate(TAMPER_AT);
    // 实际语义：`Log` 是「从 start_tick 起的**前缀**」，协议里没有总帧数/终点 tick
    // 字段（`proto::LogHeader{seed,start_tick}` + 若干 `Frame`），`replay` 只按给定的
    // 帧逐条复算，不复算到某个指定终点——所以截断到 20 帧是**通过**的，宿主拿到的
    // 结论是「这 20 tick 的提交是干净的」，不是「日志缺了后半段」。真要有「完整到哪」
    // 的语义，得由宿主记住自己提交了多少 tick，协议层不管。
    assert_eq!(replay(&mut live, log), (true, None));
}
