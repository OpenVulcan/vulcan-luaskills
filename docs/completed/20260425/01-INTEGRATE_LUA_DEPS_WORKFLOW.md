# 任务目标

将 `vulcan-mcp-client` 仓库中的 `build-lua-deps.yml` 工作流整合到 `vulcan-luaskills` 仓库，并从 MCP 端移除该编译入口，使 Lua 通用原生依赖的 GitHub 端编译归属回 `luaskills`，并确保工作流语义、缓存路径、发布产物命名与当前仓库职责一致。

# 详细执行步骤

1. 检查 `vulcan-mcp-client` 中现有 `build-lua-deps.yml` 的触发方式、矩阵构建、缓存策略、依赖版本、平台构建步骤和 Release 上传逻辑。
2. 检查 `vulcan-luaskills` 当前仓库结构，确认是否已有 `.github/workflows`、构建脚本或相关文档，避免覆盖已有用户改动。
3. 在 `vulcan-luaskills` 中创建 `.github/workflows/build-lua-deps.yml`，迁移原工作流主体。
4. 将工作流中与 `vulcan-mcp-client` 绑定的状态目录、缓存命名或注释语义调整为 `vulcan-luaskills` / `luaskills` 归属。
5. 从 `vulcan-mcp-client` 中删除原 `.github/workflows/build-lua-deps.yml`，避免 MCP 端继续承担 luaskills 通用依赖编译职责。
6. 对迁移后的 YAML 进行静态检查，确认文件结构、关键字段、注释和路径引用完整。
7. 对照计划逐项验证，确认迁移内容完整且不会依赖 MCP 端仓库上下文。
8. 在计划文件末尾追加执行变更总结，并在完成后迁移到 `docs/completed/20260425/01-INTEGRATE_LUA_DEPS_WORKFLOW.md`。

# 技术选型

- 使用 GitHub Actions 原生 `workflow_dispatch` 保持手动发布型依赖构建流程。
- 继续沿用原工作流的多平台矩阵与自托管 runner 支持，降低迁移风险。
- 继续使用 `actions/cache@v4` 和本地 runner 状态目录加速重复构建。
- 使用仓库内 `.github/workflows/build-lua-deps.yml` 作为 GitHub 端唯一入口。

# 验收标准

1. `vulcan-luaskills` 仓库新增 `.github/workflows/build-lua-deps.yml`。
2. 新工作流保留原有 Linux、macOS、Windows 平台的 Lua C 依赖构建能力。
3. 工作流中的本地缓存状态目录不再使用 `vulcan-mcp` 作为归属命名。
4. `vulcan-mcp-client` 仓库不再保留该 GitHub Actions 编译入口。
5. 工作流可通过 YAML 基础解析或等效静态检查。
6. 计划文件包含完整执行变更总结，并按规范迁移到 `docs/completed/20260425/`。

# 执行变更总结

## 1. 核心修复与调整概述

- 已将 `vulcan-mcp-client` 的 `build-lua-deps.yml` 迁入 `vulcan-luaskills` 的 `.github/workflows/` 目录。
- 已将自托管 runner 的本地 Lua 依赖缓存目录从 `.cache/vulcan-mcp/lua-deps` 调整为 `.cache/vulcan-luaskills/lua-deps`。
- 已将 Windows 便携 Perl 工具缓存目录从 `.cache/vulcan-mcp/tools/perl` 调整为 `.cache/vulcan-luaskills/tools/perl`。
- 已从 `vulcan-mcp-client` 中删除原 GitHub Actions 编译入口，避免 MCP 端继续承担 luaskills 通用依赖编译职责。

## 2. 📂文件变更清单

- 新增：`D:\projects\vulcan-luaskills\.github\workflows\build-lua-deps.yml`
- 修改：`D:\projects\vulcan-luaskills\docs\plan\20260425-01-INTEGRATE_LUA_DEPS_WORKFLOW.md`
- 删除：`D:\projects\vulcan-mcp-client\.github\workflows\build-lua-deps.yml`

## 3. 💻关键代码调整详情

- 新增的 luaskills 工作流保留了原有 `workflow_dispatch` 手动触发入口、平台 runner 输入、构建矩阵、Linux/macOS/Windows 三端依赖构建流程、产物打包与 GitHub Release 上传逻辑。
- 将 Windows 与 Unix 的本地状态恢复、单依赖持久化、最终依赖持久化路径全部切换为 `vulcan-luaskills` 命名空间。
- 将 Windows Perl 工具缓存路径切换为 `vulcan-luaskills` 命名空间，避免迁移后仍复用 MCP 端工具目录语义。
- 使用 Python YAML 解析完成静态校验，确认迁入后的 workflow 能被解析且包含 `jobs` 结构。

## 4. ⚠️遗留问题与注意事项

- 本次仅做静态 YAML 校验，未实际触发 GitHub Actions 多平台构建；真实依赖编译仍需在 GitHub 手动运行 `Build Lua C Dependencies` 工作流验证 runner、网络和 Release 权限。
- `vulcan-luaskills` 与 `vulcan-mcp-client` 分属两个 Git 仓库，本次变更需要分别提交。
