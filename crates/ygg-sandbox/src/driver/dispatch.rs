use serde_json::{json, Value as Json};

use ygg_core::bus::{HandlerError, PendingChoice};
use ygg_core::entity::Value;
use ygg_core::event::Event;
use ygg_core::save::{SaveError, SaveFile};
use ygg_core::transition::{run_transition, TransitionError};

use super::{code, KernelError, KernelFacade};

use crate::sandbox::ENGINE_VERSION;

impl KernelFacade {
    /// 唯一同步入口（A1）：命令分发 + JSON 信封封装。域错误以信封返回
    /// （永不 throw）；重入 dispatch = 驱动 bug → INTERNAL（不静默递归）。
    pub fn dispatch(&mut self, cmd: &str, payload_json: &str) -> String {
        if self.dispatching {
            return error_envelope(code::INTERNAL, "re-entrant dispatch detected (driver bug)");
        }
        self.dispatching = true;
        let reply = self.dispatch_inner(cmd, payload_json);
        self.dispatching = false;
        reply
    }

    /// 命令分发（保留名 `save/load/state/log/drain`；其余一律按外部事件名
    /// 进入转换路径 [`KernelFacade::cmd_event`]）。
    fn dispatch_inner(&mut self, cmd: &str, payload_json: &str) -> String {
        match cmd {
            "save" => self.cmd_save(),
            "load" => self.cmd_load(payload_json),
            "state" => self.cmd_state(),
            "log" => self.cmd_log(),
            // 派生事件已在转换内 FIFO 排空：drain 恒空（线格式兼容保留）。
            "drain" => json!({"ok": true, "drained": []}).to_string(),
            _ => self.cmd_event(cmd, payload_json),
        }
    }

    /// `save`：完整状态快照（实体表 + KV + pending_choice，B3）序列化为
    /// bytes（确定性序）后原样解析回 Json 嵌入信封——信封内容与存档
    /// bytes 逐字段一致（读档方可直接把 `save` 字段作为 load payload）。
    fn cmd_save(&mut self) -> String {
        let save = SaveFile::new(
            ENGINE_VERSION,
            self.entities.borrow().clone(),
            self.kv.borrow().clone(),
            self.pending_choice.clone(),
        );
        let outcome = save
            .to_bytes()
            .and_then(|bytes| serde_json::from_slice::<Json>(&bytes).map_err(SaveError::Json));
        match outcome.map_err(KernelError::Save) {
            Ok(parsed) => json!({"ok": true, "save": parsed}).to_string(),
            Err(err) => error_envelope(code::INTERNAL, err),
        }
    }

    /// `load`：payload = 存档 JSON 字符串；版本/内容不符 → VALIDATION
    /// 信封（EngineMismatch / SaveError），成功覆写后 `{"ok":true,
    /// "restored":true,"pendingChoice":<同 state 形态>}`——待决选择
    /// 随存档往返（B3），读档方零额外 dispatch 即可恢复提示面（A1）。
    fn cmd_load(&mut self, payload_json: &str) -> String {
        let save = match SaveFile::from_bytes(payload_json.as_bytes()) {
            Ok(save) => save,
            Err(err) => return error_envelope(code::VALIDATION, format!("load rejected: {err}")),
        };
        match self.apply_save(&save) {
            Ok(()) => json!({
                "ok": true,
                "restored": true,
                "pendingChoice": self.pending_choice.as_ref().map(choice_to_json),
            })
            .to_string(),
            Err(KernelError::EngineMismatch { saved, current }) => error_envelope(
                code::VALIDATION,
                format!("load rejected: engine version mismatch: save written by `{saved}`, current engine `{current}`"),
            ),
            Err(err) => error_envelope(code::INTERNAL, err),
        }
    }

    /// `state`：诊断面——引擎/实体/KV 版本号 + 快照字节数 + 待决选择。
    fn cmd_state(&self) -> String {
        let snapshot_bytes = match self.canonical_state_bytes() {
            Ok(bytes) => bytes,
            Err(err) => return error_envelope(code::INTERNAL, err),
        };
        json!({
            "ok": true,
            "engineVersion": ENGINE_VERSION,
            "revision": self.entities.borrow().revision(),
            "kvRevision": self.kv.borrow().revision(),
            "snapshotBytes": snapshot_bytes.len(),
            "pendingChoice": self.pending_choice.as_ref().map(choice_to_json),
        })
        .to_string()
    }

    /// `log`：外部事件日志（仅外部事件；驱动旁路面，不入档）。
    fn cmd_log(&self) -> String {
        json!({"ok": true, "log": self.external_log}).to_string()
    }

    /// 事件转换路径（非保留名命令）：registry 预检（fail-closed）→ payload
    /// 解析（标量门）→ 外部事件构造 → 外部日志 → snapshot-out
    /// 转换 → 信封回包。
    fn cmd_event(&mut self, name: &str, payload_json: &str) -> String {
        // ⓪⁻ registry 预检（fail-closed 门面侧）：未注册事件名在**一切副作用
        //    之前**拒绝——不进外部日志（③）、不动状态基线（④）。
        //    「事件不存在」先于「序号越界 / 载荷非法」（registry 是运行时唯一
        //    事实，宿主不持事件名白名单：`ygg.*` 保留事件同走本预检，已注册者
        //    天然通过）。先 `collect` 再判断：`event_names()` 借用 `&self.registry`，
        //    收集完即释放，错误分支才能继续用 `self`。
        //    文案三要素：事件名原样 + 已注册计数 + 全集（字典序）——Agent 一次
        //    往返即可自纠（与 daemon unknown-op 全列合法 op 同先例）。
        let names: Vec<&str> = self.registry.event_names().collect();
        if !names.contains(&name) {
            return error_envelope(
                code::VALIDATION,
                format!(
                    "未注册事件 `{name}`：本 Mod 已注册 {} 个事件：{}",
                    names.len(),
                    names.join("、")
                ),
            );
        }
        // ⓪ `ygg.choice_selected` 域内校验（A2）：待决选择存在时，序号
        //    必须是 1..=options_len 的整数——越界/非整数 = 可读 VALIDATION
        //    （玩家视角「选项不存在」，不进内核转换、不改状态）。
        if name == "ygg.choice_selected" {
            if let Some(choice) = &self.pending_choice {
                let n = choice.options().len();
                let index = serde_json::from_str::<Json>(payload_json)
                    .ok()
                    .and_then(|v| v.as_i64());
                if !matches!(index, Some(i) if (1..=n as i64).contains(&i)) {
                    return error_envelope(
                        code::VALIDATION,
                        format!("选项不存在：有效范围 1..{n}"),
                    );
                }
            }
        }
        // ① payload 标量门：JSON 解析失败或非标量（嵌套对象/数组/null）
        //    → VALIDATION（core Value 只有四标量）。
        let payload_value: Json = match serde_json::from_str(payload_json) {
            Ok(value) => value,
            Err(err) => {
                return error_envelope(
                    code::VALIDATION,
                    format!("event payload is not valid JSON: {err}"),
                )
            }
        };
        let payload = match json_payload_to_value(&payload_value) {
            Ok(value) => value,
            Err(message) => return error_envelope(code::VALIDATION, message),
        };
        // ② 外部事件构造（判据先行；失败 → VALIDATION）。
        let event = match Event::external(name.to_owned(), payload.clone()) {
            Ok(event) => event,
            Err(err) => return error_envelope(code::VALIDATION, err),
        };
        // ③ 外部日志（仅外部事件；派生事件由回放重放时自动再产生）。
        self.external_log.push(json!({
            "name": event.name(),
            "payload": value_to_json(&payload),
        }));
        // ④ dispatch 级 savepoint，**取点**：校验（⓪①②）全过、副作用
        //    （③ 日志）已落之后，转换开始之前——在此冻结 canonical
        //    的两块状态面基线。基线是 canonical 的**副本**（不是换出）：
        //    handler 的即席写经 host 桥 `Rc` 直落 canonical，而 `ygg.kv.get`
        //    / `ygg.entity.*` 的读路径同样直读 canonical——换出会让转换期的
        //    handler 读到空表。实体基线同时充当 `run_transition` 的
        //    `&mut EntityStore` 形参（该形参在 facade 路径下只作 revision
        //    记账诱饵，handler 从不持有它的引用）。借用均为语句级临时
        //    `Ref`/`RefMut`，不跨越 `run_transition` 存活。
        let mut baseline = self.entities.borrow().clone();
        let kv_baseline = self.kv.borrow().clone();
        let outcome = run_transition(&mut baseline, &mut self.registry, event);
        // ④' D1：无论转换成败都取走本次 dispatch 的警告（choice 前置丢弃
        // 面；每 dispatch 至多一条，drain 即清空，不泄漏到后续 dispatch）。
        let warning = self.host.take_dispatch_warning();
        // ⑤ 裁定：成功 = **提交**（丢弃基线——canonical 保留本次全部写入）；
        //    失败 = **先还原 canonical、再返回信封**。还原语句必须落在错误
        //    分支内、且先于该分支的返回（`map_transition_error` 只把错误映射
        //    成信封，不碰状态）；把它挪到 match 之后（错误分支已返回，不可
        //    达）或只写在成功分支，savepoint 就完全不生效。
        let result = match outcome {
            Ok(result) => {
                drop(baseline);
                drop(kv_baseline);
                result
            }
            Err(err) => {
                *self.entities.borrow_mut() = baseline;
                *self.kv.borrow_mut() = kv_baseline;
                return map_transition_error(err);
            }
        };
        self.pending_choice = result.pending_choice().cloned();
        // ⑥ 信封：revision 在转换结束后从 canonical 现读
        //    （TransitionResult.revision_after 为快照克隆的冻结诊断值，
        //    不进信封——统一 store 代决的 revision 口径）。
        let revision = self.entities.borrow().revision();
        let mut envelope = json!({
            "ok": true,
            "event": result.external().name(),
            "narrative": result.narrative(),
            "pendingChoice": self.pending_choice.as_ref().map(choice_to_json),
            "eventsProcessed": result.events_processed(),
            "eventsDerived": result.events_derived(),
            "eventsDropped": result.events_dropped(),
            "revision": revision,
        });
        // D1：加性可选键——仅在非空时写入（serde_json 字典序键序，确定性
        // 不变：同输入 → 同警告 → 同字节）。
        if let Some(warning) = warning {
            envelope["warnings"] = json!([warning]);
        }
        envelope.to_string()
    }
}

/// 转换错误 → 稳定码信封（A1：下游按码分支，message 只作人读定位）。
///
/// Handler 面（mlua 桥把 Lua 侧错误包成 HandlerError）按文案归类：
/// "sandbox denied" / "memory error"（模块拒绝桩 / 内存上限）→
/// SANDBOX_DENIED；其余 handler 错误 → VALIDATION。NotExternal 为不可能
/// 路径（外部事件构造先行判据）→ INTERNAL。
fn map_transition_error(err: TransitionError) -> String {
    match err {
        TransitionError::Handler(HandlerError(msg)) => {
            if msg.starts_with("unregistered event") {
                // 未注册事件（bus fail-closed；外部名已被门面预检拦截，走到这里的
                // 都是派生面：handler `ygg.emit` 了未注册名）→ VALIDATION。显式
                // 分支钉住码面，不依赖末尾兜底；消息带 bus 稳定前缀 + 事件名。
                error_envelope(code::VALIDATION, msg)
            } else if msg.contains("sandbox denied") || msg.contains("memory error") {
                error_envelope(code::SANDBOX_DENIED, msg)
            } else {
                error_envelope(code::VALIDATION, msg)
            }
        }
        TransitionError::NotExternal(name) => error_envelope(
            code::INTERNAL,
            format!("internal bug: transition input must be an external event, got `{name}`"),
        ),
    }
}

/// JSON payload → core `Value`（标量门）：Bool / Number（整数优先取 i64，
/// 其余按 f64）/ String 直通；嵌套对象、数组、null → 可读 VALIDATION 文案。
fn json_payload_to_value(value: &Json) -> Result<Value, String> {
    match value {
        Json::Bool(b) => Ok(Value::Bool(*b)),
        Json::Number(n) => match n.as_i64() {
            Some(i) => Ok(Value::Int(i)),
            None => Ok(Value::Float(n.as_f64().unwrap_or_default())),
        },
        Json::String(s) => Ok(Value::Str(s.clone())),
        other => Err(format!(
            "event payload must be a scalar, got {}",
            json_type_name(other)
        )),
    }
}

/// JSON 值类型名（payload 标量门的可读报错文案）。
fn json_type_name(value: &Json) -> &'static str {
    match value {
        Json::Null => "null",
        Json::Bool(_) => "boolean",
        Json::Number(_) => "number",
        Json::String(_) => "string",
        Json::Array(_) => "array",
        Json::Object(_) => "object",
    }
}

/// core `Value` → JSON（诊断/日志序列化面）：四标量直通；Float 非有限值
/// 经 `Number::from_f64` 落 `null`（信封永不携带非法 JSON 数值——事件构造
/// 门已拒绝非有限值，此为序列化层纵深）。
fn value_to_json(value: &Value) -> Json {
    match value {
        Value::Int(i) => (*i).into(),
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(Json::Number)
            .unwrap_or(Json::Null),
        Value::Bool(b) => (*b).into(),
        Value::Str(s) => s.as_str().into(),
    }
}

/// 待决选择 → JSON（prompt + options 序保真）。
fn choice_to_json(choice: &PendingChoice) -> Json {
    json!({
        "prompt": choice.prompt(),
        "options": choice.options(),
    })
}

/// 成功信封碎片无统一形态（各命令字段不同），错误信封统一于此。
fn error_envelope(code_str: &str, message: impl std::fmt::Display) -> String {
    json!({
        "ok": false,
        "error": {"code": code_str, "message": message.to_string()},
    })
    .to_string()
}
