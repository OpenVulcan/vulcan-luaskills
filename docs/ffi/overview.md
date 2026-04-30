# FFI and SDK Overview

[Documentation hub](../index.md) | [Chinese FFI guide](../zh-CN/ffi/integration-guide.md)

LuaSkills exposes the same runtime through several host integration layers.
The goal is to let each host choose the right binding cost without changing the skill model.

## Integration Layers

| Layer | Best For | Notes |
| --- | --- | --- |
| Rust API | Rust hosts | Direct crate integration is the primary path for Rust applications. |
| Standard C ABI | C, C++, low-level hosts, binding generators | Uses explicit structs, buffers, out pointers, status codes, and dedicated free functions. |
| Public `_json` FFI | Dynamic languages and SDKs | Uses JSON input/output envelopes and is easier to wrap from Python, Node.js, TypeScript, and similar hosts. |
| Language SDKs | Product teams that want fewer ABI details | TypeScript, Python, and Go SDKs wrap runtime loading, JSON envelopes, authority helpers, lifecycle calls, and provider callback boundaries. |

## How To Choose

- Rust host: call the crate directly.
- C or C++ host: start from the standard C ABI.
- TypeScript or Node.js host: prefer [luaskills-sdk-typescript](https://github.com/LuaSkills/luaskills-sdk-typescript).
- Python host: prefer [luaskills-sdk-python](https://github.com/LuaSkills/luaskills-sdk-python).
- Go host: use [luaskills-sdk-go](https://github.com/LuaSkills/luaskills-sdk-go) or standard C ABI depending on deployment and callback needs.
- Mixed host: use standard C ABI for stable core calls and public `_json` FFI for dynamic operations.

## First Integration Sequence

For a new FFI host, stabilize the smallest runtime loop first:

1. `version`
2. `engine_new`
3. `load_from_roots`
4. `list_entries`
5. `call_skill`
6. `run_lua`
7. `engine_free`

After that, add lifecycle operations, query helpers, installation/update flows, provider callbacks, host-tool callbacks, or `space_controller`.

## Key Rules

- Register host callbacks before creating an engine.
- Use `luaskills_ffi_set_host_tool_json_callback` when Lua skills need to call host-registered tools through `vulcan.host.*`.
- Do not throw exceptions across C ABI boundaries.
- Do not re-enter the same engine from the same thread.
- Free owned buffers with the matching LuaSkills free function.
- Let the host decide authority and root write policy.
- Treat the current FFI surface as a controlled host integration contract, not a sandbox boundary.

## Deep References

- [FFI beta release notes](../zh-CN/ffi/beta-release-notes.md)
- [FFI host checklist](../zh-CN/ffi/host-checklist.md)
- [FFI integration guide](../zh-CN/ffi/integration-guide.md)
- [Host database provider guide](../zh-CN/providers/host-database-provider-guide.md)

## Examples

- [C FFI demo](../../examples/ffi/c/README.md)
- [TypeScript FFI demo](../../examples/ffi/typescript/README.md)
- [Standard runtime fixture](../../examples/ffi/standard_runtime/README.md)
- [FFI demo runtime](../../examples/ffi/demo_runtime/README.md)
- [Host provider demo](../../examples/ffi/host_provider_demo/README.md)
