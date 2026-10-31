//! 无头走查（playtest）：任意 Mod 目录的装载 + 契约入口事件自动推进 +
//! 结构化报告（FR-6 验收门本体；D-12 走查策略）。
//!
//! **零硬编码**：走查用的事件名与结局行前缀全部来自该 Mod `pack.json` 的
//! `agent` 段（[`crate::host_contract::AgentContract`]）；缺失即返回
//! `ok=false` 的可读错误报告，**不**回退到任何内置事件名。
//!
//! 与内核的接驳面 = `KernelFacade::with_resources` + dispatch 信封（与
//! examples/adventure.rs 同构）；与管线的接驳面 = stdout JSON 报告 + 退出
//! 码（契约文本见 examples/playtest.rs 头注）——管线侧零内核符号依赖
//! （D-8：子进程边界即全部耦合）。
//!
//! 走查语义（D-12 定稿）：装载 → `dispatch(<agent.entryEvent>,"0")`（序章叙事
//! 非空为硬门）→ 循环{有待决选择 → 按（脚本下一位或默认 1）派发
//! `<agent.choiceEvent>`；叙事行含 `agent.endingPrefix` → 收口}。脚本耗尽停在
//! 当前选择点（reached=Choice）；无选择且未达结局 = DeadEnd；步数上限 = Limit。

use std::fs;
use std::path::Path;

use serde_json::{json, Value as Json};

use crate::driver::KernelFacade;
use crate::host_contract::AgentContract;

/// 默认步数上限（**宿主侧**的失控防护参数，不是 Mod 契约；`--max-steps` 可覆盖）。
pub const DEFAULT_MAX_STEPS: usize = 64;

/// 报告 narrativeSample 保留行数上限（报告体积面）。
const SAMPLE_LIMIT: usize = 8;

/// 走查选项。
#[derive(Debug, Clone)]
pub struct PlaytestOptions {
    /// 派发步数上限（每 dispatch 一步）。
    pub max_steps: usize,
    /// 选择脚本（1 起序号队列）；**非空且耗尽**后停在下一个待决选择点
    /// （reached = Choice）。空 = 全 1 贪心（默认，确定性最好）。
    pub choices: Vec<usize>,
}

impl Default for PlaytestOptions {
    fn default() -> Self {
        Self {
            max_steps: DEFAULT_MAX_STEPS,
            choices: Vec::new(),
        }
    }
}

/// 走查收口四态（D-12）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reached {
    /// 叙事行出现契约 `agent.endingPrefix` 前缀。
    Ending,
    /// 脚本耗尽，停在待决选择点。
    Choice,
    /// 步数上限（失败态）。
    Limit,
    /// 无待决选择且未达结局（死路，失败态）。
    DeadEnd,
}

impl Reached {
    pub fn as_str(self) -> &'static str {
        match self {
            Reached::Ending => "ending",
            Reached::Choice => "choice",
            Reached::Limit => "limit",
            Reached::DeadEnd => "deadend",
        }
    }

    fn is_success(self) -> bool {
        matches!(self, Reached::Ending | Reached::Choice)
    }
}

/// 走查报告（stdout JSON 契约的 Rust 侧本体；字段见 [`PlaytestReport::to_json`]）。
#[derive(Debug)]
pub struct PlaytestReport {
    pub ok: bool,
    pub mod_dir: String,
    pub steps: usize,
    pub reached: Option<Reached>,
    pub narrative_sample: Vec<String>,
    pub warnings: Vec<String>,
    pub events_processed: usize,
    /// 错误信封文案（含装载失败；ok=false 时通常非空——DeadEnd/Limit 除外）。
    pub errors: Vec<String>,
}

impl PlaytestReport {
    /// stdout JSON 契约（管线 playtest 门的解析面；字段名 camelCase）。
    pub fn to_json(&self) -> Json {
        json!({
            "ok": self.ok,
            "mod": self.mod_dir,
            "steps": self.steps,
            "reached": self.reached.map(Reached::as_str),
            "narrativeSample": self.narrative_sample,
            "warnings": self.warnings,
            "eventsProcessed": self.events_processed,
            "errors": self.errors,
        })
    }
}

/// 入口：读 Mod 目录五件套 → 装载 → 自动推进 → 报告（零 stdout 副作用，
/// 打印归 example 薄壳）。
pub fn playtest(mod_dir: &Path, options: &PlaytestOptions) -> PlaytestReport {
    let dir_label = mod_dir.display().to_string();
    let mut errors = Vec::new();
    let empty_report = |errors: Vec<String>| PlaytestReport {
        ok: false,
        mod_dir: dir_label.clone(),
        steps: 0,
        reached: None,
        narrative_sample: Vec::new(),
        warnings: Vec::new(),
        events_processed: 0,
        errors,
    };

    // ① 五件套读取（pack.json 声明面 → 装载参数；schema $id 从文件内容读）。
    let bundle = match read_mod_bundle(mod_dir) {
        Ok(bundle) => bundle,
        Err(message) => {
            errors.push(message);
            return empty_report(errors);
        }
    };

    // ①' host 契约（`pack.json` 的 `agent` 段，必填）：入口 / 选择事件名与结局
    //     前缀都从该 Mod 的声明里取。缺失即可读错误报告（不 panic、不兜底）。
    let contract = match AgentContract::load(mod_dir) {
        Ok(contract) => contract,
        Err(message) => {
            errors.push(message);
            return empty_report(errors);
        }
    };

    // ② 装载（fail-fast：预算/寻址/校验先于任何 Lua 状态）。
    let mut kernel = match KernelFacade::with_resources(
        &bundle.mod_source,
        &bundle.manifest_bytes,
        &bundle.schemas,
        &bundle.resources,
    ) {
        Ok(kernel) => kernel,
        Err(err) => {
            errors.push(format!("装载失败：{err}"));
            return empty_report(errors);
        }
    };

    // ③ 入口事件 → 序章非空硬门 → 选择驱动循环（事件名全来自契约）。
    let mut steps = 0usize;
    let mut events_processed = 0usize;
    let mut narrative_sample: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut choice_cursor = 0usize;
    let mut reached: Option<Reached> = None;

    let mut envelope = take_envelope(
        &mut kernel,
        &contract.entry_event,
        "0",
        &mut errors,
        &mut events_processed,
    );
    collect_narrative(&envelope, &mut narrative_sample);
    collect_warnings(&envelope, &mut warnings);
    steps += 1;
    let prologue_lines = envelope["narrative"].as_array().map_or(0, |lines| {
        lines
            .iter()
            .filter(|value| value.as_str().is_some_and(|line| !line.trim().is_empty()))
            .count()
    });
    if has_ending_line(&envelope, &contract.ending_prefix) {
        reached = Some(Reached::Ending);
    }

    while reached.is_none() && errors.is_empty() && steps < options.max_steps {
        let Some(option_count) = pending_choice_option_count(&envelope) else {
            reached = Some(Reached::DeadEnd);
            break;
        };
        if !options.choices.is_empty() && choice_cursor >= options.choices.len() {
            reached = Some(Reached::Choice); // 脚本耗尽：停在当前选择点。
            break;
        }
        let _ = option_count; // 越界序号交由内核域校验回 VALIDATION（收口为失败）。
        let picked = options.choices.get(choice_cursor).copied().unwrap_or(1);
        choice_cursor += 1;
        envelope = take_envelope(
            &mut kernel,
            &contract.choice_event,
            &picked.to_string(),
            &mut errors,
            &mut events_processed,
        );
        collect_narrative(&envelope, &mut narrative_sample);
        collect_warnings(&envelope, &mut warnings);
        steps += 1;
        if has_ending_line(&envelope, &contract.ending_prefix) {
            reached = Some(Reached::Ending);
        }
    }
    if reached.is_none() && errors.is_empty() {
        reached = Some(Reached::Limit); // 步数上限兜底。
    }

    let ok = errors.is_empty() && prologue_lines >= 1 && reached.is_some_and(Reached::is_success);
    PlaytestReport {
        ok,
        mod_dir: dir_label,
        steps,
        reached,
        narrative_sample,
        warnings,
        events_processed,
        errors,
    }
}

/// dispatch 一步：解析信封、聚合错误与 eventsProcessed。
fn take_envelope(
    kernel: &mut KernelFacade,
    cmd: &str,
    payload: &str,
    errors: &mut Vec<String>,
    events_processed: &mut usize,
) -> Json {
    let reply = kernel.dispatch(cmd, payload);
    let value: Json = match serde_json::from_str(&reply) {
        Ok(value) => value,
        Err(err) => {
            errors.push(format!("信封解析失败：{err}"));
            return json!({ "ok": false });
        }
    };
    if value["ok"] != Json::Bool(true) {
        let code_str = value["error"]["code"].as_str().unwrap_or("UNKNOWN");
        let message = value["error"]["message"].as_str().unwrap_or("");
        errors.push(format!("[{code_str}] {message}"));
    }
    *events_processed += value["eventsProcessed"].as_u64().unwrap_or(0) as usize;
    value
}

/// 叙事里是否出现结局行（前缀来自契约 `agent.endingPrefix`）。
fn has_ending_line(envelope: &Json, prefix: &str) -> bool {
    envelope["narrative"].as_array().is_some_and(|lines| {
        lines
            .iter()
            .filter_map(|value| value.as_str())
            .any(|line| line.starts_with(prefix))
    })
}

fn pending_choice_option_count(envelope: &Json) -> Option<usize> {
    envelope["pendingChoice"]["options"]
        .as_array()
        .map(|options| options.len())
}

fn collect_narrative(envelope: &Json, sample: &mut Vec<String>) {
    if let Some(lines) = envelope["narrative"].as_array() {
        for line in lines.iter().filter_map(|value| value.as_str()) {
            if sample.len() < SAMPLE_LIMIT {
                sample.push(line.to_owned());
            }
        }
    }
}

fn collect_warnings(envelope: &Json, warnings: &mut Vec<String>) {
    if let Some(items) = envelope["warnings"].as_array() {
        for item in items.iter().filter_map(|value| value.as_str()) {
            warnings.push(item.to_owned());
        }
    }
}

/// Mod 目录五件套（pack.json 声明面 → 装载参数；对齐 pack.rs 的声明消费
/// 语义：entries[].path 原样读字节、schemaFiles 存在性 + $id 提取）。
struct ModBundle {
    mod_source: String,
    manifest_bytes: Vec<u8>,
    schemas: Vec<(String, String)>,
    resources: Vec<(String, Vec<u8>)>,
}

fn read_mod_bundle(dir: &Path) -> Result<ModBundle, String> {
    let pack_text = fs::read_to_string(dir.join("pack.json"))
        .map_err(|err| format!("读取 pack.json 失败：{err}"))?;
    let pack: Json = serde_json::from_str(&pack_text)
        .map_err(|err| format!("pack.json 不是合法 JSON：{err}"))?;
    let entries = pack["entries"]
        .as_array()
        .ok_or_else(|| "pack.json 缺少 entries 数组".to_owned())?;
    let schema_files = pack["schemaFiles"]
        .as_array()
        .ok_or_else(|| "pack.json 缺少 schemaFiles 数组".to_owned())?;

    let mod_source = fs::read_to_string(dir.join("mod.lua"))
        .map_err(|err| format!("读取 mod.lua 失败：{err}"))?;
    let manifest_bytes = fs::read(dir.join("manifest.json"))
        .map_err(|err| format!("读取 manifest.json 失败：{err}"))?;

    let mut schemas = Vec::new();
    for file in schema_files {
        let rel = file
            .as_str()
            .ok_or_else(|| "schemaFiles 必须是字符串数组".to_owned())?;
        let text = fs::read_to_string(dir.join(rel))
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
        let bytes =
            fs::read(dir.join(rel)).map_err(|err| format!("读取资源文件 {rel} 失败：{err}"))?;
        resources.push((rel.to_owned(), bytes));
    }

    Ok(ModBundle {
        mod_source,
        manifest_bytes,
        schemas,
        resources,
    })
}
