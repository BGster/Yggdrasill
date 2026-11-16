-- misc：与具体流程无关的退役/边界断言（迁移自 Rust 对照套件 80、82 例）。

test("archive_sessions 已从工具桥退役：pack 与 allowedTools 都不再声明", function()
  assert_nil(
    __PACK.tools["harness.archive_sessions"],
    "pack.json must no longer declare harness.archive_sessions"
  )
  local config = __ASSETS["ygg:agent/config"]
  assert_true(type(config.allowedTools) == "table", "config.allowedTools must be an array")
  for _, name in ipairs(config.allowedTools) do
    assert_true(
      name ~= "harness.archive_sessions",
      "allowedTools must not list harness.archive_sessions"
    )
  end
end)
