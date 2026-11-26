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

    register_module(&lua, "pkg.entity.base", "entity/base.lua").expect("装 pkg.entity.base");
    register_module(&lua, "pkg.entity.schema", "entity/schema.lua").expect("装 pkg.entity.schema");
    register_module(&lua, "pkg.entity.record", "entity/record.lua").expect("装 pkg.entity.record");
    register_module(&lua, "pkg.entity.query", "entity/query.lua").expect("装 pkg.entity.query");
    register_module(&lua, "pkg.entity", "entity/init.lua").expect("装 pkg.entity");
    register_module(&lua, "pkg.codec.sha256", "codec/sha256.lua").expect("装 pkg.codec.sha256");
    register_module(&lua, "pkg.codec.runtime", "codec/runtime.lua").expect("装 pkg.codec.runtime");
    register_module(&lua, "pkg.codec.jsonenc", "codec/jsonenc.lua").expect("装 pkg.codec.jsonenc");
    register_module(&lua, "pkg.codec.jsondec", "codec/jsondec.lua").expect("装 pkg.codec.jsondec");
    register_module(&lua, "pkg.codec.bundle", "codec/bundle.lua").expect("装 pkg.codec.bundle");
    register_module(&lua, "pkg.codec.importing", "codec/importing.lua")
        .expect("装 pkg.codec.importing");
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

/// decode 双径比较：返回「用 Lua 径重编码 native 解出的值」的文本。
///
/// 三件事一次做完：① native `ygg.codec.decode` 必须成功（误拒即失败）；② 与
/// `pkg.codec.decode` 的结果**深度等价**（含数字子类型——`1` 与 `1.0` 是两个值）；
/// ③ 把 native 解出的值交回 Lua 径重编码，供调用方与原文比字节。
fn decode_via_both_paths(lua: &Lua, text: &[u8]) -> Result<Vec<u8>, mlua::Error> {
    let function: Function = lua
        .load(
            r#"
            local codec = require("pkg.codec")
            local function deep_equal(left, right)
              if type(left) ~= type(right) then return false end
              if type(left) ~= "table" then
                if type(left) == "number" then
                  return math.type(left) == math.type(right) and left == right
                end
                return left == right
              end
              local count_left, count_right = 0, 0
              for key, value in pairs(left) do
                count_left = count_left + 1
                if not deep_equal(value, right[key]) then return false end
              end
              for _ in pairs(right) do count_right = count_right + 1 end
              return count_left == count_right
            end
            return function(text)
              local native = ygg.codec.decode(text)
              local saved = ygg.codec
              ygg.codec = nil
              local ok, lua_value = pcall(codec.decode, text)
              local encoded_ok, encoded = pcall(codec.encode, native)
              ygg.codec = saved
              if not ok then error(lua_value, 0) end
              if not deep_equal(native, lua_value) then error("decode 结果不同构", 0) end
              if not encoded_ok then error(encoded, 0) end
              return encoded
            end
            "#,
        )
        .eval()?;
    let text = lua.create_string(text)?;
    let encoded: mlua::LuaString = function.call(text)?;
    Ok(encoded.as_bytes().to_vec())
}

#[test]
fn native_encode_matches_the_lua_path_byte_for_byte() {
    let (lua, cases) = build();
    let count = cases.raw_len();
    assert!(
        count >= 2_000,
        "语料不足：{count} 条（计划稿 §5.4 要求 2 000+）"
    );

    let mut compared = 0usize;
    for index in 1..=count {
        let value: Value = cases.raw_get(index as i64).expect("语料元素");
        // ① 合法值不得被原生误拒（误拒只在字节相等下看不见，只丢性能）
        let native = native_encode(&value)
            .unwrap_or_else(|error| panic!("第 {index} 条被原生误拒：{error}"));
        // ② 面包装与直调同一份字节
        let via_face = face_encode(&lua, &value)
            .unwrap_or_else(|error| panic!("第 {index} 条过面失败：{error}"));
        assert_eq!(native, via_face, "第 {index} 条：面包装与直调不一致");
        // ③ 与规范执行体逐字节相等
        let lua_bytes = lua_encode(&lua, &value)
            .unwrap_or_else(|error| panic!("第 {index} 条 Lua 径失败：{error}"));
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
    let metatables: Table = cases.raw_get("metatables").expect("语料带 metatables 段");
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

#[test]
fn decode_matches_the_lua_path_across_the_encode_corpus() {
    let (lua, cases) = build();
    let mut compared = 0usize;
    for index in 1..=cases.raw_len() {
        let value: Value = cases.raw_get(index as i64).expect("语料元素");
        // 原文由规范执行体产出（canonical 字节），再走 decode 双径
        let text = lua_encode(&lua, &value).expect("语料必须可编码");
        let reencoded = decode_via_both_paths(&lua, &text)
            .unwrap_or_else(|error| panic!("第 {index} 条 decode 双径失败：{error}"));
        assert_eq!(
            reencoded,
            text,
            "第 {index} 条：decode → 重编码与原文不同（输入 {:?}）",
            value.type_name()
        );
        compared += 1;
    }
    assert!(compared >= 2_000, "语料不足：{compared}");
}

#[test]
fn saved_saves_decode_and_reencode_byte_identically() {
    let (lua, _cases) = build();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codec-saves.jsonl");
    let raw = std::fs::read(&path).unwrap_or_else(|error| panic!("读 {}：{error}", path.display()));
    let mut documents = 0usize;
    for line in raw.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        // ① 老档必须能解（native 误拒 = 旧档不可读，这是最贵的一类缺陷）
        let decoded = ygg_sandbox::codec_face::decode(&lua, line).unwrap_or_else(|error| {
            panic!(
                "第 {documents} 份老档被原生拒：{error}（{}）",
                String::from_utf8_lossy(line)
            )
        });
        assert!(!matches!(decoded, Value::Nil));
        // ② 与 Lua 径同构；③ 重编码逐字节等于原文
        let reencoded = decode_via_both_paths(&lua, line).expect("双径 decode");
        assert_eq!(
            reencoded,
            line.to_vec(),
            "第 {documents} 份老档：重编码与原文不同（{}）",
            String::from_utf8_lossy(line)
        );
        documents += 1;
    }
    assert!(documents >= 10, "老档语料不足：{documents} 份");
}

#[test]
fn key_set_is_exactly_encode_decode_and_sha256() {
    let (lua, _cases) = build();
    let ygg: Table = lua.globals().get("ygg").expect("ygg 表");
    let face: Table = ygg.get("codec").expect("ygg.codec 表");
    let mut keys: Vec<String> = face
        .pairs::<String, Value>()
        .map(|pair| pair.expect("键值对").0)
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "decode".to_string(),
            "encode".to_string(),
            "sha256".to_string()
        ],
        "ygg.codec 的键集是闭集（契约 §3.1.4：恰好 3 个，没有逐节点 getter）"
    );
}

/// 面径的 `ygg.codec.sha256`（走 mlua 包装，原始字节进）。
fn face_sha256(lua: &Lua, bytes: &[u8]) -> Result<String, mlua::Error> {
    let function: Function = lua
        .load("return function(b) return ygg.codec.sha256(b) end")
        .eval()?;
    let text = lua.create_string(bytes)?;
    function.call(text)
}

/// Lua 径的 `pkg.codec.sha256`（临时摘掉 `ygg.codec` 逼它走纯 Lua 回退体）。
fn lua_sha256(lua: &Lua, bytes: &[u8]) -> Result<String, mlua::Error> {
    let function: Function = lua
        .load(
            r#"
            local codec = require("pkg.codec")
            return function(b)
              local saved = ygg.codec
              ygg.codec = nil
              local ok, out = pcall(codec.sha256, b)
              ygg.codec = saved
              if not ok then error(out, 0) end
              return out
            end
            "#,
        )
        .eval()?;
    let text = lua.create_string(bytes)?;
    function.call(text)
}

/// sha256 双径差分：填充边界集 + 语料字符串 + 64 KiB 伪随机，逐字节相等
/// （契约 §4.1 S19 行：两条路径对任一输入产出同一份 hex）。
#[test]
fn sha256_matches_the_lua_path_byte_for_byte() {
    let (lua, cases) = build();
    // 边界集：填充前一格 / 跨块 / 整块 / 整块±1 / 更大长度。
    let mut corpus: Vec<Vec<u8>> = [0usize, 1, 31, 55, 56, 63, 64, 65, 127, 128]
        .iter()
        .map(|&size| (0..size).map(|index| (index % 251) as u8).collect())
        .collect();
    // 64 KiB 伪随机（LCG，确定性）：整块多轮 + 非对齐尾，覆盖差分门禁的
    // 「≤ 64 KiB 保 Lua 径可跑」上限。
    let mut state: u64 = 0x0519_2026;
    let mut random = Vec::with_capacity(65_536);
    for _ in 0..65_536 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        random.push((state >> 33) as u8);
    }
    corpus.push(random);
    // 语料里的字符串值形状（同 encode 差分共语料：字符串是摘要的真实输入域）。
    let corpus_text: mlua::LuaString = lua
        .load(
            r#"
            return function(cases)
              local out = {}
              for _, case in pairs(cases) do
                if type(case) == "table" then
                  for _, value in pairs(case) do
                    if type(value) == "string" then out[#out + 1] = value end
                  end
                end
              end
              return table.concat(out, "\n")
            end
            "#,
        )
        .eval::<Function>()
        .expect("语料提取器")
        .call::<mlua::LuaString>(cases)
        .expect("语料字符串提取");
    corpus.push(corpus_text.as_bytes().to_vec());

    assert!(corpus.len() >= 12, "语料不足：{}", corpus.len());
    for bytes in &corpus {
        let native = face_sha256(&lua, bytes).expect("面径 sha256 不应拒绝字符串");
        let fallback = lua_sha256(&lua, bytes).expect("Lua 径 sha256");
        assert_eq!(native, fallback, "双径字节不等（len={}）", bytes.len());
        assert_eq!(native.len(), 64, "len={}", bytes.len());
    }
}

/// FIPS 180-4 附录 B + 填充边界向量，双径双向钉（面径与回退体各自对上
/// 发布值；差分测的是相等，这条钉的是绝对值）。
#[test]
fn sha256_known_answer_vectors_pin_both_paths() {
    let lua_state = build().0;
    let vectors: &[(&str, &str)] = &[
        (
            "",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            "abc",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        (
            "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
        ),
    ];
    for (input, expected) in vectors {
        let face = face_sha256(&lua_state, input.as_bytes()).expect("面径");
        let fallback = lua_sha256(&lua_state, input.as_bytes()).expect("Lua 径");
        assert_eq!(&face, expected, "面径 KAT（input={input:?}）");
        assert_eq!(&fallback, expected, "回退体 KAT（input={input:?}）");
    }
}

/// 半面退化：宿主只有 encode/decode 两键（旧宿主）⇒ `face.sha256` 缺席，
/// 包的 sha256 **按键独立**落回纯 Lua，不报错、产出钉值（R5）。
#[test]
fn sha256_falls_back_to_lua_on_a_half_face() {
    let (lua, _cases) = build();
    let hex: String = lua
        .load(
            r#"
            local codec = require("pkg.codec")
            local saved = ygg.codec
            ygg.codec = { encode = saved.encode, decode = saved.decode }
            local ok, out = pcall(codec.sha256, "abc")
            ygg.codec = saved
            if not ok then error(out, 0) end
            return out
            "#,
        )
        .eval()
        .expect("半面必须落回 Lua 而不是报错");
    assert_eq!(
        hex, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        "半面上的 sha256 必须与钉值相等"
    );
}

/// 面的自保分支：非字符串进面 ⇒ `YGG-CODEC-UNSUPPORTED`；包侧类型门先行，
/// 消费者看到的仍是 `YGG-PKG-CODEC-TYPE`（文案单源）。
#[test]
fn sha256_rejects_non_strings_and_the_package_gate_stays_the_single_source() {
    let (lua, _cases) = build();
    let message: String = lua
        .load(
            r#"
            local ok, err = pcall(ygg.codec.sha256, 42)
            if ok then error("面必须拒绝数字", 0) end
            return tostring(err)
            "#,
        )
        .eval()
        .expect("面径拒绝");
    assert!(
        message.contains("YGG-CODEC-UNSUPPORTED"),
        "面径错误码必须是 UNSUPPORTED：{message}"
    );
    let package_message: String = lua
        .load(
            r#"
            local codec = require("pkg.codec")
            local ok, err = pcall(codec.sha256, 42)
            if ok then error("包必须拒绝数字", 0) end
            return tostring(err)
            "#,
        )
        .eval()
        .expect("包径拒绝");
    assert!(
        package_message.starts_with("YGG-PKG-CODEC-TYPE"),
        "包侧文案必须仍是自己的码：{package_message}"
    );
}

#[test]
fn malformed_documents_are_rejected_by_the_native_path() {
    let (lua, _cases) = build();
    let bad: &[&[u8]] = &[
        b"",
        b"[1,2",
        b"{\"a\":1",
        b"{\"a\" 1}",
        b"{a:1}",
        b"[1 2]",
        b"1 2",
        b"tru",
        b"1.",
        b"1e",
        b"\"\\q\"",
        b"\"\\u12\"",
        b"{\"a\":null,\"b\":[null]}",
    ];
    for text in bad {
        let error = ygg_sandbox::codec_face::decode(&lua, text).expect_err("坏档必须被原生拒");
        assert_eq!(
            error.code(),
            ygg_sandbox::codec_face::CODEC_UNSUPPORTED,
            "{}",
            String::from_utf8_lossy(text)
        );
        // 包侧必须仍然给出它自己的文案（回退路径不变）
        let message: String = lua
            .load("local c = require('pkg.codec'); return function(t) local ok, e = pcall(c.decode, t); return tostring(ok) .. '|' .. tostring(e) end")
            .eval::<Function>()
            .expect("闭包")
            .call::<String>(lua.create_string(text).expect("字符串"))
            .expect("调用");
        assert!(
            message.starts_with("false|YGG-PKG-CODEC-TYPE"),
            "Lua 径的文案必须仍是包的码：{}",
            message
        );
    }
}

/// 规范序 fixture 的**第二个消费者**（第一个在 `crates/ygg-sim/tests/canonical_order.rs`）。
///
/// 判据与 Tier 1 侧完全一样：两边都等于**同一份 fixture** 的 `expected`，而不是
/// 「两边互相比」——后者只能证明两者一致，不能证明它们等于契约那条序。
#[test]
fn codec_key_order_follows_the_canonical_order_fixture() {
    let (lua, _cases) = build();
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/ygg-sim/fixtures/canonical-order.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读 {} 失败：{error}", path.display()));
    let fixture: serde_json::Value = serde_json::from_str(&raw).expect("fixture 是 JSON");
    let names: Vec<&str> = fixture["names"]
        .as_array()
        .expect("names 数组")
        .iter()
        .map(|item| item.as_str().expect("名字"))
        .collect();
    let expected: Vec<&str> = fixture["expected"]
        .as_array()
        .expect("expected 数组")
        .iter()
        .map(|item| item.as_str().expect("名字"))
        .collect();

    // 洗序插入（fixture 的 names 本身就是乱序），两条路径都必须给出字节序
    let table = lua.create_table().expect("建表");
    for (index, name) in names.iter().enumerate() {
        table.set(*name, index as i64 + 1).expect("写键");
    }
    let value = Value::Table(table);

    for (label, bytes) in [
        ("native", native_encode(&value).expect("合法表")),
        ("lua", lua_encode(&lua, &value).expect("Lua 径")),
    ] {
        let text = String::from_utf8(bytes).expect("键都是 UTF-8");
        // 输出形如 {"0":..,"9":..}：按键字面量在输出里的位置排序，得到出现序
        let mut seen: Vec<(usize, &str)> = names
            .iter()
            .filter_map(|name| {
                text.find(&format!("\"{name}\":"))
                    .map(|position| (position, *name))
            })
            .collect();
        seen.sort_by_key(|(position, _)| *position);
        let seen: Vec<&str> = seen.into_iter().map(|(_, name)| name).collect();
        assert_eq!(
            seen, expected,
            "{label} 径的键序必须等于 fixture 的期望序（codec.md §4.1 / sim-kernel §2）"
        );
    }
}
