use super::*;

/// 内置两级列表页资产（编译期嵌入；`--panel-dir` 可盘上覆盖）。
/// 嵌入对象 = 无构建三件（index.html/app.js/style.css，§8）。
pub(crate) static PANEL_ASSETS: include_dir::Dir = include_dir!("$CARGO_MANIFEST_DIR/assets/panel");

/// `/assets/<name>` 的白名单文件名（`index.html` 只经 `/` 提供）。
pub(crate) const ASSET_WHITELIST: [&str; 2] = ["app.js", "style.css"];

// ============================================================================
// §mod 面板托管（§4：声明解析 + 双守卫现读）
// ============================================================================

/// pack.json `panel` 段的解析结果（§4.1：声明损坏 ≠ 装载错误——降级不炸）。
pub(crate) enum PanelDecl {
    Available { root: PathBuf, entry_file: String },
    Unavailable { reason: String },
}

/// pack.json 可选顶层 `panel` 段：`{"panel":{"entry":"panel/index.html"}}`。
/// 语法门 = 不以 `/` 开头、不含 `..`、以 `.html` 结尾；entry 文件必须真实存在
/// 且 canonicalize 后仍在面板根内（拦 symlink 逃逸）。
pub(crate) fn panel_decl(mod_dir: &Path) -> PanelDecl {
    // 退役旧名（别名退役）先给专名降级理由：目录缺席且形状命中时，点名规范路径
    // （观察面不炸，§4.1：声明损坏 ≠ 装载错误）。
    if !mod_dir.exists() {
        if let Some(reason) = crate::retired_seed_name_reason(mod_dir) {
            return PanelDecl::Unavailable { reason };
        }
    }
    let pack_path = mod_dir.join(crate::PACK_FILE);
    let pack = match std::fs::read_to_string(&pack_path)
        .map_err(|err| err.to_string())
        .and_then(|text| serde_json::from_str::<Json>(&text).map_err(|err| err.to_string()))
    {
        Ok(pack) => pack,
        Err(err) => {
            return PanelDecl::Unavailable {
                reason: format!("pack.json 不可读：{err}"),
            }
        }
    };
    let Some(panel) = pack.get("panel") else {
        return PanelDecl::Unavailable {
            reason: "panel 段未声明".to_owned(),
        };
    };
    let Some(entry) = panel["entry"].as_str() else {
        return PanelDecl::Unavailable {
            reason: "panel.entry 未声明或不是字符串".to_owned(),
        };
    };
    if entry.is_empty() {
        return PanelDecl::Unavailable {
            reason: "panel.entry 为空".to_owned(),
        };
    }
    if entry.starts_with('/') {
        return PanelDecl::Unavailable {
            reason: "panel.entry 不得以 / 开头（模内相对路径）".to_owned(),
        };
    }
    if entry.split('/').any(|segment| segment == "..") {
        return PanelDecl::Unavailable {
            reason: "panel.entry 不得含 ..（目录穿越）".to_owned(),
        };
    }
    if !entry.ends_with(".html") {
        return PanelDecl::Unavailable {
            reason: "panel.entry 必须以 .html 结尾".to_owned(),
        };
    }
    let entry_path = mod_dir.join(entry);
    let Some(root) = entry_path.parent().map(Path::to_path_buf) else {
        return PanelDecl::Unavailable {
            reason: "panel.entry 无父目录".to_owned(),
        };
    };
    let (Ok(canon_root), Ok(canon_entry)) = (
        std::fs::canonicalize(&root),
        std::fs::canonicalize(&entry_path),
    ) else {
        return PanelDecl::Unavailable {
            reason: "entry 文件缺失（或 mod 目录不可达）".to_owned(),
        };
    };
    if !canon_entry.starts_with(&canon_root) {
        return PanelDecl::Unavailable {
            reason: "entry 逃出面板根（符号链接？）".to_owned(),
        };
    }
    let entry_file = entry.rsplit('/').next().unwrap_or(entry).to_owned();
    PanelDecl::Available {
        root: canon_root,
        entry_file,
    }
}

/// 行内 panel 字段（§3.4）：`{available, url?, reason?}`。url 指向 entry 本体
/// （两级页的「在面板打开」直接拼 `?save=<sid>`——query 传参可收藏可刷新）。
pub(crate) fn panel_json(run_id: &str, mod_dir: Option<&Path>) -> Json {
    match mod_dir.map(panel_decl) {
        Some(PanelDecl::Available { entry_file, .. }) => json!({
            "available": true,
            "url": format!("/panel/{run_id}/{entry_file}"),
        }),
        Some(PanelDecl::Unavailable { reason }) => json!({"available": false, "reason": reason}),
        None => json!({"available": false, "reason": "run.json 缺少 mod 字段"}),
    }
}

/// `GET /panel/<run-id>/[rel]`——mod 面板托管（§4.2：运行期现读 + 双守卫）。
pub(crate) fn serve_mod_panel(shared: &ServeShared, run_id: &str, rel: &str) -> Response {
    if let Err(response) = check_run_id(run_id) {
        return response;
    }
    let run_dir = match locate_run_dir_code(shared, run_id, "panel_unavailable") {
        Ok(run_dir) => run_dir,
        Err(response) => return response,
    };
    let meta = crate::read_meta(&run_dir.join(crate::META_FILE))
        .ok_or_else(|| "run.json 不可解析".to_owned());
    let mod_dir = meta
        .as_ref()
        .ok()
        .and_then(|meta| run_mod_dir(meta, run_dir.parent()));
    let Some(mod_dir) = mod_dir else {
        return error_response(404, "panel_unavailable", "run.json 缺少可用的 mod 路径");
    };
    match panel_decl(&mod_dir) {
        PanelDecl::Unavailable { reason } => error_response(404, "panel_unavailable", &reason),
        PanelDecl::Available { root, entry_file } => {
            // 目录形式（`/panel/<id>/`）= 服务 entry 本体（mod 内相对链接 `./` 起跳）。
            let rel = if rel.is_empty() {
                entry_file.as_str()
            } else {
                rel
            };
            // rel 语法门（canonicalize 之外的第二道守卫：不触盘先拒）。
            if rel.starts_with('/')
                || rel.contains('\\')
                || rel
                    .split('/')
                    .any(|segment| segment == ".." || segment.is_empty())
            {
                return error_response(404, "panel_path_rejected", "面板资源路径非法");
            }
            let target = root.join(rel);
            // 双守卫之一：canonicalize（解析 symlink 与 `..`）。
            let Ok(canon_target) = std::fs::canonicalize(&target) else {
                return error_response(
                    404,
                    "panel_asset_missing",
                    &format!("面板资源 `{rel}` 不存在"),
                );
            };
            // 双守卫之二：前缀校验（canonicalize 后仍必须在面板根内——拦 symlink 逃逸）。
            if !canon_target.starts_with(&root) {
                return error_response(
                    404,
                    "panel_path_rejected",
                    "面板资源逃出面板根（符号链接？）",
                );
            }
            if !canon_target.is_file() {
                return error_response(
                    404,
                    "panel_asset_missing",
                    &format!("面板资源 `{rel}` 不是文件"),
                );
            }
            match std::fs::read(&canon_target) {
                Ok(bytes) => Response {
                    status: 200,
                    reason: "OK",
                    content_type: content_type_for(rel),
                    headers: Vec::new(),
                    body: bytes,
                },
                Err(err) => error_response(500, "panel_asset_unreadable", &err.to_string()),
            }
        }
    }
}

// ============================================================================
// §内置两级列表页资产（include_dir 嵌入 + --panel-dir 盘上覆盖）
// ============================================================================

/// `GET /` 与 `GET /assets/<name>`：内置页三件（白名单），`--panel-dir` 覆盖生效。
pub(crate) fn builtin_asset(shared: &ServeShared, name: &str) -> Response {
    if name != "index.html" && !ASSET_WHITELIST.contains(&name) {
        return error_response(404, "not_found", "未知资产");
    }
    if let Some(panel_dir) = &shared.panel_dir {
        // 盘上覆盖：canonicalize 前缀守卫；缺失 = 404（fail-closed，开发期立刻发现断链）。
        return match override_file(panel_dir, name) {
            Some(bytes) => asset_response(name, bytes),
            None => error_response(
                404,
                "asset_missing",
                &format!("覆盖目录 {} 中缺少 {name}", panel_dir.display()),
            ),
        };
    }
    match PANEL_ASSETS.get_file(name) {
        Some(file) => asset_response(name, file.contents().to_vec()),
        None => error_response(404, "asset_missing", &format!("嵌入资产缺少 {name}")),
    }
}

pub(crate) fn asset_response(name: &str, bytes: Vec<u8>) -> Response {
    Response {
        status: 200,
        reason: "OK",
        content_type: content_type_for(name),
        headers: Vec::new(),
        body: bytes,
    }
}

/// `--panel-dir` 覆盖读取（文件名白名单已在外层；这里再加 canonicalize 前缀守卫）。
pub(crate) fn override_file(panel_dir: &Path, name: &str) -> Option<Vec<u8>> {
    if name.contains('/') || name.contains('\\') || name.split('/').any(|segment| segment == "..") {
        return None;
    }
    let canon_root = std::fs::canonicalize(panel_dir).ok()?;
    let canon_target = std::fs::canonicalize(canon_root.join(name)).ok()?;
    if !canon_target.starts_with(&canon_root) {
        return None;
    }
    std::fs::read(canon_target).ok()
}

/// Content-Type 小映射（§4.2）；未知扩展 → `application/octet-stream` + nosniff
/// （nosniff 由 write_response 恒定携带）。
pub(crate) fn content_type_for(name: &str) -> &'static str {
    let extension = name.rsplit('.').next().unwrap_or("");
    match extension {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "txt" => "text/plain; charset=utf-8",
        "md" => "text/markdown; charset=utf-8",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}
