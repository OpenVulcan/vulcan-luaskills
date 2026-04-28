# Runtime Architecture Overview

[Documentation hub](../index.md) | [Chinese root policy](../zh-CN/architecture/skill-root-layer-policy.md)

LuaSkills is intentionally a runtime layer, not a full product host.
Its architecture separates skill execution truth from product policy.

## Runtime Responsibilities

LuaSkills owns:

- Skill package loading.
- Entry discovery and invocation.
- Strict help parsing.
- Runtime result shaping.
- `vulcan.*` and `vulcan.runtime.*` injection.
- Skill dependency path injection.
- Optional SQLite and LanceDB provider routing.
- Lifecycle primitives such as install, update, enable, disable, and uninstall.

## Host Responsibilities

The host owns:

- User permissions.
- Product UI and command surfaces.
- Client budgets and truncation policy.
- Configuration source and storage placement.
- Which roots are writable.
- Which authority is attached to each management operation.
- Whether database ownership stays local, host-controlled, or controller-backed.

## Skill Root Layers

The active model uses three conceptual root layers:

```text
ROOT -> PROJECT -> USER
```

- `ROOT` is the system-controlled layer for host-managed core skills.
- `PROJECT` is the project-level writable layer.
- `USER` is the user-level writable layer.

When the same `skill_id` exists in multiple layers, higher-priority layers win.
`ROOT` is special: ordinary user management flows should not mutate it.

## Help Model

LuaSkills treats help as structured runtime data, not as ordinary tool output.

The host can render help as:

- Markdown documentation.
- Tool descriptions.
- Command palette entries.
- UI panels.
- Agent context.

## Deep References

- [Skill root layer policy](../zh-CN/architecture/skill-root-layer-policy.md)
- [Skill config system design](../zh-CN/architecture/skill-config-system-design.md)
- [Lua Skill developer manual](../zh-CN/skill-development.md)
