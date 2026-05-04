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

## Managed Identity Field Quick Path

If an FFI or SDK host projects LuaSkills entries into model-facing or user-facing tools, it should implement the standard `LUASKILL_SID` managed identity contract.

Host-side setup:

1. Inspect each entry input schema after `list_entries`.
2. When a schema contains `LUASKILL_SID` and the host has a stable conversation, task, workspace, or equivalent identity, hide that field from the projected tool schema.
3. Remove the hidden field from the projected `required` list.
4. Inject the stable `LUASKILL_SID` value into the entry arguments before `call_skill`.
5. Add managed-mode help text so the model or user does not ask for, print, or save the raw managed identity.
6. Redact or rewrite raw managed identities from projected results when needed.

If the host cannot provide a stable identity, leave `LUASKILL_SID` visible and let the caller or the skill's create/start/bootstrap fallback flow provide it.

## Model Capability Quick Path

Use `vulcan.models.*` when Lua skills need model capabilities that remain fully controlled by the host.
This is different from `vulcan.host.*`: the model surface is fixed and capability-specific, not a generic host tool call.

Host-side setup:

1. Keep provider settings outside LuaSkills, for example in the host's own model configuration file or product settings.
2. Register `luaskills_ffi_set_model_embed_json_callback` only when embeddings are enabled.
3. Register `luaskills_ffi_set_model_llm_json_callback` only when one-turn non-streaming LLM calls are enabled.
4. Create the engine, load roots, and call skills.
5. Clear the process-level callbacks when the host shuts down.

Callback request and response rules:

- Embedding callback request: `{ "text": string, "caller": object }`.
- LLM callback request: `{ "system": string, "user": string, "caller": object }`.
- Embedding success response: `{ "vector": number[], "dimensions": number, "usage"?: object }`.
- LLM success response: `{ "assistant": string, "usage"?: object }`.
- Failure response: `{ "ok": false, "error": { "code": string, "message": string, "provider_message"?: string, "provider_code"?: string, "provider_status"?: number } }`.

`caller` is attached by LuaSkills and may include `skill_id`, `entry_name`, `canonical_tool_name`, `root_name`, `skill_dir`, `client_name`, and `request_id`.
Use it for attribution, budget policy, rate limits, and audit logs.

SDK mapping:

| SDK | Register | Clear |
| --- | --- | --- |
| TypeScript | `setModelEmbedJsonCallback`, `setModelLlmJsonCallback` | `clearModelEmbedJsonCallback`, `clearModelLlmJsonCallback` |
| Python | `set_model_embed_json_callback`, `set_model_llm_json_callback` | `clear_model_embed_json_callback`, `clear_model_llm_json_callback` |
| Go | Typed model callback boundary APIs | Requires a host-owned cgo callback bridge for real registration |

## Key Rules

- Register host callbacks before creating an engine.
- Use `luaskills_ffi_set_host_tool_json_callback` when Lua skills need to call host-registered tools through `vulcan.host.*`.
- Use `luaskills_ffi_set_model_embed_json_callback` and `luaskills_ffi_set_model_llm_json_callback` when Lua skills need host-managed model capabilities through `vulcan.models.*`.
- When projecting entries as tools, follow the `LUASKILL_SID` managed identity contract instead of inventing host-specific session parameter names.
- Do not throw exceptions across C ABI boundaries.
- Do not re-enter the same engine from the same thread.
- Free owned buffers with the matching LuaSkills free function.
- Let the host decide authority and root write policy.
- Let the host own model provider configuration; LuaSkills only forwards fixed model requests and structured error envelopes.
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
