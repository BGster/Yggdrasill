//! `ygg.sim.*` 面的 Lua 端 e2e（lua-api §3.1.3）：未挂载降级 / 键集闭集 / 定点闸门 /
//! 挂载后的形状与借用纪律。
//!
//! 断言都以 **Lua 侧可见的东西**为准（`pcall` 捕获的 message 首段、表的字段、字符串长度），
//! 而不是 Rust 侧的中间变量——这样「Rust 面能跑」与「Lua 面能用」才是两件事都被证明了。
//!
//! # 借用纪律（`RefCell` double-borrow 的回归钉）
//!
//! 每个 handler 内部都在调用 `ygg.sim.*`：若某个面在调用期间跨 `lua.create_table` 持有
//! `RefCell` 借用，这里会直接 panic 成「already borrowed」而不是悄悄不生效。

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value as Json};
use ygg_sandbox::driver::KernelFacade;
use ygg_sandbox::modload::ModBundle;
use ygg_sim::canonical::FrameWriter;
use ygg_sim::component::ComponentDef;
use ygg_sim::fixed::ONE;
use ygg_sim::sim::Sim;

const MOD_LUA: &str = r#"
local function first_line(text)
  local line = tostring(text)
  local cut = line:find("\n", 1, true)
  if cut ~= nil then
    line = line:sub(1, cut - 1)
  end
  return line
end

local function probe(body)
  local ok, err = pcall(body)
  if ok then
    return "ok"
  end
  return first_line(err)
end

local NAMES = {
  "step", "hash", "snapshot", "restore", "spawn_batch",
  "despawn_batch", "insert_batch", "status", "trace", "replay",
}

ygg.on("sim.unmounted", function(payload)
  ygg.narrate("step:" .. probe(function() return ygg.sim.step(1) end))
  ygg.narrate("hash:" .. probe(function() return ygg.sim.hash() end))
  ygg.narrate("snapshot:" .. probe(function() return ygg.sim.snapshot() end))
  ygg.narrate("restore:" .. probe(function() return ygg.sim.restore("x") end))
  ygg.narrate("spawn:" .. probe(function() return ygg.sim.spawn_batch("x") end))
  ygg.narrate("despawn:" .. probe(function() return ygg.sim.despawn_batch("x") end))
  ygg.narrate("insert:" .. probe(function() return ygg.sim.insert_batch("x") end))
  ygg.narrate("status:" .. probe(function() return ygg.sim.status() end))
  ygg.narrate("trace:" .. probe(function() return ygg.sim.trace(true) end))
  ygg.narrate("replay:" .. probe(function() return ygg.sim.replay("a", "b") end))
end)

ygg.on("sim.keys", function(payload)
  local seen = {}
  local count = 0
  for key in pairs(ygg.sim) do
    count = count + 1
    seen[key] = true
  end
  local missing = 0
  for i = 1, #NAMES do
    if not seen[NAMES[i]] then
      missing = missing + 1
    end
  end
  local extra = 0
  for key in pairs(seen) do
    local known = false
    for i = 1, #NAMES do
      if NAMES[i] == key then
        known = true
      end
    end
    if not known then
      extra = extra + 1
    end
  end
  ygg.narrate("count:" .. count)
  ygg.narrate("missing:" .. missing)
  ygg.narrate("extra:" .. extra)
end)

ygg.on("sim.fixed", function(payload)
  ygg.narrate("frac:" .. probe(function() return ygg.sim.step(0.5) end))
  ygg.narrate("text:" .. probe(function() return ygg.sim.step("x") end))
  ygg.narrate("ok:" .. probe(function() return ygg.sim.step(1) end))
end)

ygg.on("sim.mounted", function(payload)
  local status = ygg.sim.status()
  ygg.narrate("mounted:" .. tostring(status.mounted))
  ygg.narrate("tick:" .. status.tick)
  ygg.narrate("entities:" .. status.entities)
  ygg.narrate("archetypes:" .. status.archetypes)
  ygg.narrate("hash:" .. status.hash)
  ygg.narrate("hashlen:" .. #status.hash)
  local stepped = ygg.sim.step(1)
  ygg.narrate("after_tick:" .. stepped.tick)
  ygg.narrate("step_hash_matches:" .. tostring(stepped.hash == ygg.sim.hash()))
  local snap = ygg.sim.snapshot()
  ygg.narrate("snapshot_bytes:" .. #snap)
  local restored = ygg.sim.restore(snap)
  ygg.narrate("restore_tick:" .. restored.tick)
  ygg.narrate("restore_bytes:" .. restored.bytes)
  ygg.narrate("roundtrip:" .. tostring(restored.hash == ygg.sim.hash()))
  ygg.narrate("trace:" .. tostring(ygg.sim.trace(true).tracing))
end)

ygg.on("sim.bad_frame", function(payload)
  ygg.narrate("spawn:" .. probe(function() return ygg.sim.spawn_batch("nonsense") end))
  ygg.narrate("restore:" .. probe(function() return ygg.sim.restore("nonsense") end))
  ygg.narrate("replay:" .. probe(function() return ygg.sim.replay("a", "b") end))
end)
"#;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ygg-r1-sim-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// 现造一个最小 Mod（不落 fixture 文件、不碰任何业务目录）。
fn write_mod(dir: &Path) {
    fs::create_dir_all(dir).expect("create mod dir");
    fs::write(dir.join("mod.lua"), MOD_LUA).expect("write mod.lua");
    let pack = json!({
        "schemaVersion": 1,
        "entries": [],
        "schemaFiles": [],
    });
    fs::write(
        dir.join("pack.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&pack).expect("pack json")
        ),
    )
    .expect("write pack.json");
    fs::write(
        dir.join("manifest.json"),
        format!(
            "{}\n",
            serde_json::to_string(&json!({
                "schemaVersion": 1,
                "entries": [],
                "code": [],
            }))
            .expect("manifest json")
        ),
    )
    .expect("write manifest.json");
}

fn kernel() -> KernelFacade {
    let dir = temp_dir("face");
    write_mod(&dir);
    let bundle = ModBundle::load(&dir).expect("装载 Mod 包");
    bundle.kernel().expect("构造内核")
}

fn narrate(kernel: &mut KernelFacade, event: &str) -> Vec<String> {
    let reply: Json = serde_json::from_str(&kernel.dispatch(event, "0")).expect("回执是 JSON");
    assert_eq!(reply["ok"], json!(true), "{event} 派发失败：{reply}");
    reply["narrative"]
        .as_array()
        .expect("narrative 数组")
        .iter()
        .map(|line| line.as_str().expect("叙述行是字符串").to_owned())
        .collect()
}

fn sim_with_entities() -> Sim {
    let mut sim = Sim::new(4_242);
    let pos = sim.register("Pos", ComponentDef::number()).unwrap();
    let vel = sim.register("Vel", ComponentDef::number()).unwrap();
    let mut writer = FrameWriter::new();
    writer.u32(3);
    writer.u16(2);
    for name in ["Pos", "Vel"] {
        writer.u16(name.len() as u16);
        writer.raw(name.as_bytes());
    }
    for value in [0i64, ONE, 2 * ONE] {
        writer.i64(value);
    }
    for value in [ONE, ONE, ONE] {
        writer.i64(value);
    }
    let frame = writer.finish();
    sim.spawn_batch(&frame).unwrap();
    let _ = (pos, vel);
    sim
}

#[test]
fn unmounted_face_throws_typo_free_code_for_every_key() {
    let mut kernel = kernel();
    let lines = narrate(&mut kernel, "sim.unmounted");
    assert_eq!(lines.len(), 10, "10 个键必须各报一次：{lines:?}");
    for line in &lines {
        assert!(
            line.contains("YGG-SIM-UNMOUNTED"),
            "未挂载时必须报 UNMOUNTED（不是 nil/false）：{line}"
        );
        assert!(
            line.contains("宿主未注入模拟内核"),
            "码后必须有人读解释（首段是码）：{line}"
        );
    }
}

#[test]
fn key_set_is_exactly_the_ten_documented_keys() {
    let mut kernel = kernel();
    let lines = narrate(&mut kernel, "sim.keys");
    assert_eq!(lines[0], "count:10", "键集必须恰好 10 个：{lines:?}");
    assert_eq!(lines[1], "missing:0", "缺键：{lines:?}");
    assert_eq!(
        lines[2], "extra:0",
        "多键（逐实体 getter 的退化信号）：{lines:?}"
    );
}

#[test]
fn fixed_gate_rejects_non_integers_and_accepts_integers() {
    let mut kernel = kernel();
    kernel.set_sim(Some(Box::new(sim_with_entities())));
    let lines = narrate(&mut kernel, "sim.fixed");
    assert!(
        lines[0].contains("YGG-SIM-FIXED"),
        "0.5 必须被定点闸门拒：{lines:?}"
    );
    assert!(
        lines[1].contains("YGG-SIM-FIXED"),
        "字符串必须被拒（不是隐式 tonumber）：{lines:?}"
    );
    assert_eq!(lines[2], "ok:ok", "整数 dt 必须放行：{lines:?}");
}

#[test]
fn mounted_face_reports_status_and_round_trips_snapshot() {
    let mut kernel = kernel();
    let sim = sim_with_entities();
    let before = sim.hash();
    let entities = sim.world().entity_count();
    kernel.set_sim(Some(Box::new(sim)));

    let lines = narrate(&mut kernel, "sim.mounted");
    assert_eq!(lines[0], "mounted:true");
    assert_eq!(lines[1], "tick:0");
    assert_eq!(lines[2], format!("entities:{entities}"));
    assert_eq!(lines[3], "archetypes:1");
    assert_eq!(
        lines[4],
        format!("hash:{before:016x}"),
        "hash 口径 = 16 位小写 hex"
    );
    assert_eq!(lines[5], "hashlen:16");
    assert_eq!(lines[6], "after_tick:1", "一次 step = 一次穿越");
    assert_eq!(
        lines[7], "step_hash_matches:true",
        "step 返回的 hash == hash()"
    );
    assert!(lines[8].starts_with("snapshot_bytes:"), "{lines:?}");
    assert_eq!(lines[9], "restore_tick:1");
    assert_eq!(
        lines[11], "roundtrip:true",
        "restore 后 hash 必须回到同一值"
    );
    assert_eq!(lines[12], "trace:true");
}

#[test]
fn bad_frames_fail_closed_with_sim_frame() {
    let mut kernel = kernel();
    kernel.set_sim(Some(Box::new(sim_with_entities())));
    let lines = narrate(&mut kernel, "sim.bad_frame");
    for line in &lines {
        assert!(
            line.contains("YGG-SIM-FRAME"),
            "坏帧必须 YGG-SIM-FRAME（先校验后改）：{line}"
        );
    }
}

#[test]
fn host_can_drive_batches_between_dispatches_via_handle() {
    // 宿主批量帧走 Rust 句柄（不经 Lua 边界），Lua 侧下一帧就能看见结果。
    let mut kernel = kernel();
    kernel.set_sim(Some(Box::new(sim_with_entities())));
    {
        let handle = kernel.sim_handle();
        let mut slot = handle.borrow_mut();
        let sim = slot.as_mut().expect("已注入");
        let mut writer = FrameWriter::new();
        writer.u32(2);
        writer.u16(1);
        writer.u16(3);
        writer.raw(b"Vel");
        writer.i64(0);
        writer.i64(0);
        // 只有 Vel 的桶（另一 archetype），证明桶数与实体数都经 Lua status 可见
        sim.spawn_batch(&writer.finish()).unwrap();
    }
    let lines = narrate(&mut kernel, "sim.mounted");
    assert_eq!(lines[2], "entities:5");
    assert_eq!(lines[3], "archetypes:2");
}
