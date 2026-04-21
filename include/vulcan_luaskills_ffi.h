#ifndef VULCAN_LUASKILLS_FFI_H
#define VULCAN_LUASKILLS_FFI_H

#include <stddef.h>
#include <stdint.h>

/*
Stable JSON-based C ABI surface exported by vulcan-luaskills.
vulcan-luaskills 导出的稳定 JSON 风格 C ABI 接口面。
*/

#ifdef __cplusplus
extern "C" {
#endif

typedef struct FfiLuaVmPoolConfig {
    size_t min_size;
    size_t max_size;
    uint64_t idle_ttl_secs;
} FfiLuaVmPoolConfig;

typedef struct FfiToolCacheConfig {
    size_t max_entries;
    uint64_t default_ttl_secs;
    uint64_t max_ttl_secs;
} FfiToolCacheConfig;

typedef struct FfiLuaRuntimeHostOptions {
    const char *temp_dir;
    const char *resources_dir;
    const char *lua_packages_dir;
    const char *luaexec_program;
    const char *host_provided_tool_root;
    const char *host_provided_lua_root;
    const char *host_provided_ffi_root;
    const char *download_cache_root;
    const char *dependency_dir_name;
    const char *state_dir_name;
    const char *database_dir_name;
    const char **protected_skill_ids;
    size_t protected_skill_ids_len;
    uint8_t allow_network_download;
    const char *github_base_url;
    const char *github_api_base_url;
    const char *sqlite_library_path;
    const char *lancedb_library_path;
    const FfiToolCacheConfig *cache_config;
    const char **reserved_entry_names;
    size_t reserved_entry_names_len;
} FfiLuaRuntimeHostOptions;

typedef struct FfiLuaEngineOptions {
    FfiLuaVmPoolConfig pool;
    FfiLuaRuntimeHostOptions host;
} FfiLuaEngineOptions;

typedef struct FfiRuntimeSkillRoot {
    const char *name;
    const char *skills_dir;
} FfiRuntimeSkillRoot;

typedef struct FfiLuaInvocationContext {
    const char *request_context_json;
    const char *client_budget_json;
    const char *tool_config_json;
} FfiLuaInvocationContext;

/*
Stable source-type integers used by standard install and update requests/results.
标准安装与更新请求及结果使用的稳定来源类型整数。
*/
enum {
    FFI_SOURCE_TYPE_ABSENT = -1,
    FFI_SOURCE_TYPE_GITHUB = 0,
    FFI_SOURCE_TYPE_URL = 1
};

typedef struct FfiSkillInstallRequest {
    const char *skill_id;
    const char *source;
    /* FFI_SOURCE_TYPE_GITHUB or FFI_SOURCE_TYPE_URL. */
    /* FFI_SOURCE_TYPE_GITHUB 或 FFI_SOURCE_TYPE_URL。 */
    int32_t source_type;
} FfiSkillInstallRequest;

typedef struct FfiSkillUninstallOptions {
    uint8_t remove_sqlite;
    uint8_t remove_lancedb;
} FfiSkillUninstallOptions;

typedef struct FfiStringArray {
    char **items;
    size_t len;
} FfiStringArray;

typedef struct FfiRuntimeEntryParameterDescriptor {
    char *name;
    char *param_type;
    char *description;
    uint8_t required;
} FfiRuntimeEntryParameterDescriptor;

typedef struct FfiRuntimeEntryDescriptor {
    char *canonical_name;
    char *skill_id;
    char *local_name;
    char *root_name;
    char *skill_dir;
    char *description;
    struct FfiRuntimeEntryParameterDescriptor *parameters;
    size_t parameters_len;
} FfiRuntimeEntryDescriptor;

typedef struct FfiRuntimeEntryDescriptorList {
    struct FfiRuntimeEntryDescriptor *items;
    size_t len;
} FfiRuntimeEntryDescriptorList;

typedef struct FfiRuntimeHelpNodeDescriptor {
    char *flow_name;
    char *description;
    char **related_entries;
    size_t related_entries_len;
    uint8_t is_main;
} FfiRuntimeHelpNodeDescriptor;

typedef struct FfiRuntimeSkillHelpDescriptor {
    char *skill_id;
    char *skill_name;
    char *skill_version;
    char *root_name;
    char *skill_dir;
    struct FfiRuntimeHelpNodeDescriptor main;
    struct FfiRuntimeHelpNodeDescriptor *flows;
    size_t flows_len;
} FfiRuntimeSkillHelpDescriptor;

typedef struct FfiRuntimeSkillHelpDescriptorList {
    struct FfiRuntimeSkillHelpDescriptor *items;
    size_t len;
} FfiRuntimeSkillHelpDescriptorList;

typedef struct FfiRuntimeHelpDetail {
    char *skill_id;
    char *skill_name;
    char *skill_version;
    char *root_name;
    char *skill_dir;
    char *flow_name;
    char *description;
    char **related_entries;
    size_t related_entries_len;
    uint8_t is_main;
    char *content_type;
    char *content;
} FfiRuntimeHelpDetail;

typedef struct FfiRuntimeInvocationResult {
    char *content;
    int32_t overflow_mode;
    char *template_hint;
    size_t content_bytes;
    size_t content_lines;
} FfiRuntimeInvocationResult;

typedef struct FfiSkillApplyResult {
    char *skill_id;
    char *status;
    char *message;
    char *version;
    /* FFI_SOURCE_TYPE_ABSENT, FFI_SOURCE_TYPE_GITHUB, or FFI_SOURCE_TYPE_URL. */
    /* FFI_SOURCE_TYPE_ABSENT、FFI_SOURCE_TYPE_GITHUB 或 FFI_SOURCE_TYPE_URL。 */
    int32_t source_type;
    char *source_locator;
} FfiSkillApplyResult;

typedef struct FfiSkillUninstallResult {
    char *skill_id;
    uint8_t skill_removed;
    uint8_t sqlite_removed;
    uint8_t lancedb_removed;
    uint8_t sqlite_retained;
    uint8_t lancedb_retained;
    char *message;
} FfiSkillUninstallResult;

/*
Free one heap-allocated JSON string returned by the FFI layer.
释放一段由 FFI 层返回并在堆上分配的 JSON 字符串。
*/
void vulcan_luaskills_ffi_string_free(char *value);

/*
Free one heap-allocated string-array result returned by the standard FFI layer.
释放一段由标准 FFI 层返回并在堆上分配的字符串数组结果。
*/
void vulcan_luaskills_ffi_string_array_free(FfiStringArray *value);

/*
Free one heap-allocated entry descriptor list returned by the standard FFI layer.
释放一段由标准 FFI 层返回并在堆上分配的入口描述列表。
*/
void vulcan_luaskills_ffi_entry_list_free(FfiRuntimeEntryDescriptorList *value);

/*
Free one heap-allocated help descriptor list returned by the standard FFI layer.
释放一段由标准 FFI 层返回并在堆上分配的帮助描述列表。
*/
void vulcan_luaskills_ffi_help_list_free(FfiRuntimeSkillHelpDescriptorList *value);

/*
Free one heap-allocated help detail returned by the standard FFI layer.
释放一段由标准 FFI 层返回并在堆上分配的帮助详情。
*/
void vulcan_luaskills_ffi_help_detail_free(FfiRuntimeHelpDetail *value);

/*
Free one heap-allocated invocation result returned by the standard FFI layer.
释放一段由标准 FFI 层返回并在堆上分配的调用结果。
*/
void vulcan_luaskills_ffi_invocation_result_free(FfiRuntimeInvocationResult *value);

/*
Free one heap-allocated skill apply result returned by the standard FFI layer.
释放一段由标准 FFI 层返回并在堆上分配的技能安装或更新结果。
*/
void vulcan_luaskills_ffi_skill_apply_result_free(FfiSkillApplyResult *value);

/*
Free one heap-allocated skill uninstall result returned by the standard FFI layer.
释放一段由标准 FFI 层返回并在堆上分配的技能卸载结果。
*/
void vulcan_luaskills_ffi_skill_uninstall_result_free(FfiSkillUninstallResult *value);

/*
Return one stable FFI version string through the standard C ABI surface.
通过标准 C ABI 接口返回稳定的 FFI 版本字符串。
*/
int32_t vulcan_luaskills_ffi_version(char **version_out, char **error_out);

/*
Return exported FFI entrypoint names through the standard C ABI surface.
通过标准 C ABI 接口返回已导出 FFI 入口点名称。
*/
int32_t vulcan_luaskills_ffi_describe(FfiStringArray **functions_out, char **error_out);

/*
Create one LuaSkills engine through the standard C ABI surface.
通过标准 C ABI 接口创建一个 LuaSkills 引擎。
*/
int32_t vulcan_luaskills_ffi_engine_new(
    const FfiLuaEngineOptions *options,
    uint64_t *engine_id_out,
    char **error_out
);

/*
Free one LuaSkills engine through the standard C ABI surface.
通过标准 C ABI 接口释放一个 LuaSkills 引擎。
*/
int32_t vulcan_luaskills_ffi_engine_free(uint64_t engine_id, char **error_out);

/*
Load skills from legacy directory-style roots through the standard C ABI surface.
通过标准 C ABI 接口按旧目录风格根参数加载技能。
*/
int32_t vulcan_luaskills_ffi_load_from_dirs(
    uint64_t engine_id,
    const char *base_dir,
    const char *override_dir,
    char **error_out
);

/*
Load skills from one ordered root chain through the standard C ABI surface.
通过标准 C ABI 接口按一条有序根链加载技能。
*/
int32_t vulcan_luaskills_ffi_load_from_roots(
    uint64_t engine_id,
    const FfiRuntimeSkillRoot *skill_roots,
    size_t skill_roots_len,
    char **error_out
);

/*
Reload skills from legacy directory-style roots through the standard C ABI surface.
通过标准 C ABI 接口按旧目录风格根参数重载技能。
*/
int32_t vulcan_luaskills_ffi_reload_from_dirs(
    uint64_t engine_id,
    const char *base_dir,
    const char *override_dir,
    char **error_out
);

/*
Reload skills from one ordered root chain through the standard C ABI surface.
通过标准 C ABI 接口按一条有序根链重载技能。
*/
int32_t vulcan_luaskills_ffi_reload_from_roots(
    uint64_t engine_id,
    const FfiRuntimeSkillRoot *skill_roots,
    size_t skill_roots_len,
    char **error_out
);

/*
List runtime entries through the standard C ABI surface.
通过标准 C ABI 接口列出运行时入口。
*/
int32_t vulcan_luaskills_ffi_list_entries(
    uint64_t engine_id,
    FfiRuntimeEntryDescriptorList **entries_out,
    char **error_out
);

/*
List runtime help trees through the standard C ABI surface.
通过标准 C ABI 接口列出运行时帮助树。
*/
int32_t vulcan_luaskills_ffi_list_skill_help(
    uint64_t engine_id,
    FfiRuntimeSkillHelpDescriptorList **help_out,
    char **error_out
);

/*
Render one help detail through the standard C ABI surface.
通过标准 C ABI 接口渲染单个帮助详情。
*/
int32_t vulcan_luaskills_ffi_render_skill_help_detail(
    uint64_t engine_id,
    const char *skill_id,
    const char *flow_name,
    const char *request_context_json,
    FfiRuntimeHelpDetail **detail_out,
    char **error_out
);

/*
Resolve prompt argument completions through the standard C ABI surface.
通过标准 C ABI 接口解析提示词参数补全项。
*/
int32_t vulcan_luaskills_ffi_prompt_argument_completions(
    uint64_t engine_id,
    const char *prompt_name,
    const char *argument_name,
    FfiStringArray **values_out,
    char **error_out
);

/*
Check whether one tool belongs to a Lua skill through the standard C ABI surface.
通过标准 C ABI 接口检查单个工具是否属于 Lua 技能。
*/
int32_t vulcan_luaskills_ffi_is_skill(
    uint64_t engine_id,
    const char *tool_name,
    uint8_t *value_out,
    char **error_out
);

/*
Resolve the owning skill id of one tool through the standard C ABI surface.
通过标准 C ABI 接口解析单个工具所属的技能标识符。
*/
int32_t vulcan_luaskills_ffi_skill_name_for_tool(
    uint64_t engine_id,
    const char *tool_name,
    char **skill_id_out,
    char **error_out
);

/*
Call one loaded skill entry through the standard C ABI surface.
通过标准 C ABI 接口调用单个已加载技能入口。
*/
int32_t vulcan_luaskills_ffi_call_skill(
    uint64_t engine_id,
    const char *tool_name,
    const char *args_json,
    const FfiLuaInvocationContext *invocation_context,
    FfiRuntimeInvocationResult **result_out,
    char **error_out
);

/*
Execute arbitrary Lua code through the standard C ABI surface.
通过标准 C ABI 接口执行任意 Lua 代码。
*/
int32_t vulcan_luaskills_ffi_run_lua(
    uint64_t engine_id,
    const char *code,
    const char *args_json,
    const FfiLuaInvocationContext *invocation_context,
    char **result_json_out,
    char **error_out
);

/*
Disable one skill through legacy directory-style roots via the standard C ABI surface.
通过标准 C ABI 接口按旧目录风格根参数停用单个技能。
*/
int32_t vulcan_luaskills_ffi_disable_skill_in_dirs(
    uint64_t engine_id,
    const char *base_dir,
    const char *override_dir,
    const char *skill_id,
    const char *reason,
    char **error_out
);

/*
Disable one skill through one ordered root chain via the standard C ABI surface.
通过标准 C ABI 接口按一条有序根链停用单个技能。
*/
int32_t vulcan_luaskills_ffi_disable_skill(
    uint64_t engine_id,
    const FfiRuntimeSkillRoot *skill_roots,
    size_t skill_roots_len,
    const char *skill_id,
    const char *reason,
    char **error_out
);

/*
Disable one skill on the system plane through legacy directory-style roots.
通过标准 C ABI 接口按旧目录风格根参数在 system 平面停用单个技能。
*/
int32_t vulcan_luaskills_ffi_system_disable_skill_in_dirs(
    uint64_t engine_id,
    const char *base_dir,
    const char *override_dir,
    const char *skill_id,
    const char *reason,
    char **error_out
);

/*
Disable one skill on the system plane through one ordered root chain.
通过标准 C ABI 接口按一条有序根链在 system 平面停用单个技能。
*/
int32_t vulcan_luaskills_ffi_system_disable_skill(
    uint64_t engine_id,
    const FfiRuntimeSkillRoot *skill_roots,
    size_t skill_roots_len,
    const char *skill_id,
    const char *reason,
    char **error_out
);

/*
Enable one skill through one ordered root chain via the standard C ABI surface.
通过标准 C ABI 接口按一条有序根链启用单个技能。
*/
int32_t vulcan_luaskills_ffi_enable_skill(
    uint64_t engine_id,
    const FfiRuntimeSkillRoot *skill_roots,
    size_t skill_roots_len,
    const char *skill_id,
    char **error_out
);

/*
Enable one skill on the system plane through one ordered root chain.
通过标准 C ABI 接口按一条有序根链在 system 平面启用单个技能。
*/
int32_t vulcan_luaskills_ffi_system_enable_skill(
    uint64_t engine_id,
    const FfiRuntimeSkillRoot *skill_roots,
    size_t skill_roots_len,
    const char *skill_id,
    char **error_out
);

/*
Uninstall one skill through one ordered root chain via the standard C ABI surface.
通过标准 C ABI 接口按一条有序根链卸载单个技能。
*/
int32_t vulcan_luaskills_ffi_uninstall_skill(
    uint64_t engine_id,
    const FfiRuntimeSkillRoot *skill_roots,
    size_t skill_roots_len,
    const char *skill_id,
    const FfiSkillUninstallOptions *options,
    FfiSkillUninstallResult **result_out,
    char **error_out
);

/*
Uninstall one skill on the system plane through one ordered root chain.
通过标准 C ABI 接口按一条有序根链在 system 平面卸载单个技能。
*/
int32_t vulcan_luaskills_ffi_system_uninstall_skill(
    uint64_t engine_id,
    const FfiRuntimeSkillRoot *skill_roots,
    size_t skill_roots_len,
    const char *skill_id,
    const FfiSkillUninstallOptions *options,
    FfiSkillUninstallResult **result_out,
    char **error_out
);

/*
Install one managed skill through one ordered root chain via the standard C ABI surface.
通过标准 C ABI 接口按一条有序根链安装单个受管技能。
*/
int32_t vulcan_luaskills_ffi_install_skill(
    uint64_t engine_id,
    const FfiRuntimeSkillRoot *skill_roots,
    size_t skill_roots_len,
    const FfiSkillInstallRequest *request,
    FfiSkillApplyResult **result_out,
    char **error_out
);

/*
Install one managed skill on the system plane through one ordered root chain.
通过标准 C ABI 接口按一条有序根链在 system 平面安装单个受管技能。
*/
int32_t vulcan_luaskills_ffi_system_install_skill(
    uint64_t engine_id,
    const FfiRuntimeSkillRoot *skill_roots,
    size_t skill_roots_len,
    const FfiSkillInstallRequest *request,
    FfiSkillApplyResult **result_out,
    char **error_out
);

/*
Update one managed skill through one ordered root chain via the standard C ABI surface.
通过标准 C ABI 接口按一条有序根链更新单个受管技能。
*/
int32_t vulcan_luaskills_ffi_update_skill(
    uint64_t engine_id,
    const FfiRuntimeSkillRoot *skill_roots,
    size_t skill_roots_len,
    const FfiSkillInstallRequest *request,
    FfiSkillApplyResult **result_out,
    char **error_out
);

/*
Update one managed skill on the system plane through one ordered root chain.
通过标准 C ABI 接口按一条有序根链在 system 平面更新单个受管技能。
*/
int32_t vulcan_luaskills_ffi_system_update_skill(
    uint64_t engine_id,
    const FfiRuntimeSkillRoot *skill_roots,
    size_t skill_roots_len,
    const FfiSkillInstallRequest *request,
    FfiSkillApplyResult **result_out,
    char **error_out
);

/*
Return one stable FFI version descriptor as JSON.
以 JSON 形式返回稳定的 FFI 版本描述。
*/
char *vulcan_luaskills_ffi_version_json(void);

/*
Return one JSON description of exported FFI entrypoints.
以 JSON 形式返回已导出 FFI 入口点说明。
*/
char *vulcan_luaskills_ffi_describe_json(void);

/*
Create one LuaSkills engine from one JSON request.
通过一段 JSON 请求创建一个 LuaSkills 引擎。
*/
char *vulcan_luaskills_ffi_engine_new_json(const char *input_json);

/*
Free one previously created LuaSkills engine handle.
释放一个先前创建的 LuaSkills 引擎句柄。
*/
char *vulcan_luaskills_ffi_engine_free_json(const char *input_json);

/*
Load skills from legacy directory-style roots.
从旧目录风格根参数加载技能。
*/
char *vulcan_luaskills_ffi_load_from_dirs_json(const char *input_json);

/*
Load skills from one ordered root chain.
从一条有序根链加载技能。
*/
char *vulcan_luaskills_ffi_load_from_roots_json(const char *input_json);

/*
Reload skills from legacy directory-style roots.
从旧目录风格根参数重载技能。
*/
char *vulcan_luaskills_ffi_reload_from_dirs_json(const char *input_json);

/*
Reload skills from one ordered root chain.
从一条有序根链重载技能。
*/
char *vulcan_luaskills_ffi_reload_from_roots_json(const char *input_json);

/*
List runtime entry descriptors as JSON.
以 JSON 形式列出运行时入口描述。
*/
char *vulcan_luaskills_ffi_list_entries_json(const char *input_json);

/*
List runtime help descriptors as JSON.
以 JSON 形式列出运行时帮助描述。
*/
char *vulcan_luaskills_ffi_list_skill_help_json(const char *input_json);

/*
Render one runtime help detail payload as JSON.
以 JSON 形式渲染单个运行时帮助详情。
*/
char *vulcan_luaskills_ffi_render_skill_help_detail_json(const char *input_json);

/*
Resolve prompt argument completions as JSON.
以 JSON 形式解析提示词参数补全项。
*/
char *vulcan_luaskills_ffi_prompt_argument_completions_json(const char *input_json);

/*
Check whether one canonical tool name belongs to a Lua skill.
检查某个 canonical 工具名是否属于 Lua 技能。
*/
char *vulcan_luaskills_ffi_is_skill_json(const char *input_json);

/*
Resolve the owning skill id of one canonical tool name.
解析某个 canonical 工具名所属的技能标识符。
*/
char *vulcan_luaskills_ffi_skill_name_for_tool_json(const char *input_json);

/*
Call one loaded skill entry using one JSON request.
使用一段 JSON 请求调用单个已加载技能入口。
*/
char *vulcan_luaskills_ffi_call_skill_json(const char *input_json);

/*
Execute arbitrary Lua code using one JSON request.
使用一段 JSON 请求执行任意 Lua 代码。
*/
char *vulcan_luaskills_ffi_run_lua_json(const char *input_json);

/*
Disable one skill through legacy directory-style roots.
通过旧目录风格根参数停用单个技能。
*/
char *vulcan_luaskills_ffi_disable_skill_in_dirs_json(const char *input_json);

/*
Disable one skill through one ordered root chain.
通过一条有序根链停用单个技能。
*/
char *vulcan_luaskills_ffi_disable_skill_json(const char *input_json);

/*
Disable one protected-capable skill through legacy directory-style roots.
通过旧目录风格根参数在 system 平面停用单个技能。
*/
char *vulcan_luaskills_ffi_system_disable_skill_in_dirs_json(const char *input_json);

/*
Disable one protected-capable skill through one ordered root chain.
通过一条有序根链在 system 平面停用单个技能。
*/
char *vulcan_luaskills_ffi_system_disable_skill_json(const char *input_json);

/*
Enable one skill through one ordered root chain.
通过一条有序根链启用单个技能。
*/
char *vulcan_luaskills_ffi_enable_skill_json(const char *input_json);

/*
Enable one protected-capable skill through one ordered root chain.
通过一条有序根链在 system 平面启用单个技能。
*/
char *vulcan_luaskills_ffi_system_enable_skill_json(const char *input_json);

/*
Uninstall one skill through one ordered root chain.
通过一条有序根链卸载单个技能。
*/
char *vulcan_luaskills_ffi_uninstall_skill_json(const char *input_json);

/*
Uninstall one protected-capable skill through one ordered root chain.
通过一条有序根链在 system 平面卸载单个技能。
*/
char *vulcan_luaskills_ffi_system_uninstall_skill_json(const char *input_json);

/*
Install one managed skill through one ordered root chain.
通过一条有序根链安装单个受管技能。
*/
char *vulcan_luaskills_ffi_install_skill_json(const char *input_json);

/*
Install one managed skill through one ordered root chain on the system plane.
通过一条有序根链在 system 平面安装单个受管技能。
*/
char *vulcan_luaskills_ffi_system_install_skill_json(const char *input_json);

/*
Update one managed skill through one ordered root chain.
通过一条有序根链更新单个受管技能。
*/
char *vulcan_luaskills_ffi_update_skill_json(const char *input_json);

/*
Update one managed skill through one ordered root chain on the system plane.
通过一条有序根链在 system 平面更新单个受管技能。
*/
char *vulcan_luaskills_ffi_system_update_skill_json(const char *input_json);

#ifdef __cplusplus
}
#endif

#endif
