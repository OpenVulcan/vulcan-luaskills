#!/usr/bin/env bash
set -euo pipefail

# ProjectRoot points at the repository root regardless of the caller location.
# ProjectRoot 指向仓库根目录，避免调用方当前位置影响路径解析。
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# Target selects the debug runtime dependency group.
# Target 选择调试 runtime 需要拉取的依赖分组。
TARGET="${1:-lua}"

# Database selects the optional database helper preset for all or vldb targets.
# Database 选择 all 或 vldb 目标使用的可选数据库辅助预设。
DATABASE="${2:-none}"

# RuntimeRoot receives Lua runtime packages for local debug runs.
# RuntimeRoot 接收本地调试运行所需的 Lua runtime packages。
RUNTIME_ROOT="${RUNTIME_ROOT:-output}"

mkdir -p "$RUNTIME_ROOT"
RUNTIME_ROOT="$RUNTIME_ROOT" bash "$PROJECT_ROOT/scripts/deps/fetch_deps.sh" "$TARGET" "$DATABASE"
echo "Debug runtime dependencies are ready under $RUNTIME_ROOT"
