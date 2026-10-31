//! M2.5 Godot 底座可行性 spike——gdext 绑定面（最小实证面）。
//!
//! **绑定型文件**：本文件绑定 `packages/mods/test-adventure` 的 agent 契约（事件名 / 结局前缀硬编码在字面量里）；换 Mod 需同步修改。通用工具（examples/agent.rs、src/playtest.rs）不在此列——它们从 pack.json 读契约。
//!
//! 绑定类 [`YggKernel`]：`load_mod(目录)` / `dispatch(cmd, payload)` /
//! 叙事与选择 getter。核心逻辑收在纯 Rust [`logic`] 模块——不经 Godot
//! 引擎也可原生测试（`cargo test` 走查 test-adventure，作为 headless
//! 运行之外的独立证据面）。
//!
//! 与内核的接驳面 = `ygg-sandbox::driver::KernelFacade` + dispatch JSON
//! 信封（与 playtest.rs / examples/adventure.rs 同构）。

use std::path::Path;

use godot::prelude::*;
use serde_json::Value as Json;
use ygg_sandbox::driver::KernelFacade;

// ───────────────────────── 纯 Rust 逻辑面（Godot 无关） ─────────────────────────

pub mod logic {
    use super::*;

    /// 复刻 playtest::read_mod_bundle：Mod 目录五件套读取
    /// （pack.json 声明面 → mod.lua / manifest.json / schemas / resources）。
    pub fn read_mod_bundle(
        dir: &Path,
    ) -> Result<(String, Vec<u8>, Vec<(String, String)>, Vec<(String, Vec<u8>)>), String> {
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

    /// 信封拆解（getter 缓存面）：narrative 行数组 + pendingChoice。
    /// 返回 (叙事行, Option<(题面, 选项列表)>)。
    pub fn envelope_view(envelope: &str) -> (Vec<String>, Option<(String, Vec<String>)>) {
        let value: Json = serde_json::from_str(envelope).unwrap_or(Json::Null);
        let narrative = value["narrative"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let choice = value["pendingChoice"].as_object().map(|obj| {
            let prompt = obj
                .get("prompt")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_owned();
            let options = obj
                .get("options")
                .and_then(Json::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            (prompt, options)
        });
        (narrative, choice)
    }
}

// ───────────────────────── gdext 绑定面 ─────────────────────────

struct YggSpike;

#[gdextension]
unsafe impl ExtensionLibrary for YggSpike {}

/// 内核驱动绑定（最小面）：装载 / 派发 / 叙事与选择 getter。
#[derive(GodotClass)]
#[class(base = Node)]
struct YggKernel {
    /// 内核实例（load_mod 成功后非 None）。
    facade: Option<KernelFacade>,
    /// 最近一次信封的叙事行（getter 缓存）。
    narrative: Vec<String>,
    /// 最近一次信封的待决选择题面。
    prompt: String,
    /// 最近一次信封的待决选择选项。
    options: Vec<String>,
    base: Base<Node>,
}

#[godot_api]
impl INode for YggKernel {
    fn init(base: Base<Node>) -> Self {
        Self {
            facade: None,
            narrative: Vec::new(),
            prompt: String::new(),
            options: Vec::new(),
            base,
        }
    }
}

#[godot_api]
impl YggKernel {
    /// 装载 Mod 目录（pack.json 五件套 → with_resources 全序）。
    /// 返回 JSON 状态串 `{"ok":true}` / `{"ok":false,"error":...}`。
    #[func]
    fn load_mod(&mut self, dir_path: GString) -> GString {
        match logic::load_facade(Path::new(&dir_path.to_string())) {
            Ok(facade) => {
                self.facade = Some(facade);
                self.narrative.clear();
                self.prompt.clear();
                self.options.clear();
                GString::from("{\"ok\":true}")
            }
            Err(message) => GString::from(format!(
                "{{\"ok\":false,\"error\":{}}}",
                serde_json::to_string(&message).unwrap_or_else(|_| "\"load failed\"".into())
            ).as_str()),
        }
    }

    /// 唯一同步入口（A1）：透传 dispatch JSON 信封；同步刷新
    /// narrative / prompt / options getter 缓存。未装载 → NO_MOD 信封。
    #[func]
    fn dispatch(&mut self, cmd: GString, payload_json: GString) -> GString {
        let Some(facade) = self.facade.as_mut() else {
            return GString::from(
                "{\"ok\":false,\"error\":{\"code\":\"NO_MOD\",\"message\":\"load_mod 未调用\"}}",
            );
        };
        let envelope = facade.dispatch(&cmd.to_string(), &payload_json.to_string());
        let (narrative, choice) = logic::envelope_view(&envelope);
        self.narrative = narrative;
        match choice {
            Some((prompt, options)) => {
                self.prompt = prompt;
                self.options = options;
            }
            None => {
                self.prompt.clear();
                self.options.clear();
            }
        }
        GString::from(envelope.as_str())
    }

    /// 最近一次信封的叙事行（空 = 无叙事/未派发）。
    #[func]
    fn narrative_lines(&self) -> PackedStringArray {
        self.narrative.iter().map(GString::from).collect()
    }

    /// 待决选择题面（无选择 = 空串）。
    #[func]
    fn choice_prompt(&self) -> GString {
        GString::from(self.prompt.as_str())
    }

    /// 待决选择选项（无选择 = 空）。
    #[func]
    fn choice_options(&self) -> PackedStringArray {
        self.options.iter().map(GString::from).collect()
    }
}

// ───────────────────────── 原生测试（headless 之外的第二证据面） ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn mod_dir() -> std::path::PathBuf {
        std::env::var_os("YGG_MOD_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../../packages/mods/test-adventure")
            })
    }

    /// 绑定逻辑直调走查：装载 → event.start（叙事非空 + 待决选择）
    /// → ygg.choice_selected(1)（叙事推进）——与 headless 运行同路径。
    #[test]
    fn drive_first_choice_through_binding_logic() {
        let mut facade = logic::load_facade(&mod_dir()).expect("装载 test-adventure");

        let start = facade.dispatch("event.start", "0");
        let start_json: Json = serde_json::from_str(&start).expect("start 信封可解析");
        assert_eq!(start_json["ok"], Json::Bool(true), "event.start 失败：{start}");
        let narrative = start_json["narrative"].as_array().expect("narrative 数组");
        assert!(!narrative.is_empty(), "序章叙事非空");
        let pending = start_json["pendingChoice"].as_object().expect("存在待决选择");
        let options = pending["options"].as_array().expect("options 数组");
        assert!(!options.is_empty(), "待决选择有非空选项");

        let picked = facade.dispatch("ygg.choice_selected", "1");
        let picked_json: Json = serde_json::from_str(&picked).expect("choice 信封可解析");
        assert_eq!(picked_json["ok"], Json::Bool(true), "choice(1) 失败：{picked}");
        assert!(
            picked_json["narrative"]
                .as_array()
                .is_some_and(|arr| !arr.is_empty()),
            "选择后叙事推进非空"
        );

        // envelope_view（getter 面）同信封可拆。
        let (lines, choice) = logic::envelope_view(&start);
        assert_eq!(lines.len(), narrative.len());
        assert!(choice.is_some());
    }
}
