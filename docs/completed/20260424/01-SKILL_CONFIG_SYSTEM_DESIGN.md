# Skill 配置系统设计任务计划

## 一、任务目标

围绕 `luaskills` 内建统一配置系统，输出一份可直接指导后续实现的设计方案，明确以下内容：

1. Skill 配置由 `luaskills` 自身统一实现与持久化，不再依赖宿主各自发明协议。
2. Lua 侧通过统一的 `vulcan.config.*` API 访问当前 skill 的配置。
3. 宿主侧通过单一工具形态 `runtime-config(action, skill_id?, key?, value?)` 做跨 skill 配置管理。
4. 配置值第一版统一为 `string`，不做复杂 schema、自动启停与 secret 专用存储。
5. 配置文件路径支持宿主显式覆盖；未覆盖时使用统一默认路径。

## 二、执行步骤

1. 梳理当前仓库中已有的宿主上下文、运行时上下文、system tools 与配置相关能力。
2. 明确当前 `tool_config` 与拟新增 skill 配置系统的职责边界，避免语义重叠。
3. 设计统一配置存储模型，包括文件路径、文件格式、默认行为与原子写入策略。
4. 设计 Lua 侧 `vulcan.config.*` API，明确作用域、返回值与错误行为。
5. 设计宿主侧统一工具协议 `runtime-config(action, skill_id?, key?, value?)`，明确各 action 的参数要求与返回结构。
6. 设计库内推荐的 Rust / FFI 扩展方向与测试清单，便于后续落代码。
7. 产出正式设计文档，并在计划文件中追加执行变更总结后归档。

## 三、技术选型

### 3.1 配置存储

- 第一版采用 `luaskills` 自身维护的单文件 JSON 存储。
- 文件仅维护一份主配置，不按 skill root、空间或宿主来源拆分。
- 配置值统一为字符串，由 skill 自行决定是否进一步 JSON 解码。

### 3.2 作用域模型

- 物理文件全局唯一。
- 逻辑命名空间按 `skill_id` 分组。
- Lua 侧默认只访问当前 skill 自己的配置命名空间。
- 宿主工具允许跨 skill 管理。

### 3.3 默认路径

- 宿主可通过 `skill_config_file_path` 显式指定配置文件路径。
- 未指定时，默认使用主运行时目录下的 `config/skill_config.json`。

### 3.4 非目标

- 第一版不做自动“未配置即不加载”逻辑。
- 第一版不做复杂 schema 校验。
- 第一版不做多值类型协议，统一按字符串处理。
- 第一版不做宿主自定义存储后端扩展点。

## 四、验收标准

1. 已形成一份独立的正式设计文档，能直接指导后续实现。
2. 文档明确了配置文件路径、文件格式、Lua API、宿主工具协议、错误处理与测试清单。
3. 文档明确说明不做自动启停、skill 自行处理缺配置提示。
4. 计划文件末尾已追加执行变更总结，并按规范迁移到 `docs/completed/20260424/`。

## 执行变更总结

### 1. 核心修复与调整概述

本次未直接修改运行时代码，而是先完成了 Skill 配置系统的正式设计收口，明确了统一配置存储、Lua 侧 `vulcan.config.*` 接口、宿主单工具 `runtime-config` 协议，以及默认配置文件路径与文件格式，确保后续实现阶段有稳定且一致的落地方向。

### 2. 📂文件变更清单

新增：

- `docs/SKILL_CONFIG_SYSTEM_DESIGN.md`
- `docs/plan/20260424-01-SKILL_CONFIG_SYSTEM_DESIGN.md`

删除：

- 无

修改：

- 当前计划文件已补充执行变更总结，后续将迁移到 `docs/completed/20260424/`

### 3. 💻关键代码调整详情

本轮为设计阶段，未改动 `src/` 下运行时代码、FFI 接口或示例实现。  
产出的关键技术决策如下：

- 配置协议由 `luaskills` 自身统一实现与持久化，不再依赖宿主各自发明配置协议。
- 配置文件物理上只有一个主文件，默认路径为 `<runtime_root>/config/skill_config.json`，宿主可通过 `skill_config_file_path` 显式覆盖。
- 文件内部按 `skill_id` 分组，配置值第一版统一为 `string`。
- Lua 侧只暴露 `vulcan.config.get/set/delete/has/list`，且默认只访问当前 skill 的命名空间。
- 宿主侧建议只暴露一个统一工具：`runtime-config(action, skill_id?, key?, value?)`。
- 第一版不做自动“未配置即不加载”，由 skill 自己处理缺配置提示。

### 4. ⚠️遗留问题与注意事项

- 当前仅完成设计稿，尚未开始 Rust 运行时、FFI 和 system tools 的实际实现。
- `runtime_root` 的最终代码级推导方式还需要在实现阶段结合现有 `RuntimeSkillRoot` 与宿主配置对象进一步收口。
- 第一版明确不做 schema、secret 专用安全存储与自动启停，后续如要扩展需在此设计基础上继续演进。
