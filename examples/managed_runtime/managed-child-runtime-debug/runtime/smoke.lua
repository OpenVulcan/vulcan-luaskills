-- Run one end-to-end managed Python and Node smoke test.
-- 运行一次端到端受管 Python 与 Node 冒烟测试。
return function(args)
  local python_status_before = vulcan.runtime.python.status()
  local node_status_before = vulcan.runtime.node.status()

  local python_first = vulcan.runtime.python.invoke({
    file = "python/echo.py",
    handler = "main",
    timeout_ms = 30000,
    args = {
      text = args and args.text or "lua",
      number = 40,
    },
  })
  local python_second = vulcan.runtime.python.invoke({
    file = "python/echo.py",
    handler = "main",
    timeout_ms = 30000,
    args = {
      text = "warm-python",
      number = 41,
    },
  })

  local node_first = vulcan.runtime.node.invoke({
    file = "node/echo.mjs",
    handler = "main",
    timeout_ms = 30000,
    args = {
      text = args and args.text or "lua",
      number = 40,
    },
  })
  local node_second = vulcan.runtime.node.invoke({
    file = "node/echo.mjs",
    handler = "main",
    timeout_ms = 30000,
    args = {
      text = "warm-node",
      number = 41,
    },
  })

  local python_status_after = vulcan.runtime.python.status()
  local node_status_after = vulcan.runtime.node.status()

  return vulcan.json.encode({
    python_status_before = python_status_before,
    node_status_before = node_status_before,
    python_first = python_first,
    python_second = python_second,
    node_first = node_first,
    node_second = node_second,
    python_status_after = python_status_after,
    node_status_after = node_status_after,
  })
end
