# Lua Skill Developer Manual

[Documentation hub](index.md) | [Chinese manual](zh-CN/skill-development.md)

## 1. Purpose

This manual is for **Lua Skill authors**. It documents the `vulcan.*` capability surface that the current runtime actually injects into Lua, including:

- Which `vulcan.*` APIs are available.
- How each API is called.
- Which APIs are always available and which ones depend on host or current-skill binding state.
- Which fields are implementation details and should not be treated as long-term public contracts by skills.

This document follows the current implementation, especially the Lua injection logic in `src/runtime/engine.rs`.

## 2. Quick Summary

The current skill runtime exposes these top-level capabilities:

- `vulcan.call`
- `vulcan.runtime.*`
- `vulcan.fs.*`
- `vulcan.path.*`
- `vulcan.process.*`
- `vulcan.os.*`
- `vulcan.json.*`
- `vulcan.cache.*`
- `vulcan.host.*`
- `vulcan.context.*`
- `vulcan.deps.*`
- `vulcan.sqlite.*`
- `vulcan.lancedb.*`

Three important facts:

1. The runtime currently treats skills as **trusted code** by default and does not provide a sandbox security promise.
2. Some values under `vulcan.context.*`, `vulcan.deps.*`, `vulcan.sqlite.*`, and `vulcan.lancedb.*` depend on host injection or current-skill binding state. Do not assume they are always valid.
3. Hosts can force-skip skills through `LuaRuntimeHostOptions.ignored_skill_ids`. Ignored skills do not prepare dependencies, bind databases, or register entries.

## 2.1 Skill Naming And Release Package Rules

`skill_id` is the stable primary key shared by the LuaSkills runtime, lifecycle management, config namespaces, dependency folders, database bindings, and canonical entry names. The current rule is "the directory name is the `skill_id`"; `skill_id` is not declared by a field in `skill.yaml`.

`skill_id` and every `entry.name` must use the same identifier format:

```text
^[a-z]([a-z0-9-]*[a-z0-9])?$
```

Meaning:

- Start with a lowercase ASCII letter.
- Continue only with lowercase ASCII letters, digits, and hyphens `-`.
- Do not start with a digit, use uppercase letters or underscores, or end with a hyphen.
- Valid examples: `vulcan-codekit`, `codekit2`, `vulcan-runtime-tools`.
- Invalid examples: `2codekit`, `Vulcan-codekit`, `vulcan_codekit`, `vulcan-codekit-`.

Skill package layout rules:

- The physical directory name is the final `skill_id`. For example, `skills/vulcan-codekit/` has `skill_id = vulcan-codekit`.
- `skill.yaml` must not declare a `skill_id` field. The runtime rejects a skill that declares it.
- `skill.yaml` `name` is human-readable metadata and does not participate in `skill_id` matching.
- Each `entries[].name` is a local entry name inside the current skill and must also satisfy the identifier rule above.
- Runtime canonical entry names are exposed as `{skill_id}-{entry_name}`. If a name conflicts with a host-reserved name or another entry, the runtime appends a stable numeric suffix, producing `{skill_id}-{entry_name}-{N}`.

GitHub-managed skill installs and release assets must use the same `skill_id`:

- If an install request does not pass an explicit `skill_id`, the runtime derives it from the GitHub repository name. It does not strip a `luaskills-` prefix automatically.
- The release zip file name must be `{skill_id}-v{version}-skill.zip`.
- The checksum file name must be `{skill_id}-v{version}-checksums.txt`.
- The zip must contain only one top-level directory named exactly `{skill_id}`, and it must contain `{skill_id}/skill.yaml`.
- Repository name, release asset prefix, checksum asset prefix, zip top-level directory, and final install directory should all use the same `skill_id`.

## 3. Top-Level Capability Overview

| Top-level item | Purpose | Available by default | Notes |
| --- | --- | --- | --- |
| `vulcan.call` | Call another skill entry | Yes | The second argument must be a Lua table |
| `vulcan.runtime` | Runtime helper capabilities | Yes | Includes logging, cwd, luaexec, skill management bridge, and more |
| `vulcan.fs` | File system reads and writes | Yes | No sandbox restriction is provided |
| `vulcan.path` | Path joining | Yes | Returns Lua-friendly system paths |
| `vulcan.process` | Start child processes | Yes | Returns structured results |
| `vulcan.os` | Host OS and architecture info | Yes | `os`, `arch` |
| `vulcan.json` | JSON encode/decode | Yes | JSON to/from Lua table |
| `vulcan.cache` | Runtime cache | Yes | Disabled inside `vulcan.runtime.lua.exec` |
| `vulcan.host` | Host-registered tool bridge | Yes | Empty until the host registers a callback |
| `vulcan.context` | Request and current-entry context | Yes | Most values are host-injected |
| `vulcan.deps` | Current skill dependency root paths | Yes | May be `nil` without a resolved skill context |
| `vulcan.sqlite` | Current skill SQLite binding | Conditional | `enabled/status/info` still exist when disabled |
| `vulcan.lancedb` | Current skill LanceDB binding | Conditional | `enabled/status/info` still exist when disabled |

## 3.1 Host-Forced Skill Ignore List

`ignored_skill_ids` is a host runtime policy used to skip selected skills early during load.

Typical scenarios:

- The host already provides a stronger native, gRPC, or VMM implementation.
- A default skill package overlaps with existing host functionality.
- A database-backed skill has been replaced by a host-side service, so SQLite or LanceDB binding should not start.

Matching rules:

- It matches the `skill_id` derived from the skill directory.
- It does not match `skill.yaml` `name`.
- It is not declared by the skill itself and is not part of dependency resolution.

Runtime effects:

- The whole skill is skipped when the ignore list matches.
- `dependencies.yaml` is not prepared.
- SQLite and LanceDB are not bound.
- No entries are registered.
- The skill does not appear in `list_entries` and cannot be called through `vulcan.call`.

This capability preserves the host and user's final choice. If users should still be able to use a skill, hosts should not add it to the ignore policy.

## 4. `vulcan.call`

### 4.1 Purpose

`vulcan.call(name, args)` lets one skill call another loaded skill entry.

- `name`: target entry canonical name.
- `args`: must be a Lua table.
- Return value: directly forwards the target skill's return values and supports multiple returns.

### 4.2 Minimal Example

```lua
local ok, result = pcall(vulcan.call, "demo-skill-run", {
    query = "hello",
    limit = 5,
})

if not ok then
    vulcan.runtime.log("warn", "call failed: " .. tostring(result))
    return nil
end

return result
```

### 4.3 Notes

- Missing entries raise an error directly.
- `args` must be a table, not a string or another scalar.
- `vulcan.call` inherits the current request context, budget snapshot, and tool config, then switches to the target skill's file context and database bindings.
- In `luaexec` scenarios, extra reentry protection prevents unbounded recursion back into the current runtime caller.

## 4.5 `vulcan.host.*`

`vulcan.host.*` is a fixed bridge for host-registered tools.
It is intentionally narrower than arbitrary `vulcan.xxx` injection: Lua can list, probe, and call host tools, but it cannot create new top-level namespaces or register host tools itself.

Supported methods:

- `vulcan.host.list()`: returns the current host-visible tool metadata table.
- `vulcan.host.has(tool_name)`: returns whether one host tool exists.
- `vulcan.host.has_tool(tool_name)`: alias of `has`.
- `vulcan.host.call(tool_name, args)`: calls one host tool with a Lua table argument and returns a Lua table result.

Minimal example:

```lua
if not vulcan.host.has("model.embed") then
    return {
        ok = false,
        reason = "model-embed-unavailable",
    }
end

local result = vulcan.host.call("model.embed", {
    model = "text-embedding-3-small",
    input = "hello",
})

if not result.ok then
    return result
end

return result.value
```

Recommended host-tool result envelope:

```lua
{
    ok = true,
    value = {
        embedding = { 0.1, 0.2, 0.3 },
    },
    meta = {
        provider = "openai",
        elapsed_ms = 120,
    },
}
```

Recommended error envelope:

```lua
{
    ok = false,
    error = {
        code = "tool_not_found",
        message = "host tool not found: model.embed",
    },
}
```

Behavior rules:

- `list()` returns an empty table when the host has not registered a host-tool callback.
- `has()` and `has_tool()` return `false` when the host has not registered a host-tool callback.
- `call()` returns an error envelope when the host callback is missing or the callback returns an error.
- `args` must be a Lua table. Use explicit keys for object-shaped inputs.
- Streaming is not part of this bridge. Host tools should return one complete table result.
- Permissions, timeouts, audit, secret handling, model policies, and final provider routing remain host responsibilities.

## 5. `vulcan.runtime.*`

### 5.1 `vulcan.runtime.log(level, message)`

Writes one runtime log message to the host.

```lua
vulcan.runtime.log("info", "skill started")
vulcan.runtime.log("warn", "budget is low")
vulcan.runtime.log("error", "query failed")
```

Notes:

- `level` is roughly classified by text into `error/fatal`, `warn`, or other.
- This capability is available in normal skill VMs.
- It is disabled in the isolated execution environment used by `vulcan.runtime.lua.exec(...)`.

### 5.2 `vulcan.runtime.cwd()`

Returns the current process working directory.

```lua
local cwd = vulcan.runtime.cwd()
```

### 5.3 `vulcan.runtime.temp_dir`

Host-injected temporary directory path. It may be `nil`.

```lua
local temp_dir = vulcan.runtime.temp_dir
```

### 5.4 `vulcan.runtime.resources_dir`

Host-injected resources directory path. It may be `nil`.

```lua
local resources_dir = vulcan.runtime.resources_dir
```

### 5.5 `vulcan.runtime.overflow_type`

Currently exposes two fixed constants:

- `vulcan.runtime.overflow_type.truncate`
- `vulcan.runtime.overflow_type.page`

They are mainly used by host-side budget and overflow policy logic.

### 5.6 `vulcan.runtime.internal`

Currently exposes these fields:

- `tool_name`
- `skill_name`
- `luaexec_active`
- `luaexec_caller_tool_name`

These fields are **internal execution context**. Use them for debugging and issue diagnosis only; do not treat them as long-term public protocol.

### 5.7 `vulcan.runtime.lua.exec(input)`

Runs one isolated inline Lua runtime call and returns a **Markdown string**, not a normal Lua table.

Supported input fields:

- `task`: optional human-readable task summary.
- `code`: optional inline Lua code.
- `file`: optional Lua file path to execute.
- `args`: optional structured argument object passed to the code; defaults to an empty object.
- `timeout_ms`: optional timeout in milliseconds; defaults to `60000`.

Minimal example:

```lua
local rendered = vulcan.runtime.lua.exec({
    task = "inspect args",
    code = [[
        print("hello", args.name)
        return { ok = true, name = args.name }
    ]],
    args = {
        name = "codex",
    },
})

return rendered
```

Important limits:

- The return value is rendered Markdown text, not a structured Lua table.
- The isolated environment overrides global `print` and collects output into the result text.
- The isolated environment disables:
  - `vulcan.runtime.log`
  - `vulcan.cache.put`
  - `vulcan.cache.get`
  - `vulcan.cache.delete`
  - `vulcan.runtime.lua.exec` itself, preventing recursive luaexec calls
- This environment uses an internal synthetic request context. Inside `luaexec`, values such as:
  - `vulcan.context.client_info.name`
  - `vulcan.context.request.transport_name`
  usually use internal identifiers such as `luaexec_call`, not the real external client.
- The isolated execution path has its own VM pool:
  - Default `min_size=1 / max_size=4 / idle_ttl_secs=60`.
  - Hosts can override it through `LuaRuntimeHostOptions.runlua_pool_config`.
  - This only affects `vulcan.runtime.lua.exec(...)` and does not change the normal skill VM pool.
  - The runtime no longer supports configuring a separate external executor path for `vulcan.runtime.lua.exec(...)`.

### 5.8 `vulcan.runtime.skills.*`

These capabilities let a skill ask the host to install, update, enable, disable, or uninstall skills.

The formal layer model is:

```text
ROOT -> PROJECT -> USER
```

`ROOT` is system-controlled, and a runtime must receive a `ROOT` layer when it starts or loads. Normal skills cannot use `vulcan.runtime.skills.*` to install, update, uninstall, enable, or disable `ROOT` skills. The normal bridge only targets currently existing `PROJECT` and `USER` layers exposed by the host.

`vulcan.runtime.skills.*` is always equivalent to `DelegatedTool` authority: it cannot see `ROOT` skills and cannot write `ROOT`. FFI query and prompt-completion entry points under `DelegatedTool` also do not return `ROOT` entries, help, or ROOT tool ownership. `call_skill` and `run_lua` are runtime execution surfaces: they can call currently active skills, but they are not skill-management authorization boundaries. If `ROOT` already contains the same `skill_id`, normal-layer install and update are rejected; normal-layer uninstall can still clean stale same-name entries from `PROJECT` or `USER`.

The formal bridge should include:

- `vulcan.runtime.skills.enabled`
- `vulcan.runtime.skills.status()`
- `vulcan.runtime.skills.layers()`
- `vulcan.runtime.skills.install(input)`
- `vulcan.runtime.skills.update(input)`
- `vulcan.runtime.skills.uninstall(input)`
- `vulcan.runtime.skills.enable(input)`
- `vulcan.runtime.skills.disable(input)`

`status()` currently returns:

- `enabled`
- `callback_registered`
- `mode`
- `message`

`layers()` returns the layer labels that the host allows the normal bridge to operate on. Recommended fields include:

- `default`
- `writable`
- `labels`
- `layers`

`labels` should include only currently existing `PROJECT` and `USER` layers; it should not include `ROOT`. Without a project context, the runtime returns only `USER`. If only `ROOT` exists, it returns an empty list and top-level `writable=false`. When the bridge is disabled, layer discovery can still work, but top-level `writable` and each layer's `writable` must be `false`. If future install, update, or uninstall inputs allow selecting a layer, they should only accept labels returned by `layers()`.

For the full layer and management boundary model, see [Skill root layer policy](zh-CN/architecture/skill-root-layer-policy.md).

Notes:

- These capabilities only execute when the host explicitly enables `enable_skill_management_bridge`.
- Even when host policy enables the bridge, missing callbacks return explicit errors.
- `input` and return structures are part of the host bridge contract and should be constrained by host-side docs or test fixtures.

### 5.9 `vulcan.config.*`

These capabilities read and maintain **string config for the current skill itself**.

Currently available:

- `vulcan.config.get(key)`
- `vulcan.config.set(key, value)`
- `vulcan.config.delete(key)`
- `vulcan.config.has(key)`
- `vulcan.config.list()`

Minimal example:

```lua
local api_token = vulcan.config.get("api_token")

if not api_token or api_token == "" then
    return "The current skill has no `api_token` configured. Use the host runtime-config tool to set `api_token` for this skill."
end

local endpoint = vulcan.config.get("endpoint") or "https://api.example.com"

vulcan.config.set("last_endpoint", endpoint)

return {
    ok = true,
    endpoint = endpoint,
}
```

`list()` currently returns a flat table for the current skill namespace:

```lua
local config = vulcan.config.list()
-- config.api_token
-- config.endpoint
```

Notes:

- Config values are strings in the first version.
- If you need complex structure, store JSON text as a string and decode it with `vulcan.json.decode(...)` inside the skill.
- Config defaults to the current skill. Skills cannot directly read or write other skill namespaces.
- The runtime does not automatically refuse to load a skill just because config is missing. Prefer returning clear guidance when required config is absent.
- The unified main config file defaults to `<runtime_root>/config/skill_config.json`; hosts can explicitly override the path.

## 6. `vulcan.fs.*`

### 6.1 Supported Methods

- `vulcan.fs.list(dir)`
- `vulcan.fs.read(path)`
- `vulcan.fs.write(path, content)`
- `vulcan.fs.exists(path)`
- `vulcan.fs.is_dir(path)`

### 6.2 Example

```lua
local entries = vulcan.fs.list(vulcan.context.entry_dir)
local exists = vulcan.fs.exists(vulcan.context.entry_file)
local content = vulcan.fs.read(vulcan.context.entry_file)
```

### 6.3 Notes

- There is currently no sandbox restriction; a skill can theoretically access any path that the host process can access.
- `fs.read` and `fs.write` operate on text content.
- Path arguments must be strings and pass basic path syntax validation.

## 7. `vulcan.path.*`

Currently exposes only:

- `vulcan.path.join(...)`

Example:

```lua
local config_path = vulcan.path.join(
    vulcan.context.skill_dir,
    "runtime",
    "config.json"
)
```

Path return rules:

- Returns normal path text for the host system.
- On Windows, it does not leak `\\?\` or `\\?\UNC\` verbatim prefixes directly to Lua.

## 8. `vulcan.process.*`

Currently exposes only:

- `vulcan.process.exec(spec)`

### 8.1 Request Shape

Two modes are supported.

1. Shell mode:

```lua
local result = vulcan.process.exec({
    shell = "echo hello",
    timeout_ms = 3000,
})
```

2. Program mode:

```lua
local result = vulcan.process.exec({
    program = "git",
    args = { "status", "--short" },
    cwd = vulcan.runtime.cwd(),
    env = {
        DEMO_MODE = "1",
    },
    timeout_ms = 5000,
})
```

Common fields:

- `shell`
- `program`
- `args`
- `cwd`
- `env`
- `stdin`
- `timeout_ms`

### 8.2 Result Shape

The returned table always contains:

- `ok`
- `success`
- `code`
- `stdout`
- `stderr`
- `timed_out`
- `error`

## 9. `vulcan.os.*`

Currently provides:

- `vulcan.os.info()`

Example:

```lua
local info = vulcan.os.info()
-- info.os
-- info.arch
```

## 10. `vulcan.json.*`

Currently provides:

- `vulcan.json.encode(value)`
- `vulcan.json.decode(text)`

Example:

```lua
local text = vulcan.json.encode({
    hello = "world",
    limit = 3,
})

local obj = vulcan.json.decode(text)
```

Notes:

- Lua tables are converted into JSON objects or arrays.
- Decoded JSON objects and arrays are converted back into Lua tables.

## 11. `vulcan.cache.*`

Currently provides:

- `vulcan.cache.put(value, ttl_sec?)`
- `vulcan.cache.get(cache_id)`
- `vulcan.cache.delete(cache_id)`

Example:

```lua
local cache_id = vulcan.cache.put({
    summary = "warm result",
}, 60)

local cached = vulcan.cache.get(cache_id)
local deleted = vulcan.cache.delete(cache_id)
```

Notes:

- Cache scope first falls back to the current `tool_name`, then the current `skill_name`.
- If neither is available, it falls back to the internal `__runtime` scope.
- Inside `vulcan.runtime.lua.exec(...)`, cache APIs are actively removed and unavailable.

## 12. `vulcan.context.*`

`vulcan.context` reads the current request and current-entry runtime context.

Current fields:

- `request`
- `client_info`
- `client_capabilities`
- `client_budget`
- `tool_config`
- `skill_dir`
- `entry_dir`
- `entry_file`

### 12.1 `vulcan.context.request`

The original host-provided request context object. Defaults to an empty object.

Common fields come from:

- `transport_name`
- `session_id`
- `client_info`
- `client_capabilities`

### 12.2 `vulcan.context.client_info`

Current request client metadata. Common fields:

- `kind`
- `name`
- `version`

Notes:

- If the host does not inject `client_info`, this may be `nil`.
- If you see `name = "luaexec_call"` inside `luaexec`, that is the synthetic context for internal isolated execution, not the real external client.

### 12.3 `vulcan.context.client_capabilities`

Host-provided client capability object. Defaults to an empty object.

### 12.4 `vulcan.context.client_budget`

Host-parsed budget snapshot object. Defaults to an empty object.

The host decides the shape, but common fields include:

- `client_name`
- `tool_name`
- `skill_name`
- `tool_result`
- `file_read`

### 12.5 `vulcan.context.tool_config`

Host-parsed tool config object. Defaults to an empty object.

### 12.6 `vulcan.context.skill_dir / entry_dir / entry_file`

File context for the currently executing skill:

- `skill_dir`: current skill directory.
- `entry_dir`: current entry script directory.
- `entry_file`: current entry script full path.

Notes:

- In normal skill calls, all three are usually available.
- In some runlua, help, or non-skill-file scenarios, they may be `nil`.
- The current implementation automatically strips Windows verbatim path prefixes so Lua receives normal system paths.

## 13. `vulcan.deps.*`

Current fields:

- `vulcan.deps.tools_path`
- `vulcan.deps.lua_path`
- `vulcan.deps.ffi_path`

These are dependency root paths for the current skill:

- Tool dependency directory.
- Lua dependency directory.
- FFI dependency directory.

Example:

```lua
local lua_lib_root = vulcan.deps.lua_path
local ffi_root = vulcan.deps.ffi_path
```

Notes:

- These paths depend on the current skill root and host dependency layout.
- If there is no valid current skill context, they are `nil`.
- Skills should rely on these protocol-exposed paths and should not guess the host's physical directory layout.

## 14. `vulcan.sqlite.*`

`vulcan.sqlite` is a SQLite binding isolated to the **current skill scope**.

### 14.1 Current Fields And Methods

- `vulcan.sqlite.enabled`
- `vulcan.sqlite.info()`
- `vulcan.sqlite.status()`
- `vulcan.sqlite.tokenize_text(input)`
- `vulcan.sqlite.execute_script(input)`
- `vulcan.sqlite.execute_batch(input)`
- `vulcan.sqlite.query_json(input)`
- `vulcan.sqlite.query_stream(input)`
- `vulcan.sqlite.query_stream_wait_metrics(input)`
- `vulcan.sqlite.query_stream_chunk(input)`
- `vulcan.sqlite.query_stream_close(input)`
- `vulcan.sqlite.upsert_custom_word(input)`
- `vulcan.sqlite.remove_custom_word(input)`
- `vulcan.sqlite.list_custom_words()`
- `vulcan.sqlite.ensure_fts_index(input)`
- `vulcan.sqlite.rebuild_fts_index(input)`
- `vulcan.sqlite.upsert_fts_document(input)`
- `vulcan.sqlite.delete_fts_document(input)`
- `vulcan.sqlite.search_fts(input)`

### 14.2 Behavior Rules

- `enabled = true` means the current skill has a SQLite binding.
- `info()` and `status()` always exist.
- When SQLite is not enabled:
  - `enabled = false`
  - `info()` and `status()` return disabled-state descriptions
  - other methods error directly with `current skill has not enabled sqlite`

### 14.3 Development Guidance

- Treat `info()` and `status()` as probing entry points.
- Check `enabled` before business calls so "capability not bound" is not mistaken for a query failure.
- For exact input and output fields, combine the host SQLite provider contract with:
  - [Host database provider guide](zh-CN/providers/host-database-provider-guide.md)

## 15. `vulcan.lancedb.*`

`vulcan.lancedb` is a LanceDB binding isolated to the **current skill scope**.

### 15.1 Current Fields And Methods

- `vulcan.lancedb.enabled`
- `vulcan.lancedb.info()`
- `vulcan.lancedb.status()`
- `vulcan.lancedb.create_table(input)`
- `vulcan.lancedb.vector_upsert(input)`
- `vulcan.lancedb.vector_search(input)`
- `vulcan.lancedb.delete(input)`
- `vulcan.lancedb.drop_table(input)`

### 15.2 Behavior Rules

- `enabled = true` means the current skill has a LanceDB binding.
- `info()` and `status()` always exist.
- When LanceDB is not enabled:
  - `enabled = false`
  - `info()` and `status()` return disabled-state descriptions
  - other methods error directly with `current skill has not enabled lancedb`

### 15.3 Special Note

`vector_search(input)` results may contain two payload forms:

- `data_json`
- `data`

When the result format is JSON, the result table contains `data_json`; otherwise it contains the raw binary string `data`.

For exact input and output fields, combine the host LanceDB provider contract with:

- [Host database provider guide](zh-CN/providers/host-database-provider-guide.md)

## 16. Common Development Guidance

### 16.1 Probe Before Calling

For host-conditionally injected capabilities, probe first:

```lua
if vulcan.sqlite.enabled then
    return vulcan.sqlite.query_json({
        sql = "select 1 as ok",
    })
end

return {
    ok = false,
    reason = "sqlite-disabled",
}
```

### 16.2 Do Not Depend On Internal Field Names

These are internal mechanisms and should not be long-term skill dependencies:

- `vulcan.runtime.internal.*`
- `vulcan.__sqlite_skill_name`
- `vulcan.__lancedb_skill_name`

### 16.3 Do Not Guess Host Directory Layout

Prefer:

- `vulcan.context.skill_dir`
- `vulcan.context.entry_dir`
- `vulcan.context.entry_file`
- `vulcan.deps.*`

Do not infer:

- runtime root directory
- another skill's dependency directory
- whether the host uses a fixed directory name

### 16.4 Distinguish External Request Context From Internal Luaexec Context

If you see the real client name during normal skill execution but see `luaexec_call` inside `vulcan.runtime.lua.exec(...)`, that is expected by design, not a bug.

## 17. Recommended Reading Order

If you mainly write skills, read in this order:

1. This manual: understand the actual current `vulcan.*` capability surface.
2. [README.md](../README.md): understand runtime positioning and host boundaries.
3. [Host database provider guide](zh-CN/providers/host-database-provider-guide.md): understand SQLite / LanceDB host integration contracts.

If you are integrating a host rather than writing a skill, read:

- [FFI integration guide](zh-CN/ffi/integration-guide.md)
- [FFI host checklist](zh-CN/ffi/host-checklist.md)
