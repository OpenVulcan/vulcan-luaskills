## 任务目标

整理当前仓库中实际暴露给 Lua Skill 的 `vulcan.*` 能力面，基于真实实现补齐或新增一份面向 Skill 开发者的手册文档，确保文档内容与当前代码行为一致，能够指导开发者完成日常 Skill 编写、调试与能力接入。

## 执行步骤

1. 盘点当前运行时中实际注入到 Lua 的 `vulcan` 顶级表及其子表能力。
2. 对照现有文档，确认是否已有可复用章节与需要修订的旧描述。
3. 输出或更新一份面向 Skill 开发者的手册，覆盖上下文、运行时辅助、文件系统、路径、进程、数据库与调用约束。
4. 结合当前实现补充必要示例，明确哪些能力依赖宿主注入、哪些属于默认可用。
5. 进行自检，确认文档条目与当前代码实现一致，并记录执行变更总结。

## 技术选型

- 以 `src/runtime/engine.rs` 中的实际 Lua 注入逻辑作为唯一事实来源。
- 以现有仓库文档为辅助参考，仅在与代码一致时复用。
- 文档采用中文编写，强调 Skill 开发者视角与可执行示例。

## 验收标准

- 明确列出当前支持的 `vulcan.*` 顶级能力及关键子项。
- 文档能回答 Skill 开发者“有什么、怎么用、什么条件下可用、常见限制是什么”。
- 文档内容不包含已经失效或代码中不存在的 API。
- 完成后补充执行变更总结，并将计划文件迁移到 `docs/completed/20260423/`。

---

## 执行变更总结

### 1. 核心修复与调整概述

- 新增了一份面向 Lua Skill 作者的独立手册，系统整理当前真实暴露的 `vulcan.*` 能力面。
- 手册内容基于当前 `src/runtime/engine.rs` 的实现逐项校对，覆盖默认能力、宿主条件注入能力、内部字段边界与常见限制。
- README 顶部补充了 Skill 开发者入口，避免继续把 Skill 作者引导到 FFI/宿主文档。

### 2. 📂 文件变更清单

#### 新增

- `docs/SKILL_DEVELOPER_MANUAL.md`

#### 修改

- `README.md`
- `docs/plan/20260423-01-SKILL_DEVELOPER_MANUAL.md`

### 3. 💻 关键代码调整详情

- 本次未修改运行时代码逻辑，重点是基于现有实现整理文档真相层。
- 手册中已明确列出当前支持的：
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
- 同时补充了 `luaexec_call`、`runtime.lua.exec` 限制、缓存与日志在隔离执行环境中的行为差异。

### 4. ⚠️ 遗留问题与注意事项

- 当前手册主要覆盖 Skill 作者关心的 Lua 能力面，不替代 FFI 或宿主集成文档。
- `vulcan.runtime.internal.*` 与 `vulcan.__sqlite_skill_name` / `vulcan.__lancedb_skill_name` 仍然属于内部实现细节，文档里已标注不建议依赖。
- 当前工作区还存在本轮之外的既有未提交改动，例如 `Cargo.toml`、`Cargo.lock`、`src/host/controller.rs`、`src/runtime/engine.rs` 等，本次未覆盖或回退这些内容。
- 自检结果：`cargo test -q` 通过，当前 48 个测试全绿。
