//! 稳定错误码（`YGG-SIM-*`）与唯一错误类型。
//!
//! 形状对齐既有面：`YGG-FS-*` 一族把码放在 message 首段、不新增信封码（`code::VALIDATION`
//! 仍是信封码）——新增信封码要动映射面与下游按码分支的消费者，收益只是省一次前缀解析。
//! 本 crate 沿用同一处置：码是**稳定字符串**（外部可 grep、可断言），`Display` 的第一段
//! 逐字就是它。
//!
//! **七码的共同性质：状态未变**。所有面都是「先校验后改」——拒绍不产生半改的模拟态，
//! 所以调用方拿到错误后不需要回滚，也无需猜「改到哪一步了」。这条性质比错误分类本身
//! 更重要：它让「错误处理」退化成一句 `?`，而不必在 Lua 侧写补偿逻辑。

/// 七个稳定错误码。外部（Lua 面、门禁、报告）只认这些字符串。
pub mod codes {
    /// 宿主未注入 [`crate::Sim`] 实例（`ygg.sim.*` 全部函数）。
    pub const UNMOUNTED: &str = "YGG-SIM-UNMOUNTED";
    /// 定点面拒绝：非整数值过 Lua 面，或除零。
    pub const FIXED: &str = "YGG-SIM-FIXED";
    /// 帧格式拒绝：magic / 版本 / 长度 / 尾 digest / 列类型不符（含坏快照）。
    pub const FRAME: &str = "YGG-SIM-FRAME";
    /// 组件 schema 拒绝：未知组件名、同帧重复组件名、类型与列不符。
    pub const SCHEMA: &str = "YGG-SIM-SCHEMA";
    /// 实体面拒绝：未知实体 id、`insert` 目标不存在、`next_id` 溢出。
    pub const ENTITY: &str = "YGG-SIM-ENTITY";
    /// 内部不变量破坏（`entity_index` 与列行数不符等）——出现即缺陷，不是用户输入问题。
    pub const STATE: &str = "YGG-SIM-STATE";
    /// 调度器已 `seal`：再注册系统即拒绝（先改声明、重开一次 run，而不是偷偷追加）。
    pub const SEALED: &str = "YGG-SIM-SEALED";
}

/// 模拟内核的唯一错误类型。
///
/// `Clone + PartialEq + Eq` 是刻意的：用例要对错误做断言（`assert_eq!(err, SimError::frame(..))`），
/// 而 `#[derive(Error)]` 自动给的 `source()` 链在这里没有消费者（没有下层 crate 的错误要包）。
/// 变体载荷是 `String` 而不是 `&'static str`：错误的可读一半在**带上出错的实体 id / 组件名 /
/// 期望长度**，把这些压成静态串会让「哪个实体」永久丢失。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SimError {
    /// [`codes::UNMOUNTED`]：宿主没注入实例。
    #[error("YGG-SIM-UNMOUNTED: {0}")]
    Unmounted(String),
    /// [`codes::FIXED`]：非整数值过面 / 定点除零。
    #[error("YGG-SIM-FIXED: {0}")]
    Fixed(String),
    /// [`codes::FRAME`]：帧格式拒绝。
    #[error("YGG-SIM-FRAME: {0}")]
    Frame(String),
    /// [`codes::SCHEMA`]：组件 schema 拒绝。
    #[error("YGG-SIM-SCHEMA: {0}")]
    Schema(String),
    /// [`codes::ENTITY`]：实体面拒绝。
    #[error("YGG-SIM-ENTITY: {0}")]
    Entity(String),
    /// [`codes::STATE`]：内部不变量破坏。
    #[error("YGG-SIM-STATE: {0}")]
    State(String),
    /// [`codes::SEALED`]：`seal` 之后再注册系统。
    #[error("YGG-SIM-SEALED: {0}")]
    Sealed(String),
}

impl SimError {
    /// 稳定错误码（`Display` 的首段）。
    pub fn code(&self) -> &'static str {
        match self {
            SimError::Unmounted(_) => codes::UNMOUNTED,
            SimError::Fixed(_) => codes::FIXED,
            SimError::Frame(_) => codes::FRAME,
            SimError::Schema(_) => codes::SCHEMA,
            SimError::Entity(_) => codes::ENTITY,
            SimError::State(_) => codes::STATE,
            SimError::Sealed(_) => codes::SEALED,
        }
    }

    /// 构造 `YGG-SIM-FIXED`。
    pub fn fixed(detail: impl Into<String>) -> Self {
        SimError::Fixed(detail.into())
    }

    /// 构造 `YGG-SIM-FRAME`。
    pub fn frame(detail: impl Into<String>) -> Self {
        SimError::Frame(detail.into())
    }

    /// 构造 `YGG-SIM-SCHEMA`。
    pub fn schema(detail: impl Into<String>) -> Self {
        SimError::Schema(detail.into())
    }

    /// 构造 `YGG-SIM-ENTITY`。
    pub fn entity(detail: impl Into<String>) -> Self {
        SimError::Entity(detail.into())
    }

    /// 构造 `YGG-SIM-STATE`。
    pub fn state(detail: impl Into<String>) -> Self {
        SimError::State(detail.into())
    }

    /// 构造 `YGG-SIM-SEALED`。
    pub fn sealed(detail: impl Into<String>) -> Self {
        SimError::Sealed(detail.into())
    }
}

/// 面侧的 `Result` 别名（少写一层尖括号，读起来更像一句契约）。
pub type SimResult<T> = Result<T, SimError>;

#[cfg(test)]
mod tests {
    use super::*;

    /// 码与 `Display` 首段必须逐字一致——否则「按码 grep 日志」会静默失效。
    #[test]
    fn code_is_the_first_segment_of_display() {
        let all = [
            SimError::Unmounted("x".into()),
            SimError::fixed("x"),
            SimError::frame("x"),
            SimError::schema("x"),
            SimError::entity("x"),
            SimError::state("x"),
            SimError::sealed("x"),
        ];
        for err in all {
            let shown = err.to_string();
            let first = shown.split(':').next().unwrap_or_default();
            assert_eq!(first, err.code(), "码与 Display 首段不一致：{shown}");
            assert!(!err.code().contains(' '), "码不得含空格：{}", err.code());
        }
    }
}
