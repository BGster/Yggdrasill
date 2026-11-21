use super::*;

/// 提交者身份（宿主解析产物，随命令/请求一次性解析）。
///
/// `actor` 与 `exec=` / profile handle 同一命名空间（Agent 工具句柄），三态互证、
/// resume 寻址、账本 join 都靠字符串相等；`session` 是原生会话 id，只作审计
/// （事件行 → 磁盘会话文件的 grep 链），永不参与任何判定。两者独立解析、独立缺席。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ActorIdentity {
    pub(crate) actor: Option<String>,
    pub(crate) session: Option<String>,
}

impl ActorIdentity {
    /// 未携带身份（所有派发走旧路径：不注入、不认领、不盖章）。
    const NONE: ActorIdentity = ActorIdentity {
        actor: None,
        session: None,
    };

    /// 是否携带至少一段身份（opener 认领只认 `actor` 非空）。
    pub(crate) fn has_actor(&self) -> bool {
        self.actor.is_some()
    }
}

/// 身份值形状（与 executor id 同规则的轻量版）：trim 非空、≤max 字节、无空白、无 `|`、
/// 无引号/反斜杠。形状红线不是身份认证（同机进程模型下 env/flag 可伪造）；它只拦
/// 「明显不是标识符 / 会破坏 wire 与 JSON 信封」的值。
pub(crate) fn validate_identity_value(kind: &str, raw: &str, max: usize) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("{kind} 不能为空"));
    }
    if trimmed.len() > max {
        return Err(format!("{kind} 超长（> {max} 字节）"));
    }
    if trimmed.chars().any(char::is_whitespace) {
        return Err(format!("{kind} 不得含空白字符"));
    }
    if trimmed.contains('|') {
        return Err(format!("{kind} 不得含 `|`（会破坏 wire format 段序）"));
    }
    if trimmed.contains('"') || trimmed.contains('\\') {
        return Err(format!("{kind} 不得含引号或反斜杠（会破坏 JSON 标量信封）"));
    }
    Ok(trimmed.to_owned())
}

/// 提交者身份解析链：`--actor` > `YGG_ACTOR`；`--actor-session` > `YGG_ACTOR_SESSION`。
/// 旗标与环境变量都做形状校验（解析点失败 = 用法错误）；环境变量空白串视为未设置。
pub(crate) fn resolve_identity(args: &Args) -> Result<ActorIdentity, String> {
    let actor = identity_value_from(
        args.actor.as_ref(),
        std::env::var("YGG_ACTOR").ok(),
        "--actor",
        64,
    )?;
    let session = identity_value_from(
        args.actor_session.as_ref(),
        std::env::var("YGG_ACTOR_SESSION").ok(),
        "--actor-session",
        128,
    )?;
    Ok(ActorIdentity { actor, session })
}

/// 单段身份值解析（纯核心，可测）：旗标优先，环境变量兑底（空白串 = 未设置）；
/// 两处都做形状校验——非法值是错误，不是“静默无身份”。
pub(crate) fn identity_value_from(
    flag: Option<&String>,
    env_raw: Option<String>,
    kind: &str,
    max: usize,
) -> Result<Option<String>, String> {
    match flag {
        Some(raw) => Ok(Some(validate_identity_value(kind, raw, max)?)),
        None => match env_raw {
            Some(raw) if !raw.trim().is_empty() => {
                Ok(Some(validate_identity_value(kind, &raw, max)?))
            }
            _ => Ok(None),
        },
    }
}

/// 声明路由的段注入（纯函数，可测）：宿主是唯一盖章方，段序固定
/// `…|exec=…|actor=<id>|actor_session=<id>`（actor 在前，两者都可独立缺席）。
/// 未声明 / 未携带 → 原样返回。
///
/// `payload` 是 **JSON 标量文本**（字符串形态以 `"` 收尾）：身份段插进字符串
/// 内部（闭引号之前），保证内核 JSON 解析仍是合法标量；非字符串标量（数字 /
/// 布尔）没有 wire 段可言，原样返回（声明路由上调用方用非字符串载荷本来就是
/// 契约错误，由 mod 侧拒绝）。
pub(crate) fn inject_identity_segments(
    payload: &str,
    actor: Option<&str>,
    actor_session: Option<&str>,
    declares_actor: bool,
    declares_session: bool,
) -> String {
    let mut suffix = String::new();
    if declares_actor {
        if let Some(id) = actor {
            suffix.push_str(&format!("|actor={id}"));
        }
    }
    if declares_session {
        if let Some(session_id) = actor_session {
            suffix.push_str(&format!("|actor_session={session_id}"));
        }
    }
    if suffix.is_empty() {
        return payload.to_owned();
    }
    let trimmed = payload.trim_end();
    if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
        format!("{}{}\"", &trimmed[..trimmed.len() - 1], suffix)
    } else {
        payload.to_owned()
    }
}

/// 身份解析失败 → 用法错误（退出码 2，与其它旗标错误同一口径）。
pub(crate) fn resolve_identity_or_usage(args: &Args) -> ActorIdentity {
    match resolve_identity(args) {
        Ok(identity) => identity,
        Err(message) => usage_error(&message),
    }
}

// §提交者身份通道（r3）：解析链 / 段注入 / 伪造拦截 / 回显 / opener
//
// 合成 Mod 自带一条声明了 actor+actor_session 的 proposal 路由与一条未声明的
// 事件，全部断言走宿主回执与 run.json 事实面（不读真 Mod、不跑 playtest）。
// ============================================================================
#[cfg(test)]
mod tests;
