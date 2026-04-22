-- Return one stable payload for the standard FFI demo entry.
-- 为标准 FFI 演示入口返回一份稳定载荷。
return function(args)
  local note = ""
  if type(args) == "table" and type(args.note) == "string" then
    note = args.note
  end

  if note ~= "" then
    return "standard-ffi-demo:" .. note
  end

  return "standard-ffi-demo:ok"
end
