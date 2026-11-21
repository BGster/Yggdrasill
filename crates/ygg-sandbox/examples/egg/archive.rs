use super::*;

// ---- archive-sessions ----

pub(crate) const ARCHIVE_FLAGS: &[&str] = &["--parent", "--batch", "--sessions-dir", "--trash"];

/// 标识符白名单（`[A-Za-z0-9._-]`，不含 `..`）。
pub(crate) fn archive_ident_ok(text: &str, max: usize) -> bool {
    !text.is_empty()
        && text.len() <= max
        && !text.contains("..")
        && text
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

/// `archive-sessions`：把提到了某个 parent 会话的 `.jsonl` 转录档搬进 trash。
///
/// **数据红线**（项目 `AGENTS.md` §0 硬约束）：本 op 只碰 agent 会话转录，
/// **绝不能**打开 `*.xlsx` / `*.xls` / `*.csv` 之类业务数据。实现上由两件事
/// 兜底：① 只按扩展名 `.jsonl` 入选；② 文件内容**从不回显**（匹配是布尔值）。
pub(crate) fn archive_sessions_fields(
    args: &Args,
) -> Result<Vec<(&'static str, String)>, OpFailure> {
    archive_sessions_fields_with(
        &OpFlags::parse("archive-sessions", ARCHIVE_FLAGS),
        &ArchiveInputs::from_process(args),
    )
}

/// harness 适配器的全部取值（`--harness` / `YGG_HARNESS` / `harness.adapter` 同一份）。
pub(crate) const HARNESS_ADAPTERS: &[&str] = &["auto", "pi", "claude-code", "codex", "generic"];

/// `auto` 探测的**输入面**（纯值：单测注入，不读进程环境）。
pub(crate) struct HarnessProbe {
    pub(crate) pi_session_id: bool,
    pub(crate) claude_code: bool,
    pub(crate) codex_home: bool,
}

/// 工作区路径 → 会话库路径段（`/Users/x/Repo` → `Users-x-Repo`）。
///
/// pi 的 `--<slug>--` 与 Claude Code 的 `-<slug>` 用的是**同一个** slug 口径（都是
/// cwd 去掉前导 `/` 后把 `/` 换成 `-`，差别只在两边的包裹字符）。
pub(crate) fn cwd_slug(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .trim_start_matches('/')
        .replace('/', "-")
}

/// `auto` 的现场探测（纯函数：probe / home / cwd 全可注入，不读进程环境）。
///
/// 顺序即优先级：**Pi 的痕迹最强**（会话环境变量 / 工作区的 `.pi` / 该工作区自己的
/// 会话目录），其次 Claude Code，其次 Codex；都没有 = `generic`——
/// `generic` **没有会话库**，归档记 `moved=0;remaining=0`，这不是失败。
pub(crate) fn detect_harness(
    probe: &HarnessProbe,
    home: Option<&Path>,
    cwd: &Path,
) -> &'static str {
    let pi_sessions = home.map(|home| {
        home.join(".pi")
            .join("agent")
            .join("sessions")
            .join(format!("--{}--", cwd_slug(cwd)))
    });
    if probe.pi_session_id || cwd.join(".pi").exists() || pi_sessions.is_some_and(|d| d.is_dir()) {
        return "pi";
    }
    if probe.claude_code || cwd.join(".claude").exists() {
        return "claude-code";
    }
    if probe.codex_home {
        return "codex";
    }
    "generic"
}

/// 三层解析的**唯一收敛点**：`explicit`（`--harness`）> `env_adapter`
/// （`YGG_HARNESS`，其次 `data/config.json` 的 `harness.adapter`）> 现场探测。
///
/// 返回值恒为 [`HARNESS_ADAPTERS`] 里的一个**非 auto** 值——调用方拿到就能直接查表。
/// 未知取值（`--harness` 已在解析期点名校验，这里只剩程序化调用）→ 当 auto 处理。
pub(crate) fn resolve_harness_adapter(
    explicit: Option<&str>,
    env_adapter: Option<&str>,
    cwd: &Path,
) -> &'static str {
    let requested = [explicit, env_adapter]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|value| !value.is_empty())
        .unwrap_or("auto");
    match requested {
        "pi" => "pi",
        "claude-code" => "claude-code",
        "codex" => "codex",
        "generic" => "generic",
        _ => detect_from_process(cwd),
    }
}

/// `auto` 的**进程探测**（环境变量 + `$HOME`；纯函数 [`detect_harness`] 的取值层）。
pub(crate) fn detect_from_process(cwd: &Path) -> &'static str {
    let probe = HarnessProbe {
        pi_session_id: std::env::var_os("PI_SESSION_ID").is_some(),
        claude_code: std::env::var_os("CLAUDE_CODE").is_some(),
        codex_home: std::env::var_os("CODEX_HOME").is_some(),
    };
    let home = home_dir();
    detect_harness(&probe, home.as_deref(), cwd)
}

/// `$HOME`（空 / 非 UTF-8 一律视为缺席：会话库是**推导面**，不是契约面）。
pub(crate) fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .filter(|home| !home.trim().is_empty())
        .map(PathBuf::from)
}

/// 每个适配器自己的会话库落点 `(sessions 目录, trash 根)`。
///
/// `generic`（以及任何未知适配器）→ `None`：它没有会话库，归档只能记 0/0 台账。
pub(crate) fn harness_session_dirs(
    adapter: &str,
    home: &Path,
    slug: &str,
) -> Option<(PathBuf, PathBuf)> {
    match adapter {
        "pi" => Some((
            home.join(".pi")
                .join("agent")
                .join("sessions")
                .join(format!("--{slug}--")),
            home.join(".pi").join("agent").join("sessions-trash"),
        )),
        "claude-code" => Some((
            home.join(".claude")
                .join("projects")
                .join(format!("-{slug}")),
            home.join(".claude").join("sessions-trash"),
        )),
        "codex" => Some((
            home.join(".codex").join("sessions"),
            home.join(".codex").join("sessions-trash"),
        )),
        _ => None,
    }
}

/// `data/config.json` 的 `harness.adapter`（路径由 `pack.json` 声明；缺段 / 非串 / 空 →
/// `None`）。这一层只是**静态默认**：本 Mod 的 `agent-config.json` 已把取值限定在
/// [`HARNESS_ADAPTERS`]，所以这里不再重复校验枚举。
pub(crate) fn harness_config_adapter(mod_dir: Option<&Path>) -> Option<String> {
    let dir = mod_dir?;
    let config = read_json(&dir.join(agent_config_path(dir)))?;
    let adapter = config["harness"]["adapter"].as_str()?.trim();
    (!adapter.is_empty()).then(|| adapter.to_owned())
}

/// `archive-sessions` 的**输入面**：进程环境与旗标的唯一注入点（单测不碰 env）。
///
/// Mod dir 决定「`harness.adapter` 这一层能否读到」：`--mod` 优先，其次宿主给 worker
/// 注入的 `$ENV{YGG_MOD_DIR}`（工具桥的常规路径）；两者都缺 = 该层不表态。
pub(crate) struct ArchiveInputs {
    pub(crate) harness: Option<String>,
    pub(crate) mod_dir: Option<PathBuf>,
    pub(crate) sessions_dir: Option<PathBuf>,
    pub(crate) home: Option<PathBuf>,
    pub(crate) cwd: Option<PathBuf>,
    pub(crate) env_harness: Option<String>,
}

impl ArchiveInputs {
    fn from_process(args: &Args) -> Self {
        let env = |key: &str| {
            std::env::var(key)
                .ok()
                .filter(|value| !value.trim().is_empty())
        };
        Self {
            harness: args.harness.clone(),
            mod_dir: args
                .mod_dir
                .clone()
                .or_else(|| env("YGG_MOD_DIR").map(PathBuf::from)),
            sessions_dir: env("YGG_SESSIONS_DIR").map(PathBuf::from),
            home: env("HOME").map(PathBuf::from),
            cwd: std::env::current_dir().ok(),
            env_harness: env("YGG_HARNESS"),
        }
    }

    /// 本局生效的适配器：`--harness` > `YGG_HARNESS` > `harness.adapter` > auto。
    fn adapter(&self) -> &'static str {
        let config = harness_config_adapter(self.mod_dir.as_deref());
        let declared = self
            .env_harness
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .or(config.as_deref());
        let cwd = self.cwd.clone().unwrap_or_else(|| PathBuf::from("."));
        resolve_harness_adapter(self.harness.as_deref(), declared, &cwd)
    }
}

/// 成功回执的三段（`moved=..;remaining=..;dest=..`；顺序稳定便于比对）。
pub(crate) fn archive_outcome(
    moved: usize,
    remaining: usize,
    dest: String,
) -> Vec<(&'static str, String)> {
    vec![
        ("moved", moved.to_string()),
        ("remaining", remaining.to_string()),
        ("dest", dest),
    ]
}

/// [`archive_sessions_fields`] 的注入面（测试直接给旗标 + 输入面，不读进程 argv / env）。
pub(crate) fn archive_sessions_fields_with(
    flags: &OpFlags,
    inputs: &ArchiveInputs,
) -> Result<Vec<(&'static str, String)>, OpFailure> {
    const OP: &str = "archive-sessions";
    let parent = flags.require(OP, "--parent", ARCHIVE_FLAGS);
    let batch = flags.require(OP, "--batch", ARCHIVE_FLAGS);
    if !archive_ident_ok(&parent, 128) {
        builtin_usage_error(
            OP,
            "--parent 只允许 `[A-Za-z0-9._-]`（1..=128 字节，不含 `..`）",
            ARCHIVE_FLAGS,
        );
    }
    if !archive_ident_ok(&batch, 64) {
        builtin_usage_error(
            OP,
            "--batch 只允许 `[A-Za-z0-9._-]`（1..=64 字节，不含 `..`）",
            ARCHIVE_FLAGS,
        );
    }
    // 优先级：显式 `--sessions-dir` > `YGG_SESSIONS_DIR` > **适配器推导的会话库落点**。
    // 前两层是调用方点名的目录，口径固定（缺目录 = 扫 0 件，不报错、不改写）；
    // 第三层按 harness 适配器推导：目录不存在 = 记 `<适配器>:empty:<batch>`，
    // `generic` 根本没有会话库 = 记 `generic:no-session-store:<batch>`——两者都是
    // **合法答案**（换 harness 本来就收不出东西），不能让收口卡在归档上。
    let explicit_dir = flags
        .get("--sessions-dir")
        .map(PathBuf::from)
        .or_else(|| inputs.sessions_dir.clone());
    let (sessions_dir, default_trash_root) = match explicit_dir {
        Some(dir) => {
            let trash_root = dir.parent().unwrap_or_else(|| Path::new(".")).join("trash");
            (dir, trash_root)
        }
        None => {
            let adapter = inputs.adapter();
            let cwd = inputs.cwd.clone().unwrap_or_else(|| PathBuf::from("."));
            let dirs = inputs
                .home
                .as_deref()
                .and_then(|home| harness_session_dirs(adapter, home, &cwd_slug(&cwd)));
            let Some((dir, trash_root)) = dirs else {
                // 适配器没有会话库（generic）= 合法答案；有库但推不出来（缺 HOME）= 用法错误。
                if adapter == "generic" {
                    return Ok(archive_outcome(
                        0,
                        0,
                        format!("{adapter}:no-session-store:{batch}"),
                    ));
                }
                builtin_usage_error(
                    OP,
                    "需要 --sessions-dir（或环境变量 YGG_SESSIONS_DIR / HOME）",
                    ARCHIVE_FLAGS,
                );
            };
            if !dir.is_dir() {
                return Ok(archive_outcome(0, 0, format!("{adapter}:empty:{batch}")));
            }
            (dir, trash_root)
        }
    };
    let trash = flags
        .get("--trash")
        .map(PathBuf::from)
        .unwrap_or_else(|| default_trash_root.join(&batch));
    let outcome = sweep_sessions(&sessions_dir, &trash, &parent)
        .map_err(|err| OpFailure::detail("sweep", err))?;
    let dest = fs::canonicalize(&trash)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| trash.display().to_string());
    if outcome.remaining > 0 {
        for failure in &outcome.failures {
            eprintln!("archive={}", wire_sanitize(failure));
        }
        // 清扫不全：行已打（调用方看得见进度），但退出码是失败。
        return Err(OpFailure::Detail {
            slug: "incomplete",
            detail: format!(
                "moved={};remaining={};dest={}",
                outcome.moved,
                outcome.remaining,
                wire_sanitize(&dest)
            ),
        });
    }
    Ok(archive_outcome(outcome.moved, outcome.remaining, dest))
}

/// 一次清扫的结果（`remaining > 0` = 清扫不全）。
pub(crate) struct SweepOutcome {
    pub(crate) moved: usize,
    pub(crate) remaining: usize,
    pub(crate) failures: Vec<String>,
}

/// 逐件扫 `sessions_dir` 下提到了 `parent` 的 `.jsonl`，rename 进 `trash`，
/// 然后重扫数剩下的。**匹配是布尔值：文件内容永不出现在输出里。**
pub(crate) fn sweep_sessions(
    sessions_dir: &Path,
    trash: &Path,
    parent: &str,
) -> Result<SweepOutcome, String> {
    fs::create_dir_all(trash).map_err(|err| format!("创建 {} 失败：{err}", trash.display()))?;
    let matches = |path: &Path, name: &str| -> bool {
        if !name.ends_with(".jsonl") || name.contains(parent) {
            return false;
        }
        fs::read_to_string(path)
            .map(|contents| contents.contains(parent))
            .unwrap_or(false)
    };

    let mut moved = 0usize;
    let mut failures: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(sessions_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !matches(&entry.path(), &name) {
                continue;
            }
            match fs::rename(entry.path(), trash.join(&name)) {
                Ok(()) => moved += 1,
                Err(err) => failures.push(format!("{name}：{err}")),
            }
        }
    }
    let mut remaining = 0usize;
    if let Ok(entries) = fs::read_dir(sessions_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if matches(&entry.path(), &name) {
                remaining += 1;
            }
        }
    }
    Ok(SweepOutcome {
        moved,
        remaining,
        failures,
    })
}

/// 重置原因（写进后端的归档台账；只用于审计，不参与判定）。
pub(crate) fn reset_reason(recheck: bool) -> &'static str {
    if recheck {
        "resume-failed"
    } else {
        "explicit-new-game"
    }
}

#[cfg(test)]
mod tests;
