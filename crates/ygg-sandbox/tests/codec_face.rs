//! `ygg.codec.*` 的**差分字节相等**门禁（契约 `lua-api.md` §3.1.4 /
//! `packages/codec.md` §4.1）。
//!
//! # 为什么这条测试是这个面的核心
//!
//! 原生面不是第二个规范：规范是 `pkg.codec` 的 Lua 实现（包契约 I1 的可执行形态），
//! 原生面是它的符合实现。所以判据只有一条——**同一输入、同一解释器、同一份数据，
//! 两条路径的字节相等**。差别只允许出现在「原生拒绝、Lua 接手」的那些输入上
//! （值域拒绝 / 带元方法的表），而那类输入的**文案**仍由 Lua 路径单独给出。
//!
//! # 双径怎么造（同一个 Lua 状态，不装两份解释器）
//!
//! `ygg.codec` 在场 ⇒ 包的 `M.encode` 走原生；把它摘掉 ⇒ 同一个 `M.encode` 走 Lua。
//! 于是两条路径共享同一份包代码、同一份数据引用，差异只有那一行分派——这比「两个
//! 进程比字节」强，因为解释器版本、字符串驻留、数值解析全都不构成变量。
//!
//! 语料四类（计划稿 §5.4）：行为用例的值形状、边界集、10⁴ 实体集的 bundle 形状、
//! 2 000 个固定种子 fuzz 值。

use std::path::{Path, PathBuf};

use mlua::{Function, Lua, Table, Value};

/// 仓库里的包根（`packages/libs`）。
fn repo_libs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/libs")
}

/// 把一个真包模块按名字装进 `package.loaded`（普通 Lua，不经沙箱装载面）。
fn register_module(lua: &Lua, name: &str, relative: &str) -> Result<(), mlua::Error> {
    let path = repo_libs().join(relative);
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读 {} 失败：{error}", path.display()));
    let module: Value = lua
        .load(&source)
        .set_name(format!("@{}", path.display()))
        .eval()?;
    let package: Table = lua.globals().get("package")?;
    let loaded: Table = package.get("loaded")?;
    loaded.set(name, module)?;
    Ok(())
}

/// 装出一个「双径」状态：`ygg.codec` 在场 + 真 `pkg.codec` 已 install。
fn build() -> (Lua, Table) {
    let lua = Lua::new();
    let ygg = lua.create_table().expect("建 ygg 表");
    lua.globals().raw_set("ygg", ygg).expect("挂 ygg");
    ygg_sandbox::codec_face::install(&lua).expect("装 ygg.codec");

    register_module(&lua, "pkg.entity", "entity/init.lua").expect("装 pkg.entity");
    register_module(&lua, "pkg.codec.sha256", "codec/sha256.lua").expect("装 pkg.codec.sha256");
    register_module(&lua, "pkg.codec", "codec/init.lua").expect("装 pkg.codec");
    lua.load("require('pkg.codec').install()")
        .exec()
        .expect("install 不带 deps ⇒ host() 落到 ygg 全局");

    let cases = lua.load(CORPUS).eval::<Table>().expect("语料必须可构造");
    (lua, cases)
}

/// 原生径：直接调面函数（不经包的分派——这样「合法值被原生误拒」才是硬错）。
fn native_encode(value: &Value) -> Result<Vec<u8>, ygg_sandbox::codec_face::CodecFaceError> {
    ygg_sandbox::codec_face::encode(value)
}

/// 面径（`ygg.codec.encode`，走 mlua 包装）。
fn face_encode(lua: &Lua, value: &Value) -> Result<Vec<u8>, mlua::Error> {
    let function: Function = lua
        .load("return function(v) return ygg.codec.encode(v) end")
        .eval()?;
    let text: mlua::LuaString = function.call(value.clone())?;
    Ok(text.as_bytes().to_vec())
}

/// Lua 径（`pkg.codec.encode`，临时摘掉 `ygg.codec` 逼它走回退分支）。
fn lua_encode(lua: &Lua, value: &Value) -> Result<Vec<u8>, mlua::Error> {
    let function: Function = lua
        .load(
            r#"
            local codec = require("pkg.codec")
            return function(v)
              local saved = ygg.codec
              ygg.codec = nil
              local ok, out = pcall(codec.encode, v)
              ygg.codec = saved
              if not ok then error(out, 0) end
              return out
            end
            "#,
        )
        .eval()?;
    let text: mlua::LuaString = function.call(value.clone())?;
    Ok(text.as_bytes().to_vec())
}

#[test]
fn native_encode_matches_the_lua_path_byte_for_byte() {
    let (lua, cases) = build();
    let count = cases.raw_len();
    assert!(count >= 2_000, "语料不足：{count} 条（计划稿 §5.4 要求 2 000+）");

    let mut compared = 0usize;
    for index in 1..=count {
        let value: Value = cases.raw_get(index as i64).expect("语料元素");
        // ① 合法值不得被原生误拒（误拒只在字节相等下看不见，只丢性能）
        let native = native_encode(&value)
            .unwrap_or_else(|error| panic!("第 {index} 条被原生误拒：{error}"));
        // ② 面包装与直调同一份字节
        let via_face =
            face_encode(&lua, &value).unwrap_or_else(|error| panic!("第 {index} 条过面失败：{error}"));
        assert_eq!(native, via_face, "第 {index} 条：面包装与直调不一致");
        // ③ 与规范执行体逐字节相等
        let lua_bytes =
            lua_encode(&lua, &value).unwrap_or_else(|error| panic!("第 {index} 条 Lua 径失败：{error}"));
        assert_eq!(
            native,
            lua_bytes,
            "第 {index} 条：原生径与 Lua 径字节不同（输入 {:?}）",
            value.type_name()
        );
        compared += 1;
    }
    assert_eq!(compared, count);
}

#[test]
fn metatables_are_unsupported_and_the_lua_path_still_produces_bytes() {
    let (lua, cases) = build();
    let metatables: Table = cases
        .raw_get("metatables")
        .expect("语料带 metatables 段");
    for index in 1..=metatables.raw_len() {
        let value: Value = metatables.raw_get(index as i64).expect("元表元素");
        let error = native_encode(&value).expect_err("带元方法的表必须整值回退");
        assert_eq!(error.code(), ygg_sandbox::codec_face::CODEC_UNSUPPORTED);
        let lua_bytes = lua_encode(&lua, &value).expect("Lua 径必须能编码带元方法的表");
        assert!(!lua_bytes.is_empty());
    }
}

/// 语料（普通 Lua 构造：`math` / `string` 在场，不受沙箱白名单约束）。
const CORPUS: &str = r#"
local cases = {}

-- ① 行为用例的值形状（`pkg-codec.lua` 20 条用例里出现的那几类）
cases[#cases + 1] = { 1, 2, 3 }
cases[#cases + 1] = {}
cases[#cases + 1] = { i = 1, f = 1.5 }
cases[#cases + 1] = { a = { b = { c = { d = "deep" } } } }
cases[#cases + 1] = { 1, 1.0, -0.0, 0.0, 1e308, 5e-324 }
cases[#cases + 1] = { true, false, "text", "" }
cases[#cases + 1] = { tags = { "boss", "enemy", "undead" } }
cases[#cases + 1] = { kv = { ["quest.0"] = "任务 0", ["dlg.0"] = 0 } }

-- ② 边界集：数字形态（%g 的形态边界逐个点名）
local NUMBERS = {
  0.0, -0.0, 1.5, -1.5, 0.1, 2.5, 1e8, 1e14, 1e15, 1e16, 1e21, 1e100, 1e300,
  1e-4, 1e-5, 1e-21, 1e-100, 1e-300, 1.5e-7, 5e-324, 1.7976931348623157e308,
  2.2250738585072014e-308, 123456789.0, 123456789012345678.0, 9007199254740993.0,
  1.0000000000000002, 3.141592653589793, 9.99, 99.9, 999.999, 1.005, 2.675,
  123.456e-7, -2.675, -9.99, 6.02214076e23, 1.602176634e-19, 0.001, 9.999e-5,
  99999999999999.9, 999999999999999.9, 0.0001234, 12.5, 4.9e-324, 1e-323,
}
for i = 1, #NUMBERS do
  cases[#cases + 1] = { NUMBERS[i] }
  cases[#cases + 1] = NUMBERS[i]
end
local INTEGERS = { 0, 1, -1, 42, 9007199254740993, math.maxinteger, math.mininteger }
for i = 1, #INTEGERS do
  cases[#cases + 1] = { INTEGERS[i] }
end

-- ③ 字符串：转义全集 + 非 UTF-8 字节 + 中文
cases[#cases + 1] = { 'quote"back\\slash/tab\tcr\rnl\n' }
cases[#cases + 1] = { string.char(0, 1, 7, 8, 9, 10, 12, 13, 27, 31, 127) }
cases[#cases + 1] = { string.char(128, 159, 255, 0x80, 0xC3) }
cases[#cases + 1] = { "任务 0", "emoji 🙂", "/" }
cases[#cases + 1] = { ["键"] = "值", ["a/b"] = "slash" }

-- ④ 深嵌套 64 层（上限内侧）与宽表 2000 键（键序比较的最坏规模）
local deep = {}
local tail = deep
for _ = 1, 63 do
  local next_level = {}
  tail[1] = next_level
  tail = next_level
end
cases[#cases + 1] = deep

local wide = {}
for i = 2000, 1, -1 do
  wide[string.format("k%05d", i)] = i
end
cases[#cases + 1] = wide

-- ⑤ 10⁴ 实体集的 bundle 形状（与 codec.md §5 的基准集同形；这里只关心编码器
--    看到的形状，不关心它由谁构造——真实装配在基准 harness 里走 pkg.entity）
local bundle = { codec_version = "1", exported_from = "bench", entities = {}, kv = {}, checksum = "" }
for i = 1, 10000 do
  bundle.entities[i] = {
    id = i,
    schema = "bench",
    attrs = { attack = 12, defense = 7, speed = 5, level = 3, exp = i * 2, hp = i, max_hp = 100, weight = 1.5 },
    tags = { "boss", "enemy", "undead" },
  }
end
for i = 0, 999 do
  bundle.kv["quest." .. i] = "任务 " .. i
  bundle.kv["dlg." .. i] = i
end
cases[#cases + 1] = bundle

-- ⑥ fuzz：固定种子（xorshift32），2 000 条形状随机的值
local seed = 0x9e3779b9
local function next_random()
  seed = seed ~ (seed << 13) & 0xffffffff
  seed = seed ~ (seed >> 17)
  seed = seed ~ (seed << 5) & 0xffffffff
  return seed
end
local function fuzz(depth)
  local roll = next_random()
  if depth <= 0 or roll % 10 < 4 then
    local pick = roll % 8
    if pick == 0 then return (roll % 4001) - 2000 end
    if pick == 1 then return ((roll % 200001) / 1000.0) - 100.0 end
    if pick == 2 then return (roll % 2) == 0 end
    if pick == 3 then return string.char(roll % 256) end
    if pick == 4 then return "s" .. (roll % 1000) end
    if pick == 5 then return (roll % 2 == 0) and 1e300 or 5e-324 end
    if pick == 6 then return -0.0 end
    return 0.1
  end
  local table_value = {}
  local size = roll % 4
  if roll % 2 == 0 then
    for i = 1, size do table_value[i] = fuzz(depth - 1) end
  else
    for i = 1, size do table_value["k" .. i] = fuzz(depth - 1) end
  end
  return table_value
end
for _ = 1, 2000 do
  cases[#cases + 1] = fuzz(4)
end

-- 元方法段：原生必须整值回退，Lua 路径必须仍能编码
cases.metatables = {
  setmetatable({ 1, 2 }, {}),
  setmetatable({ a = 1 }, { __index = function() return 9 end }),
  setmetatable({}, { __pairs = function() return function() return nil end end }),
  { nested = setmetatable({}, { __index = function(_, key) return key end }) },
}
return cases
"#;
