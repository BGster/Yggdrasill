//! Yggdrasill Godot 客户端绑定面（gdext）。
//!
//! 绑定类 [`YggKernel`]：`load_mod(目录)` / `dispatch(cmd, payload)` /
//! `save_snapshot()` / `load_snapshot(存档文本)` / 叙事与选择 getter。
//! 全部重逻辑收在纯 Rust [`logic`] 模块——不经 Godot 引擎也可 `cargo test`
//! 直调走查（`crates/` 只读，本 crate 只做绑定 + 缓存，不复制内核语义）。
//!
//! 与内核的接驳面 = `ygg_sandbox::driver::KernelFacade` + dispatch JSON 信封。

use std::path::Path;

use godot::prelude::*;
use serde_json::Value as Json;
use ygg_sandbox::driver::KernelFacade;

pub mod logic;

struct YggGodot;

#[gdextension]
unsafe impl ExtensionLibrary for YggGodot {}

/// 内核驱动绑定：装载 / 派发 / 存读档 / 叙事与选择 getter。
///
/// 缓存面语义：`dispatch` 后 `narrative_lines()` 只含**本次转换**产生的行
/// （内核信封口径），历史叙事由 UI 会话层累积——绑定层不做拼接，避免把
/// 「面板展示策略」焊进绑定。
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
    /// 最近一次信封的 D1 警告行。
    warnings: Vec<String>,
    /// 最近一次失败的人读文案（GDScript 侧展示用）。
    last_error: String,
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
            warnings: Vec::new(),
            last_error: String::new(),
            base,
        }
    }
}

#[godot_api]
impl YggKernel {
    /// 装载 Mod 目录（pack.json 五件套 → with_resources 全序）。
    /// 返回状态信封 JSON：`{"ok":true}` / `{"ok":false,"error":"..."}`。
    #[func]
    fn load_mod(&mut self, dir_path: GString) -> GString {
        let dir = dir_path.to_string();
        match logic::load_facade(Path::new(&dir)) {
            Ok(facade) => {
                self.facade = Some(facade);
                self.clear_caches();
                self.last_error.clear();
                GString::from("{\"ok\":true}")
            }
            Err(message) => {
                self.last_error = message.clone();
                GString::from(status_error(&message).as_str())
            }
        }
    }

    /// 唯一同步入口：透传 dispatch JSON 信封；同步刷新叙事/选择/警告缓存。
    /// 未装载 → `code=NO_MOD` 信封（永不 throw）。
    #[func]
    fn dispatch(&mut self, cmd: GString, payload_json: GString) -> GString {
        let Some(facade) = self.facade.as_mut() else {
            return GString::from(
                "{\"ok\":false,\"error\":{\"code\":\"NO_MOD\",\"message\":\"load_mod 未调用\"}}",
            );
        };
        let envelope = facade.dispatch(&cmd.to_string(), &payload_json.to_string());
        self.absorb_event_envelope(&envelope);
        GString::from(envelope.as_str())
    }

    /// 存档快照：返回存档 JSON **原文**（可直接喂给 [`Self::load_snapshot`]）。
    /// 失败 → 空串 + `last_error()` 携带原因。
    #[func]
    fn save_snapshot(&mut self) -> GString {
        let Some(facade) = self.facade.as_mut() else {
            self.last_error = "内核未装载".to_owned();
            return GString::from("");
        };
        match logic::save_snapshot(facade) {
            Ok(text) => {
                self.last_error.clear();
                GString::from(text.as_str())
            }
            Err(message) => {
                self.last_error = message;
                GString::from("")
            }
        }
    }

    /// 读档：入参 = [`Self::save_snapshot`] 原文；返回 load 信封（原样）。
    /// 成功后选择缓存取信封内 `pendingChoice`（待决选择随存档往返）。
    #[func]
    fn load_snapshot(&mut self, save_json: GString) -> GString {
        let Some(facade) = self.facade.as_mut() else {
            return GString::from(
                "{\"ok\":false,\"error\":{\"code\":\"NO_MOD\",\"message\":\"load_mod 未调用\"}}",
            );
        };
        let envelope = logic::load_snapshot(facade, &save_json.to_string());
        self.absorb_load_envelope(&envelope);
        GString::from(envelope.as_str())
    }

    /// 读档（存档槽文本）：`{"kernel": {...}, "log": [...]}` 或裸快照。
    ///
    /// 内核快照段在 Rust 侧用 serde 提取（不经 Godot 的 JSON 数值模型——
    /// Godot 把所有 JSON 数字解析为 float，u64 字段会退化成 `6.0` 被内核拒），
    /// 日志段由 GDScript 侧自行解析。
    #[func]
    fn load_bundle(&mut self, bundle_text: GString) -> GString {
        let Some(facade) = self.facade.as_mut() else {
            return GString::from(
                "{\"ok\":false,\"error\":{\"code\":\"NO_MOD\",\"message\":\"load_mod 未调用\"}}",
            );
        };
        let envelope = match logic::bundle_kernel_text(&bundle_text.to_string()) {
            Ok(kernel_text) => logic::load_snapshot(facade, &kernel_text),
            Err(message) => {
                self.last_error = message.clone();
                status_error(&message)
            }
        };
        self.absorb_load_envelope(&envelope);
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

    /// 最近一次信封的 D1 警告行（空 = 无警告）。
    #[func]
    fn warnings(&self) -> PackedStringArray {
        self.warnings.iter().map(GString::from).collect()
    }

    /// 最近一次失败的人读文案（成功路径恒空）。
    #[func]
    fn last_error(&self) -> GString {
        GString::from(self.last_error.as_str())
    }

    /// 是否已装载 Mod。
    #[func]
    fn is_loaded(&self) -> bool {
        self.facade.is_some()
    }

    /// 事件信封 → 缓存面（narrative / pendingChoice / warnings）。
    fn absorb_event_envelope(&mut self, envelope: &str) {
        let value: Json = serde_json::from_str(envelope).unwrap_or(Json::Null);
        if value["ok"] != Json::Bool(true) {
            // 错误信封：叙事为空、保留上一次选择面（UI 可重试），落错误文案。
            self.narrative.clear();
            self.warnings.clear();
            self.last_error = error_message(&value);
            return;
        }
        self.last_error.clear();
        let (narrative, choice, warnings) = logic::envelope_view(envelope);
        self.narrative = narrative;
        self.warnings = warnings;
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
    }

    fn clear_caches(&mut self) {
        self.narrative.clear();
        self.prompt.clear();
        self.options.clear();
        self.warnings.clear();
    }

    /// 读档信封 → 缓存面：读档不产生叙事（清空），选择面取 `pendingChoice`。
    fn absorb_load_envelope(&mut self, envelope: &str) {
        self.narrative.clear();
        self.warnings.clear();
        let value: Json = serde_json::from_str(envelope).unwrap_or(Json::Null);
        self.last_error = if value["ok"] == Json::Bool(true) {
            String::new()
        } else {
            error_message(&value)
        };
        match logic::choice_from(&value["pendingChoice"]) {
            Some((prompt, options)) => {
                self.prompt = prompt;
                self.options = options;
            }
            None => {
                self.prompt.clear();
                self.options.clear();
            }
        }
    }
}

/// `{"ok":false,"error":"..."}` 状态信封（load_mod 用）。
fn status_error(message: &str) -> String {
    format!(
        "{{\"ok\":false,\"error\":{}}}",
        serde_json::to_string(message).unwrap_or_else(|_| "\"load failed\"".into())
    )
}

/// 错误信封的人读文案（`error.message`；缺失回退整串）。
fn error_message(value: &Json) -> String {
    value["error"]["message"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("内核错误：{value}"))
}
