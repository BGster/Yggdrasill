//! `command` 帧（M3.4b-1）：宿主传意图，内核只派发。
//!
//! 五条用例对应这一帧的五个可断言面：成功（只动该动的格）、规则拒绝（`E_RULE` + 世界零
//! 写入）、名字不存在（`E_STATE`）、握手前（`E_PROTOCOL`）、包没导出命令面（`E_STATE`）。
//!
//! 原子性用**逐字节快照**判：拒绝前后 `snapshot()` 相等才算「什么都没发生」，比比对
//! `tick` / `hash` 更强——`hash` 是摘要，`snapshot` 是字节（含 RNG 流位置与结构），
//! 摘要相等而字节不同的可能恰好就是「拒绝写了一半」。
//!
//! 用例里的 `move` / `attack` 是 `packs/example` 的词汇，**不是**协议的：内核只把它们
//! 当字符串键看，同一个 `command` 帧换成任何名字都走同一条路。

use std::path::PathBuf;

use ygg_host::proto::{ErrorCode, Message, Value, CODEC, PROTOCOL};
use ygg_host::Session;
use ygg_sandbox::modload::{Loaded, ModBundle};

/// 仓库里的示例包：`pack.json` 声明 6 个模块，入口 `src/main.lua` 返回命令面。
fn example_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packs/example")
}

/// `packs/example/data/board.json` 的部署：eid 1 在 `(0,0)`，`(1,0)` 是空格。
const FIRST_UNIT: u64 = 1;

fn command(name: &str, args: serde_json::Value) -> Message {
    Message::Command {
        name: name.to_owned(),
        args: args
            .as_object()
            .cloned()
            .expect("用例的 args 必须是 JSON 对象"),
    }
}

/// 握手：`pack_hash` 用装载后算出的真值（假值会被 `E_PACK` 挡在 `Fresh`）。
fn handshake(loaded: Loaded, pack_id: &str) -> Session {
    let pack_hash = loaded.manifest_hash().to_owned();
    let mut session = Session::new(loaded);
    let reply = session.handle(Message::Hello {
        protocol: PROTOCOL.to_owned(),
        codec: CODEC.to_owned(),
        pack_id: pack_id.to_owned(),
        pack_hash,
    });
    assert!(
        matches!(reply, Message::Ready { .. }),
        "握手应回 ready，收到 {reply:?}"
    );
    session
}

fn ready_session() -> Session {
    handshake(
        Loaded::from_dir(&example_dir()).expect("装载 packs/example"),
        "example",
    )
}

/// (a) 合法命令 → `delta`，且 `changed` 恰好点名被写的两格（`pos` 与 `moved`）。
#[test]
fn valid_command_answers_a_delta_of_exactly_the_written_cells() {
    let mut session = ready_session();
    let tick_before = session.loaded().tick().expect("tick");

    let reply = session.handle(command(
        "move",
        serde_json::json!({ "id": FIRST_UNIT, "dx": 1, "dy": 0 }),
    ));

    let Message::Delta { tick, changed } = reply else {
        panic!("合法命令应回 delta，收到 {reply:?}");
    };
    assert_eq!(tick, tick_before, "命令不推进时间");
    assert_eq!(changed.len(), 1, "只有一个实体被动过：{changed:?}");
    assert_eq!(changed[0].eid, FIRST_UNIT);

    let mut cids: Vec<u32> = changed[0].comps.iter().map(|comp| comp.cid).collect();
    cids.sort_unstable();
    assert_eq!(cids, vec![0, 3], "move 只写 pos(0) 与 moved(3)");

    let values_of = |cid: u32| {
        changed[0]
            .comps
            .iter()
            .find(|comp| comp.cid == cid)
            .map(|comp| comp.values.clone())
    };
    assert_eq!(values_of(0), Some(vec![Value::Uint(1000)]), "(0,0) → (1,0)");
    // 包把 `moved` 声明为 `u8`，它的线上值是 `int`（同 `hp` / `team` 的 Number 面）。
    assert_eq!(values_of(3), Some(vec![Value::Int(1)]), "动过即 moved=1");
}

/// (b) 规则拒绝 → `E_RULE`，且世界里一个字节都没动。
#[test]
fn refused_command_changes_neither_tick_hash_nor_snapshot() {
    let mut session = ready_session();
    let snapshot_before = session.loaded().snapshot().expect("快照");
    let tick_before = session.loaded().tick().expect("tick");
    let hash_before = session.loaded().hash().expect("hash");

    // (0,0) 向 x 负向挪一格 = 出界，包自己拒绝。
    let reply = session.handle(command(
        "move",
        serde_json::json!({ "id": FIRST_UNIT, "dx": -1, "dy": 0 }),
    ));

    let Message::Error(err) = reply else {
        panic!("越界 move 应回 error，收到 {reply:?}");
    };
    assert_eq!(err.code, ErrorCode::ERule, "包拒绝是规则面：{err}");
    assert!(err.msg.contains("越界"), "错误要带包给的原因：{err}");

    assert_eq!(session.loaded().tick().expect("tick"), tick_before);
    assert_eq!(session.loaded().hash().expect("hash"), hash_before);
    assert_eq!(
        session.loaded().snapshot().expect("快照"),
        snapshot_before,
        "拒绝必须零写入（tick / RNG 流位置 / 结构全回到调用前）"
    );
}

/// (c) 名字不在命令面上 → `E_STATE`，错误里点名那个命令。
#[test]
fn unknown_command_answers_e_state_naming_it() {
    let mut session = ready_session();

    let reply = session.handle(command("teleport", serde_json::json!({})));

    let Message::Error(err) = reply else {
        panic!("未知命令应回 error，收到 {reply:?}");
    };
    assert_eq!(err.code, ErrorCode::EState, "名字不认识是状态面：{err}");
    assert!(err.msg.contains("teleport"), "错误要点名命令：{err}");
}

/// (d) 握手之前发 `command` → `E_PROTOCOL`（状态门先于一切载荷解释）。
#[test]
fn command_before_hello_answers_e_protocol() {
    let mut session = Session::new(Loaded::from_dir(&example_dir()).expect("装载 packs/example"));

    let reply = session.handle(command(
        "move",
        serde_json::json!({ "id": FIRST_UNIT, "dx": 1, "dy": 0 }),
    ));

    let Message::Error(err) = reply else {
        panic!("未握手的 command 应回 error，收到 {reply:?}");
    };
    assert_eq!(err.code, ErrorCode::EProtocol, "先握手再发帧：{err}");
}

/// (e) 入口 chunk 没有返回值 → 没有命令面，答 `E_STATE` 并说明「未导出命令面」。
#[test]
fn pack_without_a_return_value_has_no_command_surface() {
    // 单文件包：入口只声明世界，不返回命令表。
    let mut session = handshake(
        Loaded::from_bundle(&ModBundle::in_memory("world.seal()\n")).expect("装载单文件包"),
        "no-commands",
    );

    let reply = session.handle(command("move", serde_json::json!({})));

    let Message::Error(err) = reply else {
        panic!("无命令面的包应回 error，收到 {reply:?}");
    };
    assert_eq!(err.code, ErrorCode::EState, "缺命令面是状态面：{err}");
    assert!(
        err.msg.contains("未导出命令面"),
        "错误要说清「没有命令面」而不是「没有这个命令」：{err}"
    );
}
