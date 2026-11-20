-- `pkg.dialogue.memory` —— 线程记忆：轮次追加、窗口截断、线程键名表（I4 / D2）。
--
-- # 只搬键，不造文案
--
-- `turns[i].text` 存的是**文本键**（如 `beats.dlg.greet`），包不读 `ygg.data`、
-- 不渲染、不拼句子——文案的唯一副本在 mod 的 `data/texts/*.json`（d16 用静态
-- 命中数钉「包内零 `ygg.data(`」）。
--
-- # 线程键
--
-- `memory = "pair"`（缺省）→ `"<source>:<target>"`；`memory = "entity"` → `"<source>"`。
-- `source` / `target` 用 `0` 表示「系统」，不用 `nil`——表里 `nil` 值等于缺键，
-- 且 `codec` 的 I7 拒绝数组洞。
--
-- # 窗口（I4）
--
-- `turns` 超出 `opts.window`（缺省 `S.CONFIG.window` = 12）时**丢最旧**，
-- 不写摘要（摘要是 AI 侧策略，属 mod，见契约 §1）。
--
-- # `dlg.keys` 就是 D2 的索引
--
-- `ygg.kv` 没有遍历（`crates/ygg-core/src/kv.rs`），没有这份名表就枚举不出线程键
-- ——导出与「泄漏检测」都不可达。名表升序，`keys()` 的枚举面从这里读。

local S = require("pkg.dialogue.store")

local M = {}

--- 线程键（见文件头注）。
local function thread_of(spec, frame)
  local mode = spec.opts.memory or "pair"
  if mode == "entity" then
    return tostring(frame.source)
  end
  return tostring(frame.source) .. ":" .. tostring(frame.target)
end

local function window_of(spec)
  local window = spec.opts.window
  if window == nil then
    return S.CONFIG.window
  end
  return window
end

--- 读一条线程 → `{ {who, text, seq}, … }`（不存在 = 空数组）。
local function read_turns(h, thread)
  local count = S.get(h, S.thread_key(thread, "turns"))
  if count == nil then
    return {}
  end
  local out = {}
  for i = 1, count do
    out[i] = {
      who = S.get(h, S.thread_key(thread, "t." .. i .. ".who")),
      text = S.get(h, S.thread_key(thread, "t." .. i .. ".text")),
      seq = S.get(h, S.thread_key(thread, "t." .. i .. ".seq")),
    }
  end
  return out
end

--- 写回一条线程（先写轮数再写各轮；轮数收窄即截断）。
local function write_turns(h, thread, turns)
  for i = 1, #turns do
    local prefix = "t." .. i .. "."
    S.set(h, S.thread_key(thread, prefix .. "who"), turns[i].who)
    S.set(h, S.thread_key(thread, prefix .. "text"), turns[i].text)
    S.set(h, S.thread_key(thread, prefix .. "seq"), turns[i].seq)
  end
  S.set(h, S.thread_key(thread, "turns"), #turns)
end

--- 把线程登记进名表（已登记则不写：幂等零写）。
local function register_thread(h, thread)
  local keys = S.split(S.get(h, S.K.threads))
  for i = 1, #keys do
    if keys[i] == thread then
      return false
    end
  end
  keys[#keys + 1] = thread
  S.sort(keys)
  S.set(h, S.K.threads, S.join(keys))
  return true
end

--- 追加一轮（`seq` 恰好 +1，窗口外丢最旧）。返回新轮次号。
function M.append(h, thread, who, text, window)
  if not S.key_name_ok(thread) then
    S.fail(S.E.TYPE, "线程键不在合法字符集内：" .. tostring(thread))
  end
  if type(text) ~= "string" or #text == 0 then
    S.fail(S.E.TYPE, "text 必须是文本键（非空字符串），得到 " .. tostring(text))
  end
  if type(window) ~= "number" or window % 1 ~= 0 or window < 1 then
    S.fail(S.E.WINDOW, "window 必须是正整数，得到 " .. tostring(window))
  end
  local turns = read_turns(h, thread)
  local seq = S.seq(h) + 1
  S.set(h, S.K.seq, seq)
  turns[#turns + 1] = { who = who, text = text, seq = seq }

  local drop = #turns - window
  if drop > 0 then
    local kept = {}
    for i = drop + 1, #turns do
      kept[#kept + 1] = turns[i]
    end
    turns = kept
  end

  write_turns(h, thread, turns)
  register_thread(h, thread)
  return seq
end

--- `remember(spec, frame)`：handler 认领后按它的 memory/window 记一轮。
function M.remember(h, spec, frame)
  local mode = spec.opts.memory or "pair"
  if mode == "none" then
    return false
  end
  local text = frame.payload.text_key
  if type(text) ~= "string" or #text == 0 then
    -- 没给文本键就用事件名当键：仍然是「键」，不造文案。
    text = frame.type
  end
  M.append(h, thread_of(spec, frame), frame.source, text, window_of(spec))
  return true
end

--- `history(thread)` → 轮次副本（不存在 = 空数组）。
function M.history(h, thread)
  local turns = read_turns(h, thread)
  local out = {}
  for i = 1, #turns do
    out[i] = { who = turns[i].who, text = turns[i].text, seq = turns[i].seq }
  end
  return out
end

--- 线程键名表（升序）。
function M.threads(h)
  return S.split(S.get(h, S.K.threads))
end

return M
