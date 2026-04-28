# @luaskills/sdk

TypeScript / Node.js SDK，用于通过公共 `_json` FFI 集成 LuaSkills 运行时。

这个包的目标是把动态库加载、FFI buffer、JSON 包络、engine 生命周期、root 链、authority、skill-config、普通管理面与 system 管理面都收进 SDK 内部。宿主不需要再手写 `FfiBorrowedBuffer` / `FfiOwnedBuffer` 或逐个拆解 JSON FFI 响应。

## 安装

```bash
npm install @luaskills/sdk
```

本包当前不内置 `luaskills` 原生动态库。调用时需要通过 `libraryPath` 或 `LUASKILLS_LIB` 指向已构建或已发布包中的动态库：

```bash
set LUASKILLS_LIB=D:\path\to\luaskills.dll
```

Linux / macOS 使用对应的 `.so` / `.dylib` 路径。

## 基础用法

```ts
import { Authority, LuaSkillsClient, RuntimeRoots } from "@luaskills/sdk";

const runtimeRoot = "D:/runtime/luaskills";
const roots = RuntimeRoots.standard(runtimeRoot);

const client = LuaSkillsClient.create({
  libraryPath: "D:/path/to/luaskills.dll",
  runtimeRoot,
});

try {
  client.loadFromRoots(roots);

  const entries = client.listEntries(Authority.DelegatedTool);
  const result = client.callSkill("demo-standard-ffi-skill-ping", {
    note: "typescript-sdk",
  });

  console.log(entries);
  console.log(result.content);
} finally {
  client.close();
}
```

## CLI / npx

包内提供 `luaskills` bin，可用于 `npx` 或全局安装后的命令行集成：

```bash
npx @luaskills/sdk version --lib D:\path\to\luaskills.dll
npx @luaskills/sdk list --lib D:\path\to\luaskills.dll --runtime-root D:\runtime\luaskills
npx @luaskills/sdk call demo-standard-ffi-skill-ping "{\"note\":\"npx\"}" --lib D:\path\to\luaskills.dll
```

常用管理命令：

```bash
npx @luaskills/sdk install LuaSkills/luaskills-demo-skill --target-root USER
npx @luaskills/sdk update LuaSkills/luaskills-demo-skill --target-root USER
npx @luaskills/sdk uninstall luaskills-demo-skill --target-root USER
```

system 入口必须显式理解为宿主注入权限。真正的系统管理面使用：

```bash
npx @luaskills/sdk system-install LuaSkills/luaskills-demo-skill --target-root ROOT --authority system
```

如果 system 工具被封装给普通 tools，应固定传入：

```bash
--authority delegated_tool
```

## JSON Provider Callback

SQLite / LanceDB 的 `host_callback + json` 模式可以直接通过 SDK 注册，宿主无需手写 `FfiOwnedBuffer`：

```ts
import { LuaSkillsClient, LuaSkillsJsonFfi } from "@luaskills/sdk";

const ffi = new LuaSkillsJsonFfi({ libraryPath: "D:/path/to/luaskills.dll" });

ffi.setSqliteProviderJsonCallback((request) => {
  return { ok: true, request };
});

try {
  const client = LuaSkillsClient.create({
    libraryPath: "D:/path/to/luaskills.dll",
    runtimeRoot: "D:/runtime/luaskills",
    hostOptions: {
      sqlite_provider_mode: "host_callback",
      sqlite_callback_mode: "json",
    },
  });
  client.close();
} finally {
  ffi.clearSqliteProviderJsonCallback();
}
```

callback 必须在 `engine_new` 前注册；engine 创建后再切换 callback 不会 retroactive 影响已存在 engine。完整示例见 `examples/provider-callback.mjs`。

## Root 与 Authority 规则

SDK 默认使用正式三层 root 语义：

```text
ROOT    = 系统保护层
PROJECT = 项目普通层
USER    = 用户普通层
```

`RuntimeRoots.standard(runtimeRoot)` 会生成：

```text
runtimeRoot/root_skills
runtimeRoot/project_skills
runtimeRoot/user_skills
```

查询类接口默认使用 `Authority.DelegatedTool`，因此看不到 `ROOT` skills：

```ts
client.listEntries();
client.listSkillHelp();
client.isSkill("some-root-tool");
```

`Authority.System` 只表示宿主允许管理 ROOT 层，不表示可以绕过 ROOT 同名占用规则。只要 ROOT 中已有某个 `skill_id`，无论 system 还是 delegated，都不应在 PROJECT / USER 主动 install 或 update 同名 skill。普通层 uninstall 仍可用于清理同名残留。

## 调用面不是可见性过滤面

`callSkill` 与 `runLua` 是运行时执行面，面向“已经激活的 skill”。它们不承担 `DelegatedTool` 查询可见性过滤职责。

如果宿主不希望普通用户执行任意 Lua，应该不要把 `runLua` 直接暴露给普通用户；如果宿主需要限制可调用 tool，也应该在宿主自己的工具封装层做白名单或路由控制。

## Skill Config 语义

`client.config` 直接按 `skill_id + key` 读写统一 skill 配置：

```ts
client.config.set("my-skill", "api_key", "value");
client.config.get("my-skill", "api_key");
client.config.list("my-skill");
client.config.delete("my-skill", "api_key");
```

配置是否真正影响行为，取决于对应 Lua skill 是否读取该配置。它不是运行时强制策略层。

如果宿主不希望用户修改某个核心能力的配置，建议把该能力做成宿主硬编码逻辑或受控 system 管理面，而不是把可写配置开放给普通用户。

## 覆盖的 JSON FFI 能力

当前 SDK 覆盖公共 `_json` FFI 的主要入口：

- version / describe
- engine_new / engine_free
- load_from_dirs / load_from_roots
- reload_from_dirs / reload_from_roots
- list_entries / list_skill_help / render_skill_help_detail
- prompt_argument_completions / is_skill / skill_name_for_tool
- call_skill / run_lua
- skill_config list / get / set / delete
- SQLite / LanceDB JSON provider callback register / clear
- disable / enable / install / update / uninstall
- system_disable / system_enable / system_install / system_update / system_uninstall

install / update / uninstall 支持可选 `targetRoot`，用于直接封装 USER / PROJECT / ROOT 目标 root。

## 发布说明

`package.json` 提供：

- `main`: `dist/index.js`
- `types`: `dist/index.d.ts`
- `bin`: `dist/cli.js`
- `prepack`: 发布或打包前自动构建

发布前运行：

```bash
npm install
npm run check
npm run build
npm pack --dry-run
```
