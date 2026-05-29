#!/usr/bin/env python3
"""Run the repository-side luaskills-debug binary for one local skill package.
运行仓库内的 luaskills-debug 二进制程序来调试单个本地 skill 包。
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path

# Define the repository or standalone package root that owns debug runtime files.
# 定义拥有调试运行文件的仓库根目录或独立包根目录。
DEBUG_WORKSPACE_ROOT = Path(__file__).resolve().parents[2]

# Define the binary name with a Windows-specific extension when needed.
# 根据平台定义调试二进制名称，在 Windows 下追加专用扩展名。
BIN_NAME = "luaskills-debug.exe" if os.name == "nt" else "luaskills-debug"


def is_repository_workspace(root: Path) -> bool:
    """Return whether one root looks like the LuaSkills source repository.
    判断单个根目录是否像 LuaSkills 源码仓库。
    """

    # Cargo.toml and scripts/ are the stable markers used by repository-side wrappers.
    # Cargo.toml 与 scripts/ 是仓库侧包装脚本使用的稳定标记。
    return (root / "Cargo.toml").exists() and (root / "scripts").is_dir()


def packaged_binary_path(root: Path) -> Path:
    """Return the expected packaged release debug binary path.
    返回预期的包内 release 调试二进制路径。
    """

    # Standalone debug packages place the binary under bin/ for direct execution.
    # 独立调试包会把二进制放在 bin/ 下以便直接执行。
    return root / "bin" / BIN_NAME


def is_standalone_debug_package(root: Path) -> bool:
    """Return whether one root looks like an extracted debug tool package.
    判断单个根目录是否像已解压的调试工具包。
    """

    # The manifest and packaged binary together identify the release debug package layout.
    # manifest 与包内二进制共同标识发布调试包目录结构。
    return (root / "debug-tool-manifest.json").exists() and packaged_binary_path(root).exists()


def parse_args(argv: list[str]) -> argparse.Namespace:
    """Parse one CLI argument list for the debug skill wrapper.
    解析调试 skill 包装脚本的一组命令行参数。
    """

    # Build the top-level parser that documents the wrapper purpose.
    # 构建顶层解析器，并说明包装脚本的用途。
    parser = argparse.ArgumentParser(
        description="Wrap the repository-side luaskills-debug binary for one local skill package."
    )
    parser.add_argument(
        "command",
        choices=("sync", "inspect", "list-tools", "call"),
        help="Debug command to forward to luaskills-debug.",
    )
    parser.add_argument(
        "--skill-path",
        help="Source LuaSkills package directory that contains skill.yaml.",
    )
    parser.add_argument(
        "--skill-id",
        help="Previously synchronized LuaSkills skill id under runtime_root/skills.",
    )
    parser.add_argument(
        "--runtime-root",
        help="Optional runtime_root override. Defaults to output/luaskills-debug-runtime/<skill_id>.",
    )
    parser.add_argument(
        "--tool",
        help="Tool name used by the call command. Accept either local or canonical name.",
    )
    parser.add_argument(
        "--args-json",
        help="Inline JSON payload forwarded to the call command.",
    )
    parser.add_argument(
        "--args-file",
        help="JSON file path forwarded to the call command.",
    )
    parser.add_argument(
        "--output",
        choices=("pretty", "json", "content"),
        help="Optional output mode forwarded to luaskills-debug.",
    )
    parser.add_argument(
        "--enable-host-result",
        action="store_true",
        help="Enable the host_result bridge for the call command.",
    )
    parser.add_argument(
        "--rebuild",
        action="store_true",
        help="Force cargo build before invoking the debug binary.",
    )

    # Parse the arguments before applying command-specific validation.
    # 在应用命令专属校验之前，先完成基础参数解析。
    args = parser.parse_args(argv)
    if args.skill_path and args.skill_id:
        parser.error("--skill-path and --skill-id are mutually exclusive")
    if args.command == "sync" and not args.skill_path:
        parser.error("--skill-path is required for the sync command")
    if args.command != "sync" and not args.skill_path and not args.skill_id:
        parser.error("--skill-path or --skill-id is required")
    if args.command == "call" and not args.tool:
        parser.error("--tool is required for the call command")
    if args.args_json and args.args_file:
        parser.error("--args-json and --args-file are mutually exclusive")
    return args


def resolve_path(raw_path: str) -> Path:
    """Resolve one user-provided path against the current working directory.
    基于当前工作目录解析单个用户提供的路径。
    """

    # Expand user syntax and preserve explicit absolute paths as-is.
    # 展开用户目录语法，并按原样保留显式绝对路径。
    candidate_path = Path(raw_path).expanduser()
    if candidate_path.is_absolute():
        return candidate_path.resolve()
    return (Path.cwd() / candidate_path).resolve()


def compute_default_runtime_root(skill_key: str) -> Path:
    """Compute the default runtime_root used by this wrapper.
    计算本包装脚本使用的默认 runtime_root。
    """

    # Reuse one stable output subdirectory per physical skill directory name.
    # 以物理 skill 目录名为单位复用稳定的输出子目录。
    if is_standalone_debug_package(DEBUG_WORKSPACE_ROOT):
        return DEBUG_WORKSPACE_ROOT / "runtime"
    return DEBUG_WORKSPACE_ROOT / "output" / "luaskills-debug-runtime" / skill_key


def ensure_binary(rebuild: bool) -> Path:
    """Build the debug binary when it is missing or when the caller forces a rebuild.
    当调试二进制缺失或调用方强制重建时构建该二进制。
    """

    # Prefer the packaged release binary when the script is running from a standalone debug package.
    # 当脚本运行在独立调试包中时，优先使用包内 release 二进制。
    package_binary_path = packaged_binary_path(DEBUG_WORKSPACE_ROOT)
    if is_standalone_debug_package(DEBUG_WORKSPACE_ROOT):
        return package_binary_path

    # Resolve the expected cargo debug binary path under the repository target directory.
    # 解析仓库 target 目录下预期的 cargo debug 二进制路径。
    binary_path = DEBUG_WORKSPACE_ROOT / "target" / "debug" / BIN_NAME
    if rebuild or not binary_path.exists():
        if not is_repository_workspace(DEBUG_WORKSPACE_ROOT):
            raise SystemExit(
                "Cannot rebuild luaskills-debug because this wrapper is not running inside a LuaSkills source repository."
            )
        # Rebuild the binary through cargo so the wrapper always uses the repository source of truth.
        # 通过 cargo 重建二进制，确保包装脚本始终使用仓库内的真实实现。
        build_command = ["cargo", "build", "--bin", "luaskills-debug"]
        completed_process = subprocess.run(build_command, cwd=DEBUG_WORKSPACE_ROOT, check=False)
        if completed_process.returncode != 0:
            raise SystemExit(completed_process.returncode)
    return binary_path


def validate_skill_path(skill_path: Path) -> None:
    """Validate that the source skill path points at a real package directory.
    校验源 skill 路径是否指向真实的 skill 包目录。
    """

    # Reject missing or non-directory paths before invoking the debug binary.
    # 在调用调试二进制之前，先拒绝缺失路径和非目录路径。
    if not skill_path.exists():
        raise SystemExit(f"Skill path does not exist: {skill_path}")
    if not skill_path.is_dir():
        raise SystemExit(f"Skill path is not a directory: {skill_path}")

    # Require the physical package to expose the standard skill.yaml manifest.
    # 要求物理 skill 包必须暴露标准的 skill.yaml 清单文件。
    manifest_path = skill_path / "skill.yaml"
    if not manifest_path.exists():
        raise SystemExit(f"skill.yaml was not found under: {skill_path}")


def build_forwarded_command(args: argparse.Namespace, binary_path: Path) -> list[str]:
    """Build the exact luaskills-debug process command forwarded by this wrapper.
    构建本包装脚本转发给 luaskills-debug 的精确进程命令。
    """

    # Resolve and validate the optional source skill directory before forwarding it.
    # 在转发可选源 skill 目录前先解析并校验它。
    skill_path = resolve_path(args.skill_path) if args.skill_path else None
    if skill_path is not None:
        validate_skill_path(skill_path)

    # SkillKey chooses the stable default runtime directory for repository-side runs.
    # SkillKey 为仓库侧运行选择稳定的默认运行时目录。
    skill_key = skill_path.name if skill_path is not None else args.skill_id

    # Resolve the runtime root, or create a stable default under the repository output directory.
    # 解析 runtime root，或在仓库 output 目录下创建稳定默认值。
    runtime_root = (
        resolve_path(args.runtime_root)
        if args.runtime_root
        else compute_default_runtime_root(skill_key)
    )

    # Start from the fixed command prefix shared by all subcommands.
    # 从所有子命令共享的固定前缀开始组装命令。
    forwarded_command = [
        str(binary_path),
        args.command,
        "--runtime-root",
        str(runtime_root),
    ]
    if skill_path is not None:
        forwarded_command.extend(["--skill-path", str(skill_path)])
    if args.skill_id:
        forwarded_command.extend(["--skill-id", args.skill_id])

    # Append command-specific invocation data only when it is relevant.
    # 仅在相关时追加命令专属的调用数据。
    if args.tool:
        forwarded_command.extend(["--tool", args.tool])
    if args.args_json:
        forwarded_command.extend(["--args-json", args.args_json])
    if args.args_file:
        forwarded_command.extend(["--args-file", str(resolve_path(args.args_file))])
    if args.output:
        forwarded_command.extend(["--output", args.output])
    if args.enable_host_result:
        forwarded_command.append("--enable-host-result")

    return forwarded_command


def main(argv: list[str] | None = None) -> int:
    """Execute the wrapper from argument parsing to subprocess exit code forwarding.
    从参数解析到子进程退出码透传，执行完整包装流程。
    """

    # Normalize the argv source so the function stays testable and reusable.
    # 规范化 argv 来源，让该函数保持可测试和可复用。
    effective_argv = argv if argv is not None else sys.argv[1:]
    parsed_args = parse_args(effective_argv)
    binary_path = ensure_binary(parsed_args.rebuild)
    forwarded_command = build_forwarded_command(parsed_args, binary_path)

    # Run the real debug binary inside the repository or package root and return its exit code unchanged.
    # 在仓库根目录或包根目录内运行真实调试二进制，并原样返回它的退出码。
    completed_process = subprocess.run(forwarded_command, cwd=DEBUG_WORKSPACE_ROOT, check=False)
    return completed_process.returncode


if __name__ == "__main__":
    sys.exit(main())
