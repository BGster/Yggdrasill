use super::{json_kind, read_json, CONTRACT_DOC};
use serde_json::Value as Json;
use std::path::Path;

// ============================================================================
// §自动推进阈值（data/config.json 的 `drive` 段，三项全部必填正整数）
// ============================================================================

/// 自动推进阈值（宿主消费；内核不解释）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriveLimits {
    /// 基础步数硬顶。
    pub max_steps: usize,
    /// 连续无进展判停阈值。
    pub stall_limit: usize,
    /// 每条需求额外放宽的步数。
    pub steps_per_requirement: usize,
}

/// `drive` 段的应有形状。
const DRIVE_SHAPE: &str = r#"{
  "drive": {
    "maxSteps": 64,
    "stallLimit": 12,
    "stepsPerRequirement": 16
  }
}"#;

impl DriveLimits {
    /// 从已解析的 `config.json` 取 `drive` 段（缺段 / 缺键 / 非正整数 → 错误）。
    pub fn from_config(config: &Json, source: &Path) -> Result<Self, String> {
        let Some(section) = config.get("drive") else {
            return Err(format!(
                "{} 缺少 `drive` 段（自动推进阈值必需）。该段的完整形状：\n{DRIVE_SHAPE}\n{CONTRACT_DOC}",
                source.display()
            ));
        };
        if !section.is_object() {
            return Err(format!(
                "{} 的 `drive` 段必须是对象（当前是 {}）。该段的完整形状：\n{DRIVE_SHAPE}\n{CONTRACT_DOC}",
                source.display(),
                json_kind(section)
            ));
        }
        let positive = |key: &str| -> Result<usize, String> {
            match section.get(key).and_then(Json::as_u64) {
                Some(value) if value > 0 => Ok(value as usize),
                _ => Err(format!(
                    "{} 的 `drive` 段缺少字段 `{key}`（必需，正整数）。该段的完整形状：\n{DRIVE_SHAPE}\n{CONTRACT_DOC}",
                    source.display()
                )),
            }
        };
        let limits = Self {
            max_steps: positive("maxSteps")?,
            stall_limit: positive("stallLimit")?,
            steps_per_requirement: positive("stepsPerRequirement")?,
        };
        Ok(limits)
    }

    /// 从配置文件路径现读（任何问题都以可读错误返回）。
    pub fn load(path: &Path) -> Result<Self, String> {
        Self::from_config(&read_json(path)?, path)
    }

    /// 本局步数预算：基础硬顶 + 每条需求额外步数（饱和运算，不溢出）。
    pub fn step_limit(&self, requirement_count: usize) -> usize {
        self.max_steps
            .saturating_add(self.steps_per_requirement.saturating_mul(requirement_count))
    }
}
