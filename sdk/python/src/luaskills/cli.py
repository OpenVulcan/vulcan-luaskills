"""
Command-line interface for the Python LuaSkills SDK.
Python LuaSkills SDK 的命令行接口。
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

from .client import LuaSkillsClient
from .roots import RuntimeRoots
from .types import Authority, RuntimeSkillRoot


def main(argv: list[str] | None = None) -> None:
    """
    Dispatch one CLI command and print JSON output.
    分发单个 CLI 命令并输出 JSON。
    """

    parser = build_parser()
    args = parser.parse_args(normalize_global_args(argv if argv is not None else sys.argv[1:]))
    if args.command == "version":
        print_json(LuaSkillsClient.version(library_path=args.lib))
        return
    if args.command == "describe":
        print_json(LuaSkillsClient.describe(library_path=args.lib))
        return

    runtime_root = Path(args.runtime_root).expanduser().resolve()
    skill_roots = build_roots(args, runtime_root)
    RuntimeRoots.ensure_layout(runtime_root, skill_roots)
    with LuaSkillsClient(library_path=args.lib, runtime_root=runtime_root, ensure_runtime_layout=False) as client:
        if args.command != "load":
            client.load_from_roots(skill_roots)
        dispatch_engine_command(client, skill_roots, args)


def build_parser() -> argparse.ArgumentParser:
    """
    Build the top-level command-line parser.
    构造顶层命令行解析器。
    """

    parser = argparse.ArgumentParser(prog="luaskills")
    parser.add_argument("--lib", help="LuaSkills dynamic library path")
    parser.add_argument("--runtime-root", default=str(Path.cwd() / "luaskills-runtime"))
    parser.add_argument("--authority", default=Authority.DELEGATED_TOOL.value, choices=[Authority.SYSTEM.value, Authority.DELEGATED_TOOL.value])
    parser.add_argument("--root-only", action="store_true")
    parser.add_argument("--no-project", action="store_true")
    parser.add_argument("--no-user", action="store_true")
    parser.add_argument("--root-skills")
    parser.add_argument("--project-skills")
    parser.add_argument("--user-skills")
    subparsers = parser.add_subparsers(dest="command", required=True)

    for command in ["version", "describe", "load", "reload", "list", "help-list"]:
        subparsers.add_parser(command)

    one_value_commands = ["help-detail", "is-skill", "skill-name", "enable", "disable", "system-enable", "system-disable"]
    for command in one_value_commands:
        subparser = subparsers.add_parser(command)
        subparser.add_argument("first")
        subparser.add_argument("second", nargs="?")

    completion_parser = subparsers.add_parser("prompt-completions")
    completion_parser.add_argument("prompt_name")
    completion_parser.add_argument("argument_name")

    call_parser = subparsers.add_parser("call")
    call_parser.add_argument("tool_name")
    call_parser.add_argument("args_json", nargs="?", default="{}")

    run_lua_parser = subparsers.add_parser("run-lua")
    run_lua_parser.add_argument("code")
    run_lua_parser.add_argument("args_json", nargs="?", default="{}")

    config_parser = subparsers.add_parser("config")
    config_parser.add_argument("action", choices=["list", "get", "set", "delete"])
    config_parser.add_argument("values", nargs="*")

    for command in ["install", "update", "system-install", "system-update"]:
        subparser = subparsers.add_parser(command)
        subparser.add_argument("source")
        subparser.add_argument("--skill-id")
        subparser.add_argument("--source-type", default="github")
        subparser.add_argument("--target-root")

    for command in ["uninstall", "system-uninstall"]:
        subparser = subparsers.add_parser(command)
        subparser.add_argument("skill_id")
        subparser.add_argument("--target-root")
        subparser.add_argument("--remove-sqlite", action="store_true")
        subparser.add_argument("--remove-lancedb", action="store_true")

    return parser


def normalize_global_args(raw_args: list[str]) -> list[str]:
    """
    Move recognized global options before the subcommand so users may place them anywhere.
    将已识别的全局选项移动到子命令之前，使用户可把它们放在任意位置。
    """

    value_options = {
        "--lib",
        "--runtime-root",
        "--authority",
        "--root-skills",
        "--project-skills",
        "--user-skills",
    }
    boolean_options = {"--root-only", "--no-project", "--no-user"}
    global_args: list[str] = []
    command_args: list[str] = []
    index = 0
    while index < len(raw_args):
        value = raw_args[index]
        name = value.split("=", 1)[0]
        if name in boolean_options:
            global_args.append(value)
        elif name in value_options:
            global_args.append(value)
            if "=" not in value:
                if index + 1 >= len(raw_args):
                    raise ValueError(f"{value} requires a value")
                global_args.append(raw_args[index + 1])
                index += 1
        else:
            command_args.append(value)
        index += 1
    return global_args + command_args


def dispatch_engine_command(client: LuaSkillsClient, skill_roots: list[RuntimeSkillRoot], args: argparse.Namespace) -> None:
    """
    Dispatch one command that requires an engine handle.
    分发一个需要引擎句柄的命令。
    """

    authority = args.authority
    if args.command == "load":
        print_json(client.load_from_roots(skill_roots))
    elif args.command == "reload":
        print_json(client.reload_from_roots(skill_roots))
    elif args.command == "list":
        print_json(client.list_entries(authority))
    elif args.command == "help-list":
        print_json(client.list_skill_help(authority))
    elif args.command == "help-detail":
        print_json(client.render_skill_help_detail(args.first, args.second or "main", authority=authority))
    elif args.command == "is-skill":
        print_json({"value": client.is_skill(args.first, authority)})
    elif args.command == "skill-name":
        print_json({"skill_id": client.skill_name_for_tool(args.first, authority)})
    elif args.command == "prompt-completions":
        print_json(client.prompt_argument_completions(args.prompt_name, args.argument_name, authority))
    elif args.command == "call":
        print_json(client.call_skill(args.tool_name, json.loads(args.args_json)))
    elif args.command == "run-lua":
        print_json(client.run_lua(args.code, json.loads(args.args_json)))
    elif args.command == "config":
        dispatch_config_command(client, args)
    elif args.command == "enable":
        print_json(client.skills.enable(skill_roots, args.first))
    elif args.command == "disable":
        print_json(client.skills.disable(skill_roots, args.first, args.second))
    elif args.command == "install":
        print_json(client.skills.install(skill_roots, install_request(args), target_root=target_root(args, skill_roots)))
    elif args.command == "update":
        print_json(client.skills.update(skill_roots, install_request(args), target_root=target_root(args, skill_roots)))
    elif args.command == "uninstall":
        print_json(client.skills.uninstall(skill_roots, args.skill_id, options=uninstall_options(args), target_root=target_root(args, skill_roots)))
    elif args.command == "system-enable":
        print_json(client.system(authority).enable(skill_roots, args.first))
    elif args.command == "system-disable":
        print_json(client.system(authority).disable(skill_roots, args.first, args.second))
    elif args.command == "system-install":
        print_json(client.system(authority).install(skill_roots, install_request(args), target_root=target_root(args, skill_roots)))
    elif args.command == "system-update":
        print_json(client.system(authority).update(skill_roots, install_request(args), target_root=target_root(args, skill_roots)))
    elif args.command == "system-uninstall":
        print_json(client.system(authority).uninstall(skill_roots, args.skill_id, options=uninstall_options(args), target_root=target_root(args, skill_roots)))
    else:
        raise ValueError(f"unknown command: {args.command}")


def dispatch_config_command(client: LuaSkillsClient, args: argparse.Namespace) -> None:
    """
    Dispatch one skill-config subcommand.
    分发单个 skill-config 子命令。
    """

    values = args.values
    if args.action == "list":
        require_config_value_count(args.action, values)
        print_json(client.config.list(values[0] if values else None))
    elif args.action == "get":
        require_config_value_count(args.action, values)
        print_json(client.config.get(values[0], values[1]))
    elif args.action == "set":
        require_config_value_count(args.action, values)
        print_json(client.config.set(values[0], values[1], values[2]))
    elif args.action == "delete":
        require_config_value_count(args.action, values)
        print_json(client.config.delete(values[0], values[1]))


def require_config_value_count(action: str, values: list[str]) -> None:
    """
    Validate the positional value count for one skill-config CLI action.
    校验单个 skill-config CLI 动作的位置参数数量。
    """

    expected_ranges = {
        "list": (0, 1, "config list [skill-id]"),
        "get": (2, 2, "config get <skill-id> <key>"),
        "set": (3, 3, "config set <skill-id> <key> <value>"),
        "delete": (2, 2, "config delete <skill-id> <key>"),
    }
    minimum, maximum, usage = expected_ranges[action]
    if minimum <= len(values) <= maximum:
        return
    raise ValueError(usage)


def build_roots(args: argparse.Namespace, runtime_root: Path) -> list[RuntimeSkillRoot]:
    """
    Build the formal root chain from CLI flags.
    从 CLI 标志构造正式 root 链。
    """

    roots = RuntimeRoots.standard(
        runtime_root,
        include_project=not args.no_project and not args.root_only,
        include_user=not args.no_user and not args.root_only,
    )
    replacements = {
        "ROOT": args.root_skills,
        "PROJECT": args.project_skills,
        "USER": args.user_skills,
    }
    return [RuntimeSkillRoot(root.name, str(Path(replacements.get(root.name) or root.skills_dir).expanduser().resolve()).replace("\\", "/")) for root in roots]


def target_root(args: argparse.Namespace, skill_roots: list[RuntimeSkillRoot]) -> RuntimeSkillRoot | None:
    """
    Resolve the optional target root requested by lifecycle flags.
    解析生命周期标志请求的可选目标 root。
    """

    label = getattr(args, "target_root", None)
    return RuntimeRoots.find_by_label(skill_roots, label) if label else None


def install_request(args: argparse.Namespace) -> dict[str, Any]:
    """
    Build one install or update request from CLI arguments.
    从 CLI 参数构造单个安装或更新请求。
    """

    return {
        "skill_id": args.skill_id,
        "source": args.source,
        "source_type": args.source_type,
    }


def uninstall_options(args: argparse.Namespace) -> dict[str, bool]:
    """
    Build uninstall cleanup options from CLI flags.
    从 CLI 标志构造卸载清理选项。
    """

    return {
        "remove_sqlite": args.remove_sqlite,
        "remove_lancedb": args.remove_lancedb,
    }


def print_json(value: Any) -> None:
    """
    Print one value as pretty JSON.
    将单个值以美化 JSON 输出。
    """

    print(json.dumps(value, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
