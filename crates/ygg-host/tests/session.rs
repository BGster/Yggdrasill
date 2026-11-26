//! `ygg/1` 会话状态机的行为用例：M3.2 的六条不变式一案一条。
//!
//! 这里不用真文件系统：`ModBundle::in_memory` 提供同一套装载参数（manifest 走
//! `MINIMAL_MANIFEST`），于是 `hello.pack_hash` 是**算出来**的声明指纹，不是常量。

use ygg_host::{
    Changed, ComponentValues, ErrorCode, Frame, Log, LogHeader, Message, Op, Session, Snapshot,
    State, Value, CODEC, PROTOCOL,
};
use ygg_sandbox::modload::{Loaded, ModBundle};

/// 组件 id：包里第一个注册的组件是 `hp`（`session()` 里断言）。
const HP: u32 = 0;

/// 一个系统 + 一个整数组件 + 封盘：`seal` 之后世界才允许推进时间。
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

fn loaded() -> Loaded {
    Loaded::from_bundle(&ModBundle::in_memory(PACK)).expect("内存包应装载成功")
}

/// 已握手会话（顺带断言 `ready` 的静态面：sealed / 组件表 / 系统表）。
fn session() -> Session {
    let loaded = loaded();
    let pack_hash = loaded.manifest_hash().to_string();
    let mut session = Session::new(loaded);
    assert_eq!(session.state(), State::Fresh);
    let reply = session.handle(hello(&pack_hash));
    let Message::Ready {
        sealed,
        components,
        systems,
        tick,
        hash,
    } = reply
    else {
        panic!("握手应得 ready，得到 {reply:?}");
    };
    assert!(sealed, "脚本调用过 world.seal()");
    assert_eq!(
        (tick, hash),
        (0, session.loaded().hash().expect("内核已挂载"))
    );
    assert_eq!(components.len(), 1);
    assert_eq!(components[0].id, HP);
    assert_eq!(components[0].name, "hp");
    assert_eq!(components[0].kind, ygg_host::ComponentKind::Number);
    assert_eq!(systems.len(), 1);
    assert_eq!(systems[0].name, "damage");
    assert_eq!(systems[0].stage, ygg_host::Stage::Rules);
    assert_eq!(session.state(), State::Ready);
    session
}

fn hello(pack_hash: &str) -> Message {
    Message::Hello {
        protocol: PROTOCOL.to_string(),
        codec: CODEC.to_string(),
        pack_id: "session-test".to_string(),
        pack_hash: pack_hash.to_string(),
    }
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

/// 发一条 `snapshot`，取回 (tick, hash, bytes)。
fn snapshot(session: &mut Session) -> (u64, u64, String) {
    let reply = session.handle(Message::Snapshot(Snapshot::Request {}));
    let Message::Snapshot(Snapshot::Reply { tick, hash, bytes }) = reply else {
        panic!("snapshot 应得 reply，得到 {reply:?}");
    };
    (tick, hash, bytes)
}

/// 从一帧 `events` 里取步进后的哈希。
fn events_hash(frames: &[Message]) -> u64 {
    for frame in frames {
        if let Message::Events { hash, .. } = frame {
            return *hash;
        }
    }
    panic!("step 必须答 events 帧，得到 {frames:?}");
}

/// 从一帧 `delta` 里取增量。
fn delta(frames: &[Message]) -> &Vec<Changed> {
    for frame in frames {
        if let Message::Delta { changed, .. } = frame {
            return changed;
        }
    }
    panic!("step 必须答 delta 帧，得到 {frames:?}");
}

#[test]
fn unhandshaken_call_is_rejected() {
    let mut session = Session::new(loaded());
    let reply = session.handle(Message::Step { ticks: 1, dt: 1 });
    match reply {
        Message::Error(err) => {
            assert_eq!(err.code, ErrorCode::EProtocol, "{err:?}");
            assert!(err.msg.contains("hello"), "文案要指向握手：{err:?}");
        }
        other => panic!("未握手应被拒，得到 {other:?}"),
    }
    assert_eq!(session.state(), State::Fresh, "拒绝不改变状态");
}

#[test]
fn wrong_pack_hash_is_rejected_and_state_stays_fresh() {
    let loaded = loaded();
    let real = loaded.manifest_hash().to_string();
    let mut session = Session::new(loaded);
    let wrong = "0".repeat(real.len());
    match session.handle(hello(&wrong)) {
        Message::Error(err) => assert_eq!(err.code, ErrorCode::EPack, "{err:?}"),
        other => panic!("指纹不符应得 E_PACK，得到 {other:?}"),
    }
    assert_eq!(session.state(), State::Fresh, "指纹不符不使会话可用");
    assert!(
        matches!(session.handle(hello(&real)), Message::Ready { .. }),
        "改正后应能重试握手"
    );
}

#[test]
fn submit_with_one_bad_op_writes_nothing() {
    let mut session = session();
    let (_, before, _) = snapshot(&mut session);
    let ops = vec![
        // 合法的一条：新建一个实体。
        spawn(1, &[5]),
        // 非法的一条：没有这个实体（整批必须被拒，前一条也不许落笔）。
        Op::Set {
            id: 9999,
            comps: vec![hp(&[1])],
        },
    ];
    match session.handle(Message::Submit { ops }) {
        Message::Error(err) => assert_eq!(err.code, ErrorCode::ERule, "{err:?}"),
        other => panic!("整批含非法 op 应得 error，得到 {other:?}"),
    }
    let (_, after, _) = snapshot(&mut session);
    assert_eq!(before, after, "整批被拒：世界哈希不变");
    assert!(session.loaded().changed().is_empty(), "被拒的批次不留写点");
    assert_eq!(
        session
            .loaded()
            .sim()
            .as_ref()
            .map(|sim| sim.world().entity_count()),
        Some(0)
    );
}

#[test]
fn step_returns_a_delta_with_exactly_the_written_cells() {
    let mut session = session();
    // spawn 只改结构，不写列：回执的 delta 应当为空。
    let reply = session.handle(Message::Submit {
        ops: vec![spawn(2, &[5, 5])],
    });
    match reply {
        Message::Delta { changed, tick } => {
            assert_eq!(tick, 0);
            assert!(changed.is_empty(), "结构操作不产生单元格：{changed:?}");
        }
        other => panic!("submit 应回 delta，得到 {other:?}"),
    }
    let frames = session.handle_all(Message::Step { ticks: 1, dt: 1 });
    assert_eq!(frames.len(), 2, "step 答两帧：events + delta");
    let changed = delta(&frames);
    assert_eq!(changed.len(), 2, "两个实体各写一列：{changed:?}");
    for (index, cell) in changed.iter().enumerate() {
        assert_eq!(cell.comps.len(), 1, "每实体只动 hp：{cell:?}");
        assert_eq!(cell.comps[0].cid, HP);
        // 系统把 5 减到 4：值取自**步进之后**的世界。
        assert_eq!(cell.comps[0].values, vec![Value::Int(4)]);
        if index > 0 {
            assert!(changed[index - 1].eid < cell.eid, "按 (eid, cid) 升序");
        }
    }
}

#[test]
fn snapshot_then_open_continues_from_the_same_tick_and_hash() {
    let mut session = session();
    session.handle(Message::Submit {
        ops: vec![spawn(1, &[9])],
    });
    session.handle_all(Message::Step { ticks: 2, dt: 1 });
    let (tick, hash, bytes) = snapshot(&mut session);
    assert_eq!(tick, 2);

    // 新连接：同一份装载参数 + 同一份快照 → 同一时钟、同一哈希。
    let mut resumed = session_from(&bytes);
    let reply = resumed.handle(Message::Open { bytes });
    match reply {
        Message::Ready {
            tick: got_tick,
            hash: got_hash,
            ..
        } => assert_eq!((got_tick, got_hash), (tick, hash)),
        other => panic!("open 应得 ready，得到 {other:?}"),
    }
}

#[test]
fn replay_detects_a_tampered_frame() {
    let mut live = session();
    // tick 1：先 spawn（写面），再推进——replay 里把 spawn 挂在第 1 帧上。
    let ops = vec![spawn(1, &[5])];
    live.handle(Message::Submit { ops: ops.clone() });
    let first = events_hash(&live.handle_all(Message::Step { ticks: 1, dt: 1 }));
    let second = events_hash(&live.handle_all(Message::Step { ticks: 1, dt: 1 }));
    let seed = live
        .loaded()
        .sim()
        .as_ref()
        .map(|sim| sim.seed())
        .expect("内核已挂载");
    // `Frame.tick` 是**该帧开跑前**的内核时钟（内核录制器的口径：`push_tick` 收的是
    // 步进前的 tick），所以第一帧是 0 而不是 1；`hash` 才是该帧跑完后的摘要。
    let log = Log {
        header: LogHeader {
            seed,
            start_tick: 0,
        },
        frames: vec![
            Frame {
                tick: 0,
                dt: 1,
                ops,
                hash: first,
            },
            Frame {
                tick: 1,
                dt: 1,
                ops: Vec::new(),
                hash: second,
            },
        ],
    };
    match live.handle(Message::Replay { log: log.clone() }) {
        Message::ReplayResult { ok, first_bad_tick } => {
            assert!(ok, "未篡改的日志必须复算通过（bad={first_bad_tick:?}）");
            assert_eq!(first_bad_tick, None);
        }
        other => panic!("replay 应得 replay_result，得到 {other:?}"),
    }

    let mut tampered = log;
    tampered.frames[1].hash = tampered.frames[1].hash.wrapping_add(1);
    match live.handle(Message::Replay { log: tampered }) {
        Message::ReplayResult { ok, first_bad_tick } => {
            assert!(!ok, "篡改过的帧必须被检出");
            assert_eq!(first_bad_tick, Some(1), "报第一处不符的帧 tick");
        }
        other => panic!("replay 应得 replay_result，得到 {other:?}"),
    }
}

/// 未握手的会话（快照字节要在一个**新连接**上打开）。
fn session_from(_bytes: &str) -> Session {
    let loaded = loaded();
    let pack_hash = loaded.manifest_hash().to_string();
    let mut session = Session::new(loaded);
    assert!(matches!(
        session.handle(hello(&pack_hash)),
        Message::Ready { .. }
    ));
    session
}
