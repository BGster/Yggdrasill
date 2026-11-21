use super::*;

/// 环境变量三层巢（解析序：进程环境 > `<mod>/.env.local` > `<mod>/.env`）。
pub(crate) struct EnvTable {
    pub(crate) process: Vec<(String, String)>,
    pub(crate) env_local: Vec<(String, String)>,
    pub(crate) env: Vec<(String, String)>,
}

impl EnvTable {
    pub(crate) fn lookup(&self, key: &str) -> Option<&str> {
        for table in [&self.process, &self.env_local, &self.env] {
            if let Some((_, value)) = table.iter().find(|(name, _)| name == key) {
                return Some(value);
            }
        }
        None
    }
}

/// 读三层环境变量（Mod 目录缺 `.env*` → 空表，不报错；缺失在解析处才报）。
///
/// **不**读 CWD 的 `.env`：run 目录在 `runs/` 下、可能被提交，CWD 不可控。
pub(crate) fn load_env_table(mod_dir: Option<&Path>) -> EnvTable {
    EnvTable {
        process: std::env::vars().collect(),
        env_local: mod_dir.map_or_else(Vec::new, |dir| read_env_file(&dir.join(".env.local"))),
        env: mod_dir.map_or_else(Vec::new, |dir| read_env_file(&dir.join(".env"))),
    }
}

/// 读一个 `.env` 文件（不存在 = 空表）。
pub(crate) fn read_env_file(path: &Path) -> Vec<(String, String)> {
    fs::read_to_string(path)
        .map(|text| parse_env_file(&text))
        .unwrap_or_default()
}

/// `.env` 词法：`KEY=VALUE`，跳过 `#` 与空行，容忍 `export ` 前缀，剥一层引号。
pub(crate) fn parse_env_file(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let legal = !key.is_empty()
            && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !key.starts_with(|c: char| c.is_ascii_digit());
        if !legal {
            continue;
        }
        out.push((key.to_owned(), strip_quotes(value.trim()).to_owned()));
    }
    out
}

/// 剥一层引号（`"x"` / `'x'` → `x`）。
pub(crate) fn strip_quotes(value: &str) -> &str {
    for quote in ['"', '\''] {
        if value.len() >= 2 && value.starts_with(quote) && value.ends_with(quote) {
            return &value[1..value.len() - 1];
        }
    }
    value
}

/// `$ENV{VAR}` 解析（错误文案**只出现变量名**，绝不出现值）。
pub(crate) fn resolve_env(name: &str, env: &EnvTable) -> Result<String, String> {
    env.lookup(name)
        .map(str::to_owned)
        .ok_or_else(|| format!("$ENV{{{name}}} 未设置（已查 进程环境 / .env.local / .env）"))
}

/// 模板替换：`{{key}}` 取请求参数，`$ENV{VAR}` 取环境变量。
pub(crate) fn substitute(
    template: &str,
    args: &[(String, String)],
    env: &EnvTable,
) -> Result<String, String> {
    let bytes = template.as_bytes();
    let mut out = String::with_capacity(template.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"{{") {
            let rest = &template[index + 2..];
            let Some(end) = rest.find("}}") else {
                return Err("模板里的 `{{` 未闭合".to_owned());
            };
            let key = rest[..end].trim();
            let Some((_, value)) = args.iter().find(|(name, _)| name == key) else {
                return Err(format!("占位符 {}{}{} 未提供", "{{", key, "}}"));
            };
            out.push_str(value);
            index += 2 + end + 2;
            continue;
        }
        if bytes[index..].starts_with(b"$ENV{") {
            let rest = &template[index + 5..];
            let Some(end) = rest.find('}') else {
                return Err("模板里的 `$ENV{` 未闭合".to_owned());
            };
            out.push_str(&resolve_env(&rest[..end], env)?);
            index += 5 + end + 1;
            continue;
        }
        let ch = template[index..].chars().next().unwrap_or('?');
        out.push(ch);
        index += ch.len_utf8();
    }
    Ok(out)
}

/// 父进程侧的展开预检：占位与环境变量现在就能解析、且解析后仍 wire 安全。
///
/// 只用于**早报错**（把工具从本步剔掉）；真正执行在 worker 里重新解析，
/// 解析结果不落盘、不进日志（I4）。
pub(crate) fn validate_expansion(
    spec: &ToolSpec,
    args: &[(String, String)],
    env: &EnvTable,
) -> Result<(), String> {
    let mut templates: Vec<&str> = spec.args.iter().map(String::as_str).collect();
    if let Some(url) = &spec.url {
        templates.push(url);
    }
    if let Some(body) = &spec.body_template {
        templates.push(body);
    }
    for (_, value) in &spec.headers {
        templates.push(value);
    }
    for template in templates {
        let expanded = substitute(template, args, env)
            .map_err(|message| format!("工具 `{}` 的{message}，已跳过", spec.id))?;
        if expanded.len() > MAX_TOOL_ARG_BYTES {
            return Err(format!(
                "工具 `{}` 的一个参数解析后超过 {MAX_TOOL_ARG_BYTES} 字节（防凭据走私），已跳过",
                spec.id
            ));
        }
        if expanded.contains(['\n', '\r', '\0']) {
            return Err(format!(
                "工具 `{}` 的参数解析后含换行或控制字符（会破坏 wire format），已跳过",
                spec.id
            ));
        }
        if expanded.contains('|') {
            return Err(format!(
                "工具 `{}` 的参数解析后含 `|`（会破坏 wire format），已跳过",
                spec.id
            ));
        }
    }
    Ok(())
}

/// 工具输出清洗：`|` → 全角、换行 / NUL → 空格、按 `max_bytes` 在**字符边界**截断。
pub(crate) fn sanitize_tool_output(text: &str, max_bytes: usize) -> String {
    let cleaned: String = text
        .chars()
        .filter(|ch| *ch != '\0')
        .map(|ch| match ch {
            '|' => '｜',
            '\n' | '\r' => ' ',
            other => other,
        })
        .collect();
    let (mut out, truncated) = truncate_bytes(&cleaned, max_bytes);
    if truncated {
        out.push_str("…[truncated]");
    }
    out.trim().to_owned()
}

/// 按字节上限截断（落在字符中间时回退到边界；返回是否发生截断）。
pub(crate) fn truncate_bytes(text: &str, max_bytes: usize) -> (String, bool) {
    if text.len() <= max_bytes {
        return (text.to_owned(), false);
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_owned(), true)
}
