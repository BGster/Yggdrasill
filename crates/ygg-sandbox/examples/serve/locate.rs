use super::*;

/// 跨根定位一局：0 命中 → NotFound；1 命中 → Found；>1 命中 → Ambiguous（409，
/// 响亮上报，不静默任选一份——serve 不代调用方选择事实）。
pub(crate) enum RunLocation {
    NotFound,
    Found(PathBuf),
    Ambiguous(Vec<PathBuf>),
}

/// run 目录是否已是「局」（run.json 存在）；半壳目录不是局。
pub(crate) fn is_run_dir(root: &Path, run_id: &str) -> Option<PathBuf> {
    let run_dir = root.join(run_id);
    run_dir.join(crate::META_FILE).is_file().then_some(run_dir)
}

/// 统一跨根定位（数据端点与面板托管共用同一条寻址事实）。
pub(crate) fn locate_run(shared: &ServeShared, run_id: &str) -> RunLocation {
    let mut hits: Vec<PathBuf> = Vec::new();
    for root in shared.roots_snapshot() {
        let hit = if root.single_run {
            // 单局根：名字匹配且 run.json 在 = 命中。
            root.path
                .file_name()
                .map(|name| name.to_string_lossy() == run_id)
                .unwrap_or(false)
                .then(|| root.path.clone())
                .filter(|dir| dir.join(crate::META_FILE).is_file())
        } else {
            is_run_dir(&root.path, run_id)
        };
        if let Some(dir) = hit {
            hits.push(dir);
        }
    }
    match hits.len() {
        0 => RunLocation::NotFound,
        1 => RunLocation::Found(hits.remove(0)),
        _ => RunLocation::Ambiguous(hits),
    }
}

/// 定位并把三态折叠成「目录或错误响应」（404 / 409；409 的 error 对象内附 roots）。
/// `not_found_code` 由路由面决定：数据端点 = `run_not_found`（既有契约），
/// /panel = `panel_unavailable`（既有契约）。
pub(crate) fn locate_run_dir_code(
    shared: &ServeShared,
    run_id: &str,
    not_found_code: &'static str,
) -> Result<PathBuf, Response> {
    match locate_run(shared, run_id) {
        RunLocation::Found(run_dir) => Ok(run_dir),
        RunLocation::NotFound => Err(error_response(
            404,
            not_found_code,
            &format!("局 `{run_id}` 不存在（run.json 缺失）"),
        )),
        RunLocation::Ambiguous(roots) => Err(Response::json(
            409,
            &json!({
                "ok": false,
                "error": {
                    "code": "run_id_ambiguous",
                    "message": format!(
                        "局 `{run_id}` 在多个观察根下同名（serve 不代选，请合并或改名其中一份）"
                    ),
                    "roots": roots
                        .iter()
                        .map(|root| json!(root.display().to_string()))
                        .collect::<Vec<_>>(),
                }
            }),
        )),
    }
}

pub(crate) fn locate_run_dir(shared: &ServeShared, run_id: &str) -> Result<PathBuf, Response> {
    locate_run_dir_code(shared, run_id, "run_not_found")
}

/// 后端裁决（只读）：run.json 的 `store` 事实 > 产物探针；失败 = 混代等，如实上抛。
pub(crate) fn resolve_backend(run_dir: &Path) -> Result<ygg_store::Backend, String> {
    let declared = crate::read_meta(&run_dir.join(crate::META_FILE))
        .and_then(|meta| meta["store"].as_str().map(str::to_owned))
        .and_then(|text| ygg_store::Backend::parse(&text));
    ygg_store::resolve(run_dir, declared, None).map_err(|err| err.to_string())
}

/// run.json 里的 mod 路径（只影响第三级 mod 面板托管；一二级数据面不依赖它）。
/// 相对路径的两级解析链：① 进程 cwd（既有语义，项目内 serve 行为不变）→
/// ② 该局所属根的父目录 / 祖父目录（规范布局 `<project>/.ygg/runs` 的直接推论：
/// 祖父 = `<project>`，`.ygg/mods/...` 命中；扁平布局 `<project>/runs` 的父目录
/// 同理命中；候选都做存在性检查，错候选无害）→ 皆不存 → 返回原值，交由既有的
/// `PanelDecl::Unavailable` 降级（不炸观察）。
pub(crate) fn run_mod_dir(meta: &Json, run_root: Option<&Path>) -> Option<PathBuf> {
    let declared = PathBuf::from(meta["mod"].as_str()?);
    if declared.is_absolute() || declared.exists() {
        return Some(declared);
    }
    if let Some(root) = run_root {
        for base in [root.parent(), root.parent().and_then(Path::parent)] {
            let Some(base) = base else { continue };
            let candidate = base.join(&declared);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    Some(declared)
}
