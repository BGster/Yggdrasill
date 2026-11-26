//! `pkg.codec` 的**同面 before/after** 基准（契约 `packages/codec.md` §4.1 / §5）。
//!
//! # 这个基准回答什么
//!
//! 一句话：**同一个沙箱、同一份数据、同一次运行**里，原生径比 Lua 径快多少。
//! 「同面」是硬要求——契约数字（2.77 MB/s）产在普通 Lua 上，而真实跑局在沙箱里，
//! 跨面比数字等于拿两个测量系统的读数相减。
//!
//! # before 怎么取（零生产开关）
//!
//! `pkg.codec` 的原生面是**可选**的（`native_face()` 读 `M.deps.host` 优先于 `ygg`），
//! 所以 `codec.install({ host = shim })` 就能让同一个包实例走 Lua 径，而 `shim` 只是
//! 把 `codec` 键藏起来的 `ygg` 代理（`kv` / `entity` 照旧）。这条不新增任何宿主参数、
//! 不改一行生产代码，也不依赖构建配置。
//!
//! 字节相等在这里顺带断言一次：before 与 after 的编码结果必须**同一份字节**——
//! 否则「快了」这件事就没有意义（那是换了格式，不是换了实现）。
//!
//! # 沙箱不装燃料钩子（历史注记）
//!
//! 原燃料钩子（每 1 000 条 VM 指令一触发）对两条路径同等作用，且一次 10k 实体
//! 编码需要的指令数远超原装载预算（50M）；S34 燃料机制整体退役后本已无钩子
//! 可装——本基准量的是**编解码吞吐**，报告里照旧写明。
//!
//! # dev profile 只打印跳过
//!
//! 与 `ygg-sim/tests/perf_tick.rs` 同款：dev profile 下打 `bench_skipped=dev-profile`
//! 并跳过断言，否则字面三件套第一件（`cargo test --workspace`）会因为 debug 构建
//! 必然不达标而变红。数字只在 `--release` 下有意义。

mod support;

use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use ygg_core::entity::EntityStore;
use ygg_core::kv::KvStore;
use ygg_sandbox::host_api::{self, HostHandles};
use ygg_sandbox::modload::ModBundle;
use ygg_sandbox::sandbox::Sandbox;

/// 目标吞吐（契约 §5：基线 2.77 MB/s 的 ~10× ⇒ ≥30 MiB/s）。
const TARGET_MIB_PER_SECOND: f64 = 30.0;

/// 仓库里的包根（`packages/libs`）。
fn repo_libs() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/libs")
}

/// 仓库里的消费夹具（`packages/mods/pkg-selftest`）。
fn repo_fixture() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/mods/pkg-selftest")
}

/// 合成树：`<R>/packages/libs/<产品包…>` + `<R>/mods/pkg-selftest`（照
/// `packages_sandbox.rs` 的先例——走真实推导，不给测试开后门）。
struct Tree {
    root: PathBuf,
}

impl Tree {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("ygg-pka2-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("packages/libs")).expect("mkdir package root");
        fs::create_dir_all(root.join("mods")).expect("mkdir mods");
        let mut packages: Vec<String> = fs::read_dir(repo_libs())
            .expect("read packages/libs")
            .filter_map(|item| item.ok())
            .filter(|item| item.path().is_dir())
            .filter_map(|item| item.file_name().into_string().ok())
            .filter(|id| id != "fixture")
            .collect();
        packages.sort();
        for id in packages {
            support::copy_dir(
                &repo_libs().join(&id),
                &root.join("packages/libs").join(&id),
            );
        }
        support::copy_dir(&repo_fixture(), &root.join("mods/pkg-selftest"));
        Tree { root }
    }

    fn mod_dir(&self) -> PathBuf {
        self.root.join("mods/pkg-selftest")
    }
}

/// 装一个「包 + 真宿主面」的沙箱（不装燃料，理由见文件头）。
fn bench_sandbox(tag: &str) -> Sandbox {
    let tree = Tree::new(tag);
    let bundle = ModBundle::load(&tree.mod_dir()).expect("pkg-selftest 必须可装载");
    let sandbox = Sandbox::new().expect("沙箱构造");
    sandbox.modules().set_modules(bundle.modules.clone());
    sandbox
        .modules()
        .set_chunk_paths(bundle.chunk_paths.clone());
    let entities = Rc::new(RefCell::new(EntityStore::new()));
    let kv = Rc::new(RefCell::new(KvStore::new()));
    host_api::install(sandbox.lua(), HostHandles { kv, entities }).expect("host API 必须可装");
    sandbox
}

/// 构造基准数据集：10k 实体（8 属性 + 3 tag）+ quest/dlg 各 1k KV，并导出 bundle。
const BUILD_BUNDLE: &str = r#"
local entity = require("pkg.entity")
local codec = require("pkg.codec")
entity.install()
codec.install()

entity.define("bench", {
  attrs = {
    attack = { type = "number", default = 12 },
    defense = { type = "number", default = 7 },
    speed = { type = "number", default = 5 },
    level = { type = "number", default = 3 },
    exp = { type = "number", default = 0 },
    hp = { type = "number", default = 100 },
    max_hp = { type = "number", default = 100 },
    weight = { type = "number", default = 1.5 },
  },
  tags = { boss = {}, enemy = {}, undead = {} },
})

local tags = { "boss", "enemy", "undead" }
for i = 1, 10000 do
  entity.spawn("bench", { hp = i, exp = i * 2 }, tags)
end

local keys = {}
for i = 0, 999 do
  ygg.kv.set("quest." .. i, "任务 " .. i)
  ygg.kv.set("dlg." .. i, i)
  keys[#keys + 1] = "quest." .. i
  keys[#keys + 1] = "dlg." .. i
end

local bundle = codec.export({ entities = true, ids = "index", keys = keys, mod = "bench" })
BUNDLE = bundle
return #keys
"#;

/// 两条路径的分派切换与三次取中位都在 Rust 侧做（沙箱里没有 `os.clock`）。
fn median3(mut samples: [f64; 3]) -> f64 {
    samples.sort_by(|left, right| left.partial_cmp(right).expect("非 NaN"));
    samples[1]
}

fn mib_per_second(bytes: usize, milliseconds: f64) -> f64 {
    (bytes as f64 / 1_048_576.0) / (milliseconds / 1000.0)
}

#[test]
fn codec_native_before_after_on_the_same_surface() {
    if cfg!(debug_assertions) {
        println!("bench_skipped=dev-profile");
        return;
    }

    let sandbox = bench_sandbox("bench");
    let lua = sandbox.lua();
    let keys: usize = lua.load(BUILD_BUNDLE).eval().expect("数据集必须可构造");
    assert_eq!(keys, 2000, "KV 点名集 = 2000 键（契约 §5 的基准集）");

    // 先用 Lua 径产出一份 canonical 文本，供 decode 与「字节相等」用
    let text: mlua::LuaString = lua
        .load(
            r#"
            local codec = require("pkg.codec")
            local saved = ygg.codec
            local shim = setmetatable({}, { __index = function(_, key)
              if key == "codec" then return nil end
              return ygg[key]
            end })
            codec.install({ host = shim })
            local text = codec.encode(BUNDLE)
            codec.install()
            ygg.codec = saved
            return text
            "#,
        )
        .eval()
        .expect("Lua 径必须能编码");
    let text_bytes = text.as_bytes().to_vec();
    let size = text_bytes.len();
    println!("bench_bundle_bytes={size}");

    // ── after：面在场（原生径）──────────────────────────────────────────────
    let encode_native: mlua::Function = lua
        .load(
            "local codec = require('pkg.codec'); return function() return codec.encode(BUNDLE) end",
        )
        .eval()
        .expect("encode 闭包");
    let decode_native: mlua::Function = lua
        .load("local codec = require('pkg.codec'); return function() return codec.decode(TEXT) end")
        .eval()
        .expect("decode 闭包");
    lua.globals()
        .set("TEXT", lua.create_string(&text_bytes).expect("文本"))
        .expect("挂 TEXT");

    let mut encode_native_samples = [0.0f64; 3];
    for slot in encode_native_samples.iter_mut() {
        let began = Instant::now();
        encode_native
            .call::<mlua::LuaString>(())
            .expect("原生 encode");
        *slot = began.elapsed().as_secs_f64() * 1000.0;
    }
    let mut decode_native_samples = [0.0f64; 3];
    for slot in decode_native_samples.iter_mut() {
        let began = Instant::now();
        decode_native.call::<mlua::Value>(()).expect("原生 decode");
        *slot = began.elapsed().as_secs_f64() * 1000.0;
    }
    // sha256（S19 第三键）：同一份 bundle 文本的摘要，同面同进程。
    let sha256_native: mlua::Function = lua
        .load("local codec = require('pkg.codec'); return function() return codec.sha256(TEXT) end")
        .eval()
        .expect("sha256 闭包（原生）");
    let mut sha256_native_samples = [0.0f64; 3];
    let mut sha256_native_hex = String::new();
    for slot in sha256_native_samples.iter_mut() {
        let began = Instant::now();
        sha256_native_hex = sha256_native.call::<String>(()).expect("原生 sha256");
        *slot = began.elapsed().as_secs_f64() * 1000.0;
    }
    assert_eq!(
        sha256_native_hex.len(),
        64,
        "原生 sha256 必须返回 64 位 hex"
    );

    // ── before：把面藏起来（Lua 径）────────────────────────────────────────
    let switch_to_lua = r#"
        local codec = require("pkg.codec")
        local shim = setmetatable({}, { __index = function(_, key)
          if key == "codec" then return nil end
          return ygg[key]
        end })
        codec.install({ host = shim })
        return "lua"
    "#;
    lua.load(switch_to_lua).exec().expect("切到 Lua 径");
    let encode_lua: mlua::Function = lua
        .load(
            "local codec = require('pkg.codec'); return function() return codec.encode(BUNDLE) end",
        )
        .eval()
        .expect("encode 闭包（lua）");
    let decode_lua: mlua::Function = lua
        .load("local codec = require('pkg.codec'); return function() return codec.decode(TEXT) end")
        .eval()
        .expect("decode 闭包（lua）");

    let mut encode_lua_samples = [0.0f64; 3];
    let mut lua_bytes = Vec::new();
    for slot in encode_lua_samples.iter_mut() {
        let began = Instant::now();
        let encoded: mlua::LuaString = encode_lua.call(()).expect("Lua encode");
        *slot = began.elapsed().as_secs_f64() * 1000.0;
        lua_bytes = encoded.as_bytes().to_vec();
    }
    let mut decode_lua_samples = [0.0f64; 3];
    for slot in decode_lua_samples.iter_mut() {
        let began = Instant::now();
        decode_lua.call::<mlua::Value>(()).expect("Lua decode");
        *slot = began.elapsed().as_secs_f64() * 1000.0;
    }
    let sha256_lua: mlua::Function = lua
        .load("local codec = require('pkg.codec'); return function() return codec.sha256(TEXT) end")
        .eval()
        .expect("sha256 闭包（lua）");
    let mut sha256_lua_samples = [0.0f64; 3];
    let mut sha256_lua_hex = String::new();
    for slot in sha256_lua_samples.iter_mut() {
        let began = Instant::now();
        sha256_lua_hex = sha256_lua.call::<String>(()).expect("Lua sha256");
        *slot = began.elapsed().as_secs_f64() * 1000.0;
    }

    // ── 字节相等（before/after 必须同一份字节，否则「快」没有意义）──────────
    let native_bytes: mlua::LuaString = lua
        .load(
            r#"
            local codec = require("pkg.codec")
            codec.install()
            return codec.encode(BUNDLE)
            "#,
        )
        .eval()
        .expect("恢复原生径并编码");
    assert_eq!(
        native_bytes.as_bytes().to_vec(),
        lua_bytes,
        "原生径与 Lua 径的字节必须相等（同面 before/after 的前提）"
    );
    assert_eq!(native_bytes.as_bytes().to_vec(), text_bytes);
    // sha256 双径同值（字节相等在同一面上顺带断言一次）。
    assert_eq!(
        sha256_native_hex, sha256_lua_hex,
        "原生与 Lua 径的摘要必须逐字节相等"
    );

    let encode_native_ms = median3(encode_native_samples);
    let encode_lua_ms = median3(encode_lua_samples);
    let decode_native_ms = median3(decode_native_samples);
    let decode_lua_ms = median3(decode_lua_samples);

    let encode_native_mbps = mib_per_second(size, encode_native_ms);
    let encode_lua_mbps = mib_per_second(size, encode_lua_ms);
    let decode_native_mbps = mib_per_second(size, decode_native_ms);
    let decode_lua_mbps = mib_per_second(size, decode_lua_ms);
    let sha256_native_ms = median3(sha256_native_samples);
    let sha256_lua_ms = median3(sha256_lua_samples);
    let sha256_native_mbps = mib_per_second(size, sha256_native_ms);
    let sha256_lua_mbps = mib_per_second(size, sha256_lua_ms);

    println!(
        "PERF codec_lua.encode_mbps={encode_lua_mbps:.2} ({encode_lua_ms:.1} ms; samples {:.1}/{:.1}/{:.1})",
        encode_lua_samples[0], encode_lua_samples[1], encode_lua_samples[2]
    );
    println!(
        "PERF codec_native.encode_mbps={encode_native_mbps:.2} ({encode_native_ms:.1} ms; samples {:.1}/{:.1}/{:.1})",
        encode_native_samples[0], encode_native_samples[1], encode_native_samples[2]
    );
    println!("PERF codec_lua.decode_mbps={decode_lua_mbps:.2} ({decode_lua_ms:.1} ms)",);
    println!("PERF codec_native.decode_mbps={decode_native_mbps:.2} ({decode_native_ms:.1} ms)",);
    println!(
        "PERF codec.encode_speedup={:.2}x decode_speedup={:.2}x",
        encode_native_mbps / encode_lua_mbps,
        decode_native_mbps / decode_lua_mbps
    );
    println!(
        "PERF codec_lua.sha256_mbps={sha256_lua_mbps:.2} ({sha256_lua_ms:.1} ms; samples {:.1}/{:.1}/{:.1})",
        sha256_lua_samples[0], sha256_lua_samples[1], sha256_lua_samples[2]
    );
    println!(
        "PERF codec_native.sha256_mbps={sha256_native_mbps:.2} ({sha256_native_ms:.1} ms; samples {:.1}/{:.1}/{:.1})",
        sha256_native_samples[0], sha256_native_samples[1], sha256_native_samples[2]
    );
    println!(
        "PERF codec.sha256_speedup={:.2}x",
        sha256_native_mbps / sha256_lua_mbps
    );

    assert!(
        encode_native_mbps >= TARGET_MIB_PER_SECOND,
        "原生 encode {encode_native_mbps:.2} MiB/s < {TARGET_MIB_PER_SECOND}（Lua 径 {encode_lua_mbps:.2}）"
    );
    assert!(
        decode_native_mbps >= TARGET_MIB_PER_SECOND,
        "原生 decode {decode_native_mbps:.2} MiB/s < {TARGET_MIB_PER_SECOND}（Lua 径 {decode_lua_mbps:.2}）"
    );
    assert!(
        sha256_native_mbps >= TARGET_MIB_PER_SECOND,
        "原生 sha256 {sha256_native_mbps:.2} MiB/s < {TARGET_MIB_PER_SECOND}（Lua 径 {sha256_lua_mbps:.2}）"
    );
}
