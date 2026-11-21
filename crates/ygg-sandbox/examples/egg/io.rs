use super::*;

/// 宿主侧时钟（unix 秒）：时间戳只进 events.jsonl / 元信息，不触内核状态。
pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// 宿主侧时钟（unix 纳秒）：只用于归档文件命名（同秒撞名不可能）。
pub(crate) fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos())
}

/// pretty JSON 落盘（人读证据面；末尾补换行便于 diff）。
pub(crate) fn write_json_pretty(path: &Path, value: &Json) -> Result<(), String> {
    let mut text = serde_json::to_string_pretty(value)
        .map_err(|err| format!("序列化 {} 失败：{err}", path.display()))?;
    text.push('\n');
    fs::write(path, text).map_err(|err| format!("写入 {} 失败：{err}", path.display()))
}

/// 原子写（tmp + rename）：读者要么看到旧全文、要么看到新全文，绝无半截（W3）。
/// tmp 名带 pid：固定名会让两个并发写者互踩 tmp 内容，rename 后拿到「别人的
/// 中间态」——pid 后缀是正确性要求，不是风格（plan-r5 §2.4）。
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let name = path.file_name().map_or_else(
        || format!(".atomic-{}", std::process::id()),
        |name| format!("{}.tmp.{}", name.to_string_lossy(), std::process::id()),
    );
    let tmp = path.with_file_name(name);
    let mut file =
        File::create(&tmp).map_err(|err| format!("写入 {} 失败：{err}", tmp.display()))?;
    file.write_all(bytes)
        .map_err(|err| format!("写入 {} 失败：{err}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|err| format!("替换 {} 失败：{err}", path.display()))
}

// ============================================================================
// §P0 持久化原子性单测（plan-r5 §4.1；U1–U2 随 S1，U3 随 S2，U4 随 S3）
// ============================================================================
#[cfg(test)]
#[path = "io/tests.rs"]
mod tests;
