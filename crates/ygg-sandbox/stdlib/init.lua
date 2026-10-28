-- ygg stdlib 入口（S10）：声明命名空间，不携带任何行为面。
--
-- 装载序（语义性，见 src/stdlib.rs 的 STDLIB_ORDER）：本文件最先执行——
-- 在 fsm.lua / narrative.lua 之前保证 `ygg.fsm` 与 `ygg.narrative` 命名空间
-- 表存在。`or {}` 形态使命名空间声明幂等（重复 install 不清空已注册成员）。
-- 本文件与后继两文件均只在装载期执行（stdlib 嵌入先于 Mod chunk 与 seal，
-- 故其中的 ygg.on 调用合法——装载期注册禁令只约束运行期）。

ygg.fsm = ygg.fsm or {}
ygg.narrative = ygg.narrative or {}
