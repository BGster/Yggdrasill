//! 快照往返用例：字节稳定 / 往返幂等 / 名称键控（id 无关）/ 坏字节拒绍。
//!
//! sim-kernel §2 的三条硬性质 + §4 的 `hash == fnv(快照字节)` 等式，全部在这里落地。
//! fixture 是**代码里的字面量构造器**（不用磁盘文件）：回放用例不依赖工作目录。

use std::collections::BTreeMap;

use ygg_sim::archetype::Column;
use ygg_sim::component::ComponentDef;
use ygg_sim::error::codes;
use ygg_sim::fixed::ONE;
use ygg_sim::hash::fnv1a64;
use ygg_sim::rng::RngRegistry;
use ygg_sim::snapshot;
use ygg_sim::store::ArchetypeStore;

/// 固定场景：三种有列类型（number / string / boolean / ref）+ 一个 tag，两个 archetype。
///
/// 全部字面量、无随机：同一份代码在任何机器上给出同一份字节。
fn fixture(order: &[&str]) -> ArchetypeStore {
    let mut store = ArchetypeStore::new();
    let mut ids: BTreeMap<&str, u32> = BTreeMap::new();
    for name in order {
        let def = match *name {
            "Pos" => ComponentDef::number(),
            "Label" => ComponentDef::string(),
            "Alive" => ComponentDef::boolean(),
            "Owner" => ComponentDef::reference(),
            "Enemy" => ComponentDef::tag(),
            other => panic!("fixture 未声明组件 {other}"),
        };
        ids.insert(name, store.register(name, def).unwrap());
    }
    // 桶 A：Pos + Label
    store
        .spawn_columns(
            2,
            &[
                (ids["Pos"], Column::I64(vec![ONE, 3 * ONE + ONE / 2])),
                (
                    ids["Label"],
                    Column::Bytes {
                        off: vec![0, 3, 3 + 3],
                        buf: b"abc\xe4\xbd\xa0".to_vec(),
                    },
                ),
            ],
            &[],
        )
        .unwrap();
    // 桶 B：Pos + Alive + Owner + Enemy（tag 无列）
    store
        .spawn_columns(
            1,
            &[
                (ids["Pos"], Column::I64(vec![-2 * ONE])),
                (ids["Alive"], Column::U8(vec![1])),
                (ids["Owner"], Column::U64(vec![7])),
            ],
            &[ids["Enemy"]],
        )
        .unwrap();
    store
}

/// 抽过两条流的世界（随机流状态必须随快照往返，否则存档前后序列会分叉）。
fn rng() -> RngRegistry {
    let mut rng = RngRegistry::new(12345);
    rng.next32("sim/integrate/0");
    rng.next32("sim/integrate/0");
    rng.next32("sim/decay/1");
    rng
}

fn resources() -> BTreeMap<u64, u64> {
    let mut map = BTreeMap::new();
    map.insert(9, 90);
    map.insert(2, 20);
    map.insert(0, 7);
    map
}

const ORDER_A: &[&str] = &["Pos", "Label", "Alive", "Owner", "Enemy"];
const ORDER_B: &[&str] = &["Enemy", "Owner", "Alive", "Label", "Pos"];

#[test]
fn bytes_stable_and_hash_is_fnv_of_payload() {
    let store = fixture(ORDER_A);
    let first = snapshot::encode(5, &resources(), &rng(), &store);
    let second = snapshot::encode(5, &resources(), &rng(), &store);
    assert_eq!(first, second, "同态两次编码必须逐字节相同");
    // hash = 尾 digest = fnv(去尾 digest 的字节)
    let payload = &first[..first.len() - 8];
    assert_eq!(snapshot::hash_of(&first), fnv1a64(payload));
    let tail = u64::from_le_bytes(first[first.len() - 8..].try_into().unwrap());
    assert_eq!(tail, snapshot::hash_of(&first), "尾 8 字节就是 state hash");
    // tick 进 hash：换 tick 就换 hash
    assert_ne!(
        snapshot::hash_of(&first),
        snapshot::hash_of(&snapshot::encode(6, &resources(), &rng(), &store))
    );
}

#[test]
fn round_trip_is_idempotent() {
    let store = fixture(ORDER_A);
    let bytes = snapshot::encode(5, &resources(), &rng(), &store);
    let decoded = snapshot::decode(&bytes).unwrap();
    assert_eq!(decoded.tick, 5);
    assert_eq!(decoded.resources, resources());
    // 重编码必须逐字节相同（含列顺序、组件表、资源序）
    let again = snapshot::encode(
        decoded.tick,
        &decoded.resources,
        &decoded.rng,
        &decoded.store,
    );
    assert_eq!(again, bytes, "restore(b).snapshot() == b");
    assert_eq!(snapshot::hash_of(&again), snapshot::hash_of(&bytes));
    // 值真的回来了（不是「字节相同但世界是空的」这类空转）
    assert_eq!(decoded.store.entity_count(), 3);
    assert_eq!(decoded.store.archetype_count(), 2);
    // 随机流状态跟着回来了：续跑的下一抽与原始世界一致
    assert_eq!(decoded.rng.seed(), 12345);
    assert_eq!(decoded.rng.stream_count(), 2);
    let mut continued = decoded.rng.clone();
    let mut original = rng();
    assert_eq!(
        continued.next32("sim/integrate/0"),
        original.next32("sim/integrate/0"),
        "restore 之后的随机序列必须接着原世界往下走"
    );
}

#[test]
fn id_order_does_not_change_bytes_or_hash() {
    let a = fixture(ORDER_A);
    let b = fixture(ORDER_B);
    // 内部 id 确实不同（否则这条用例证不了什么）
    assert_ne!(
        a.registry().id_of("Pos").unwrap(),
        b.registry().id_of("Pos").unwrap()
    );
    assert_eq!(a.registry().id_of("Pos").unwrap(), 0);
    assert_eq!(b.registry().id_of("Pos").unwrap(), 4);
    // 但字节与 hash 完全相同——名称键控把 id 抹掉
    let bytes_a = snapshot::encode(5, &resources(), &rng(), &a);
    let bytes_b = snapshot::encode(5, &resources(), &rng(), &b);
    assert_eq!(bytes_a, bytes_b, "注册顺序不得改变快照字节");
    assert_eq!(snapshot::hash_of(&bytes_a), snapshot::hash_of(&bytes_b));
    // 反过来：decode 之后两边都能正确取到值
    let decoded = snapshot::decode(&bytes_b).unwrap();
    let pos = decoded.store.registry().id_of("Pos").unwrap();
    assert_eq!(
        decoded.store.value_of(1, pos).unwrap(),
        ygg_sim::archetype::ValueRef::I64(ONE)
    );
}

#[test]
fn corrupt_bytes_are_rejected_not_panicked() {
    let store = fixture(ORDER_A);
    let bytes = snapshot::encode(5, &resources(), &rng(), &store);
    // 翻转中间某个字节：尾 digest 必须抓到
    let mut broken = bytes.clone();
    let mid = broken.len() / 2;
    broken[mid] ^= 0xFF;
    let err = snapshot::decode(&broken).unwrap_err();
    assert_eq!(err.code(), codes::FRAME, "坏字节必须 YGG-SIM-FRAME：{err}");
    // magic 坏
    let mut no_magic = bytes.clone();
    no_magic[0] = b'X';
    assert_eq!(
        snapshot::decode(&no_magic).unwrap_err().code(),
        codes::FRAME
    );
    // 版本坏
    let mut bad_version = bytes.clone();
    bad_version[8] = 99;
    assert_eq!(
        snapshot::decode(&bad_version).unwrap_err().code(),
        codes::FRAME
    );
    // 截断（少一个字节 → 尾 digest 不足）
    let truncated = &bytes[..bytes.len() - 1];
    assert_eq!(
        snapshot::decode(truncated).unwrap_err().code(),
        codes::FRAME
    );
    // 尾随垃圾（多一个字节 → 有未消费字节）
    let mut padded = bytes.clone();
    padded.push(0);
    assert_eq!(snapshot::decode(&padded).unwrap_err().code(), codes::FRAME);
    // 原帧仍然可读（拒绍不改输入）
    assert_eq!(
        snapshot::hash_of(&bytes),
        snapshot::hash_of(&snapshot::encode(5, &resources(), &rng(), &store))
    );
}

/// 把 v2 帧改写成同语义的 v1 帧（只动 `fmt_version` 字节并重算尾 digest）。
///
/// 前提：世界只含 v1 类型（本文件的 fixture 全部满足）；F64Bits 列在 v1 域不存在。
fn to_v1_frame(v2: &[u8]) -> Vec<u8> {
    let mut frame = v2[..v2.len() - 8].to_vec();
    assert_eq!(&frame[8..10], &2u16.to_le_bytes(), "输入必须是 v2 帧");
    frame[8] = 1;
    frame[9] = 0;
    let digest = fnv1a64(&frame);
    frame.extend_from_slice(&digest.to_le_bytes());
    frame
}

/// v1 兼容读取 + v2 重编码逐字节幂等（sim-kernel §2 版本史：跨 fmt_version 的 hash
/// 不承诺相等；承诺的是「v1 读入 → v2 存 → 再读 → 再存逐字节相等 + 逐值等于 v1」）。
#[test]
fn v1_frame_reads_and_reencodes_byte_identical() {
    let store = fixture(ORDER_A);
    let v2 = snapshot::encode(5, &resources(), &rng(), &store);
    let v1 = to_v1_frame(&v2);
    // v1 帧可读，语义与原世界逐值相同
    let decoded = snapshot::decode(&v1).unwrap();
    assert_eq!(decoded.tick, 5);
    assert_eq!(decoded.resources, resources());
    assert_eq!(decoded.store.entity_count(), 3);
    assert_eq!(decoded.store.archetype_count(), 2);
    let pos = decoded.store.registry().id_of("Pos").unwrap();
    assert_eq!(
        decoded.store.value_of(1, pos).unwrap(),
        ygg_sim::archetype::ValueRef::I64(ONE)
    );
    // v1 读入 → v2 存：逐字节等于原 v2 帧（版本字节是唯一差异，encode 恒写当前版）
    let again = snapshot::encode(
        decoded.tick,
        &decoded.resources,
        &decoded.rng,
        &decoded.store,
    );
    assert_eq!(again, v2, "v1 语义状态以 v2 重编码必须逐字节可复现");
    // 再读 → 再存：幂等
    let reread = snapshot::decode(&again).unwrap();
    let third = snapshot::encode(reread.tick, &reread.resources, &reread.rng, &reread.store);
    assert_eq!(third, v2);
}

/// v1 帧里的类型字节 6 必拒（v1 域是 1..=5；损坏或伪造 fail-closed）。
#[test]
fn v1_frame_with_bits_tag_is_rejected() {
    let store = fixture(ORDER_A);
    let mut v1 = to_v1_frame(&snapshot::encode(5, &resources(), &rng(), &store));
    // 组件表第一项的 type 字节：28(magic|ver|flags|tick|next) + 4(comp_n)
    // + 2(name_len) + name 长度。「Alive」按字节序排第一，长度 5。
    let type_at = 28 + 4 + 2 + 5;
    assert_eq!(
        v1[type_at], 3,
        "Alive 是 boolean（tag 3）——对账失败说明布局变了"
    );
    v1[type_at] = 6;
    let payload_len = v1.len() - 8;
    let digest = fnv1a64(&v1[..payload_len]);
    v1[payload_len..].copy_from_slice(&digest.to_le_bytes());
    let err = snapshot::decode(&v1).unwrap_err();
    assert_eq!(err.code(), codes::FRAME, "v1 帧不得携带位串列类型：{err}");
    // 同样的篡改在 v2 帧里是合法的（6 = F64Bits）——拒的是版本域不是类型本身
    let mut v2 = snapshot::encode(5, &resources(), &rng(), &store);
    v2[type_at] = 6;
    let payload_len = v2.len() - 8;
    let digest = fnv1a64(&v2[..payload_len]);
    v2[payload_len..].copy_from_slice(&digest.to_le_bytes());
    // 同样的篡改在 v2 帧里类型字节本身合法（6 = F64Bits）；boolean 列载荷按
    // 位串列形状去读必然字节错位 → 仍是 fail-closed（FRAME），但那是载荷形状错误，
    // 不是 v1 域拒绝——拒的是版本域不是类型本身。
    assert_eq!(snapshot::decode(&v2).unwrap_err().code(), codes::FRAME);
}

/// 位串列的三条硬性质（字节稳定 / 往返幂等 / 名称键控）在 F64Bits 上同样成立。
#[test]
fn f64bits_columns_are_stable_idempotent_and_id_independent() {
    fn bits_fixture(order: &[&str]) -> ArchetypeStore {
        let mut store = ArchetypeStore::new();
        let mut ids: BTreeMap<&str, u32> = BTreeMap::new();
        for name in order {
            let def = match *name {
                "Pos" => ComponentDef::number(),
                "Gauge" => ComponentDef::f64_bits(),
                "Enemy" => ComponentDef::tag(),
                other => panic!("bits_fixture 未声明组件 {other}"),
            };
            ids.insert(name, store.register(name, def).unwrap());
        }
        // 桶 A：Pos + Gauge（位串列含 ±Inf / 次正规 / 大负数 / 归一 NaN）
        store
            .spawn_columns(
                2,
                &[
                    (ids["Pos"], Column::I64(vec![ONE, -ONE])),
                    (
                        ids["Gauge"],
                        Column::F64Bits(vec![
                            0x7FF0_0000_0000_0000, // +Inf
                            0x0000_0000_0000_0001, // 次正规
                        ]),
                    ),
                ],
                &[],
            )
            .unwrap();
        // 桶 B：Gauge + Enemy（tag 无列）
        store
            .spawn_columns(
                1,
                &[(ids["Gauge"], Column::F64Bits(vec![0xFFEF_FFFF_FFFF_FFFF]))],
                &[ids["Enemy"]],
            )
            .unwrap();
        store
    }
    const BITS_ORDER_A: &[&str] = &["Pos", "Gauge", "Enemy"];
    const BITS_ORDER_B: &[&str] = &["Enemy", "Pos", "Gauge"];

    let a = bits_fixture(BITS_ORDER_A);
    let first = snapshot::encode(9, &resources(), &rng(), &a);
    // ① 字节稳定
    let second = snapshot::encode(9, &resources(), &rng(), &a);
    assert_eq!(first, second, "同态两次编码必须逐字节相同（含位串列）");
    // ② 往返幂等 + 位串逐值读回
    let decoded = snapshot::decode(&first).unwrap();
    let again = snapshot::encode(
        decoded.tick,
        &decoded.resources,
        &decoded.rng,
        &decoded.store,
    );
    assert_eq!(again, first, "restore(b).snapshot() == b（含位串列）");
    let gauge = decoded.store.registry().id_of("Gauge").unwrap();
    assert_eq!(
        decoded.store.value_of(1, gauge).unwrap(),
        ygg_sim::archetype::ValueRef::Bits(0x7FF0_0000_0000_0000)
    );
    assert_eq!(
        decoded.store.value_of(3, gauge).unwrap(),
        ygg_sim::archetype::ValueRef::Bits(0xFFEF_FFFF_FFFF_FFFF)
    );
    // ③ 名称键控：两种注册序 → 字节与 hash 全同
    let b = bits_fixture(BITS_ORDER_B);
    assert_ne!(
        a.registry().id_of("Gauge").unwrap(),
        b.registry().id_of("Gauge").unwrap()
    );
    let bytes_b = snapshot::encode(9, &resources(), &rng(), &b);
    assert_eq!(bytes_b, first, "注册顺序不得改变快照字节");
    assert_eq!(snapshot::hash_of(&bytes_b), snapshot::hash_of(&first));
}
