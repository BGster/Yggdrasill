//! 纯 Rust 逻辑面（Godot 无关）：Mod 装载、信封拆解、存读档。
//!
//! **绑定型文件**：本文件绑定 `packages/mods/test-adventure` 的 agent 契约（事件名 / 结局前缀硬编码在字面量里）；换 Mod 需同步修改。通用工具（examples/agent.rs、src/playtest.rs）不在此列——它们从 pack.json 读契约。
//!
//! 与内核的接驳面 = `ygg_sandbox::driver::KernelFacade` + dispatch JSON 信封
//! （与 playtest.rs / examples/adventure.rs 同构）。本模块刻意不 `use godot`：
//! `cargo test` 不经 Godot 引擎即可直调走查（headless 运行之外的第二证据面）。

use std::path::Path;

use serde_json::Value as Json;
use ygg_sandbox::driver::KernelFacade;

/// 结局叙事行前缀（与 playtest.rs / client-web 同源同值）。
pub const ENDING_PREFIX: &str = "【结局";

/// Mod 目录五件套（pack.json 声明面 → mod.lua / manifest.json / schemas / resources）。
pub type ModBundle = (
    String,
    Vec<u8>,
    Vec<(String, String)>,
    Vec<(String, Vec<u8>)>,
);

/// 复刻 playtest::read_mod_bundle：按 pack.json 声明读全 Mod 资源。
pub fn read_mod_bundle(dir: &Path) -> Result<ModBundle, String> {
    let pack_text = std::fs::read_to_string(dir.join("pack.json"))
        .map_err(|err| format!("读取 pack.json 失败：{err}"))?;
    let pack: Json = serde_json::from_str(&pack_text)
        .map_err(|err| format!("pack.json 不是合法 JSON：{err}"))?;
    let entries = pack["entries"]
        .as_array()
        .ok_or_else(|| "pack.json 缺少 entries 数组".to_owned())?;
    let schema_files = pack["schemaFiles"]
        .as_array()
        .ok_or_else(|| "pack.json 缺少 schemaFiles 数组".to_owned())?;

    let mod_source = std::fs::read_to_string(dir.join("mod.lua"))
        .map_err(|err| format!("读取 mod.lua 失败：{err}"))?;
    let manifest_bytes = std::fs::read(dir.join("manifest.json"))
        .map_err(|err| format!("读取 manifest.json 失败：{err}"))?;

    let mut schemas = Vec::new();
    for file in schema_files {
        let rel = file
            .as_str()
            .ok_or_else(|| "schemaFiles 必须是字符串数组".to_owned())?;
        let text = std::fs::read_to_string(dir.join(rel))
            .map_err(|err| format!("读取 schema 文件 {rel} 失败：{err}"))?;
        let parsed: Json = serde_json::from_str(&text)
            .map_err(|err| format!("schema 文件 {rel} 不是合法 JSON：{err}"))?;
        let id = parsed["$id"]
            .as_str()
            .ok_or_else(|| format!("schema 文件 {rel} 缺少 $id"))?
            .to_owned();
        schemas.push((id, text));
    }

    let mut resources = Vec::new();
    for entry in entries {
        let rel = entry["path"]
            .as_str()
            .ok_or_else(|| "entries[].path 必须是字符串".to_owned())?;
        let bytes = std::fs::read(dir.join(rel))
            .map_err(|err| format!("读取资源文件 {rel} 失败：{err}"))?;
        resources.push((rel.to_owned(), bytes));
    }
    Ok((mod_source, manifest_bytes, schemas, resources))
}

/// 目录装载 → [`KernelFacade`]（with_resources 全序，fail-fast）。
pub fn load_facade(dir: &Path) -> Result<KernelFacade, String> {
    let (mod_source, manifest_bytes, schemas, resources) = read_mod_bundle(dir)?;
    KernelFacade::with_resources(&mod_source, &manifest_bytes, &schemas, &resources)
        .map_err(|err| format!("装载失败：{err}"))
}

/// 待决选择视图（题面 + 选项列表）。
pub type ChoiceView = (String, Vec<String>);

/// 事件信封拆解（getter 缓存面）：叙事行 / 待决选择 / D1 警告行。
///
/// 信封形态见 `driver::cmd_event`：`narrative` 只含**本次转换**产生的行
/// （历史叙事由调用方累积），故本函数不做去重/拼接。
pub fn envelope_view(envelope: &str) -> (Vec<String>, Option<ChoiceView>, Vec<String>) {
    let value: Json = serde_json::from_str(envelope).unwrap_or(Json::Null);
    let narrative = string_array(&value["narrative"]);
    let warnings = string_array(&value["warnings"]);
    (narrative, choice_from(&value["pendingChoice"]), warnings)
}

/// `pendingChoice` 字段 → 选择视图（null / 缺字段 / 非对象 → None）。
pub fn choice_from(node: &Json) -> Option<ChoiceView> {
    let obj = node.as_object()?;
    let prompt = obj
        .get("prompt")
        .and_then(Json::as_str)
        .unwrap_or_default()
        .to_owned();
    Some((
        prompt,
        string_array(obj.get("options").unwrap_or(&Json::Null)),
    ))
}

/// JSON 数组 → String 向量（非字符串项丢弃）。
fn string_array(node: &Json) -> Vec<String> {
    node.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// 存档快照：`dispatch("save")` 信封的 `save` 字段 → 原始 JSON 文本。
///
/// 返回文本可直接作为 [`load_snapshot`] 的入参（驱动侧契约：`save` 字段
/// 与存档 bytes 逐字段一致）。**刻意不经 Godot 的 JSON 类型往返**——Variant
/// 的数值类型（int/float）与内核 serde 面不完全对齐，往返会引入字节漂移。
pub fn save_snapshot(facade: &mut KernelFacade) -> Result<String, String> {
    let envelope = facade.dispatch("save", "0");
    let value: Json =
        serde_json::from_str(&envelope).map_err(|err| format!("save 信封不是合法 JSON：{err}"))?;
    if value["ok"] != Json::Bool(true) {
        return Err(format!("save 被驱动拒绝：{envelope}"));
    }
    serde_json::to_string(&value["save"]).map_err(|err| format!("存档序列化失败：{err}"))
}

/// 读档：存档 JSON 文本 → `dispatch("load")` 信封（原样回传）。
pub fn load_snapshot(facade: &mut KernelFacade, save_json: &str) -> String {
    facade.dispatch("load", save_json)
}

/// 存档槽（`{"kernel": {...}, "log": [...]}` 或裸快照）→ 内核快照原文。
///
/// **为什么在 Rust 侧拆**：Godot 的 `JSON.parse_string` 把所有 JSON 数字
/// 解析为 float，`6` → `6.0`；存档面有 u64 字段（revision / entity id /
/// format_version），这种文本再喂回内核会被拒（`invalid type: floating
/// point 1.0, expected u64`，实证）。故客户端把存档槽文本**整体**交给本
/// 函数，由 serde 按 int/float 原样分离出 `kernel` 段——内核快照字面值
/// 不经 Godot 数值模型往返。
pub fn bundle_kernel_text(bundle_text: &str) -> Result<String, String> {
    let value: Json =
        serde_json::from_str(bundle_text).map_err(|err| format!("存档槽不是合法 JSON：{err}"))?;
    let kernel = match value.get("kernel") {
        Some(section) if section.is_object() => section.clone(),
        Some(_) => return Err("存档槽 kernel 段不是对象".to_owned()),
        None => value,
    };
    if !kernel.is_object() {
        return Err("存档槽不包含内核快照（既无 kernel 段也不是快照对象）".to_owned());
    }
    serde_json::to_string(&kernel).map_err(|err| format!("存档快照序列化失败：{err}"))
}

/// 叙事行是否已到结局（【结局 前缀；空白裁剪后判定）。
pub fn is_ending_line(line: &str) -> bool {
    line.trim_start().starts_with(ENDING_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mod_dir() -> std::path::PathBuf {
        std::env::var_os("YGG_MOD_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../packages/mods/test-adventure")
            })
    }

    fn start(facade: &mut KernelFacade) -> Json {
        let raw = facade.dispatch("event.start", "0");
        let value: Json = serde_json::from_str(&raw).expect("start 信封可解析");
        assert_eq!(value["ok"], Json::Bool(true), "event.start 失败：{raw}");
        value
    }

    fn choose(facade: &mut KernelFacade, index: usize) -> Json {
        let raw = facade.dispatch("ygg.choice_selected", &index.to_string());
        let value: Json = serde_json::from_str(&raw).expect("choice 信封可解析");
        assert_eq!(value["ok"], Json::Bool(true), "choice({index}) 失败：{raw}");
        value
    }

    /// 绑定逻辑直调走查：装载 → event.start（叙事非空 + 待决选择）
    /// → ygg.choice_selected(1)（叙事推进）——与 headless 运行同路径。
    #[test]
    fn drive_first_choice_through_binding_logic() {
        let mut facade = load_facade(&mod_dir()).expect("装载 test-adventure");

        let start_envelope = facade.dispatch("event.start", "0");
        let start_json: Json = serde_json::from_str(&start_envelope).expect("start 信封可解析");
        assert_eq!(
            start_json["ok"],
            Json::Bool(true),
            "event.start 失败：{start_envelope}"
        );
        let narrative = start_json["narrative"].as_array().expect("narrative 数组");
        assert!(!narrative.is_empty(), "序章叙事非空");
        let pending = start_json["pendingChoice"]
            .as_object()
            .expect("存在待决选择");
        let options = pending["options"].as_array().expect("options 数组");
        assert!(!options.is_empty(), "待决选择有非空选项");

        let picked = choose(&mut facade, 1);
        assert!(
            picked["narrative"]
                .as_array()
                .is_some_and(|arr| !arr.is_empty()),
            "选择后叙事推进非空"
        );

        // envelope_view（getter 面）同信封可拆。
        let (lines, choice, warnings) = envelope_view(&start_envelope);
        assert_eq!(lines.len(), narrative.len());
        assert!(choice.is_some());
        assert!(warnings.is_empty(), "首帧无 D1 警告");
    }

    /// 通关路径：选择序列 [1, 1] → 结局 A（提灯 → 地窖 → 灯下秘藏）。
    #[test]
    fn walkthrough_reaches_ending_a() {
        let mut facade = load_facade(&mod_dir()).expect("装载 test-adventure");
        start(&mut facade);
        for index in [1usize, 1] {
            let reply = choose(&mut facade, index);
            let lines: Vec<String> = reply["narrative"]
                .as_array()
                .expect("narrative 数组")
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
            if lines.iter().any(|line| is_ending_line(line)) {
                assert!(
                    lines.iter().any(|line| line.contains("【结局A】")),
                    "达成结局 A"
                );
                assert_eq!(reply["pendingChoice"], Json::Null, "结局后无待决选择");
                return;
            }
        }
        panic!("选择序列 [1,1] 未到达结局");
    }

    /// 存读档一致性：中途存档 → 新 facade 装载 → load → 状态指纹一致。
    ///
    /// 状态指纹取 `state` 信封的 revision / kvRevision / pendingChoice 与
    /// 快照字节数——与 GDScript 侧状态栏显示同一口径。
    #[test]
    fn save_then_load_restores_state_fingerprint() {
        let dir = mod_dir();
        let mut facade = load_facade(&dir).expect("装载 test-adventure");
        start(&mut facade);
        choose(&mut facade, 1); // 提灯 → 进入 crossroad 选择点（中途存档点）

        let fingerprint_before = fingerprint(&mut facade);
        let save_json = save_snapshot(&mut facade).expect("存档");

        // 进程内「重开一局」：全新 facade（模拟退出后重启）→ 读档覆写。
        let mut fresh = load_facade(&dir).expect("重新装载");
        let reply = load_snapshot(&mut fresh, &save_json);
        let reply_json: Json = serde_json::from_str(&reply).expect("load 信封可解析");
        assert_eq!(reply_json["ok"], Json::Bool(true), "load 失败：{reply}");
        assert_eq!(reply_json["restored"], Json::Bool(true));

        assert_eq!(
            fingerprint(&mut fresh),
            fingerprint_before,
            "读档后状态指纹一致"
        );
        // 读档后待决选择可继续推进（pendingChoice 随存档往返）。
        let reply_choice = choice_from(&reply_json["pendingChoice"]).expect("待决选择已恢复");
        assert!(!reply_choice.1.is_empty(), "恢复的选项非空");

        // 读档后继续按通关路径走完 → 结局 A（状态面真恢复，不只是字节相等）。
        let after = choose(&mut fresh, 1);
        assert!(
            after["narrative"].as_array().is_some_and(|arr| arr
                .iter()
                .any(|line| line.as_str().is_some_and(is_ending_line))),
            "读档后继续推进到达结局：{after}"
        );
    }

    /// 存档往返字节稳定（同状态两次存档 = 同文本）。
    #[test]
    fn save_snapshot_is_deterministic() {
        let mut facade = load_facade(&mod_dir()).expect("装载 test-adventure");
        start(&mut facade);
        let first = save_snapshot(&mut facade).expect("存档 1");
        let second = save_snapshot(&mut facade).expect("存档 2");
        assert_eq!(first, second);
    }

    /// 存档槽往返：内核快照嵌在槽文档里 → 拆分 → 新 facade 读档 → 指纹一致。
    ///
    /// 槽文档由 GDScript 侧拼接（快照原文 + 日志数组），本测试复刻同一形态；
    /// 关键面是「槽文档里的内核数值不被改写」（u64 字段仍是整数，不是 `6.0`）。
    #[test]
    fn bundle_kernel_text_round_trips_through_slot_document() {
        let dir = mod_dir();
        let mut facade = load_facade(&dir).expect("装载 test-adventure");
        start(&mut facade);
        choose(&mut facade, 1);
        let before = fingerprint(&mut facade);
        let snapshot = save_snapshot(&mut facade).expect("存档");

        let slot = format!(
            "{{\n  \"kind\": \"ygg.godot.save\",\n  \"version\": 1,\n  \"steps\": 1,\n  \"ended\": false,\n  \"kernel\": {snapshot},\n  \"log\": [{{\"kind\": \"narrative\", \"text\": \"序章\"}}]\n}}\n"
        );
        let kernel_text = bundle_kernel_text(&slot).expect("拆分内核快照");
        assert!(
            kernel_text.contains("\"revision\":6") && !kernel_text.contains("6.0"),
            "快照数值保持整数形态：{kernel_text}"
        );

        let mut fresh = load_facade(&dir).expect("重新装载");
        let reply = load_snapshot(&mut fresh, &kernel_text);
        let reply_json: Json = serde_json::from_str(&reply).expect("load 信封可解析");
        assert_eq!(reply_json["ok"], Json::Bool(true), "load 失败：{reply}");
        assert_eq!(fingerprint(&mut fresh), before, "槽文档往返后指纹一致");
    }

    /// 裸快照文本也可直接作槽文档（无 kernel 段时按快照本身解释）。
    #[test]
    fn bundle_kernel_text_accepts_bare_snapshot() {
        let mut facade = load_facade(&mod_dir()).expect("装载 test-adventure");
        start(&mut facade);
        let snapshot = save_snapshot(&mut facade).expect("存档");
        let extracted = bundle_kernel_text(&snapshot).expect("裸快照可拆");
        assert_eq!(extracted, snapshot);
    }

    /// 非 JSON / 缺 kernel 段 → 可读错误（不 panic）。
    #[test]
    fn bundle_kernel_text_rejects_garbage() {
        assert!(bundle_kernel_text("not json").is_err());
        assert!(bundle_kernel_text("{\"kernel\": 3}").is_err());
        assert!(bundle_kernel_text("[1, 2]").is_err());
    }

    /// 非法存档文本 → 驱动拒绝（VALIDATION 信封，不 panic）。
    #[test]
    fn load_rejects_garbage_save() {
        let mut facade = load_facade(&mod_dir()).expect("装载 test-adventure");
        start(&mut facade);
        let reply = load_snapshot(&mut facade, "{\"nope\":true}");
        let value: Json = serde_json::from_str(&reply).expect("错误信封可解析");
        assert_eq!(value["ok"], Json::Bool(false));
        assert_eq!(value["error"]["code"], Json::String("VALIDATION".into()));
    }

    fn fingerprint(facade: &mut KernelFacade) -> (Json, Json, Json, Json) {
        let raw = facade.dispatch("state", "0");
        let value: Json = serde_json::from_str(&raw).expect("state 信封可解析");
        assert_eq!(value["ok"], Json::Bool(true), "state 失败：{raw}");
        (
            value["revision"].clone(),
            value["kvRevision"].clone(),
            value["snapshotBytes"].clone(),
            value["pendingChoice"].clone(),
        )
    }
}
