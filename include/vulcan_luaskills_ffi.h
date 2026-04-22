#ifndef VULCAN_LUASKILLS_FFI_H
#define VULCAN_LUASKILLS_FFI_H

#include <stddef.h>
#include <stdint.h>

/*
Stable dual-surface C ABI exported by vulcan-luaskills.
vulcan-luaskills 导出的稳定双接口 C ABI 接口面。
*/

/*
Beta integration contract for v0.1.0:
- This header is a low-level ABI for controlled host integrations.
- Returned memory must be released only with the matching luaskills free function.
- Host callbacks must be registered before engine creation when callback-based modes are used.
- Callbacks must not unwind across the C ABI boundary.
- Same-thread reentry into the same engine is not supported.
- Skills are treated as trusted code by default; this ABI does not promise sandbox isolation.
v0.1.0 beta 集成契约：
- 当前头文件是面向受控宿主集成的低层 ABI。
- 所有返回内存都只能使用匹配的 luaskills 释放函数处理。
- 使用 callback 模式时，宿主必须先注册 callback，再创建 engine。
- callback 不允许把异常跨越 C ABI 边界传播。
- 不支持同一线程内对同一 engine 的重入调用。
- 当前默认将 skill 视为受信代码，本 ABI 不承诺沙箱隔离。
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

typedef struct FfiBorrowedBuffer {
    const uint8_t *ptr;
    size_t len;
} FfiBorrowedBuffer;

typedef struct FfiOwnedBuffer {
    uint8_t *ptr;
    size_t len;
} FfiOwnedBuffer;

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
    /*
    SQLite provider mode where 0=dynamic_library, 1=host_callback, and 2=space_controller.
    SQLite provider 模式，其中 0=dynamic_library、1=host_callback、2=space_controller。
    */
    int32_t sqlite_provider_mode;
    /*
    SQLite callback mode used only when sqlite_provider_mode=host_callback.
    sqlite_provider_mode=host_callback 时使用的 SQLite 回调模式。
    */
    int32_t sqlite_callback_mode;
    const char *lancedb_library_path;
    /*
    LanceDB provider mode where 0=dynamic_library, 1=host_callback, and 2=space_controller.
    LanceDB provider 模式，其中 0=dynamic_library、1=host_callback、2=space_controller。
    */
    int32_t lancedb_provider_mode;
    /*
    LanceDB callback mode used only when lancedb_provider_mode=host_callback.
    lancedb_provider_mode=host_callback 时使用的 LanceDB 回调模式。
    */
    int32_t lancedb_callback_mode;
    /*
    Optional shared space-controller endpoint used when one provider mode is space_controller.
    当某个 provider 模式为 space_controller 时使用的可选共享空间控制器端点。
    */
    const char *space_controller_endpoint;
    /*
    Whether the runtime may auto-spawn one space-controller process when the endpoint is unavailable.
    当空间控制器端点不可用时，运行时是否允许自动唤起空间控制器进程。
    */
    uint8_t space_controller_auto_spawn;
    /*
    Optional copied local controller executable path managed by the host.
    由宿主复制并管理的可选本地控制器可执行文件路径。
    */
    const char *space_controller_executable_path;
    /*
    Space-controller process mode where 0=service and 1=managed.
    空间控制器进程模式，其中 0=service、1=managed。
    */
    int32_t space_controller_process_mode;
    const FfiToolCacheConfig *cache_config;
    const char **reserved_entry_names;
    size_t reserved_entry_names_len;
    /*
    Whether Lua may use `vulcan.runtime.skills.*` management bridges.
    Lua 是否允许使用 `vulcan.runtime.skills.*` 管理桥接。
    */
    uint8_t enable_skill_management_bridge;
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

enum {
    FFI_PROVIDER_MODE_DYNAMIC_LIBRARY = 0,
    FFI_PROVIDER_MODE_HOST_CALLBACK = 1,
    FFI_PROVIDER_MODE_SPACE_CONTROLLER = 2
};

/*
Stable callback-mode integers used when one provider mode is host_callback.
当 provider 模式为 host_callback 时所使用的稳定回调模式整数。
*/
enum {
    FFI_CALLBACK_MODE_STANDARD = 0,
    FFI_CALLBACK_MODE_JSON = 1
};

/*
Stable process-mode integers used when one provider mode is space_controller.
当 provider 模式为 space_controller 时所使用的稳定进程模式整数。
*/
enum {
    FFI_SPACE_CONTROLLER_PROCESS_MODE_SERVICE = 0,
    FFI_SPACE_CONTROLLER_PROCESS_MODE_MANAGED = 1
};

enum {
    FFI_DATABASE_KIND_SQLITE = 0,
    FFI_DATABASE_KIND_LANCEDB = 1
};

enum {
    FFI_SQLITE_PROVIDER_ACTION_EXECUTE_SCRIPT = 0,
    FFI_SQLITE_PROVIDER_ACTION_EXECUTE_BATCH = 1,
    FFI_SQLITE_PROVIDER_ACTION_QUERY_JSON = 2,
    FFI_SQLITE_PROVIDER_ACTION_QUERY_STREAM = 3,
    FFI_SQLITE_PROVIDER_ACTION_QUERY_STREAM_WAIT_METRICS = 4,
    FFI_SQLITE_PROVIDER_ACTION_QUERY_STREAM_CHUNK = 5,
    FFI_SQLITE_PROVIDER_ACTION_QUERY_STREAM_CLOSE = 6,
    FFI_SQLITE_PROVIDER_ACTION_TOKENIZE_TEXT = 7,
    FFI_SQLITE_PROVIDER_ACTION_UPSERT_CUSTOM_WORD = 8,
    FFI_SQLITE_PROVIDER_ACTION_REMOVE_CUSTOM_WORD = 9,
    FFI_SQLITE_PROVIDER_ACTION_LIST_CUSTOM_WORDS = 10,
    FFI_SQLITE_PROVIDER_ACTION_ENSURE_FTS_INDEX = 11,
    FFI_SQLITE_PROVIDER_ACTION_REBUILD_FTS_INDEX = 12,
    FFI_SQLITE_PROVIDER_ACTION_UPSERT_FTS_DOCUMENT = 13,
    FFI_SQLITE_PROVIDER_ACTION_DELETE_FTS_DOCUMENT = 14,
    FFI_SQLITE_PROVIDER_ACTION_SEARCH_FTS = 15
};

enum {
    FFI_LANCEDB_PROVIDER_ACTION_CREATE_TABLE = 0,
    FFI_LANCEDB_PROVIDER_ACTION_VECTOR_UPSERT = 1,
    FFI_LANCEDB_PROVIDER_ACTION_VECTOR_SEARCH = 2,
    FFI_LANCEDB_PROVIDER_ACTION_DELETE = 3,
    FFI_LANCEDB_PROVIDER_ACTION_DROP_TABLE = 4
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

typedef struct FfiRuntimeDatabaseBindingContext {
    const char *space_label;
    const char *skill_id;
    const char *binding_tag;
    const char *root_name;
    const char *space_root;
    const char *skill_dir;
    const char *skill_dir_name;
    int32_t database_kind;
    const char *default_database_path;
} FfiRuntimeDatabaseBindingContext;

typedef struct FfiSqliteProviderRequest {
    int32_t action;
    FfiRuntimeDatabaseBindingContext binding;
    FfiBorrowedBuffer input_json;
} FfiSqliteProviderRequest;

typedef struct FfiLanceDbProviderRequest {
    int32_t action;
    FfiRuntimeDatabaseBindingContext binding;
    FfiBorrowedBuffer input_json;
} FfiLanceDbProviderRequest;

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
Standard callbacks must fill outputs with luaskills-owned allocations and must never unwind across the ABI boundary.
标准 callback 必须写入 luaskills 所有的输出内存，且绝不能把异常跨越 ABI 边界传播。
*/
typedef int32_t (*FfiSqliteProviderCallback)(
    const FfiSqliteProviderRequest *request,
    void *user_data,
    FfiOwnedBuffer *response_json_out,
    FfiOwnedBuffer *error_out
);
typedef int32_t (*FfiLanceDbProviderCallback)(
    const FfiLanceDbProviderRequest *request,
    void *user_data,
    FfiOwnedBuffer *meta_json_out,
    FfiOwnedBuffer *data_out,
    FfiOwnedBuffer *error_out
);

/*
Free one heap-allocated string returned by standard string-producing FFI functions.
Only pass pointers returned by luaskills FFI string-producing functions to string_free.
释放一段由标准字符串型 FFI 函数返回的堆字符串。
只能将 luaskills FFI 产出的字符串指针传给 string_free。
*/
void vulcan_luaskills_ffi_string_free(char *value);
/*
Clone one host-owned string into one luaskills-owned heap string for callback returns.
将宿主拥有的字符串克隆为 luaskills 自主管理的堆字符串，供 callback 返回使用。
*/
char *vulcan_luaskills_ffi_string_clone(const char *value);
/*
Clone one host-owned byte buffer into one luaskills-owned owned-buffer container.
将宿主拥有的字节缓冲克隆为 luaskills 自主管理的拥有型缓冲容器。
*/
int32_t vulcan_luaskills_ffi_buffer_clone(
    const uint8_t *value,
    size_t len,
    FfiOwnedBuffer *buffer_out,
    FfiOwnedBuffer *error_out
);
/*
Clone one host-owned byte buffer into one luaskills-owned heap buffer for callback returns.
将宿主拥有的字节缓冲克隆为 luaskills 自主管理的堆缓冲，供 callback 返回使用。
*/
uint8_t *vulcan_luaskills_ffi_bytes_clone(const uint8_t *value, size_t len);
/*
Free one luaskills-owned buffer container created by vulcan_luaskills_ffi_buffer_clone.
释放由 vulcan_luaskills_ffi_buffer_clone 创建的 luaskills 自主管理缓冲容器。
*/
void vulcan_luaskills_ffi_buffer_free(FfiOwnedBuffer value);
/*
Free one luaskills-owned heap byte buffer created by vulcan_luaskills_ffi_bytes_clone.
释放由 vulcan_luaskills_ffi_bytes_clone 创建的 luaskills 自主管理堆字节缓冲。
*/
void vulcan_luaskills_ffi_bytes_free(uint8_t *value, size_t len);
/*
Register or clear the SQLite host callback before engine creation.
在创建 engine 前注册或清理 SQLite 宿主 callback。
*/
int32_t vulcan_luaskills_ffi_set_sqlite_provider_callback(
    FfiSqliteProviderCallback callback,
    void *user_data,
    FfiOwnedBuffer *error_out
);
/*
Register or clear the LanceDB host callback before engine creation.
在创建 engine 前注册或清理 LanceDB 宿主 callback。
*/
int32_t vulcan_luaskills_ffi_set_lancedb_provider_callback(
    FfiLanceDbProviderCallback callback,
    void *user_data,
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
int32_t vulcan_luaskills_ffi_version(char **version_out, FfiOwnedBuffer *error_out);

/*
Return exported FFI entrypoint names through the standard C ABI surface.
通过标准 C ABI 接口返回已导出 FFI 入口点名称。
*/
int32_t vulcan_luaskills_ffi_describe(FfiStringArray **functions_out, FfiOwnedBuffer *error_out);

/*
Create one LuaSkills engine through the standard C ABI surface.
通过标准 C ABI 接口创建一个 LuaSkills 引擎。
*/
int32_t vulcan_luaskills_ffi_engine_new(
    const FfiLuaEngineOptions *options,
    uint64_t *engine_id_out,
    FfiOwnedBuffer *error_out
);

/*
Free one LuaSkills engine through the standard C ABI surface.
通过标准 C ABI 接口释放一个 LuaSkills 引擎。
*/
int32_t vulcan_luaskills_ffi_engine_free(uint64_t engine_id, FfiOwnedBuffer *error_out);

/*
Load skills from legacy directory-style roots through the standard C ABI surface.
通过标准 C ABI 接口按旧目录风格根参数加载技能。
*/
int32_t vulcan_luaskills_ffi_load_from_dirs(
    uint64_t engine_id,
    const char *base_dir,
    const char *override_dir,
    FfiOwnedBuffer *error_out
);

/*
Load skills from one ordered root chain through the standard C ABI surface.
通过标准 C ABI 接口按一条有序根链加载技能。
*/
int32_t vulcan_luaskills_ffi_load_from_roots(
    uint64_t engine_id,
    const FfiRuntimeSkillRoot *skill_roots,
    size_t skill_roots_len,
    FfiOwnedBuffer *error_out
);

/*
Reload skills from legacy directory-style roots through the standard C ABI surface.
通过标准 C ABI 接口按旧目录风格根参数重载技能。
*/
int32_t vulcan_luaskills_ffi_reload_from_dirs(
    uint64_t engine_id,
    const char *base_dir,
    const char *override_dir,
    FfiOwnedBuffer *error_out
);

/*
Reload skills from one ordered root chain through the standard C ABI surface.
通过标准 C ABI 接口按一条有序根链重载技能。
*/
int32_t vulcan_luaskills_ffi_reload_from_roots(
    uint64_t engine_id,
    const FfiRuntimeSkillRoot *skill_roots,
    size_t skill_roots_len,
    FfiOwnedBuffer *error_out
);

/*
List runtime entries through the standard C ABI surface.
通过标准 C ABI 接口列出运行时入口。
*/
int32_t vulcan_luaskills_ffi_list_entries(
    uint64_t engine_id,
    FfiRuntimeEntryDescriptorList **entries_out,
    FfiOwnedBuffer *error_out
);

/*
List runtime help trees through the standard C ABI surface.
通过标准 C ABI 接口列出运行时帮助树。
*/
int32_t vulcan_luaskills_ffi_list_skill_help(
    uint64_t engine_id,
    FfiRuntimeSkillHelpDescriptorList **help_out,
    FfiOwnedBuffer *error_out
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
    FfiOwnedBuffer *error_out
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
    FfiOwnedBuffer *error_out
);

/*
Check whether one tool belongs to a Lua skill through the standard C ABI surface.
通过标准 C ABI 接口检查单个工具是否属于 Lua 技能。
*/
int32_t vulcan_luaskills_ffi_is_skill(
    uint64_t engine_id,
    const char *tool_name,
    uint8_t *value_out,
    FfiOwnedBuffer *error_out
);

/*
Resolve the owning skill id of one tool through the standard C ABI surface.
通过标准 C ABI 接口解析单个工具所属的技能标识符。
*/
int32_t vulcan_luaskills_ffi_skill_name_for_tool(
    uint64_t engine_id,
    const char *tool_name,
    char **skill_id_out,
    FfiOwnedBuffer *error_out
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
    FfiOwnedBuffer *error_out
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
    FfiOwnedBuffer *error_out
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
    FfiOwnedBuffer *error_out
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
    FfiOwnedBuffer *error_out
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
    FfiOwnedBuffer *error_out
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
    FfiOwnedBuffer *error_out
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
    FfiOwnedBuffer *error_out
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
    FfiOwnedBuffer *error_out
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
    FfiOwnedBuffer *error_out
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
    FfiOwnedBuffer *error_out
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
    FfiOwnedBuffer *error_out
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
    FfiOwnedBuffer *error_out
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
    FfiOwnedBuffer *error_out
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

