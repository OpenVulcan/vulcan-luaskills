-- Call the host-provided official Skills Hub resolve bridge.
-- 调用宿主提供的官方 Skills Hub 解析桥接。
return function(args)
  -- Resolve returns install metadata for host confirmation instead of installing by itself.
  -- resolve 只返回供宿主确认的安装元数据，而不自行安装。
  local tool_name = "luaskills.skill_hub.resolve"
  if type(vulcan) ~= "table" or type(vulcan.host) ~= "table" or not vulcan.host.has(tool_name) then
    return {
      ok = false,
      code = "skill_hub_resolve_unavailable",
      message = "host skill Hub resolve bridge is not available",
    }
  end

  return vulcan.host.call(tool_name, args or {})
end
