---
name: luaskills-debug-skill
description: Debug one local LuaSkills skill package through the real `luaskills-debug` runtime flow. Use when Codex needs to inspect one skill package, list its callable tools, or call one tool with a real `runtime_root`, dependency roots, state directories, databases, and canonical tool-name resolution. Trigger for requests like “调试这个 skill”, “看看这个 skill 暴露了什么工具”, “用真实 runtime_root 调一下这个 tool”, or “验证这个 skill 的 host_result / change_set 返回”.
---

# LuaSkills Debug Skill

Use this skill to debug one local LuaSkills skill package through the repository-side `luaskills-debug` binary.
使用这个 skill 通过仓库内的 `luaskills-debug` 二进制程序调试单个本地 LuaSkills skill 包。

## Quick Start

Run the wrapper script instead of typing the full binary command by hand.
优先运行包装脚本，不要手写完整的二进制调试命令。

```powershell
python luaskills-debug-skill/scripts/run_debug.py sync --skill-path D:\path\to\skill
python luaskills-debug-skill/scripts/run_debug.py list-tools --skill-id your-skill --output content
python luaskills-debug-skill/scripts/run_debug.py call --skill-id your-skill --tool ping --args-json "{\"note\":\"hello\"}" --output json
```

In a source checkout, the wrapper defaults `runtime_root` to `output/luaskills-debug-runtime/<skill_id>`. In a standalone debug-tool package, it defaults to the package-local `runtime/` directory.
在源码仓库中，包装脚本默认把 `runtime_root` 设为 `output/luaskills-debug-runtime/<skill_id>`；在独立 debug-tool 包中，它默认使用包内 `runtime/` 目录。

LuaSkills derives all runtime directories from `runtime_root`.
LuaSkills 会从 `runtime_root` 推导所有运行时目录。

- Tools are searched directly under `runtime_root/bin`, not `runtime_root/bin/tools`.
- 工具直接在 `runtime_root/bin` 下查找，不再使用 `runtime_root/bin/tools`。
- Lua packages are loaded from `runtime_root/lua_packages`, and native / FFI libraries are loaded from `runtime_root/libs`.
- Lua 包从 `runtime_root/lua_packages` 加载，原生库 / FFI 库从 `runtime_root/libs` 加载。
- Runtime state uses `runtime_root/skills`, `runtime_root/dependencies`, `runtime_root/state`, `runtime_root/databases`, `runtime_root/config`, and `runtime_root/system_lua_lib`.
- 运行时状态使用 `runtime_root/skills`、`runtime_root/dependencies`、`runtime_root/state`、`runtime_root/databases`、`runtime_root/config` 与 `runtime_root/system_lua_lib`。

## Workflow

Start with `sync`.
先从 `sync` 开始。

- Use `sync` to copy the physical source skill into `runtime_root/skills/<skill_id>` once.
- 用 `sync` 将物理源 skill 复制到 `runtime_root/skills/<skill_id>`，只做一次同步。

Then run `inspect`.
然后运行 `inspect`。

- Use `inspect --skill-id <id>` to confirm the effective `skill_id`, synchronized target path, and loaded entries without rewriting the skill directory.
- 使用 `inspect --skill-id <id>` 确认生效 `skill_id`、同步后的目标路径和已加载入口，不再重写 skill 目录。

Then run `list-tools`.
然后运行 `list-tools`。

- Use `list-tools --skill-id <id>` to see local tool names and canonical tool names before calling anything.
- 使用 `list-tools --skill-id <id>` 在真正调用之前查看 local tool name 和 canonical tool name。

Finally run `call`.
最后运行 `call`。

- Use `call --skill-id <id>` with `--output json` when you need full `content / overflow_mode / template_hint / host_result`.
- 当你需要完整的 `content / overflow_mode / template_hint / host_result` 时，用 `call --skill-id <id> --output json`。
- Use `call --output content` only when you only care about the primary text result.
- 只有在你只关心主文本结果时，才使用 `call --output content`。

## Rules

Pass the source package directory to `--skill-path`.
把源 skill 包目录传给 `--skill-path`。

- Point `--skill-path` at the actual skill package directory that contains `skill.yaml`.
- `--skill-path` 必须指向真实 skill 包目录，也就是里面直接包含 `skill.yaml` 的目录。
- Prefer `--skill-path` only for `sync`; use `--skill-id` for repeated `inspect`, `list-tools`, and `call`.
- 优先只在 `sync` 时使用 `--skill-path`；重复执行 `inspect`、`list-tools` 和 `call` 时使用 `--skill-id`。

Keep `runtime_root` real.
保持 `runtime_root` 的真实语义。

- This skill reuses the real `luaskills-debug` bin. `sync` writes `runtime_root/skills/<skill_id>`, while `inspect`, `list-tools`, and `call` can run read-only against `--skill-id`.
- 这个 skill 复用了真实的 `luaskills-debug` bin。`sync` 会写入 `runtime_root/skills/<skill_id>`，而 `inspect`、`list-tools` 和 `call` 可以基于 `--skill-id` 只读运行。
- Use an explicit `--runtime-root` when you need a fixed state/database/dependency layout across repeated runs.
- 如果你需要在多次调试之间固定 state/database/dependency 目录布局，请显式传入 `--runtime-root`。

Use `--enable-host-result` only when needed.
只在确实需要时开启 `--enable-host-result`。

- Add `--enable-host-result` when debugging `host_result` and `change_set`.
- 调试 `host_result` 和 `change_set` 时再加 `--enable-host-result`。
- Leave it off for ordinary content-only calls.
- 普通纯文本调用时保持关闭即可。

Avoid parallel syncs against the same runtime root.
避免针对同一个 runtime root 并发同步。

- Run `sync` serially for one `(runtime_root, skill_id)` pair, then run parallel `inspect`, `list-tools`, or `call` commands with `--skill-id`.
- 对同一个 `(runtime_root, skill_id)` 先串行执行 `sync`，之后再用 `--skill-id` 并发执行 `inspect`、`list-tools` 或 `call`。
- Use different explicit `--runtime-root` values when you need isolated parallel syncs.
- 需要隔离并发同步时，传入不同的显式 `--runtime-root`。

## Script

Use `scripts/run_debug.py` as the primary entrypoint.
把 `scripts/run_debug.py` 当作主要入口。

- In a source checkout, the script builds `luaskills-debug` with `cargo build --bin luaskills-debug` when the local debug binary is missing.
- 在源码仓库中，当本地调试二进制缺失时，脚本会自动执行 `cargo build --bin luaskills-debug` 进行构建。
- In a standalone debug-tool package, the script uses the packaged `bin/luaskills-debug` executable and does not require Cargo.
- 在独立 debug-tool 包中，脚本会使用包内 `bin/luaskills-debug` 可执行文件，不要求存在 Cargo。
- The script forwards `sync`, `inspect`, `list-tools`, and `call` to the real bin without introducing any SDK or FFI layer.
- 这个脚本会把 `sync`、`inspect`、`list-tools` 和 `call` 原样转发给真实 bin，不会额外引入 SDK 或 FFI 层。
- Add `--rebuild` when you know the debug bin source changed and you want a fresh build first.
- 当你知道调试 bin 源码已经变更并希望先重新构建时，追加 `--rebuild`。

## Decision Guide

Choose the command by the debugging goal.
根据调试目标选择命令。

- Need to refresh the runtime copy from source: use `sync --skill-path <dir>`.
- 需要从源目录刷新运行时副本：用 `sync --skill-path <dir>`。
- Need package identity, loaded entries, or synchronized path after sync: use `inspect --skill-id <id>`.
- 同步后需要看包身份、已加载入口或同步路径：用 `inspect --skill-id <id>`。
- Need local/canonical tool names: use `list-tools --skill-id <id>`.
- 需要看 local/canonical tool name：用 `list-tools --skill-id <id>`。
- Need actual execution output: use `call --skill-id <id>`.
- 需要看真实执行结果：用 `call --skill-id <id>`。

If a call fails, rerun with `--output json`.
如果调用失败，优先用 `--output json` 重新执行一次。

- Preserve the exact `skill-path`, `runtime-root`, `tool`, and argument payload in the rerun.
- 重跑时保持完全相同的 `skill-path`、`runtime-root`、`tool` 和参数载荷。
- Report the actual stderr/stdout instead of paraphrasing it.
- 汇报真实 stderr/stdout，不要只做转述。
