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
python luaskills-debug-skill/scripts/run_debug.py inspect --skill-path D:\path\to\skill
python luaskills-debug-skill/scripts/run_debug.py list-tools --skill-path D:\path\to\skill --output content
python luaskills-debug-skill/scripts/run_debug.py call --skill-path D:\path\to\skill --tool ping --args-json "{\"note\":\"hello\"}" --output json
```

The wrapper defaults `runtime_root` to `D:\projects\vulcan-luaskills\output\luaskills-debug-runtime\<skill_id>` when you do not pass `--runtime-root`.
当你不传 `--runtime-root` 时，包装脚本会默认把 `runtime_root` 设为 `D:\projects\vulcan-luaskills\output\luaskills-debug-runtime\<skill_id>`。

## Workflow

Start with `inspect`.
先从 `inspect` 开始。

- Use `inspect` to confirm the physical skill directory, effective `skill_id`, synchronized target path, and loaded entries.
- 用 `inspect` 确认物理 skill 目录、生效 `skill_id`、同步后的目标路径和已加载入口。

Then run `list-tools`.
然后运行 `list-tools`。

- Use `list-tools` to see local tool names and canonical tool names before calling anything.
- 用 `list-tools` 在真正调用之前查看 local tool name 和 canonical tool name。

Finally run `call`.
最后运行 `call`。

- Use `call` with `--output json` when you need full `content / overflow_mode / template_hint / host_result`.
- 当你需要完整的 `content / overflow_mode / template_hint / host_result` 时，用 `call --output json`。
- Use `call --output content` only when you only care about the primary text result.
- 只有在你只关心主文本结果时，才使用 `call --output content`。

## Rules

Pass the source package directory to `--skill-path`.
把源 skill 包目录传给 `--skill-path`。

- Point `--skill-path` at the actual skill package directory that contains `skill.yaml`.
- `--skill-path` 必须指向真实 skill 包目录，也就是里面直接包含 `skill.yaml` 的目录。
- Do not point `--skill-path` at `runtime_root/skills/...` unless that is the source package you really want to debug.
- 不要把 `--skill-path` 指到 `runtime_root/skills/...`，除非那就是你真正要调试的源 skill 包。

Keep `runtime_root` real.
保持 `runtime_root` 的真实语义。

- This skill reuses the real `luaskills-debug` bin, which first syncs the target skill into `runtime_root/skills/<skill_id>` and then runs the normal `load_from_roots -> call_skill` path.
- 这个 skill 复用了真实的 `luaskills-debug` bin，它会先把目标 skill 同步到 `runtime_root/skills/<skill_id>`，再走正式的 `load_from_roots -> call_skill` 链路。
- Use an explicit `--runtime-root` when you need a fixed state/database/dependency layout across repeated runs.
- 如果你需要在多次调试之间固定 state/database/dependency 目录布局，请显式传入 `--runtime-root`。

Use `--enable-host-result` only when needed.
只在确实需要时开启 `--enable-host-result`。

- Add `--enable-host-result` when debugging `host_result` and `change_set`.
- 调试 `host_result` 和 `change_set` 时再加 `--enable-host-result`。
- Leave it off for ordinary content-only calls.
- 普通纯文本调用时保持关闭即可。

Avoid parallel runs against the same default runtime root.
避免针对同一个默认 runtime root 并发运行。

- Do not run `inspect`, `list-tools`, and `call` in parallel against the same skill when they share the same default `runtime_root`.
- 当 `inspect`、`list-tools` 和 `call` 共享同一个默认 `runtime_root` 时，不要并发执行它们。
- Run them serially, or pass different explicit `--runtime-root` values when you need isolated parallel checks.
- 请串行执行，或者在需要隔离的并发检查时传入不同的显式 `--runtime-root`。

## Script

Use `scripts/run_debug.py` as the primary entrypoint.
把 `scripts/run_debug.py` 当作主要入口。

- The script builds `luaskills-debug` with `cargo build --bin luaskills-debug` when the local debug binary is missing.
- 当本地调试二进制缺失时，脚本会自动执行 `cargo build --bin luaskills-debug` 进行构建。
- The script forwards `inspect`, `list-tools`, and `call` to the real bin without introducing any SDK or FFI layer.
- 这个脚本会把 `inspect`、`list-tools` 和 `call` 原样转发给真实 bin，不会额外引入 SDK 或 FFI 层。
- Add `--rebuild` when you know the debug bin source changed and you want a fresh build first.
- 当你知道调试 bin 源码已经变更并希望先重新构建时，追加 `--rebuild`。

## Decision Guide

Choose the command by the debugging goal.
根据调试目标选择命令。

- Need package identity, loaded entries, or synchronized path: use `inspect`.
- 需要看包身份、已加载入口或同步路径：用 `inspect`。
- Need local/canonical tool names: use `list-tools`.
- 需要看 local/canonical tool name：用 `list-tools`。
- Need actual execution output: use `call`.
- 需要看真实执行结果：用 `call`。

If a call fails, rerun with `--output json`.
如果调用失败，优先用 `--output json` 重新执行一次。

- Preserve the exact `skill-path`, `runtime-root`, `tool`, and argument payload in the rerun.
- 重跑时保持完全相同的 `skill-path`、`runtime-root`、`tool` 和参数载荷。
- Report the actual stderr/stdout instead of paraphrasing it.
- 汇报真实 stderr/stdout，不要只做转述。
