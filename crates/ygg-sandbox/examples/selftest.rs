//! `selftest` —— agent-seed Mod 的纯 Lua 自检入口。
//!
//! 设计边界（见 `docs/design/mod/testing-strategy.md`）：
//!
//! - 这里跑的是**普通 Lua 解释器**，不是内核沙箱。mod 的 `src/**` 与 `mod.lua`
//!   原样加载（走 `package.path`，不注入 pack.json 的 module 影子表），`ygg.*`
//!   由 `tests/harness.lua` 提供一个可控 mock。
//! - 因此**不覆盖** driver 层语义：事务回滚、fuel 预算、save receipt 结构 /
//!   state_hash、事件路由 fail-closed。这四类由真实跑局暴露，属已知缺口。
//! - 不修改宿主源码：只读 mod 目录、注入资源、执行 Lua。
//!
//! 用法：`cargo run -q --example selftest -- --mod .ygg/mods/agent-seed [--filter <substr>]`

use std::path::{Path, PathBuf};

use mlua::{Lua, Value};
use serde_json::Value as Json;

fn main() {
    let mut mod_dir: Option<PathBuf> = None;
    let mut filter: Option<String> = None;
    let mut fixture: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--mod" => mod_dir = args.next().map(PathBuf::from),
            "--filter" => filter = args.next(),
            "--fixture" => fixture = args.next(),
            other => {
                eprintln!("未知参数：{other}");
                std::process::exit(2);
            }
        }
    }
    let Some(mut mod_dir) = mod_dir else {
        eprintln!("用法：selftest --mod <mod 目录> [--filter <substr>] [--fixture <tag>]");
        std::process::exit(2);
    };
    // 仓库锚点必须在 fixture 复制**之前**取：fixture 目录在 tmp 下，`../../..` 指不到仓库根。
    let source_mod_dir = mod_dir.clone();
    if let Some(tag) = fixture.clone() {
        match prepare_fixture(&mod_dir, &tag) {
            Ok(dir) => {
                println!("fixture: {tag} -> {}", dir.display());
                mod_dir = dir;
            }
            Err(err) => {
                eprintln!("fixture 准备失败：{err}");
                std::process::exit(1);
            }
        }
    }
    if !mod_dir.is_dir() {
        eprintln!("mod 目录不存在：{}", mod_dir.display());
        std::process::exit(2);
    }

    let lua = Lua::new();

    // 资源注入：pack.json entries 的 id → 解析后的 JSON（转成 Lua 值）。
    // 沙箱里由宿主负责这一步；自检里由本文件负责，语义等同「ygg.data(id) 拿到表」。
    match install_assets(&lua, &mod_dir, &source_mod_dir) {
        Ok(n) => println!("assets: {n} entries"),
        Err(err) => {
            eprintln!("资源装载失败：{err}");
            std::process::exit(1);
        }
    }

    // mod 的 src/** 走文件 require（普通 Lua 的 package.path）。
    let pattern = format!("{}/?.lua", mod_dir.display());
    let init = format!("{}/?/init.lua", mod_dir.display());
    let fixture_literal = match fixture.as_deref() {
        Some(tag) => format!("\"{tag}\""),
        None => "nil".to_owned(),
    };
    let prelude = format!(
        "package.path = [[{pattern};{init};]] .. package.path\n\
         __MOD_DIR = [[{}]]\n\
         __FIXTURE = {}",
        mod_dir.display(),
        fixture_literal
    );
    if let Err(err) = lua.load(&prelude).set_name("prelude").exec() {
        eprintln!("prelude 失败：{err}");
        std::process::exit(1);
    }

    let tests_dir = mod_dir.join("tests");
    let harness = tests_dir.join("harness.lua");
    if !harness.is_file() {
        eprintln!("缺少 {}", harness.display());
        std::process::exit(1);
    }
    if let Err(err) = run_file(&lua, &harness) {
        eprintln!("harness 装载失败：{err}");
        std::process::exit(1);
    }

    // mod.lua 是装配入口：装完 handlers 才谈得上断言。
    let entry = mod_dir.join("mod.lua");
    if let Err(err) = run_file(&lua, &entry) {
        eprintln!("mod 装载失败：{err}");
        std::process::exit(1);
    }

    let mut files: Vec<PathBuf> = std::fs::read_dir(&tests_dir)
        .expect("read tests dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("lua"))
        .filter(|p| p.file_name().and_then(|s| s.to_str()) != Some("harness.lua"))
        .collect();
    files.sort();
    if let Some(needle) = &filter {
        files.retain(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .contains(needle.as_str())
        });
    }
    if files.is_empty() {
        eprintln!("没有可执行的测试文件（{}）", tests_dir.display());
        std::process::exit(1);
    }

    let mut hard_failures = 0usize;
    for file in &files {
        let name = file
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_owned();
        match run_file(&lua, file) {
            Ok(()) => println!("file ok   {name}"),
            Err(err) => {
                hard_failures += 1;
                println!("file FAIL {name}: {err}");
            }
        }
    }

    let (pass, fail) = counters(&lua);
    println!("---");
    println!("cases passed={pass} failed={fail}");
    if fail > 0 || hard_failures > 0 {
        println!("ok=0");
        std::process::exit(1);
    }
    println!("ok=1");
}

fn run_file(lua: &Lua, path: &Path) -> Result<(), String> {
    let source =
        std::fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("chunk")
        .to_owned();
    // 每个文件一个独立作用域：用 do-string 包住，避免局部变量互相污染。
    let chunk = format!("do\n{source}\nend");
    lua.load(&chunk)
        .set_name(name)
        .exec()
        .map_err(|err| err.to_string())
}

fn counters(lua: &Lua) -> (i64, i64) {
    let globals = lua.globals();
    let Ok(Value::Table(t)) = globals.get::<Value>("__SELFTEST") else {
        return (0, 0);
    };
    let pass = t.get::<i64>("pass").unwrap_or(0);
    let fail = t.get::<i64>("fail").unwrap_or(0);
    (pass, fail)
}

/// 把 `pack.json` 声明的资源按 id 注入成 Lua 表：`ygg.data(id)` 直接取到它。
fn install_assets(lua: &Lua, mod_dir: &Path, anchor: &Path) -> Result<usize, String> {
    let pack_text = std::fs::read_to_string(mod_dir.join("pack.json"))
        .map_err(|err| format!("read pack.json: {err}"))?;
    let pack: Json = serde_json::from_str(&pack_text).map_err(|err| format!("pack.json: {err}"))?;
    let entries = pack["entries"]
        .as_array()
        .ok_or_else(|| "pack.json 缺少 entries".to_owned())?;

    let assets = lua.create_table().map_err(|e| e.to_string())?;
    let mut count = 0usize;
    for entry in entries {
        let (Some(id), Some(path)) = (entry["id"].as_str(), entry["path"].as_str()) else {
            continue;
        };
        let file = mod_dir.join(path);
        let text = std::fs::read_to_string(&file)
            .map_err(|err| format!("read {}: {err}", file.display()))?;
        let json: Json =
            serde_json::from_str(&text).map_err(|err| format!("{}: {err}", file.display()))?;
        let value = json_to_lua(lua, &json).map_err(|e| e.to_string())?;
        assets.set(id, value).map_err(|e| e.to_string())?;
        count += 1;
    }
    lua.globals()
        .set("__ASSETS", assets)
        .map_err(|e| e.to_string())?;
    // 仓库根与 manifest：seed 布局类断言要读它们（Lua 无 JSON 解析器，故由宿主转值）。
    let repo_root = anchor
        .canonicalize()
        .map_err(|e| e.to_string())?
        .join("../../..")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    lua.globals()
        .set("__REPO_ROOT", repo_root.display().to_string())
        .map_err(|e| e.to_string())?;
    let manifest_text = std::fs::read_to_string(mod_dir.join("manifest.json"))
        .map_err(|err| format!("read manifest.json: {err}"))?;
    let manifest: Json =
        serde_json::from_str(&manifest_text).map_err(|err| format!("manifest.json: {err}"))?;
    let manifest_lua = json_to_lua(lua, &manifest).map_err(|e| e.to_string())?;
    lua.globals()
        .set("__MANIFEST", manifest_lua)
        .map_err(|e| e.to_string())?;

    // docs/context/scenes.json：reference / scene / reqStage 的 id 解析与读签模拟都要它。
    let scenes_path = repo_root.join("docs/context/scenes.json");
    let scenes_text = std::fs::read_to_string(&scenes_path)
        .map_err(|err| format!("read {}: {err}", scenes_path.display()))?;
    let scenes: Json =
        serde_json::from_str(&scenes_text).map_err(|err| format!("scenes.json: {err}"))?;
    let scenes_lua = json_to_lua(lua, &scenes).map_err(|e| e.to_string())?;
    lua.globals()
        .set("__SCENES", scenes_lua)
        .map_err(|e| e.to_string())?;

    // pack.json 本身也注入：结构类断言（模块表、entries、tools 声明）要读它。
    let pack_lua = json_to_lua(lua, &pack).map_err(|e| e.to_string())?;
    lua.globals()
        .set("__PACK", pack_lua)
        .map_err(|e| e.to_string())?;
    Ok(count)
}

/// JSON → Lua 值。手写而非 `LuaSerdeExt`：mlua 的 serialize feature 未启用，
/// 而这里只需要「结构 + 标量」这一层，不需要 serde 的通用桥。
fn json_to_lua(lua: &Lua, v: &Json) -> mlua::Result<Value> {
    Ok(match v {
        Json::Null => Value::Nil,
        Json::Bool(b) => Value::Boolean(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) => Value::Integer(i),
            None => Value::Number(n.as_f64().unwrap_or(0.0)),
        },
        Json::String(s) => Value::String(lua.create_string(s)?),
        Json::Array(items) => {
            let t = lua.create_table()?;
            for (i, item) in items.iter().enumerate() {
                t.set(i + 1, json_to_lua(lua, item)?)?;
            }
            Value::Table(t)
        }
        Json::Object(map) => {
            let t = lua.create_table()?;
            for (k, item) in map {
                t.set(k.as_str(), json_to_lua(lua, item)?)?;
            }
            Value::Table(t)
        }
    })
}

/// 递归复制目录（fixture 需要一份可改写的 mod 副本）。
fn copy_dir(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("mkdir {}: {e}", dst.display()))?;
    let entries = std::fs::read_dir(src).map_err(|e| format!("read {}: {e}", src.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).map_err(|e| format!("copy {}: {e}", from.display()))?;
        }
    }
    Ok(())
}

/// 按 tag 准备一份 fixture mod 副本（复制 + 改写 `data/config.json`）。
///
/// `manifest.json` 只列文件与 schemaVersion、不含内容哈希，所以改写 config 后无需重新
/// 封签（`sync-seed-mod.sh --check` 仍通过）。
fn prepare_fixture(src: &Path, tag: &str) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("ygg-selftest-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    copy_dir(src, &dir)?;
    let cfg_path = dir.join("data/config.json");
    match tag {
        // 裸拷贝：一行都不改，用来证明「未开闸的副本是惰性的」。
        "bare" => {}
        // 开闸副本：读签台账与覆盖清单全部上膛。
        "armed" => {
            let text = std::fs::read_to_string(&cfg_path)
                .map_err(|e| format!("read config.json: {e}"))?;
            let mut cfg: Json =
                serde_json::from_str(&text).map_err(|e| format!("parse config.json: {e}"))?;
            cfg["readPlan"] = serde_json::json!({
                "coordinate": ["acts/coordinate"],
                "settle": ["acts/settle", "rituals/cleanup"]
            });
            cfg["readGate"] = serde_json::json!({
                "ver": {
                    "acts/coordinate": "aaaaaa",
                    "req-acts/intake": "bbbbbb",
                    "req-acts/delivery": "cccccc",
                    "acts/settle": "dddddd",
                    "rituals/cleanup": "eeeeee"
                },
                "reqActs": { "plan": ["req-acts/intake"], "development": ["req-acts/delivery"] },
                "gates": { "plan": ["req-acts/delivery"], "accept": ["req-acts/delivery"] }
            });
            let out = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
            std::fs::write(&cfg_path, out)
                .map_err(|e| format!("write config.json: {e}"))?;
        }
        // 覆盖面 k/n：一幕计划两件文档，用来把「本幕覆盖了几件」与「我签了几件」分开。
        "plan2" => {
            let text = std::fs::read_to_string(&cfg_path)
                .map_err(|e| format!("read config.json: {e}"))?;
            let mut cfg: Json =
                serde_json::from_str(&text).map_err(|e| format!("parse config.json: {e}"))?;
            cfg["readPlan"] = serde_json::json!({
                "coordinate": ["acts/coordinate", "coordinate/dispatch"]
            });
            cfg["readGate"] = serde_json::json!({
                "ver": {
                    "acts/coordinate": "9f2c1a",
                    "coordinate/dispatch": "c0ffee",
                    "acts/charter": "aaaaaa"
                },
                "reqActs": {},
                "gates": {}
            });
            let out = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
            std::fs::write(&cfg_path, out)
                .map_err(|e| format!("write config.json: {e}"))?;
        }
        // 惰性副本：剥掉 readGate 与 readPlan（等价于「没开闸的第三方拷贝」）。
        "inert" => {
            let text = std::fs::read_to_string(&cfg_path)
                .map_err(|e| format!("read config.json: {e}"))?;
            let mut cfg: Json =
                serde_json::from_str(&text).map_err(|e| format!("parse config.json: {e}"))?;
            if let Some(obj) = cfg.as_object_mut() {
                obj.remove("readGate");
                obj.remove("readPlan");
            }
            let out = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
            std::fs::write(&cfg_path, out)
                .map_err(|e| format!("write config.json: {e}"))?;
        }
        // 关掉「全绿才放行」的安全阀（schema 合法：required 只管存在，不管值）。
        "nostrict" => {
            let text = std::fs::read_to_string(&cfg_path)
                .map_err(|e| format!("read config.json: {e}"))?;
            let mut cfg: Json =
                serde_json::from_str(&text).map_err(|e| format!("parse config.json: {e}"))?;
            cfg["gates"]["require_all_green"] = serde_json::json!(false);
            let out = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
            std::fs::write(&cfg_path, out)
                .map_err(|e| format!("write config.json: {e}"))?;
        }
        other => return Err(format!("未知 fixture tag：{other}")),
    }
    Ok(dir)
}
