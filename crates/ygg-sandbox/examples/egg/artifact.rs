use super::*;

// ============================================================================
// §inbox 异步需求队列的宿主侧：追加 / 载荷构造 / 拒绝识别 / 步数预算
// ——"新需求随时能进"的真实入口（不依赖 Mod 内部提示词）。
// ============================================================================

/// `--requirement` 的载荷切分：`标题|描述`（只切首段——描述里的竖线保留）。
/// 标题 / 描述里的 `|` 换成全角 `｜`：竖线是 Mod 侧的分段分隔符，混进去会把
/// 描述尾部误当幂等键。
pub(crate) fn split_requirement(text: &str) -> (String, String) {
    let (title, description) = match text.split_once('|') {
        Some((head, tail)) => (head.trim(), tail.trim()),
        None => (text.trim(), ""),
    };
    (sanitize_field(title), sanitize_field(description))
}

/// Mod 侧拒绝的识别（叙事前缀）：内核信封仍是 ok，宿主必须显式看叙事。
///
/// 前缀集合由 Mod 契约声明（`agent.rejectionPrefixes`），宿主不认识具体文案。
pub(crate) fn dispatch_rejected(contract: &AgentContract, receipt: &Json) -> bool {
    narrative_lines(receipt)
        .iter()
        .any(|line| contract.is_rejection(line))
}

/// 拒绝原因（回执里的第一条拒绝叙事；缺失则给错误码文案）。
pub(crate) fn first_rejection(contract: &AgentContract, receipt: &Json) -> String {
    narrative_lines(receipt)
        .into_iter()
        .find(|line| contract.is_rejection(line))
        .unwrap_or_else(|| error_text(receipt))
}
/// pendingChoice 指纹（自动推进的「无进展」判据：提示词 + 全部选项文本）。
pub(crate) fn choice_signature(receipt: &Json) -> String {
    let Some(choice) = receipt["pendingChoice"].as_object() else {
        return "(none)".to_owned();
    };
    let prompt = choice["prompt"].as_str().unwrap_or("");
    let options = choice["options"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .collect::<Vec<_>>()
        .join("\u{1f}");
    format!("{prompt}\u{1e}{options}")
}

/// run 目录里**尚未消费**的 inbox 行数（现读端口 + 现读 cursor，不信任缓存）。
/// 这是结算硬门禁：inbox 还有行却报 `ok=true` = 静默丢需求。
///
/// 口径必须与消费侧（`drain_inbox_locked`）完全一致，所以两者都只走端口：
/// 自己数文件会让 sqlite 局直接报 0（静默放行）。
pub(crate) fn unconsumed_inbox_lines(store: &dyn RunStore) -> usize {
    let lines = store.inbox_len().unwrap_or(0);
    let consumed = store.inbox_cursor().unwrap_or(0);
    lines.saturating_sub(consumed)
}

/// 解析并打开本局的持久化后端（宿主**唯一**入口）。
///
/// `run.json` 的 `store` 字段是权威事实，`--store` 只是开局断言：两者不一致、
/// 或目录里混着另一后端的产物 → `Err`（fail-closed，零写入）。
pub(crate) fn open_store_resolved(
    run_dir: &Path,
    flag: Option<Backend>,
) -> Result<Box<dyn RunStore>, String> {
    let declared = read_meta(&run_dir.join(META_FILE))
        .and_then(|meta| meta["store"].as_str().map(str::to_owned))
        .and_then(|text| Backend::parse(&text));
    let backend = ygg_store::resolve(run_dir, declared, flag).map_err(|err| err.to_string())?;
    open_store(backend, run_dir)
}

/// 按后端打开端口（建目录已由调用方完成）。
///
/// sqlite 后端的可用性**不在这里判断**：默认构建（`default = ["store-sqlite"]`）
/// 就带 sqlite；只有 `--no-default-features` 的回退构建才会走到 `Unavailable`，
/// 由 `ygg_store::open` fail-closed 报「本构建未启用 sqlite 后端」。
pub(crate) fn open_store(backend: Backend, run_dir: &Path) -> Result<Box<dyn RunStore>, String> {
    ygg_store::open(backend, run_dir, clock()).map_err(|err| err.to_string())
}

/// 宿主时钟（端口不取时铜——时间戳一律由这里给）。
pub(crate) fn clock() -> Tick {
    Tick::at(now_secs(), now_nanos())
}

/// 本局目录是否已经存在（有 `run.json` 或任一后端产物）。
///
/// 制品旗标**不新造 run**：`ygg_store::resolve` 对空目录会落默认后端产物（= 半个局），
/// 所以“目录里还没东西”就是可读拒绝，而不是静默建一个空局。
pub(crate) fn run_dir_exists(run_dir: &Path) -> bool {
    run_dir.join(META_FILE).exists()
        || ygg_store::detect(run_dir).any()
        || run_dir.join(ygg_store::SQLITE_FILE_NAME).exists()
}

/// `--artifact-put` 的核心（可测）：读源文件 → 写本局后端制品面 → 回执 JSON。
///
/// 路径与大小门在**端口内**（`ygg_store::validate_artifact_path`），宿主不重写
/// 一份；`bytes` / `sha256` 也由端口给，宿主不现算（否则又是两份口径）。
pub(crate) fn artifact_put_receipt(args: &Args, spec: &str) -> Result<Json, String> {
    let (path, source) = spec.split_once('=').ok_or_else(|| {
        format!("--artifact-put 需要 `artifacts/<名>=<源文件路径>` 形态（当前是 `{spec}`）")
    })?;
    let path = path.trim();
    let source = source.trim();
    if path.is_empty() || source.is_empty() {
        return Err(format!(
            "--artifact-put 的路径与源文件路径都不能为空（当前是 `{spec}`）"
        ));
    }
    let run_dir = args.run_root.clone();
    if !run_dir_exists(&run_dir) {
        return Err(format!(
            "本局目录 {} 不存在（或还没有任何后端产物）——制品旗标不新造 run，\
             请先用 `egg new` 开局",
            run_dir.display()
        ));
    }
    let content = fs::read(source).map_err(|err| format!("读取源文件 {source} 失败：{err}"))?;
    let mut store = open_store_resolved(&run_dir, args.store)?;
    let meta = store
        .artifact_put(path, &content, clock())
        .map_err(|err| err.to_string())?;
    Ok(json!({
        "ok": true,
        "path": meta.path,
        "bytes": meta.bytes,
        "sha256": meta.sha256,
    }))
}

/// `--artifact-get` 的核心（可测）：制品原文（缺失 = `Err`，stdout 由调用方保证为空）。
pub(crate) fn artifact_get_bytes(args: &Args, path: &str) -> Result<Vec<u8>, String> {
    let path = path.trim();
    let run_dir = args.run_root.clone();
    if !run_dir_exists(&run_dir) {
        return Err(format!(
            "本局目录 {} 不存在（或还没有任何后端产物）",
            run_dir.display()
        ));
    }
    let store = open_store_resolved(&run_dir, args.store)?;
    match store.artifact_get(path) {
        Ok(Some(record)) => Ok(record.content),
        Ok(None) => Err(format!(
            "制品 `{path}` 不存在（本局 {}，后端 {}）",
            run_dir.display(),
            store.backend()
        )),
        Err(err) => Err(format!("读制品失败：{err}")),
    }
}

/// `--artifact-put` 的 CLI 外壳：stdout 只放回执 JSON。
pub(crate) fn run_artifact_put(args: &Args, spec: &str) -> i32 {
    // 形态错误 = 用法错误（退出 2）；其余 = 运行期失败（退出 1 + 回执）。
    if !spec.contains('=') {
        eprintln!(
            "error: --artifact-put 需要 `artifacts/<名>=<源文件路径>` 形态（当前是 `{spec}`）"
        );
        return 2;
    }
    match artifact_put_receipt(args, spec) {
        Ok(receipt) => {
            println!("{receipt}");
            0
        }
        Err(message) => {
            println!(
                "{}",
                json!({"ok": false, "error": {"code": "ARTIFACT", "message": message}})
            );
            1
        }
    }
}

/// `--artifact-get` 的 CLI 外壳：制品原文直出 stdout（**不补换行**，字节保真）。
///
/// 缺失 / 打开失败 → stderr 可读错误 + 退出 1（stdout 保持空）——这样
/// `$(agent --artifact-get …)` 拿到的要么是完整原文、要么是空串，不会混。
pub(crate) fn run_artifact_get(args: &Args, path: &str) -> i32 {
    match artifact_get_bytes(args, path) {
        Ok(bytes) => {
            // 不用 `println!`：它会补一个调用方没要的换行，也会弄坏二进制相近的内容。
            let mut stdout = std::io::stdout();
            match stdout.write_all(&bytes).and_then(|()| stdout.flush()) {
                Ok(()) => 0,
                Err(err) => {
                    eprintln!("error: 写 stdout 失败：{err}");
                    1
                }
            }
        }
        Err(message) => {
            eprintln!("error: {message}");
            1
        }
    }
}
