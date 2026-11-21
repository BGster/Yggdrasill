use super::*;

// ============================================================================
// §内置工具（`--builtin-tool <op>`）
//
// 六个子命令的**共同合约**（改动前先读）：
//
// 1. **stdout = 恰好一行**，且只在成功时输出。格式 `k=v;k=v;…`：`;` 分隔字段，
//    `,` 分隔列表项。**绝不**出现 `|`（那是外层 `tool.result` 段的分隔符，
//    `host_contract.rs` 的 wire 契约）、不出现换行、不出现 NUL。
// 2. **失败 ⇒ stdout 全空 + stderr 一行人读错误 + 退出 1**。这条是承重的：
//    `execute_cli` 只在 stdout 为空时回落读 stderr，所以「stdout 空」才是失败
//    在 `tool.result` 里可见的前提。
// 3. 每个落进输出的值都过 [`wire_sanitize`]（git 路径**可以**带逗号，所以
//    清洗是必需的，不是美化）。列表先排序再 join：同输入同字节。
// 4. 用法错误（未知旗标 / 缺必需旗标 / 坏数字）→ 退出 **2** 并列出该 op 的旗标。
// 5. 打印的路径一律先 `fs::canonicalize`：`path=<wt>` 是绝对且可复现的。
//
// 这些子命令**不读 Mod、不推进状态**：它们只操作 worktree / 制品面 / 会话目录。
// 在 `pack.json` 里写 `"program": "@self"` 即可调用（`@self` = 本可执行文件）。
// ============================================================================

/// op 级旗标集合（`--builtin-tool` 之后那批）。
///
/// `parse_args` 只负责**不拒绍**它们；真正的白名单在这里——只有这一处知道
/// 「这个 op 该有哪些旗标」，报错才能点名 op。
pub(crate) struct OpFlags {
    pub(crate) values: Vec<(String, String)>,
}

/// 全局旗标里「带值」的那批（重扫 argv 时跳过它们的值）。
pub(crate) const GLOBAL_VALUE_FLAGS: &[&str] = &[
    "--mod",
    "--run-id",
    "--runs-root",
    "--store",
    "--event",
    "--payload",
    "--drive",
    "--harness",
    "--tool-timeout",
    "--run-tool-worker",
    "--artifact-put",
    "--artifact-get",
];

/// 全局旗标里「裸旗标」的那批。
pub(crate) const GLOBAL_BARE_FLAGS: &[&str] = &[
    "--builtin-tool",
    "--resume",
    "--settle",
    "--watch",
    "--wait-tools",
];

impl OpFlags {
    /// 重扫 `argv`，只收 `allowed` 里的旗标；其余必须是已知全局旗标，否则
    /// 退出 2（点名 op 与合法旗标表）。
    pub(crate) fn parse(op: &str, allowed: &[&str]) -> Self {
        let mut values: Vec<(String, String)> = Vec::new();
        let mut iter = std::env::args().skip(1);
        while let Some(argument) = iter.next() {
            let name = argument.as_str();
            if allowed.contains(&name) {
                match iter.next() {
                    Some(value) => values.push((name.to_owned(), value)),
                    None => builtin_usage_error(op, &format!("`{name}` 缺少取值"), allowed),
                }
            } else if GLOBAL_VALUE_FLAGS.contains(&name) {
                // 全局旗标的值不属本 op，丢掉。
                let _ = iter.next();
            } else if GLOBAL_BARE_FLAGS.contains(&name) {
                // 裸旗标无值。
            } else if name.starts_with('-') {
                builtin_usage_error(op, &format!("未知旗标 `{name}`"), allowed);
            }
            // 其余位置参数（非旗标）一律忽略：它们只能来自全局位置面。
        }
        Self { values }
    }

    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .rev()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    pub(crate) fn require(&self, op: &str, name: &str, allowed: &[&str]) -> String {
        match self.get(name) {
            Some(value) if !value.trim().is_empty() => value.to_owned(),
            _ => builtin_usage_error(op, &format!("缺少必填旗标 `{name}`"), allowed),
        }
    }
}

/// 用法错误：退出 2 并列出该 op 的合法旗标表（**不**打印全局 usage：那会把
/// 一行定位信息埋在十几行通用帮助里）。
pub(crate) fn builtin_usage_error(op: &str, message: &str, allowed: &[&str]) -> ! {
    eprintln!("error: --builtin-tool {op}: {message}");
    eprintln!("合法旗标：{}", allowed.join(" "));
    exit(2)
}

/// wire 安全的字段值：`;` / `,` / `|` / 换行 / NUL 一律换成 `_`。
///
/// git 路径**可以**合法地包含逗号与空格（以及任意非 NUL 字节），所以不清洗
/// 就会把外层 `tool.result` 的字段边界撕碎。
pub(crate) fn wire_sanitize(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            ';' | ',' | '|' | '\n' | '\r' | '\0' => '_',
            other => other,
        })
        .collect()
}

/// 失败行：`op=<name>;error=<slug>;detail=<sanitized>`（stdout 保持空）。
pub(crate) fn builtin_fail(op: &str, slug: &str, detail: &str) -> i32 {
    eprintln!(
        "op={};error={};detail={}",
        wire_sanitize(op),
        wire_sanitize(slug),
        wire_sanitize(detail)
    );
    1
}

/// 捕获 `git -C <repo> …` 的标准输出（非零退出 = `Err`，带 git 自己的 stderr）。
pub(crate) fn git_capture(repo: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|err| format!("执行 git {} 失败：{err}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail = stderr.trim();
        return Err(format!(
            "git {} 失败：{}",
            args.join(" "),
            if tail.is_empty() {
                "非零退出"
            } else {
                tail
            }
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// 必须恰好一行（`rev-parse HEAD` 这类）。
pub(crate) fn git_one(repo: &Path, args: &[&str]) -> Result<String, String> {
    let text = git_capture(repo, args)?;
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    match lines.as_slice() {
        [line] => Ok((*line).to_owned()),
        _ => Err(format!(
            "git {} 的输出不是恰好一行（{} 行）",
            args.join(" "),
            lines.len()
        )),
    }
}

/// 逐行 trim（空行丢弃）。
pub(crate) fn git_lines(repo: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    Ok(git_capture(repo, args)?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

/// 目标仓库根（`--repo` 或 CWD 的 toplevel）。
pub(crate) fn repo_root(explicit: Option<&str>) -> Result<PathBuf, String> {
    let start = match explicit {
        Some(dir) => PathBuf::from(dir),
        None => std::env::current_dir().map_err(|err| format!("取 CWD 失败：{err}"))?,
    };
    let text = git_one(&start, &["rev-parse", "--show-toplevel"])?;
    Ok(PathBuf::from(text))
}

/// 仓库名（worktree 目录名的前缀；取不到就用 `repo` 兵底）。
pub(crate) fn repo_label(repo: &Path) -> String {
    repo.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".to_owned())
}

/// worktree 根目录：`--root` > `$YGG_WORKTREE_ROOT` > `$HOME/tmp/worktrees` > `<temp>/ygg-worktrees`。
pub(crate) fn worktree_root(explicit: Option<&str>) -> PathBuf {
    worktree_root_for(
        explicit,
        std::env::var("YGG_WORKTREE_ROOT").ok().as_deref(),
        home_dir().as_deref(),
    )
}

pub(crate) fn worktree_root_for(
    explicit: Option<&str>,
    env_root: Option<&str>,
    home: Option<&Path>,
) -> PathBuf {
    if let Some(dir) = explicit.map(str::trim).filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(dir) = env_root.map(str::trim).filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(home) = home {
        return home.join("tmp").join("worktrees");
    }
    std::env::temp_dir().join("ygg-worktrees")
}

/// lane 号：`1..=64`（每车道一个 worktree，编号是给人和脚本看的稳定键）。
pub(crate) fn lane_number(op: &str, text: &str, allowed: &[&str]) -> u8 {
    match text.trim().parse::<u8>() {
        Ok(value) if (1..=64).contains(&value) => value,
        _ => builtin_usage_error(
            op,
            &format!("--lane 需要 1..=64 的整数（当前是 `{text}`）"),
            allowed,
        ),
    }
}

/// 分支名：`[A-Za-z0-9._/-]`，无 `..`、不以 `/` 开头。
pub(crate) fn validate_branch(op: &str, branch: &str, allowed: &[&str]) -> String {
    let ok = !branch.is_empty()
        && !branch.starts_with('/')
        && !branch.contains("..")
        && branch
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '/' | '-'));
    if !ok {
        builtin_usage_error(
            op,
            &format!(
                "--branch `{branch}` 不合法（只允许 `[A-Za-z0-9._/-]`，不得含 `..` 或以 `/` 开头）"
            ),
            allowed,
        );
    }
    branch.to_owned()
}

/// lane 的 worktree 路径（claim / release / probe 三处**同源**，否则 release 会
/// 找错目录）。
pub(crate) fn lane_worktree_path(root: &Path, repo: &Path, lane: u8) -> PathBuf {
    root.join(format!("{}-lane-{lane}", repo_label(repo)))
}

/// 把字段拼成**一行** wire 文本（纯函数：格式断言直接测它）。
///
/// 键序 = 传入序（调用方按位解析，别重排）；值逐个过 [`wire_sanitize`]。
pub(crate) fn wire_line(fields: &[(&str, String)]) -> String {
    fields
        .iter()
        .map(|(key, value)| format!("{key}={}", wire_sanitize(value)))
        .collect::<Vec<_>>()
        .join(";")
}

/// 输出一行并返回退出码（成功路径的唯一出口；失败不走这里）。
pub(crate) fn builtin_line(fields: &[(&str, String)]) -> i32 {
    println!("{}", wire_line(fields));
    0
}

/// 失败形状：能成型为 `k=v;k=v` 行的直接给行，其余由 [`run_op`] 拼通用形状。
#[derive(Debug)]
pub(crate) enum OpFailure {
    /// 已经成型的一行（`lane=<k>;error=lane-busy;on=…;head=…` 这类带定位的现场）。
    Line(String),
    /// 通用失败：`op=<name>;error=<slug>;detail=<sanitized>`。
    Detail { slug: &'static str, detail: String },
    /// 已经自己在 stderr 上把话说完了（如 `verify-suite` 的 gate 尾部）：
    /// wrapper 不再补一行，只保持 stdout 空 + 退出 1。
    Reported,
}

impl OpFailure {
    pub(crate) fn detail(slug: &'static str, detail: impl std::fmt::Display) -> Self {
        Self::Detail {
            slug,
            detail: detail.to_string(),
        }
    }
}

/// 成功 → 打一行并退出 0；失败 → **stdout 保持空**、stderr 一行、退出 1。
pub(crate) fn run_op(op: &str, result: Result<Vec<(&'static str, String)>, OpFailure>) -> i32 {
    match result {
        Ok(fields) => builtin_line(&fields),
        Err(OpFailure::Line(line)) => {
            eprintln!("{line}");
            1
        }
        Err(OpFailure::Detail { slug, detail }) => builtin_fail(op, slug, &detail),
        Err(OpFailure::Reported) => 1,
    }
}

/// 内置工具入口（六个 op 的分发；argv 在 [`OpFlags`] 里二次解析）。
pub(crate) fn run_builtin_tool(args: &Args) -> i32 {
    let Some(tool) = args.builtin_tool else {
        // main 只在 `is_some()` 时进来；防御性兜底不走 usage（那是 2 号退出）。
        return builtin_fail("builtin", "no-op", "缺少 --builtin-tool 取值");
    };
    match tool {
        BuiltinTool::WorktreeClaim => run_op(tool.as_str(), claim_fields()),
        BuiltinTool::WorktreeRelease => run_op(tool.as_str(), release_fields()),
        BuiltinTool::LaneProbe => run_op(tool.as_str(), lane_probe_fields()),
        BuiltinTool::ArtifactProbe => run_op(tool.as_str(), artifact_probe_fields()),
        BuiltinTool::VerifySuite => run_op(tool.as_str(), verify_suite_fields()),
        BuiltinTool::ArchiveSessions => run_op(tool.as_str(), archive_sessions_fields(args)),
    }
}

// ---- worktree-claim ----

pub(crate) const CLAIM_FLAGS: &[&str] = &["--lane", "--req", "--branch", "--root", "--repo"];

pub(crate) fn claim_fields() -> Result<Vec<(&'static str, String)>, OpFailure> {
    claim_fields_with(&OpFlags::parse("worktree-claim", CLAIM_FLAGS))
}

/// [`claim_fields`] 的注入面（测试直接给旗标，不读进程 argv）。
pub(crate) fn claim_fields_with(flags: &OpFlags) -> Result<Vec<(&'static str, String)>, OpFailure> {
    const OP: &str = "worktree-claim";
    let lane = lane_number(OP, &flags.require(OP, "--lane", CLAIM_FLAGS), CLAIM_FLAGS);
    let req = flags.require(OP, "--req", CLAIM_FLAGS);
    let branch = validate_branch(OP, &flags.require(OP, "--branch", CLAIM_FLAGS), CLAIM_FLAGS);
    // `req` 形态：`r<数字>`（Issue 里泳道的稳定标识）。
    let req_ok =
        req.len() > 1 && req.starts_with('r') && req[1..].chars().all(|ch| ch.is_ascii_digit());
    if !req_ok {
        builtin_usage_error(
            OP,
            &format!("--req 需要 `r<数字>` 形态（当前是 `{req}`）"),
            CLAIM_FLAGS,
        );
    }

    let repo = repo_root(flags.get("--repo")).map_err(|err| OpFailure::detail("repo", err))?;
    let base =
        git_one(&repo, &["rev-parse", "HEAD"]).map_err(|err| OpFailure::detail("repo", err))?;
    let wt = lane_worktree_path(&worktree_root(flags.get("--root")), &repo, lane);

    // ⓪ 若 `<branch>` 已被其他 worktree 检出（例如 gate reject 后需求重派到另一条泳道），
    // 先将那个 worktree 切为 `--detach HEAD` 释放分支锁。
    let want = fs::canonicalize(&wt).unwrap_or_else(|_| wt.clone());
    let branch_ref = format!("refs/heads/{branch}");
    if let Ok(lines) = git_lines(&repo, &["worktree", "list", "--porcelain"]) {
        let mut current_wt: Option<PathBuf> = None;
        for line in &lines {
            if let Some(path) = line.strip_prefix("worktree ") {
                current_wt = Some(PathBuf::from(path));
            } else if let Some(b) = line.strip_prefix("branch ") {
                if b == branch_ref {
                    if let Some(ref other_wt) = current_wt {
                        let got = fs::canonicalize(other_wt).unwrap_or_else(|_| other_wt.clone());
                        if got != want && other_wt.is_dir() {
                            let _ = git_capture(other_wt, &["checkout", "--detach", "HEAD"]);
                        }
                    }
                }
            }
        }
    }

    // ① 若泳道目录与 `.git` 已存在（持久化泳道池）：原位复用（`reused=1`），
    // `clean -fd` 不带 `-x`，保留 `target/`、`node_modules/` 等 gitignored 构建缓存。
    if wt.is_dir() && wt.join(".git").exists() {
        git_capture(&wt, &["checkout", "--force", "-B", &branch, &base])
            .map_err(|err| OpFailure::detail("worktree-checkout", err))?;
        git_capture(&wt, &["reset", "--hard", &base])
            .map_err(|err| OpFailure::detail("worktree-reset", err))?;
        git_capture(&wt, &["clean", "-fd"])
            .map_err(|err| OpFailure::detail("worktree-clean", err))?;
        return Ok(claim_output(lane, &req, &branch, &base, true, &wt));
    }

    // ② 新建（`reused=0`）：先 prune 清理可能残留的陈旧注册，再用 `-B` 挂载分支。
    if let Some(parent) = wt.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| OpFailure::detail("mkdir", format!("{}：{err}", parent.display())))?;
    }
    let _ = git_capture(&repo, &["worktree", "prune"]);
    let wt_text = wt.display().to_string();
    git_capture(&repo, &["worktree", "add", "-B", &branch, &wt_text, &base])
        .map_err(|err| OpFailure::detail("worktree-add", err))?;

    // ③ 新 worktree 必须干净：脏 = 基线本身脏，**报告，不修**。
    let status = git_lines(&wt, &["status", "--porcelain"])
        .map_err(|err| OpFailure::detail("status", err))?;
    if !status.is_empty() {
        return Err(OpFailure::Line(format!(
            "lane={lane};req={req};branch={branch};clean=0;path={}",
            wire_sanitize(&wt.display().to_string())
        )));
    }
    Ok(claim_output(lane, &req, &branch, &base, false, &wt))
}

/// 成功行（**键序固定**：调用方按位解析，别重排）。
pub(crate) fn claim_output(
    lane: u8,
    req: &str,
    branch: &str,
    base: &str,
    reused: bool,
    wt: &Path,
) -> Vec<(&'static str, String)> {
    let path = fs::canonicalize(wt)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| wt.display().to_string());
    vec![
        ("lane", lane.to_string()),
        ("req", req.to_owned()),
        ("branch", branch.to_owned()),
        ("base", base.to_owned()),
        ("clean", "1".to_owned()),
        ("reused", if reused { "1" } else { "0" }.to_owned()),
        ("path", path),
    ]
}

// ---- worktree-release ----

pub(crate) const RELEASE_FLAGS: &[&str] = &["--lane", "--root", "--repo"];

pub(crate) fn release_fields() -> Result<Vec<(&'static str, String)>, OpFailure> {
    release_fields_with(&OpFlags::parse("worktree-release", RELEASE_FLAGS))
}

/// [`release_fields`] 的注入面（测试直接给旗标，不读进程 argv）。
pub(crate) fn release_fields_with(
    flags: &OpFlags,
) -> Result<Vec<(&'static str, String)>, OpFailure> {
    const OP: &str = "worktree-release";
    let lane = lane_number(
        OP,
        &flags.require(OP, "--lane", RELEASE_FLAGS),
        RELEASE_FLAGS,
    );
    let repo = repo_root(flags.get("--repo")).map_err(|err| OpFailure::detail("repo", err))?;
    let wt = lane_worktree_path(&worktree_root(flags.get("--root")), &repo, lane);
    if wt.is_dir() && wt.join(".git").exists() {
        git_capture(&wt, &["checkout", "--force", "--detach", "HEAD"])
            .map_err(|err| OpFailure::detail("worktree-detach", err))?;
        git_capture(&wt, &["reset", "--hard", "HEAD"])
            .map_err(|err| OpFailure::detail("worktree-reset", err))?;
        git_capture(&wt, &["clean", "-fd"])
            .map_err(|err| OpFailure::detail("worktree-clean", err))?;
        let path = fs::canonicalize(&wt)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| wt.display().to_string());
        return Ok(vec![
            ("lane", lane.to_string()),
            ("released", "1".to_owned()),
            ("parked", "1".to_owned()),
            ("path", path),
        ]);
    }
    // 目录不存在：prune 清理可能残留的注册。
    let _ = git_capture(&repo, &["worktree", "prune"]);
    Ok(vec![
        ("lane", lane.to_string()),
        ("released", "1".to_owned()),
        ("parked", "0".to_owned()),
        ("path", wt.display().to_string()),
    ])
}

// ---- lane-probe ----

pub(crate) const PROBE_FLAGS: &[&str] = &["--lane", "--base", "--branch", "--root", "--repo"];

pub(crate) fn lane_probe_fields() -> Result<Vec<(&'static str, String)>, OpFailure> {
    lane_probe_fields_with(&OpFlags::parse("lane-probe", PROBE_FLAGS))
}

/// [`lane_probe_fields`] 的注入面（测试直接给旗标，不读进程 argv）。
pub(crate) fn lane_probe_fields_with(
    flags: &OpFlags,
) -> Result<Vec<(&'static str, String)>, OpFailure> {
    const OP: &str = "lane-probe";
    let lane = lane_number(OP, &flags.require(OP, "--lane", PROBE_FLAGS), PROBE_FLAGS);
    // `base` 原样回显（短 sha 保持短）：调用方要能对上自己的台账。
    let base = flags.require(OP, "--base", PROBE_FLAGS);
    let branch = validate_branch(OP, &flags.require(OP, "--branch", PROBE_FLAGS), PROBE_FLAGS);
    let repo = repo_root(flags.get("--repo")).map_err(|err| OpFailure::detail("repo", err))?;
    let spec = format!("{base}..{branch}");
    let commits = git_one(&repo, &["rev-list", "--count", &spec])
        .map_err(|err| OpFailure::detail("rev-list", err))?;
    let head = git_one(&repo, &["rev-parse", &branch])
        .map_err(|err| OpFailure::detail("rev-parse", err))?;
    // 干净度在**泳道 worktree**（若已在）里测：车道自己的未提交改动不在仓库根。
    let lane_wt = lane_worktree_path(&worktree_root(flags.get("--root")), &repo, lane);
    let tree = if lane_wt.is_dir() {
        lane_wt
    } else {
        repo.clone()
    };
    let clean = git_lines(&tree, &["status", "--porcelain"])
        .map(|lines| u8::from(lines.is_empty()))
        .map_err(|err| OpFailure::detail("status", err))?;
    let mut files = git_lines(&repo, &["diff", "--name-only", &spec])
        .map_err(|err| OpFailure::detail("diff", err))?;
    files.sort();
    Ok(vec![
        ("lane", lane.to_string()),
        ("branch", branch),
        ("base", base),
        ("head", head),
        ("commits", commits),
        ("clean", clean.to_string()),
        ("files", files.join(",")),
    ])
}

// ---- artifact-probe ----

pub(crate) const ARTIFACT_PROBE_FLAGS: &[&str] = &["--path", "--run-dir"];

/// `artifact-probe`：本局某件制品在不在、多大、什么摘要。
///
/// **永远退出 0**（只要读取路径本身没崩）：这是个**探针**，`exists=0` 是 Mod
/// 必须能读到的合法答案（它据此分支）。别把它「修」成缺失即退出 1。
pub(crate) fn artifact_probe_fields() -> Result<Vec<(&'static str, String)>, OpFailure> {
    artifact_probe_fields_with(&OpFlags::parse("artifact-probe", ARTIFACT_PROBE_FLAGS))
}

/// [`artifact_probe_fields`] 的注入面（测试直接给旗标，不读进程 argv）。
pub(crate) fn artifact_probe_fields_with(
    flags: &OpFlags,
) -> Result<Vec<(&'static str, String)>, OpFailure> {
    const OP: &str = "artifact-probe";
    let path = flags.require(OP, "--path", ARTIFACT_PROBE_FLAGS);
    if let Err(err) = ygg_store::validate_artifact_path(&path) {
        // 调用方 bug（不是 miss）：违法的路径不能当成「不存在」。
        return Err(OpFailure::detail("path", err));
    }
    let run_dir = flags.get("--run-dir").map(PathBuf::from).or_else(|| {
        std::env::var("YGG_RUN_DIR")
            .ok()
            .filter(|dir| !dir.trim().is_empty())
            .map(PathBuf::from)
    });
    let Some(run_dir) = run_dir else {
        builtin_usage_error(
            OP,
            "需要 --run-dir（或环境变量 YGG_RUN_DIR）",
            ARTIFACT_PROBE_FLAGS,
        );
    };
    match probe_artifact(&run_dir, &path).map_err(|err| OpFailure::detail("read", err))? {
        Some(record) => Ok(vec![
            ("path", record.meta.path),
            ("exists", "1".to_owned()),
            ("bytes", record.meta.bytes.to_string()),
            ("sha256", record.meta.sha256),
        ]),
        None => Ok(vec![
            ("path", path),
            ("exists", "0".to_owned()),
            ("bytes", "0".to_owned()),
            ("sha256", String::new()),
        ]),
    }
}

/// 读一件制品（已存在的局走端口；不存在的局走纯文件面——**不建库**）。
pub(crate) fn probe_artifact(
    run_dir: &Path,
    path: &str,
) -> Result<Option<ygg_store::ArtifactRecord>, String> {
    if !run_dir_exists(run_dir) {
        return Ok(read_artifact_file(run_dir, path));
    }
    match open_store_resolved(run_dir, None) {
        Ok(store) => store.artifact_get(path).map_err(|err| err.to_string()),
        // 端口打不开（混代 / 坏库）：回落文件面，而不是把探针变成失败。
        Err(_) => Ok(read_artifact_file(run_dir, path)),
    }
}

/// 纯文件面读制品（JSON 后端布局；不存在 = `None`）。
pub(crate) fn read_artifact_file(run_dir: &Path, path: &str) -> Option<ygg_store::ArtifactRecord> {
    let content = fs::read(run_dir.join(path)).ok()?;
    Some(ygg_store::ArtifactRecord {
        meta: ygg_store::ArtifactMeta {
            path: path.to_owned(),
            bytes: content.len(),
            // 宿主不自己拼一份 SHA-256：用端口暴露的唯一实现（同编码口径）。
            sha256: ygg_store::sha256_hex(&content),
            updated_at: 0,
        },
        content,
    })
}
