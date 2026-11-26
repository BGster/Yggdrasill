// ============================================================================
// §后端裁决（混用 / 断言不一致 → fail-closed，且零写入）
// ============================================================================

use std::fmt;
use std::path::Path;

use crate::{Backend, StoreError, AUTOSAVE_FILE, DEFAULT_BACKEND, EVENTS_FILE};

/// run 目录里的后端产物探针。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Products {
    /// 有 `autosave.json` 或 `events.jsonl`（既有 JSON 产物）。
    pub json: bool,
}

impl Products {
    pub fn any(self) -> bool {
        self.json
    }
}

/// 探测 run 目录里的后端产物（只读 `Path::exists`，不开文件、不建目录）。
pub fn detect(run_dir: &Path) -> Products {
    Products {
        json: run_dir.join(AUTOSAVE_FILE).exists() || run_dir.join(EVENTS_FILE).exists(),
    }
}

/// 后端裁决：`run.json` 的 `store` 事实 > 产物探针 > `--store` 旗标 > 默认后端。
///
/// 严格按「权威性」排：`run.json` 是权威事实，`--store` 只是开局断言。允许的
/// 唯一推断是**旧局**（`run.json` 没有 `store` 键，或连 `run.json` 都没有）——
/// 此时按产物推断，既有的纯 JSON 目录一律落回 `json`（向后兼容）。**两者都没有**
/// 才是新局，落 [`DEFAULT_BACKEND`]。
pub fn resolve(
    run_dir: &Path,
    declared: Option<Backend>,
    flag: Option<Backend>,
) -> Result<Backend, StoreError> {
    resolve_with(run_dir, declared, flag, detect(run_dir))
}

/// [`resolve`] 的可注入版本（测试用：不落盘也能裁决）。
pub fn resolve_with(
    run_dir: &Path,
    declared: Option<Backend>,
    flag: Option<Backend>,
    products: Products,
) -> Result<Backend, StoreError> {
    let where_ = run_dir.display();
    let probed = if products.json {
        Some(Backend::Json)
    } else {
        None
    };

    if let Some(declared) = declared {
        // 声明了后端，但目录里是另一个后端的产物 → 混代（同一族错误，零写入）。
        if let Some(probed) = probed {
            if probed != declared {
                return Err(declared_conflicts_with_products(where_, declared, probed));
            }
        }
        if let Some(flag) = flag {
            if flag != declared {
                return Err(StoreError::Misconfigured {
                    detail: format!(
                        "本局已声明存储后端 `{declared}`（来自 {where_}/run.json 的 \"store\" 字段），\
                         与 --store {flag} 不一致——run.json 是权威事实，--store 只是开局断言。\
                         要用另一后端请开新局（换 --run-id）；本局未做任何写入。"
                    ),
                });
            }
        }
        return Ok(declared);
    }

    // 无声明：旧局（按产物推断）或新局（按旗标 / [`DEFAULT_BACKEND`] 定型）。
    if let Some(flag) = flag {
        if let Some(probed) = probed {
            if probed != flag {
                return Err(flag_conflicts_with_products(where_, flag, probed));
            }
        }
        return Ok(flag);
    }
    Ok(probed.unwrap_or(DEFAULT_BACKEND))
}

/// 产物文件名（错误文案里点名，用户才知道该看哪个文件）。
fn product_files(backend: Backend) -> &'static str {
    match backend {
        Backend::Json => "autosave.json / events.jsonl",
    }
}

/// `run.json` 声明了 `declared`，但目录里是 `probed` 的产物 → 混代。
fn declared_conflicts_with_products(
    where_: impl fmt::Display,
    declared: Backend,
    probed: Backend,
) -> StoreError {
    StoreError::Misconfigured {
        detail: format!(
            "本局已声明存储后端 `{declared}`（来自 {where_}/run.json 的 \"store\" 字段），\
             但目录里只有 `{probed}` 的产物（{}）——混代状态无法判定谁是权威，拒绝继续；\
             请人工确认后修正 run.json 或删除多余的那一份（宿主不自动删证据）；\
             本局未做任何写入。",
            product_files(probed)
        ),
    }
}

/// `--store <flag>` 与目录里已有的产物（`probed`）冲突 → 拒绝（旧局漏写事实的常见情形）。
fn flag_conflicts_with_products(
    where_: impl fmt::Display,
    flag: Backend,
    probed: Backend,
) -> StoreError {
    StoreError::Misconfigured {
        detail: format!(
            "{where_} 里是 `{probed}` 后端的产物（{}），与 --store {flag} 不一致——\
             存储后端由目录产物与 run.json 的 \"store\" 字段决定，--store 只是开局断言。\
             要用另一后端请开新局（换 --run-id）；本局未做任何写入。",
            product_files(probed)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dir(label: &str) -> PathBuf {
        Path::new("/tmp").join(format!("ygg-store-resolve-{label}"))
    }

    /// 既有的纯 JSON 目录（无 `store` 字段）必须推断为 json——向后兼容的判据。
    #[test]
    fn legacy_json_products_infer_json() {
        let products = Products { json: true };
        let backend = resolve_with(&dir("legacy"), None, None, products).expect("旧局必须可推断");
        assert_eq!(backend, Backend::Json);
    }

    /// 空目录（新局）：默认落 [`DEFAULT_BACKEND`] = json。
    ///
    /// 这里断言的是**字面量** `Backend::Json`（而不是回引 `DEFAULT_BACKEND`）：
    /// 常量自己被改回去时，测试必须红——自证自话的断言等于没断言。
    #[test]
    fn empty_dir_defaults_to_json() {
        let backend =
            resolve_with(&dir("new"), None, None, Products::default()).expect("空目录必须可裁决");
        assert_eq!(backend, Backend::Json);
        assert_eq!(DEFAULT_BACKEND, Backend::Json, "默认后端常量与裁决结果同源");
    }

    /// 回退通道：空目录 + `--store json` → json（旗标与默认值一致不得报冲突）。
    #[test]
    fn empty_dir_flag_json_still_wins() {
        assert_eq!(
            resolve_with(&dir("new"), None, Some(Backend::Json), Products::default()).unwrap(),
            Backend::Json
        );
    }

    /// 真值表第 2 行：`run.json` 已声明 json（且无任何产物）→ 必须回 json。
    /// 防「默认值压过权威事实」——这是旧局往后兼容的最后一道门。
    #[test]
    fn declared_json_without_products_beats_new_default() {
        let backend = resolve_with(
            &dir("declared-json"),
            Some(Backend::Json),
            None,
            Products::default(),
        )
        .expect("已声明的后端必须放行");
        assert_eq!(backend, Backend::Json);
    }
}
