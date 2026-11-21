use super::{json_kind, read_json, CONTRACT_DOC};
use serde_json::Value as Json;
use std::path::Path;

// ============================================================================
// §结算口径（data/config.json 的 `settlement` 段，四项全部必填）
// ============================================================================

/// 结算判定口径：四项都是必填布尔（无「缺省开启」的隐式行为）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettlementPolicy {
    /// 事件日志必须非空。
    pub require_events: bool,
    /// 不得有失败步。
    pub require_no_failed_steps: bool,
    /// 必须出现结局叙事行。
    pub require_ending: bool,
    /// inbox 不得有未消费行。
    pub require_inbox_consumed: bool,
}

/// `settlement` 段的应有形状。
const SETTLEMENT_SHAPE: &str = r#"{
  "settlement": {
    "requireEvents": true,
    "requireNoFailedSteps": true,
    "requireEnding": true,
    "requireInboxConsumed": true
  }
}"#;

impl SettlementPolicy {
    /// 从已解析的 `config.json` 取 `settlement` 段（缺段 / 缺键 / 非布尔 → 错误）。
    pub fn from_config(config: &Json, source: &Path) -> Result<Self, String> {
        let Some(section) = config.get("settlement") else {
            return Err(format!(
                "{} 缺少 `settlement` 段（结算口径必需）。该段的完整形状：\n{SETTLEMENT_SHAPE}\n{CONTRACT_DOC}",
                source.display()
            ));
        };
        if !section.is_object() {
            return Err(format!(
                "{} 的 `settlement` 段必须是对象（当前是 {}）。该段的完整形状：\n{SETTLEMENT_SHAPE}\n{CONTRACT_DOC}",
                source.display(),
                json_kind(section)
            ));
        }
        let flag = |key: &str| -> Result<bool, String> {
            section.get(key).and_then(Json::as_bool).ok_or_else(|| {
                format!(
                    "{} 的 `settlement` 段缺少字段 `{key}`（必需，布尔）。该段的完整形状：\n{SETTLEMENT_SHAPE}\n{CONTRACT_DOC}",
                    source.display()
                )
            })
        };
        Ok(Self {
            require_events: flag("requireEvents")?,
            require_no_failed_steps: flag("requireNoFailedSteps")?,
            require_ending: flag("requireEnding")?,
            require_inbox_consumed: flag("requireInboxConsumed")?,
        })
    }

    /// 从配置文件路径现读（任何问题都以可读错误返回）。
    pub fn load(path: &Path) -> Result<Self, String> {
        Self::from_config(&read_json(path)?, path)
    }
}
