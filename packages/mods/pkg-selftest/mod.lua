-- `packages/mods/pkg-selftest/mod.lua` —— **包自测的消费夹具**，不是产品 Mod。
--
-- 作用只有一个：让 `examples/selftest` 有一个**声明了 `packages` 段**的 Mod 可跑，
-- 于是 `<包根>/<id>/test/*.lua` 会被收集执行（`packages::resolve` 的闭包决定收集哪
-- 些包，见 `crates/ygg-sandbox/examples/selftest.rs` 的 `package_plan`）。
--
-- 它不注册 handler、不 emit、不写 KV：包自测自带 mock `ygg`（`packages/README.md`
-- §5「包测试不依赖消费 mod 的 harness」），入口只做装配。
--
-- 为什么必须存在这一层：生产 Mod（`agent-seed`）**不声明包**，所以它的
-- `cases passed=` 永远看不到 `pkg.*`。夹具把「包装载面」变成一条可复算的命令：
--   cargo run -q --example selftest -p ygg-sandbox -- \
--     --mod packages/mods/pkg-selftest --packages packages/libs --filter pkg-

local entity = require("pkg.entity")
local codec = require("pkg.codec")
local quest = require("pkg.quest")

-- 装配：三个包的 `install` 都只记 deps（不预读、不预写，契约 I7 / I8）。
entity.install()
codec.install()
quest.install()
