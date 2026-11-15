-- src/domain/readsig.lua —— 「幕前必读」签名账本（r3 销票制）：`read.*` 写面单源。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）。
-- 本模块写的 KV 前缀：`settings.read.*`（逐件值）+ `settings.read_index`（平铺索引）。
--
-- **销票制**（r3 定稿，用户裁决）：作用域边界由本模块的 clear() 主动清除全部签名，
-- 于是「键非空」本身就等于「本作用域内已读并签名」——消费者（r4）只需 signed(doc)
-- 判空，不需要时钟、不需要比较序号。
--
-- 为什么不是「验票」（记下序号再比较）：沙箱没有可用的单调时钟——
--   `kvRevision` 与事件 seq 都没有 Lua 读取面（ygg 只暴露 kv.get/set 与 fs/tool/entity），
--   `ygg.ticks()` 的唯一推进路径在宿主句柄、生产路径永不推进。任何时钟比较在重放 /
--   重启 / 换代后都会给出错答案；「边界主动销票」只依赖同一个 clear() 调用，可复算。
--
-- 值形状 `ver|scene|seq`（三段固定分隔符，不拆成多键）：
--   ver   —— 被读文档文件头的版本（r1 的 `> **ver 9f2c1a**`），签名者抄写；
--   scene —— **签名时所在的幕**（handler 盖，签名者无法伪造），值自描述、可审计；
--   seq   —— 值形状占位，**恒为 0**（r3 定稿：本模块只读 `settings.event_seq` 且不改它）。
--            r4 的权威签名序号存在 `settings.sign_seq.<doc>`（单写者 = clock.mark_sign），
--            消费侧先读值段、为 0 时回落读快照。把序号写进值段会当场打红 4 条既有用例
--            的 `ver|scene|0` 断言，也会让两环抢同一个键——所以值形状冻结。
-- 选单键而不是 `read.<doc>.ver` / `read.<doc>.seq` 两键：销票要逐件清除，两键就是
-- 两条要清、两处可能漏；单键一次写一次清，索引只需记 doc 名。

local M = {}

local clock = require("src.core.clock")
local util = require("src.core.util")

local split, trim = util.split, util.trim

-- 索引键与序号键（都不在 config.set 用户面：无分支 → 直写被 bad_config_key 拒，
-- 只能由本模块维护。与 settings.profile_index 同一范式）。
local INDEX = "settings.read_index"
local SEQ = "settings.event_seq"
local DOC_PREFIX = "settings.read."
-- r9 签署人侧键（**不翻倍值形状、不翻倍索引**）：值形状 `ver|scene|seq` 已被逐字钉死
-- （`…|0` 断言），把签署人塞进值段会当场打红，并让「销票」多一个要清的字段。
-- 侧键与值同生共死：sign 同事务写，forget / clear 同事务清（半状态会让判定读到
-- 「值空了、签署人还在」或反之）。键名以 `read_actor.` 开头而**不是** `read.actor.`：
-- 后者会被 config.set 的 `read.` 分支认成签名键，把不可写面暴露给用户面。
local ACTOR_PREFIX = "settings.read_actor."

-- 值组装（单一事实源；handler 与测试共用口径）。
local function value_of(ver, scene, seq)
  return ver .. "|" .. scene .. "|" .. tostring(seq)
end

-- 值解析 → ver, scene, seq；空值 = 未签名 → nil（这是「销票」的判定面）。
local function parse_value(raw)
  if type(raw) ~= "string" or raw == "" then
    return nil
  end
  local parts = split(raw, "|")
  local ver = trim(parts[1] or "")
  if ver == "" then
    return nil
  end
  return ver, trim(parts[2] or ""), tonumber(parts[3] or "") or 0
end

-- ============================================================================
-- 索引维护（沙箱无键枚举，用平铺逗号串记已签 doc；只增不删，收口随 run 终结）
-- ============================================================================

local function index_docs()
  local raw = ygg.kv.get(INDEX)
  local out = {}
  for _, part in ipairs(split(type(raw) == "string" and raw or "", ",")) do
    local doc = trim(part)
    if doc ~= "" then
      out[#out + 1] = doc
    end
  end
  return out
end

local function index_has(docs, doc)
  for i = 1, #docs do
    if docs[i] == doc then
      return true
    end
  end
  return false
end

local function index_write(docs)
  local out = ""
  for i = 1, #docs do
    out = (i == 1) and docs[i] or (out .. "," .. docs[i])
  end
  ygg.kv.set(INDEX, out)
end

local function current_seq()
  local n = tonumber(ygg.kv.get(SEQ))
  if n == nil then
    return 0
  end
  return math.floor(n)
end

-- ============================================================================
-- 写面：签名 / 单件清除 / 边界销票
-- ============================================================================

-- 签一件：值三段 + 索引补 doc + 签署人侧键（同键重写幂等 upsert）。
-- `actor` 必需：handler 已在调用前拒签无身份签名。空侧键 **不写**——「不知道谁读的」
-- 签名在按人判定里等于不存在，却会占着值让人误以为有效（宁可不写，不写空的）。
function M.sign(doc, ver, scene, actor)
  ygg.kv.set(DOC_PREFIX .. doc, value_of(ver, scene, current_seq()))
  ygg.kv.set(ACTOR_PREFIX .. doc, trim(actor or ""))
  local docs = index_docs()
  if not index_has(docs, doc) then
    docs[#docs + 1] = doc
    index_write(docs)
  end
end

-- 读一件：返回 ver, scene, seq；未签 / 已清除 → nil。
function M.signed(doc)
  return parse_value(ygg.kv.get(DOC_PREFIX .. doc))
end

-- 签署人（r9）：未签 / r3 旧存档（无侧键）→ ""。调用方必须把 "" 当「未知」而不当
-- 「不是我」：旧存档不该因为机制升级被一夜判死（降级面由 gates 叙事告警）。
function M.actor_of(doc)
  local value = ygg.kv.get(ACTOR_PREFIX .. doc)
  if type(value) ~= "string" then
    return ""
  end
  return trim(value)
end

-- 单件清除（`config.set read.<doc>=`）：值置空 + 索引摘除 + 侧键同清。空值不写新键（幂等）。
function M.forget(doc)
  if ygg.kv.get(DOC_PREFIX .. doc) ~= nil then
    ygg.kv.set(DOC_PREFIX .. doc, "")
  end
  if ygg.kv.get(ACTOR_PREFIX .. doc) ~= nil then
    ygg.kv.set(ACTOR_PREFIX .. doc, "")
  end
  local docs = index_docs()
  local kept = {}
  local dropped = false
  for i = 1, #docs do
    if docs[i] == doc then
      dropped = true
    else
      kept[#kept + 1] = docs[i]
    end
  end
  if dropped then
    index_write(kept)
  end
  -- r4：签名时刻快照与值同生共死（否则会出现「值空了、序号还在」的半状态）。
  clock.clear_sign(doc)
end

-- 边界销票：清本作用域全部签名。无索引 = 无签名 = 零写入（新局不留痕）。
-- 只写 KV、不叙事、不渲染——可以在 effect 体内安全调用（§9.1 四不变量）。
function M.clear()
  local docs = index_docs()
  if #docs == 0 then
    return 0
  end
  for i = 1, #docs do
    ygg.kv.set(DOC_PREFIX .. docs[i], "")
    ygg.kv.set(ACTOR_PREFIX .. docs[i], "")
    clock.clear_sign(docs[i])
  end
  ygg.kv.set(INDEX, "")
  return #docs
end

function M.count()
  return #index_docs()
end

-- 覆盖面：本幕已签件数 / 本幕清单件数（r9 追加 `mine` = 其中**本人**签的件数）。
--   清单在场（r2 的 readPlan 已烘进 data/config.json）→ k = 清单内已签件数、n = 清单件数；
--   清单缺席 → k = 已签件数、n = nil（**降级**：签名照旧成立，只是不算比例）。
-- `k` 口径**不变**（本幕覆盖了几个文件）——`mine` 是第二个口径（其中几个是我签的），
-- 两个必须在回执里并列出现：单报 `k/n` 就是「像我已签全」的歧义本身（r9 要修的东西）。
-- 边界：清单里重复的 doc 只算一次；清单长度封顶 32（防手抖写成无限长）。
function M.coverage(scene, plan, actor)
  local who = trim(actor or "")
  local docs = index_docs()
  local function mine_of(doc)
    if who == "" or not index_has(docs, doc) then
      return 0
    end
    if M.actor_of(doc) ~= who then
      return 0
    end
    return 1
  end
  if type(plan) == "table" and type(plan[scene]) == "table" then
    local seen = {}
    local n, k, mine = 0, 0, 0
    for _, item in ipairs(plan[scene]) do
      local doc = trim(type(item) == "string" and item or "")
      if doc ~= "" and seen[doc] == nil and n < 32 then
        seen[doc] = true
        n = n + 1
        if index_has(docs, doc) then
          k = k + 1
        end
        mine = mine + mine_of(doc)
      end
    end
    return k, n, mine
  end
  local mine = 0
  for i = 1, #docs do
    mine = mine + mine_of(docs[i])
  end
  return #docs, nil, mine
end

M.value_of = value_of
M.parse_value = parse_value

return M
