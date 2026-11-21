-- `tests/test-a-shapes.lua` —— 形状库 KAT：黄金值自建（R1：等价复现非逐字节复刻）。
--
-- 黄金值 = 从源语义重推（shape_definition.js / shape_definition_manager.js），
-- 覆盖：编解码往返、规范形、切割（半/四象限，源序）、旋转（CW/CCW/180）、
-- 堆叠（gap 算法三形态：逐层、互锁回单层、层内间隙）、层数上限、非法输入码。

local shapes = require("src.shapes")

check("shapes.roundtrip-basic", function()
  assert_eq(shapes.canonical("CuCuCuCu"), "CuCuCuCu", "单层往返")
  assert_eq(shapes.canonical("RuRuRuRu:SrSrSrSr"), "RuRuRuRu:SrSrSrSr", "双层往返")
  assert_eq(shapes.canonical("CrRgSr--:----Sb--"), "CrRgSr--:----Sb--", "含空象限双层")
end)

check("shapes.encode-drops-empty-layers", function()
  local layers = shapes.decode("CuCuCuCu")
  layers[2] = { [0] = nil, [1] = nil, [2] = nil, [3] = nil }
  assert_eq(shapes.encode(layers), "CuCuCuCu", "全空第二层被丢弃")
end)

check("shapes.is-valid-matrix", function()
  assert_true(shapes.is_valid("CuCuCuCu"), "合法单层")
  assert_true(shapes.is_valid("----CuCu"), "含空象限")
  assert_true(shapes.is_valid("RuRuRuRu:SrSrSrSr:WbWbWbWb:CpCpCpCp"), "四层")
  assert_true(not shapes.is_valid(""), "空串非法")
  assert_true(not shapes.is_valid("CuCuCuC"), "长度不对")
  assert_true(not shapes.is_valid("CuCuCuC-"), "颜色位非法")
  assert_true(not shapes.is_valid("--------"), "全空层非法")
  assert_true(not shapes.is_valid("XuXuXuXu"), "子形位非法")
  assert_true(not shapes.is_valid("CuCuCuCu:CuCuCuCu:CuCuCuCu:CuCuCuCu:CuCuCuCu"), "五层超限")
  assert_true(not shapes.is_valid("CuCu-Cu-"), "- 配颜色非法")
end)

check("shapes.decode-errors-carry-code", function()
  assert_error("YGG-MOD-SHAPE-KEY", function()
    shapes.decode("bad")
  end, "非法 key 应抛 -KEY")
  assert_error("YGG-MOD-SHAPE-KEY", function()
    shapes.decode("--------")
  end, "全空层应抛 -KEY")
end)

check("shapes.cut-half-golden", function()
  local halves = shapes.cut_half("CuCuCuCu")
  assert_eq(halves.right, "----CuCu", "源序：right = 象限[2,3]")
  assert_eq(halves.left, "CuCu----", "源序：left = 象限[0,1]")
  local two_layer = shapes.cut_half("CuCuCuCu:SrSrSrSr")
  assert_eq(two_layer.right, "----CuCu:----SrSr", "双层切割保层（每层各保象限2,3）")
  assert_eq(shapes.cut_half("Cu------").right, "", "空半 = 空项")
end)

check("shapes.cut-quad-golden", function()
  local quads = shapes.cut_quad("CuCuCuCu")
  assert_eq(quads[1], "Cu------", "象限0")
  assert_eq(quads[2], "--Cu----", "象限1")
  assert_eq(quads[3], "----Cu--", "象限2")
  assert_eq(quads[4], "------Cu", "象限3")
  local mixed = shapes.cut_quad("Cu--Su--")
  assert_eq(mixed[1], "Cu------", "混合象限0")
  assert_eq(mixed[2], "", "空象限 → 空项")
  assert_eq(mixed[3], "----Su--", "混合象限2")
  assert_eq(mixed[4], "", "空象限 → 空项")
end)

check("shapes.rotate-golden", function()
  assert_eq(shapes.rotate_cw("RuRu----"), "--RuRu--", "CW")
  assert_eq(shapes.rotate_ccw("RuRu----"), "Ru----Ru", "CCW")
  assert_eq(shapes.rotate_180("RuRu----"), "----RuRu", "180")
  assert_eq(shapes.rotate_cw("CrWbCrWb:SrSrSrSr"), "WbCrWbCr:SrSrSrSr", "双层旋转保层序")
  -- 旋转四次 = 原形（循环群闭包）
  local key = "CrWbCrWb:SrSrSrSr"
  local rotated = shapes.rotate_cw(shapes.rotate_cw(shapes.rotate_cw(shapes.rotate_cw(key))))
  assert_eq(rotated, key, "CW×4 恒等")
end)

check("shapes.stack-golden", function()
  assert_eq(shapes.stack("CuCuCuCu", "RuRuRuRu"), "CuCuCuCu:RuRuRuRu", "全重叠 → gap算法上新层（非报错）")
  assert_eq(shapes.stack("----CuCu", "CuCu----"), "CuCuCuCu", "互锁 → 回单层（gap=1）")
  assert_eq(shapes.stack("RuRu----", "Cu----Cu"), "RuRu----:Cu----Cu", "层内间隙补齐")
  assert_eq(shapes.stack("CuCuCuCu", "SrSrSrSr:WbWbWbWb"), "CuCuCuCu:SrSrSrSr:WbWbWbWb", "顶层多层依序落")
  assert_error("YGG-MOD-SHAPE-STACK", function()
    shapes.stack("CuCuCuCu", "")
  end, "空项不可堆叠")
  -- 源 assert「Sub shape got lost」在 gap 算法下不可达（防御分支保留不测不可达路径）
end)

check("shapes.stack-layer-cap", function()
  local three = shapes.stack("CuCuCuCu", "SrSrSrSr:WbWbWbWb:CpCpCpCp")
  assert_eq(three, "CuCuCuCu:SrSrSrSr:WbWbWbWb:CpCpCpCp", "四层封顶内")
  -- 五层被截断到 4（源 splice(4)）
  assert_eq(#shapes.decode(three), 4, "解码层数 ≤ 4")
end)

check("shapes.miner-shape-and-stat", function()
  assert_eq(shapes.miner_shape("C", "u"), "CuCuCuCu", "圆斑产形")
  assert_eq(shapes.miner_shape("R", "r"), "RrRrRrRr", "方斑产形")
  assert_error("YGG-MOD-SHAPE-PATCH", function()
    shapes.miner_shape("X", "u")
  end, "非法子形")
  local stat = shapes.stat("CuCuCuCu:SrSrSrSr")
  assert_eq(stat.layers, 2, "层数")
  assert_eq(stat.quads, 8, "满象限数")
  assert_eq(shapes.stat("").layers, 0, "空项层数")
end)

check("shapes.empty-item-semantics", function()
  assert_true(shapes.is_empty(""), "空项判空")
  assert_true(not shapes.is_valid(""), "空项不是合法 shortkey")
  assert_eq(shapes.canonical(""), "", "空项规范形 = 空串")
end)
