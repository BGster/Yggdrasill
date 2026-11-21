// ============================================================================
// §制品（artifact）面：run 内的内容寻址小文件
// ============================================================================

use sha2::{Digest, Sha256};

use crate::StoreError;
// [`RunStore`] / [`Tick`] 仅为 doc 链接入 scope（搬移时 doc comment 逐字节不动；
// 生产码不直接使用这两个符号，故 allow）。
#[allow(unused_imports)]
use crate::{RunStore, Tick};

/// 路径前缀：**唯一合法起点**（与 Mod 侧 `settlement.evolution.artifact` 的
/// `artifacts/<file>.md` 口径同源）。
pub const ARTIFACT_PREFIX: &str = "artifacts/";

/// 路径字节上限（含前缀）。
pub const ARTIFACT_PATH_MAX_BYTES: usize = 128;

/// 单件内容字节上限（256 KiB）。
pub const ARTIFACT_CONTENT_MAX_BYTES: usize = 256 * 1024;

/// 一件制品的元信息（内容在 [`ArtifactRecord`] / 后端内）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactMeta {
    /// run 内相对路径，恒以 [`ARTIFACT_PREFIX`] 起头。
    pub path: String,
    pub bytes: usize,
    /// 小写十六进制 SHA-256（64 字符；与 `ygg_schema::validate::sha256_hex`
    /// 同一编码口径）。
    pub sha256: String,
    /// unix 秒，**宿主提供**（端口不取时钟，与 [`Tick`] 同纪律）。
    ///
    /// **跨后端不强求相等**（`ygg.fs.list` 的回执也**不**包含它，见
    /// [`RunStore::artifact_list`]）：sqlite 存的是写入那次 `Tick.secs`
    /// （可重放），JSON 后端无 sidecar，只能回文件的 mtime（读不到 = 0）。
    /// 两个后端的跨端一致面是 `path` / `bytes` / `sha256` 与升序。
    pub updated_at: u64,
}

/// 元信息 + 原文（[`RunStore::artifact_get`] 的返回形状）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactRecord {
    pub meta: ArtifactMeta,
    pub content: Vec<u8>,
}

/// 摘要的小写十六进制编码（零额外依赖面：只有 `sha2` 的 `Sha256`）。
///
/// 制品面的**唯一**摘要实现：`--artifact-put`（宿主 CLI）与 VFS 落盘（宿主
/// 的 dirty 排空）两条写入口，加上 JSON 后端无 sidecar 的读入口，共三处需要
/// 同一口径——放在端口里只有一份。
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// 制品路径门（**唯一实现**）。`Ok(())` 之外的每个分支都带人读原因。
///
/// 规则（全部为拒绍条件，顺序即报错顺序）：
///   ① 字节长度 1..=[`ARTIFACT_PATH_MAX_BYTES`]；
///   ② 必须以 [`ARTIFACT_PREFIX`] 起头；
///   ③ 前缀之后非空、且不含量段（`artifacts//x`）、不以 `/` 结尾（`artifacts/x/`）；
///   ④ 全字符 ∈ `[A-Za-z0-9._/-]`（NUL / 空白 / 反斜杠 / `@` 一律拒绍）；
///   ⑤ 任何位置不得出现 `..`。
///
/// **双门声明（有意为之）**：`ygg-sandbox` 的 `VfsStore` 还有一份同规则的本地门
/// （`host_api::validate_vfs_path`）——`ygg-store` 是 sandbox 的 **dev-dependency**，
/// 运行时无权调用（会把 `rusqlite` 拉进 wasm32）。两份门的角色不同：本地门在
/// Lua 调用点 fail-fast（`ygg.fs.write("…/../x")` 当场可读报错），本函数是
/// **权威持久化门**（兼盖 `--artifact-put` 这类不经 Lua 的入口）。改规则时两边
/// 的向量表必须同一次提交一起改。
pub fn validate_artifact_path(path: &str) -> Result<(), StoreError> {
    let bytes = path.len();
    if bytes == 0 || bytes > ARTIFACT_PATH_MAX_BYTES {
        return Err(StoreError::invalid(
            "制品路径",
            format!(
                "长度必须在 1..={ARTIFACT_PATH_MAX_BYTES} 字节之间（当前 {bytes} 字节：`{path}`）"
            ),
        ));
    }
    let Some(rest) = path.strip_prefix(ARTIFACT_PREFIX) else {
        return Err(StoreError::invalid(
            "制品路径",
            format!("必须以 `{ARTIFACT_PREFIX}` 起头（当前是 `{path}`）"),
        ));
    };
    if rest.is_empty() {
        return Err(StoreError::invalid(
            "制品路径",
            format!("`{ARTIFACT_PREFIX}` 之后必须还有文件名（当前是 `{path}`）"),
        ));
    }
    if rest.ends_with('/') {
        return Err(StoreError::invalid(
            "制品路径",
            format!("不得以 `/` 结尾（当前是 `{path}`）"),
        ));
    }
    if rest.split('/').any(str::is_empty) {
        return Err(StoreError::invalid(
            "制品路径",
            format!("不得含空路径段（`{ARTIFACT_PREFIX}` 之后是 `{rest}`）"),
        ));
    }
    if let Some(ch) = path
        .chars()
        .find(|ch| !(ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '/' | '-')))
    {
        return Err(StoreError::invalid(
            "制品路径",
            format!("含非法字符 `{ch}`（只允许 `[A-Za-z0-9._/-]`）"),
        ));
    }
    if path.contains("..") {
        return Err(StoreError::invalid(
            "制品路径",
            format!("不得含 `..`（目录穿越；当前是 `{path}`）"),
        ));
    }
    Ok(())
}

/// 内容长度门（[`ARTIFACT_CONTENT_MAX_BYTES`]）。
pub fn validate_artifact_content(content: &[u8]) -> Result<(), StoreError> {
    if content.len() > ARTIFACT_CONTENT_MAX_BYTES {
        return Err(StoreError::invalid(
            "制品内容",
            format!(
                "{} 字节超过上限 {ARTIFACT_CONTENT_MAX_BYTES} 字节（256 KiB）",
                content.len()
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- 制品门（路径 / 大小）----

    /// 路径门向量表（**与 `crates/ygg-sandbox/src/host_api.rs` 的同名用例
    /// 逐字同步**：两边规则漂移时必有一边红）。
    fn path_gate_vectors() -> (Vec<String>, Vec<String>) {
        // 128 字节边界：`artifacts/` (10) + 115 个 a + `.md` (3) = 128。
        let max_ok = format!(
            "artifacts/{}.md",
            "a".repeat(ARTIFACT_PATH_MAX_BYTES - ARTIFACT_PREFIX.len() - 3)
        );
        // 129 字节：多一个 a。
        let too_long = format!(
            "artifacts/{}.md",
            "a".repeat(ARTIFACT_PATH_MAX_BYTES - ARTIFACT_PREFIX.len() - 2)
        );
        let accepted = vec![
            "artifacts/a.md".to_owned(),
            "artifacts/notes/x.md".to_owned(),
            "artifacts/a.b-c_d/e.md".to_owned(),
            "artifacts/0.md".to_owned(),
            "artifacts/a/b/c/d.md".to_owned(),
            max_ok,
        ];
        let rejected = vec![
            "artifacts/".to_owned(),
            "artifacts//x".to_owned(),
            "artifacts/x/".to_owned(),
            "artifacts/../x".to_owned(),
            "artifacts/a/../../x".to_owned(),
            "artifacts/a..b".to_owned(),
            "artifact/x".to_owned(),
            "/artifacts/x".to_owned(),
            "artifacts/a b.md".to_owned(),
            "artifacts/a\\b.md".to_owned(),
            "artifacts/a|b.md".to_owned(),
            "artifacts/a@b.md".to_owned(),
            too_long,
            String::new(),
            "artifacts".to_owned(),
        ];
        (accepted, rejected)
    }

    #[test]
    fn artifact_path_gate_accepts_legal_shapes() {
        let (accepted, _) = path_gate_vectors();
        for path in accepted {
            validate_artifact_path(&path)
                .unwrap_or_else(|err| panic!("`{path}`（{} 字节）必须放行：{err}", path.len()));
        }
        // 边界自证：最长向量恰好是上限（否则「刚好 128」根本没被测到）。
        let max_ok = format!(
            "artifacts/{}.md",
            "a".repeat(ARTIFACT_PATH_MAX_BYTES - ARTIFACT_PREFIX.len() - 3)
        );
        assert_eq!(max_ok.len(), ARTIFACT_PATH_MAX_BYTES);
    }

    #[test]
    fn artifact_path_gate_rejects_illegal_shapes() {
        let (_, rejected) = path_gate_vectors();
        for path in rejected {
            let err = validate_artifact_path(&path).expect_err(&format!("`{path}` 必须被拒"));
            assert!(
                matches!(err, StoreError::Invalid { .. }),
                "必须走 Invalid（不是 Corrupt）：{err}"
            );
            // 报错点名字段名，用户才知道看哪里。
            assert!(err.to_string().contains("制品路径"), "{err}");
        }
    }

    #[test]
    fn artifact_content_gate_rejects_oversize() {
        assert!(validate_artifact_content(b"").is_ok());
        let exactly = vec![0u8; ARTIFACT_CONTENT_MAX_BYTES];
        assert!(
            validate_artifact_content(&exactly).is_ok(),
            "恰好上限必须放行"
        );
        let over = vec![0u8; ARTIFACT_CONTENT_MAX_BYTES + 1];
        let err = validate_artifact_content(&over).expect_err("超一个字节也必须拒");
        assert!(matches!(err, StoreError::Invalid { .. }));
        assert!(err.to_string().contains("制品内容"), "{err}");
    }

    /// 摘要口径钉住：空串与 `abc` 的 SHA-256 标准向量（与
    /// `ygg_schema::validate::sha256_hex` 同一编码）。
    #[test]
    fn sha256_hex_matches_standard_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(sha256_hex(b"abc").len(), 64);
    }
}
