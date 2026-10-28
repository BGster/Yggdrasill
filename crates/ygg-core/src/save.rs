//! 存档格式（FR8 / B3）：完整状态快照 + 双版本号；事件日志与输出缓冲旁路不入档。
//!
//! - **完整状态快照**（B3，S5 amendment 修正）：存档 = 实体表 + 持久 KV +
//!   `pending_choice`——挂起选择是**内核状态**（D-B2 choice-as-state：挂起即数据），
//!   读档后必须原样重现待决选项（否则 restore 后玩家面对的剧情分支丢失，
//!   状态快照不完整）；handler 为代码不入档（重装载即恢复）；转换残余的
//!   输出缓冲属瞬态诊断面（已交付表现层），旁路不入档；
//! - **双版本号**：`format_version` = 存档格式自身版本（格式演进/迁移锚点，
//!   读档即校验）；`engine_version` = 引擎版本字符串（与引擎侧暴露同源，
//!   S10 stdlib 引擎版本化同一口径），记录「由哪个引擎写出」；
//! - **确定性序**：实体表 ID 升序、KV 键字典序均为容器天然序（S2/S5 纪律），
//!   同一状态无论构造次序如何，存档字节逐字节一致。

use serde::{Deserialize, Serialize};

use crate::bus::PendingChoice;
use crate::entity::EntityStore;
use crate::kv::KvStore;

/// M0 存档格式版本（读档校验锚点；格式演进时 +1 并配迁移策略）。
pub const SAVE_FORMAT_VERSION: u32 = 1;

/// 存档错误（thiserror，白名单内）。
#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("save snapshot encoding failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported save format version {found} (expected {expected})")]
    UnsupportedFormatVersion { found: u32, expected: u32 },
}

/// 存档文件：双版本号 + 完整状态快照（FR8 / B3 + S5 amendment）。
///
/// 状态面 = 实体表 + 持久 KV + `pending_choice`（内核状态，读档重现待决选项）；
/// `pending_choice` 为 `None` 时表示无挂起选择（非 choice 终结的常规存档）。
/// 构造即定型 `format_version`（恒为 [`SAVE_FORMAT_VERSION`]）；
/// 读档经 [`SaveFile::from_bytes`] 校验版本后恢复。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SaveFile {
    format_version: u32,
    engine_version: String,
    entities: EntityStore,
    kv: KvStore,
    pending_choice: Option<PendingChoice>,
}

impl SaveFile {
    pub fn new(
        engine_version: impl Into<String>,
        entities: EntityStore,
        kv: KvStore,
        pending_choice: Option<PendingChoice>,
    ) -> Self {
        Self {
            format_version: SAVE_FORMAT_VERSION,
            engine_version: engine_version.into(),
            entities,
            kv,
            pending_choice,
        }
    }

    pub fn format_version(&self) -> u32 {
        self.format_version
    }

    pub fn engine_version(&self) -> &str {
        &self.engine_version
    }

    pub fn entities(&self) -> &EntityStore {
        &self.entities
    }

    pub fn kv(&self) -> &KvStore {
        &self.kv
    }

    /// 挂起选择（读档后原样重现待决选项；`None` = 无挂起）。
    pub fn pending_choice(&self) -> Option<&PendingChoice> {
        self.pending_choice.as_ref()
    }

    /// 序列化为 JSON bytes（实体序 = ID 升序、KV 序 = 键字典序，字节确定）。
    pub fn to_bytes(&self) -> Result<Vec<u8>, SaveError> {
        Ok(serde_json::to_vec(self)?)
    }

    /// 从 bytes 恢复：`format_version` 不符即拒绝（格式演进锚点）。
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SaveError> {
        let file: SaveFile = serde_json::from_slice(bytes)?;
        if file.format_version != SAVE_FORMAT_VERSION {
            return Err(SaveError::UnsupportedFormatVersion {
                found: file.format_version,
                expected: SAVE_FORMAT_VERSION,
            });
        }
        Ok(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::Value;

    fn sample(engine_version: &str) -> SaveFile {
        let mut store = EntityStore::new();
        let id = store.spawn().unwrap();
        store.set_attribute(id, "hp", Value::Int(30)).unwrap();
        let mut kv = KvStore::new();
        kv.set("gold", Value::Int(7)).unwrap();
        SaveFile::new(engine_version, store, kv, None)
    }

    #[test]
    fn round_trip_preserves_versions_and_state() {
        let save = sample("0.1.0");
        let bytes = save.to_bytes().unwrap();
        let restored = SaveFile::from_bytes(&bytes).unwrap();
        assert_eq!(save, restored);
        assert_eq!(restored.format_version(), SAVE_FORMAT_VERSION);
        assert_eq!(restored.engine_version(), "0.1.0");
        assert_eq!(restored.entities().len(), 1);
        assert_eq!(restored.entities().revision(), 2); // spawn(+1) + set(+1) 随快照恢复
        assert_eq!(restored.kv().get("gold"), Some(&Value::Int(7)));
    }

    #[test]
    fn round_trip_preserves_pending_choice() {
        // S5 amendment（D-B3 完整状态快照语义修正）：挂起选择是内核状态，
        // 存档面必须往返保真——读档后原样重现待决选项（prompt + options 序）。
        let choice = PendingChoice::new("继续前行？", vec!["go".to_string(), "stay".to_string()]);
        let save = SaveFile::new("0.1.0", EntityStore::new(), KvStore::new(), Some(choice));
        let bytes = save.to_bytes().unwrap();
        let restored = SaveFile::from_bytes(&bytes).unwrap();
        let picked = restored.pending_choice().unwrap();
        assert_eq!(picked.prompt(), "继续前行？");
        assert_eq!(picked.options(), ["go".to_string(), "stay".to_string()]);
        assert_eq!(save, restored);
    }

    #[test]
    fn save_bytes_are_deterministic_across_construction_orders() {
        // 同内容 store 以不同构造次序（spawn vs 指定 ID insert）→ 存档字节一致
        let mut a_store = EntityStore::new();
        let id = a_store.spawn().unwrap();
        a_store.set_attribute(id, "hp", Value::Int(30)).unwrap();
        let mut b_store = EntityStore::new();
        b_store.insert(crate::entity::Entity::new(1)).unwrap();
        b_store.set_attribute(1, "hp", Value::Int(30)).unwrap();
        let mut a_kv = KvStore::new();
        a_kv.set("gold", Value::Int(7)).unwrap();
        let mut b_kv = KvStore::new();
        b_kv.set("gold", Value::Int(7)).unwrap();
        let a = SaveFile::new("0.1.0", a_store, a_kv, None)
            .to_bytes()
            .unwrap();
        let b = SaveFile::new("0.1.0", b_store, b_kv, None)
            .to_bytes()
            .unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn unsupported_format_version_is_rejected() {
        let save = sample("0.1.0");
        let bytes = save.to_bytes().unwrap();
        let s = String::from_utf8(bytes).unwrap();
        // 篡改锚点依赖 serde_json 的结构体字段序列化序（声明序），`format_version`
        // 恒为首个字段；若未来改为 Map 形态（如 preserve_order 反转字段序），
        // 本字符串替换须改写为「反序列化 → 改字段 → 重序列化」形态。
        let tampered = s.replace(
            &format!("\"format_version\":{SAVE_FORMAT_VERSION}"),
            "\"format_version\":999",
        );
        let err = SaveFile::from_bytes(tampered.as_bytes()).unwrap_err();
        assert!(matches!(
            err,
            SaveError::UnsupportedFormatVersion {
                found: 999,
                expected: SAVE_FORMAT_VERSION
            }
        ));
        assert!(err.to_string().contains("unsupported save format version"));
    }

    #[test]
    fn corrupt_bytes_are_rejected_as_json_error() {
        let err = SaveFile::from_bytes(b"not json").unwrap_err();
        assert!(matches!(err, SaveError::Json(_)));
    }

    #[test]
    fn fresh_save_from_empty_state_round_trips() {
        let save = SaveFile::new("0.1.0", EntityStore::new(), KvStore::new(), None);
        let bytes = save.to_bytes().unwrap();
        let restored = SaveFile::from_bytes(&bytes).unwrap();
        assert!(restored.entities().is_empty());
        assert!(restored.kv().is_empty());
        assert_eq!(restored.entities().revision(), 0);
    }
}
