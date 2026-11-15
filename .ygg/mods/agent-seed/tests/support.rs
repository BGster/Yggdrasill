// 迁移期桥接：共享夹具工具仍在内核测试目录（modules_e2e / lifecycle / negative /
// pure_export 四个通用测试共用同一份，复制会违反单副本纪律；提升为 crate 会成环）。
// 套件迁往 Lua 后本文件随之删除。
#[path = "../../../../crates/ygg-sandbox/tests/support/mod.rs"]
mod inner;
pub use inner::*;
