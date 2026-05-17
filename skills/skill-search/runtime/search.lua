-- Call the host-provided official Skills Hub search bridge.
-- 调用宿主提供的官方 Skills Hub 搜索桥接。
return function(args)
  -- Keep the Hub access in the host so UI policy and credentials stay centralized.
  -- 将 Hub 访问保留在宿主侧，使界面策略与凭证保持集中管控。
  local tool_name = "luaskills.skill_hub.search"
  if type(vulcan) ~= "table" or type(vulcan.host) ~= "table" or not vulcan.host.has(tool_name) then
    return {
      ok = false,
      code = "skill_hub_search_unavailable",
      message = "host skill Hub search bridge is not available",
    }
  end

  return vulcan.host.call(tool_name, args or {})
end
