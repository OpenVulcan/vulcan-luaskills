-- Call the host-provided skill source capability bridge.
-- 调用宿主提供的技能来源能力桥接。
return function(args)
  -- Source discovery lets SDK demos hide URL install while showing GitHub and official Hub.
  -- 来源发现允许 SDK demo 隐藏 URL 安装，同时展示 GitHub 与官方 Hub。
  local tool_name = "luaskills.skill_hub.sources"
  if type(vulcan) ~= "table" or type(vulcan.host) ~= "table" or not vulcan.host.has(tool_name) then
    return {
      ok = false,
      code = "skill_source_capabilities_unavailable",
      message = "host skill source capability bridge is not available",
    }
  end

  return vulcan.host.call(tool_name, args or {})
end
