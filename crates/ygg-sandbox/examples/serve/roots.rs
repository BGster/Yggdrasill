use super::*;

/// 退役的 serve 根集配置文件（`$HOME` 相对；存在时启动给迁移指引，不自动导入）。
pub(crate) const SERVE_CONFIG_REL: &str = ".config/ygg/serve.json";
/// 注册表根数上限（防误注册风暴；远超出正常使用面）。
pub(crate) const ROOTS_CAP: usize = 32;

// ============================================================================
// §观察根注册表端点（r1：进程内观察配置；对象分离——不写存档/局状态/目录）
// ============================================================================

/// `GET /api/roots`——注册表自描述（单实例探针的正面证据面）。
pub(crate) fn handle_roots_get(shared: &ServeShared) -> Response {
    let roots = shared.roots_snapshot();
    Response::json(
        200,
        &json!({
            "roots": roots
                .iter()
                .map(|root| json!({
                    "root": root.path.display().to_string(),
                    "singleRun": root.single_run,
                }))
                .collect::<Vec<_>>(),
            "cap": ROOTS_CAP,
        }),
    )
}

/// `POST /api/roots`——注册观察根。幂等：同一 canonical 根重复注册 = added:false。
/// 形状校验：绝对路径（相对/`~` 拒绝）、is_dir 必须为真（零创建）、上限 32。
pub(crate) fn handle_roots_post(shared: &ServeShared, body: &[u8]) -> Response {
    let parsed = serde_json::from_slice::<Json>(body);
    let Ok(value) = parsed else {
        return error_response(400, "bad_request", "POST /api/roots 需要 JSON 体");
    };
    let Some(raw) = value["root"].as_str().map(str::trim) else {
        return error_response(
            400,
            "bad_request",
            "缺少 `root` 字段（示例：{\"root\": \"/abs/path/.ygg/runs\"}）",
        );
    };
    if raw.is_empty() {
        return error_response(400, "bad_request", "`root` 不能为空");
    }
    let path = PathBuf::from(raw);
    if !path.is_absolute() {
        return error_response(
            400,
            "bad_request",
            &format!("`root` 必须是绝对路径（相对路径「相对于谁」是无解的猜测）：`{raw}`"),
        );
    }
    if !path.is_dir() {
        return error_response(
            404,
            "root_not_found",
            &format!("目录不存在或不是目录：`{raw}`（serve 零创建——请先创建目录或检查路径）"),
        );
    }
    let canonical = std::fs::canonicalize(&path).unwrap_or(path);
    let single_run = canonical.join(crate::META_FILE).is_file();
    let entry = RootEntry {
        path: canonical,
        single_run,
    };
    let mut added = false;
    {
        let Ok(mut roots) = shared.roots.write() else {
            return error_response(500, "registry_poisoned", "注册表锁已中毒");
        };
        if roots.iter().any(|existing| existing.path == entry.path) {
            // 幂等命中：单局/父目录归类以首次注册为准（run.json 后来才出现
            // 不改变已注册根的观察语义——注销后重注册可重新归类）。
        } else if roots.len() >= ROOTS_CAP {
            return error_response(
                409,
                "registry_full",
                &format!("注册表已满（{ROOTS_CAP} 根上限）——请先 DELETE 注销不再观察的根"),
            );
        } else {
            roots.push(entry.clone());
            added = true;
        }
    }
    if added {
        // 根级变更帧：SSE 客户端收帧后重取 /api/runs（watcher 下周期接管指纹）。
        shared.broadcaster.bump(None, Some(entry.path.as_path()));
    }
    let roots = shared.roots_snapshot();
    Response::json(
        200,
        &json!({
            "ok": true,
            "added": added,
            "root": entry.path.display().to_string(),
            "singleRun": entry.single_run,
            "roots": roots
                .iter()
                .map(|root| json!(root.path.display().to_string()))
                .collect::<Vec<_>>(),
        }),
    )
}

/// `DELETE /api/roots?root=<绝对路径>`——注销。幂等：缺席 = removed:false。
/// 只从注册表摘除（不删盘上任何字节）；watcher 下周期发现名单变化并发帧。
pub(crate) fn handle_roots_delete(shared: &ServeShared, query: &str) -> Response {
    let Some(raw) = query_param(query, "root") else {
        return error_response(
            400,
            "bad_request",
            "DELETE /api/roots 需要 `?root=<绝对路径>` 查询参数",
        );
    };
    let path = PathBuf::from(raw);
    let canonical = std::fs::canonicalize(&path).unwrap_or(path);
    let removed = {
        let Ok(mut roots) = shared.roots.write() else {
            return error_response(500, "registry_poisoned", "注册表锁已中毒");
        };
        let before = roots.len();
        roots.retain(|entry| entry.path != canonical);
        roots.len() != before
    };
    if removed {
        shared.broadcaster.bump(None, Some(canonical.as_path()));
    }
    let roots = shared.roots_snapshot();
    Response::json(
        200,
        &json!({
            "ok": true,
            "removed": removed,
            "roots": roots
                .iter()
                .map(|root| json!(root.path.display().to_string()))
                .collect::<Vec<_>>(),
        }),
    )
}
