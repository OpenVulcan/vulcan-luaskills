# Skill 配置系统设计稿

## 1. 背景

当前 `luaskills` 已有：

- 宿主请求级 `vulcan.context.tool_config`
- 宿主强制忽略 `ignored_skill_ids`
- 宿主可选的 `vulcan.runtime.skills.*` 管理桥接
- 固定三层 skill root 语义：`ROOT -> PROJECT -> USER`

但还没有一套**统一的、可持久化的、面向 skill 自身配置**的正式协议。

这会带来几个问题：

1. Skill 作者无法用统一方式告诉宿主“我需要哪些配置”。
2. 宿主如果自己发明配置协议，未来会形成碎片化实现。
3. 用户让 AI 帮忙配置 skill 时，缺少统一工具入口。
4. Skill 若自行读写配置文件，会引入路径、权限与生态不一致问题。

因此需要由 `luaskills` 本身定义统一配置协议，并由宿主消费这套协议。

Skill root 的正式对外层级为 `ROOT -> PROJECT -> USER`，启动或加载时必须传入 `ROOT` root，但配置系统仍保持单一主配置文件，不按层级拆分。`ROOT` 是系统控制级，普通 `vulcan.runtime.skills.*` 管理面只应操作实际存在的 `PROJECT` / `USER`。

## 2. 设计目标

本设计的目标是：

1. 由 `luaskills` 统一实现 Skill 配置的存储与读写协议。
2. Lua 侧提供统一的 `vulcan.config.*` API。
3. 宿主侧只需要暴露一个统一工具：
   - `runtime-config(action, skill_id?, key?, value?)`
4. 配置值第一版统一为 `string`，降低宿主与 FFI 复杂度。
5. 配置文件物理上只有一份主文件，不按 skill root 拆分。

## 3. 非目标

第一版明确**不做**以下内容：

1. 不做“缺配置则不加载 skill”的自动逻辑。
2. 不做复杂 schema 校验或自动表单生成。
3. 不做多值类型协议，统一按字符串处理。
4. 不做 secret 专用密钥库或宿主专属安全后端。
5. 不做 Lua 侧跨 skill 直接读取其它 skill 配置。

缺配置时，由 skill 自己决定如何提示 AI / 用户。例如：

- 提示当前未配置某个 key
- 提示请使用 `runtime-config` 工具设置该 key
- 告知秘钥可从哪里获取

## 4. 总体方案

### 4.1 统一原则

- 物理存储：一份主配置文件
- 逻辑隔离：按 `skill_id` 分组
- Lua 访问：默认只访问当前 skill 的配置
- 宿主管理：通过统一工具跨 skill 管理

### 4.2 数据模型

第一版统一使用：

- `skill_id: string`
- `key: string`
- `value: string`

其中：

- `skill_id` 由运行时自动作为命名空间使用
- `key` 由 skill 自己定义，例如 `api_token`、`endpoint`、`model`
- `value` 一律为字符串

如果某个 skill 需要结构化内容，可以自行存储 JSON 字符串，再由 skill 自己解码。

## 5. 配置文件路径

### 5.1 宿主显式路径

宿主可通过一个新的宿主配置字段显式指定：

- `skill_config_file_path: Option<PathBuf>`

当该字段存在时，运行时直接使用该路径，并在引擎创建时固定为绝对路径。

### 5.2 默认路径

宿主未传入 `skill_config_file_path` 时，运行时使用默认路径：

```text
<runtime_root>/config/skill_config.json
```

这里的 `runtime_root` 指当前主运行时目录。  
当前设计不按多 skill root 拆分配置，也不为每个 root 单独派生配置文件。
即使宿主使用 `ROOT -> PROJECT -> USER` 三层 skill root，配置文件也仍然只有一份主文件。

### 5.3 文件创建策略

- 文件不存在时，视为空配置。
- 首次 `set` 时自动创建父目录和配置文件。
- 写入必须使用**原子替换**策略：
  1. 先写 `skill_config.json.tmp`
  2. 再 rename 覆盖 `skill_config.json`

## 6. 文件格式

建议使用按 `skill_id` 分组的对象格式：

```json
{
  "skills": {
    "vulcan-codekit": {
      "api_token": "sk-xxx",
      "endpoint": "https://api.example.com"
    },
    "grpc-memory": {
      "base_url": "http://127.0.0.1:18080",
      "provider": "grpc"
    }
  }
}
```

这样有几个优点：

1. 人眼可读
2. 手工编辑方便
3. 删除某个 skill 的配置简单
4. 后续扩展元数据时容易保持结构稳定

## 7. Lua 侧 API 设计

### 7.1 顶级入口

新增：

- `vulcan.config.get(key)`
- `vulcan.config.set(key, value)`
- `vulcan.config.delete(key)`
- `vulcan.config.has(key)`
- `vulcan.config.list()`

### 7.2 作用域规则

Lua 侧 API 默认只操作**当前 skill 的配置命名空间**。

也就是说，skill 中这样写：

```lua
local api_token = vulcan.config.get("api_token")
```

运行时实际访问的是：

- `skill_id = 当前 skill 的 skill_id`
- `key = "api_token"`

Skill 不需要自己把 `skill_id` 编进 key。

### 7.3 返回规则

#### `vulcan.config.get(key)`

- 有值：返回字符串
- 无值：返回 `nil`

#### `vulcan.config.has(key)`

- 存在：返回 `true`
- 不存在：返回 `false`

#### `vulcan.config.set(key, value)`

- 成功：返回 `true`
- 失败：抛出 runtime error

#### `vulcan.config.delete(key)`

- 删除成功或目标不存在：返回布尔值

#### `vulcan.config.list()`

建议返回一个 Lua table，对应当前 skill 下的所有键值：

```lua
{
  api_token = "sk-xxx",
  endpoint = "https://api.example.com"
}
```

### 7.4 错误规则

以下情况应返回明确错误：

1. 当前调用没有 skill 上下文
   - 例如某些纯 runtime 场景或无 skill 归属的执行路径
2. `key` 为空字符串
3. `value` 不是字符串
4. 配置文件损坏或写入失败

### 7.5 缺配置处理

第一版不做自动禁用逻辑。  
Skill 应自行判断缺配置场景，并返回面向 AI / 用户的明确提示。

示例：

```lua
local api_token = vulcan.config.get("api_token")
if not api_token or api_token == "" then
    return "当前未配置 api_token。请使用 runtime-config 工具为当前 skill 设置 api_token。秘钥可通过 xxx 获取。"
end
```

## 8. 宿主工具设计

### 8.1 单工具原则

宿主建议只暴露一个工具：

```text
runtime-config(action, skill_id?, key?, value?)
```

这样最省上下文，也更方便 AI 帮用户完成配置。

### 8.2 支持的 action

第一版只支持：

- `list`
- `get`
- `set`
- `delete`

### 8.3 参数规则

#### `list`

- `skill_id` 可选
- `key` 不传
- `value` 不传

用途：

- 列出所有 skill 配置
- 或只列某个 skill 的所有配置

#### `get`

- `skill_id` 必填
- `key` 必填
- `value` 不传

#### `set`

- `skill_id` 必填
- `key` 必填
- `value` 必填

说明：

- `set` 同时承担新增与更新语义

#### `delete`

- `skill_id` 必填
- `key` 必填
- `value` 不传

### 8.4 返回结构

建议宿主工具的 `list` 返回展平数组：

```json
[
  {
    "skill_id": "vulcan-codekit",
    "key": "api_token",
    "value": "sk-xxx"
  },
  {
    "skill_id": "grpc-memory",
    "key": "base_url",
    "value": "http://127.0.0.1:18080"
  }
]
```

`get` 返回建议：

```json
{
  "found": true,
  "skill_id": "vulcan-codekit",
  "key": "api_token",
  "value": "sk-xxx"
}
```

`set/delete` 返回建议：

```json
{
  "ok": true,
  "action": "set",
  "skill_id": "vulcan-codekit",
  "key": "api_token",
  "value": "sk-xxx"
}
```

## 9. 库内实现建议

### 9.1 存储组件

建议新增一个内部组件，例如：

- `SkillConfigStore`

职责包括：

1. 解析配置文件路径
2. 读取与解析 `skill_config.json`
3. 执行 `get/set/delete/list`
4. 负责原子写回
5. 对外屏蔽底层文件结构

### 9.2 运行时集成

建议把该存储组件放到 `LuaEngine` 生命周期内统一持有，避免每次调用都重新解析路径与 JSON。

同时在运行时内部保留当前 skill 上下文，使 `vulcan.config.*` 能自动解析出当前 `skill_id`。

### 9.3 宿主暴露方式

建议在库内部同时提供两层能力：

1. Rust API
2. FFI API

宿主再把它们映射成一个外部工具 `runtime-config`。

## 10. 与现有能力的边界

### 10.1 与 `vulcan.context.tool_config` 的区别

- `tool_config`：
  - 请求级
  - 宿主临时注入
  - 偏当前工具调用上下文
- `vulcan.config.*`：
  - skill 级
  - 持久化
  - 由 `luaskills` 自己统一管理

二者不能混用。

### 10.2 与 `ignored_skill_ids` 的区别

- `ignored_skill_ids` 是宿主强制忽略
- `vulcan.config.*` 是 skill 自己消费配置

第一版不通过配置系统直接驱动 skill 是否加载。

## 11. 测试建议

建议至少覆盖以下测试：

1. 宿主显式传入 `skill_config_file_path` 时，实际读写落在指定路径。
2. 未传路径时，默认落在 `<runtime_root>/config/skill_config.json`。
3. `set` 后 `get` 能读回，且重建引擎后仍然持久存在。
4. `delete` 后目标 key 消失。
5. `list` 能正确按 `skill_id` 展平返回。
6. Lua 侧默认只能读取当前 skill 的配置。
7. 无 skill 上下文时调用 `vulcan.config.*` 返回明确错误。
8. 配置文件不存在时视为空配置，不报错。
9. 配置文件损坏时返回清晰错误。
10. 原子写入流程不会留下半写文件。

## 12. 第一版推荐范围

为了让范围可控，我建议第一版只做这些：

1. 单文件配置存储
2. 字符串值
3. `vulcan.config.*`
4. 宿主单工具 `runtime-config`
5. Rust / FFI 统一能力

以下内容留到后续版本：

1. schema 声明
2. 自动配置 UI
3. secret 专用安全存储
4. 自动启停
5. 跨 skill Lua 配置访问

## 13. 结论

本设计的核心结论是：

1. 配置协议应由 `luaskills` 统一实现，而不是让宿主各自发明。
2. 物理配置文件只有一份主文件，不按 skill root 拆分。
3. 逻辑命名空间按 `skill_id` 分组。
4. Skill 通过 `vulcan.config.*` 访问自己的配置。
5. 宿主通过单工具 `runtime-config(action, skill_id?, key?, value?)` 做统一管理。
6. 第一版配置值统一使用字符串，降低复杂度。
