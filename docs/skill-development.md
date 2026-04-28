# Skill Development Overview

[Documentation hub](index.md) | [Chinese manual](zh-CN/skill-development.md)

LuaSkills skills are Lua packages that expose callable entries, structured help, runtime code, resources, and optional dependency declarations.
The runtime gives every skill stable `vulcan.*` APIs so skill authors do not need to guess the host application's directory layout or product policy.

## What A Skill Contains

A typical skill repository contains:

- `skill.yaml` for package metadata and entry definitions.
- `runtime/` Lua files for entry implementation.
- `help/` Markdown files for strict help trees and workflow documentation.
- Optional resources and dependency metadata.

Use [demo-skill](https://github.com/LuaSkills/demo-skill) to learn the minimal package shape.
Use [vulcan-codekit](https://github.com/LuaSkills/vulcan-codekit) to study a real, production-oriented skill.

## Naming Rules

`skill_id` is the stable identifier used by the runtime, lifecycle operations, config namespaces, dependency folders, database bindings, and canonical entry names. The physical skill directory name is the only `skill_id`; `skill.yaml` must not declare a `skill_id` field.

`skill_id` and every `entry.name` must match:

```text
^[a-z]([a-z0-9-]*[a-z0-9])?$
```

Use lowercase ASCII letters, digits, and `-`; start with a lowercase letter; do not end with `-`. Valid examples include `vulcan-codekit`, `codekit2`, and `vulcan-runtime-tools`. Invalid examples include `2codekit`, `Vulcan-codekit`, `vulcan_codekit`, and `vulcan-codekit-`.

Runtime canonical entry names use `{skill_id}-{entry_name}` and may become `{skill_id}-{entry_name}-{N}` when a stable collision suffix is required.

For GitHub-managed skills, the repository-derived or explicit `skill_id`, release zip prefix, checksum prefix, zip top-level directory, and final installed directory must all use the same value. Release assets use `{skill_id}-v{version}-skill.zip`, checksum files use `{skill_id}-v{version}-checksums.txt`, and the zip must contain `{skill_id}/skill.yaml`.

## Runtime APIs

LuaSkills injects standard namespaces into Lua:

- `vulcan.call`
- `vulcan.runtime.*`
- `vulcan.fs.*`
- `vulcan.path.*`
- `vulcan.process.*`
- `vulcan.os.*`
- `vulcan.json.*`
- `vulcan.cache.*`
- `vulcan.context.*`
- `vulcan.deps.*`
- `vulcan.sqlite.*`
- `vulcan.lancedb.*`

The Chinese [Lua Skill developer manual](zh-CN/skill-development.md) is the complete reference for these APIs.

## Authoring Rules

Skill authors should:

- Use `vulcan.context.*` to locate the current skill, entry, resources, and request context.
- Use `vulcan.deps.*` to locate tools, Lua dependencies, and FFI dependencies.
- Treat SQLite and LanceDB bindings as host-controlled capabilities.
- Return clear user-facing guidance when required configuration is missing.
- Keep help files structured enough for hosts to render them as docs, tools, or command palettes.

Skill authors should not:

- Walk upward through `..` to infer the runtime root.
- Hard-code host-specific directory names.
- Depend on another skill's private dependency paths.
- Assume database bindings are always available.
- Treat `vulcan.runtime.internal` fields as stable public API.

## Recommended Reading

1. [demo-skill](https://github.com/LuaSkills/demo-skill)
2. [vulcan-codekit](https://github.com/LuaSkills/vulcan-codekit)
3. [Lua Skill developer manual](zh-CN/skill-development.md)
4. [Skill root layer policy](zh-CN/architecture/skill-root-layer-policy.md)
5. [Host database provider guide](zh-CN/providers/host-database-provider-guide.md)
