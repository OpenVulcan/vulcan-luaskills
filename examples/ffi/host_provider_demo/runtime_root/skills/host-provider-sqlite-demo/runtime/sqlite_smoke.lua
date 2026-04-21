-- Build one stable default note for the host-provider SQLite smoke test.
-- 为宿主 provider SQLite 烟测构造一个稳定的默认 note。
local function build_default_note()
    return "host provider sqlite smoke demo"
end

-- Safely inspect the current vulcan.sqlite status object.
-- 安全检查当前的 vulcan.sqlite 状态对象。
local function get_sqlite_status()
    if type(vulcan.sqlite) ~= "table" or type(vulcan.sqlite.status) ~= "function" then
        return {
            enabled = false,
            initialized = false,
            reason = "vulcan.sqlite is unavailable",
        }
    end

    local ok, status = pcall(vulcan.sqlite.status)
    if not ok or type(status) ~= "table" then
        return {
            enabled = false,
            initialized = false,
            reason = tostring(status),
        }
    end
    return status
end

-- Execute one minimal host-managed SQLite smoke test and return a structured JSON string.
-- 执行一个最小宿主管理 SQLite 烟测并返回结构化 JSON 字符串。
return function(args)
    local status = get_sqlite_status()
    if not status.enabled then
        return vulcan.json.encode({
            ok = false,
            error = "sqlite_not_enabled",
            status = status,
        })
    end

    local note = build_default_note()
    if type(args) == "table" and args.note ~= nil and tostring(args.note) ~= "" then
        note = tostring(args.note)
    end
    local sql_note = note:gsub("'", "''")

    local execute_result = vulcan.sqlite.execute_script({
        sql = [[
            CREATE TABLE IF NOT EXISTS host_provider_demo_notes(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                note TEXT NOT NULL
            );
            DELETE FROM host_provider_demo_notes;
            INSERT INTO host_provider_demo_notes(note) VALUES (']] .. sql_note .. [[');
        ]],
    })

    local query_result = vulcan.sqlite.query_json({
        sql = "SELECT note FROM host_provider_demo_notes ORDER BY id DESC LIMIT 1",
    })

    return vulcan.json.encode({
        ok = true,
        success = true,
        message = "Host-managed SQLite smoke test completed.",
        status = status,
        execute_result = execute_result,
        query_result = query_result,
        note = note,
    })
end
