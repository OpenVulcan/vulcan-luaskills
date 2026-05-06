# LuaSkills Documentation

[English](index.md) | [简体中文](zh-CN/index.md) | [日本語](ja/index.md) | [한국어](ko/index.md) | [Español](es/index.md) | [Français](fr/index.md) | [Deutsch](de/index.md) | [Português (BR)](pt-BR/index.md)

[Repository README](../README.md) | [Why LuaSkills](product/why-luaskills.md)

This is the English documentation hub for LuaSkills.
The skill author manual is available in English, while some deep host and FFI references still live in Chinese under `docs/zh-CN`.

## Product Overview

LuaSkills is a runtime for product-grade Lua skills.
It lets a host application load, run, manage, and document Lua skill packages while keeping product policy in the host.

Read:

- [Why LuaSkills](product/why-luaskills.md)
- [Skill development manual](skill-development.md)
- [FFI and SDK overview](ffi/overview.md)
- [Database provider overview](providers/database-providers.md)
- [Runtime architecture overview](architecture/runtime-model.md)
- [Chinese product overview](../README.zh-CN.md)
- [Chinese technical documentation index](zh-CN/index.md)

## Choose Your Path

| Reader | Start Here |
| --- | --- |
| Product or platform owner | [Why LuaSkills](product/why-luaskills.md) |
| Lua skill author | [Skill development manual](skill-development.md) |
| Rust host developer | [Repository README](../README.md#integration-paths) |
| C ABI or SDK integrator | [FFI and SDK overview](ffi/overview.md) |
| Deep FFI integrator | [FFI integration guide](zh-CN/ffi/integration-guide.md) |
| Existing `0.2.x` integrator upgrading to `0.3.x` | [`0.2 -> 0.3` upgrade guide](upgrade-from-0.2-to-0.3.md) |
| Database provider implementer | [Database provider overview](providers/database-providers.md) |
| Runtime architecture reader | [Runtime architecture overview](architecture/runtime-model.md) |

## Skill Naming Rules

`skill_id` and every `entry.name` must match `^[a-z]([a-z0-9-]*[a-z0-9])?$`.
The physical skill directory name is the only `skill_id`; `skill.yaml` must not declare a `skill_id` field.
Canonical entries are exposed as `{skill_id}-{entry_name}` and may receive a stable `-N` suffix on conflicts.
For GitHub-managed skills, the repository-derived or explicit `skill_id`, release zip prefix, checksum prefix, zip top-level directory, and installed directory must be identical.
Use `{skill_id}-v{version}-skill.zip`, `{skill_id}-v{version}-checksums.txt`, and a zip containing `{skill_id}/skill.yaml`.

## English Documents

- [Why LuaSkills](product/why-luaskills.md)
- [Skill development manual](skill-development.md)
- [FFI and SDK overview](ffi/overview.md)
- [Database provider overview](providers/database-providers.md)
- [Runtime architecture overview](architecture/runtime-model.md)

## Main Technical Documents

Skill development is available in English. The deepest host, FFI, provider, and architecture references are still maintained in Chinese.

- [Lua Skill developer manual](skill-development.md)
- [Chinese Lua Skill developer manual](zh-CN/skill-development.md)
- [FFI beta release notes](zh-CN/ffi/beta-release-notes.md)
- [FFI host checklist](zh-CN/ffi/host-checklist.md)
- [FFI integration guide](zh-CN/ffi/integration-guide.md)
- [`0.2 -> 0.3` upgrade guide](upgrade-from-0.2-to-0.3.md)
- [Host database provider guide](zh-CN/providers/host-database-provider-guide.md)
- [Skill root layer policy](zh-CN/architecture/skill-root-layer-policy.md)
- [Skill config system design](zh-CN/architecture/skill-config-system-design.md)
- [FFI refactor draft archive](zh-CN/archive/ffi-refactor-draft.md)

## Ecosystem References

- [vulcan-codekit](https://github.com/LuaSkills/vulcan-codekit): production-grade LuaSkills example.
- [demo-skill](https://github.com/LuaSkills/demo-skill): minimal skill repository template.
- [luaskills-sdk-typescript](https://github.com/LuaSkills/luaskills-sdk-typescript): TypeScript / Node.js SDK.
- [luaskills-sdk-python](https://github.com/LuaSkills/luaskills-sdk-python): Python SDK.
- [luaskills-sdk-go](https://github.com/LuaSkills/luaskills-sdk-go): Go SDK.

## Local Examples

- [C FFI demo](../examples/ffi/c/README.md)
- [TypeScript FFI demo](../examples/ffi/typescript/README.md)
- [Standard runtime fixture](../examples/ffi/standard_runtime/README.md)
- [FFI demo runtime](../examples/ffi/demo_runtime/README.md)
- [Host provider demo](../examples/ffi/host_provider_demo/README.md)
- [Rust demo](../examples/demo-rust/README.md): direct crate host integration with `call_skill` and `vulcan.host.*`.
- [FFI demo package entry](../examples/demo-ffi/README.md)
