-- Call the host-provided official Skills Hub detail bridge.
-- 调用宿主提供的官方 Skills Hub 详情桥接。
return function(args)
  -- Detail lookup stays read-only and never performs installation directly.
  -- 详情查询保持只读，绝不直接执行安装。
  local tool_name = "luaskills.skill_hub.detail"
  if type(vulcan) ~= "table" or type(vulcan.host) ~= "table" or not vulcan.host.has(tool_name) then
    return {
      ok = false,
      code = "skill_hub_detail_unavailable",
      message = "host skill Hub detail bridge is not available",
    }
  end

  return vulcan.host.call(tool_name, args or {})
end
