#ifndef VULCAN_LUASKILLS_JSON_FFI_H
#define VULCAN_LUASKILLS_JSON_FFI_H

#include "vulcan_luaskills_ffi.h"

/*
Public high-level JSON FFI exported by vulcan-luaskills.
vulcan-luaskills 导出的公共高层 JSON FFI 接口面。
*/

/*
Beta integration contract for v0.1.0:
- This header is the public high-level JSON FFI for dynamic languages and rapid integrations.
- Shared structs and free helpers come from vulcan_luaskills_ffi.h.
- Returned buffers must be released only with the matching luaskills free function.
- JSON callbacks must be registered before engine creation when callback-based modes are used.
- Callbacks must not unwind across the C ABI boundary.
- Same-thread reentry into the same engine is not supported.
v0.1.0 beta 集成契约：
- 当前头文件是面向动态语言与快速集成场景的公共高层 JSON FFI。
- 共享结构体与释放辅助函数来自 vulcan_luaskills_ffi.h。
- 所有返回缓冲都只能使用匹配的 luaskills 释放函数处理。
- 使用 JSON callback 模式时，宿主必须先注册 callback，再创建 engine。
- callback 不允许把异常跨越 C ABI 边界传播。
- 不支持同一线程内对同一 engine 的重入调用。
*/

#ifdef __cplusplus
extern "C" {
#endif

/*
JSON callback must consume one borrowed UTF-8 request buffer and fill one owned response buffer.
JSON callback 必须消费一个借用 UTF-8 请求缓冲，并填充一个拥有型响应缓冲。
*/
typedef int32_t (*FfiJsonProviderCallback)(
    FfiBorrowedBuffer request_json,
    void *user_data,
    FfiOwnedBuffer *response_out,
    FfiOwnedBuffer *error_out
);

/*
Register or clear the SQLite JSON callback before engine creation.
在创建 engine 前注册或清理 SQLite JSON callback。
*/
int32_t vulcan_luaskills_ffi_set_sqlite_provider_json_callback(
    FfiJsonProviderCallback callback,
    void *user_data,
    FfiOwnedBuffer *error_out
);

/*
Register or clear the LanceDB JSON callback before engine creation.
在创建 engine 前注册或清理 LanceDB JSON callback。
*/
int32_t vulcan_luaskills_ffi_set_lancedb_provider_json_callback(
    FfiJsonProviderCallback callback,
    void *user_data,
    FfiOwnedBuffer *error_out
);

/*
Return one stable FFI version descriptor as JSON.
以 JSON 形式返回稳定的 FFI 版本描述。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_version_json(void);

/*
Return one JSON description of exported FFI entrypoints.
以 JSON 形式返回已导出 FFI 入口点说明。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_describe_json(void);

/*
Create one LuaSkills engine from one JSON request.
通过一段 JSON 请求创建一个 LuaSkills 引擎。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_engine_new_json(FfiBorrowedBuffer input_json);

/*
Free one previously created LuaSkills engine handle.
释放一个先前创建的 LuaSkills 引擎句柄。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_engine_free_json(FfiBorrowedBuffer input_json);

/*
Load skills from legacy directory-style roots.
从旧目录风格根参数加载技能。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_load_from_dirs_json(FfiBorrowedBuffer input_json);

/*
Load skills from one ordered root chain.
从一条有序根链加载技能。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_load_from_roots_json(FfiBorrowedBuffer input_json);

/*
Reload skills from legacy directory-style roots.
从旧目录风格根参数重载技能。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_reload_from_dirs_json(FfiBorrowedBuffer input_json);

/*
Reload skills from one ordered root chain.
从一条有序根链重载技能。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_reload_from_roots_json(FfiBorrowedBuffer input_json);

/*
List runtime entry descriptors as JSON.
以 JSON 形式列出运行时入口描述。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_list_entries_json(FfiBorrowedBuffer input_json);

/*
List runtime help descriptors as JSON.
以 JSON 形式列出运行时帮助描述。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_list_skill_help_json(FfiBorrowedBuffer input_json);

/*
Render one runtime help detail payload as JSON.
以 JSON 形式渲染单个运行时帮助详情。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_render_skill_help_detail_json(FfiBorrowedBuffer input_json);

/*
Resolve prompt argument completions as JSON.
以 JSON 形式解析提示词参数补全项。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_prompt_argument_completions_json(FfiBorrowedBuffer input_json);

/*
Check whether one canonical tool name belongs to a Lua skill.
检查某个 canonical 工具名是否属于 Lua 技能。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_is_skill_json(FfiBorrowedBuffer input_json);

/*
Resolve the owning skill id of one canonical tool name.
解析某个 canonical 工具名所属的技能标识符。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_skill_name_for_tool_json(FfiBorrowedBuffer input_json);

/*
Call one loaded skill entry using one JSON request.
使用一段 JSON 请求调用单个已加载技能入口。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_call_skill_json(FfiBorrowedBuffer input_json);

/*
Execute arbitrary Lua code using one JSON request.
使用一段 JSON 请求执行任意 Lua 代码。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_run_lua_json(FfiBorrowedBuffer input_json);

/*
Disable one skill through legacy directory-style roots.
通过旧目录风格根参数停用单个技能。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_disable_skill_in_dirs_json(FfiBorrowedBuffer input_json);

/*
Disable one skill through one ordered root chain.
通过一条有序根链停用单个技能。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_disable_skill_json(FfiBorrowedBuffer input_json);

/*
Disable one protected-capable skill through legacy directory-style roots.
通过旧目录风格根参数在 system 平面停用单个技能。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_system_disable_skill_in_dirs_json(FfiBorrowedBuffer input_json);

/*
Disable one protected-capable skill through one ordered root chain.
通过一条有序根链在 system 平面停用单个技能。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_system_disable_skill_json(FfiBorrowedBuffer input_json);

/*
Enable one skill through one ordered root chain.
通过一条有序根链启用单个技能。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_enable_skill_json(FfiBorrowedBuffer input_json);

/*
Enable one protected-capable skill through one ordered root chain.
通过一条有序根链在 system 平面启用单个技能。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_system_enable_skill_json(FfiBorrowedBuffer input_json);

/*
Uninstall one skill through one ordered root chain.
通过一条有序根链卸载单个技能。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_uninstall_skill_json(FfiBorrowedBuffer input_json);

/*
Uninstall one protected-capable skill through one ordered root chain.
通过一条有序根链在 system 平面卸载单个技能。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_system_uninstall_skill_json(FfiBorrowedBuffer input_json);

/*
Install one managed skill through one ordered root chain.
通过一条有序根链安装单个受管技能。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_install_skill_json(FfiBorrowedBuffer input_json);

/*
Install one managed skill through one ordered root chain on the system plane.
通过一条有序根链在 system 平面安装单个受管技能。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_system_install_skill_json(FfiBorrowedBuffer input_json);

/*
Update one managed skill through one ordered root chain.
通过一条有序根链更新单个受管技能。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_update_skill_json(FfiBorrowedBuffer input_json);

/*
Update one managed skill through one ordered root chain on the system plane.
通过一条有序根链在 system 平面更新单个受管技能。
*/
FfiOwnedBuffer vulcan_luaskills_ffi_system_update_skill_json(FfiBorrowedBuffer input_json);

#ifdef __cplusplus
}
#endif

#endif
