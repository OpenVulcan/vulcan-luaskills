/**
 * JSON value accepted by the LuaSkills JSON FFI surface.
 * LuaSkills JSON FFI 接口接受的 JSON 值。
 */
export type JsonValue = null | boolean | number | string | JsonValue[] | {
    [key: string]: JsonValue | undefined;
};
/**
 * Host-injected authority used by visibility queries and system management calls.
 * 可见性查询与 system 管理调用使用的宿主注入权限。
 */
export declare enum Authority {
    /**
     * Full host authority that may manage the ROOT layer.
     * 可管理 ROOT 层的完整宿主权限。
     */
    System = "system",
    /**
     * Delegated tool authority that follows ordinary user-facing boundaries.
     * 遵守普通用户可见边界的委托工具权限。
     */
    DelegatedTool = "delegated_tool"
}
/**
 * Supported managed skill source type.
 * 支持的受管 skill 来源类型。
 */
export declare enum SkillInstallSourceType {
    /**
     * GitHub Release backed managed skill.
     * 基于 GitHub Release 的受管 skill。
     */
    Github = "github",
    /**
     * Remote source descriptor URL.
     * 远程 source 描述文件 URL。
     */
    Url = "url"
}
/**
 * Named runtime skill root used by the formal ROOT, PROJECT, USER chain.
 * 正式 ROOT、PROJECT、USER 链使用的命名运行时 skill 根。
 */
export interface RuntimeSkillRoot {
    /**
     * Formal root label.
     * 正式 root 标签。
     */
    name: "ROOT" | "PROJECT" | "USER" | string;
    /**
     * Physical skills directory represented by this root.
     * 当前 root 对应的物理 skills 目录。
     */
    skills_dir: string;
}
/**
 * Lua VM pool sizing options.
 * Lua 虚拟机池容量选项。
 */
export interface LuaVmPoolConfig {
    /**
     * Minimum warm VM count.
     * 最小保温虚拟机数量。
     */
    min_size: number;
    /**
     * Maximum VM count.
     * 最大虚拟机数量。
     */
    max_size: number;
    /**
     * Idle TTL in seconds.
     * 空闲回收秒数。
     */
    idle_ttl_secs: number;
}
/**
 * Runtime capability toggles exposed by the host.
 * 宿主暴露的运行时能力开关。
 */
export interface LuaRuntimeCapabilityOptions {
    /**
     * Whether vulcan.runtime.skills is available inside Lua.
     * Lua 内是否可使用 vulcan.runtime.skills。
     */
    enable_skill_management_bridge: boolean;
}
/**
 * Optional isolated run-lua pool configuration.
 * 可选隔离 run-lua 池配置。
 */
export interface LuaRuntimeRunLuaPoolConfig extends LuaVmPoolConfig {
}
/**
 * Space controller options used by database providers.
 * 数据库 provider 使用的空间控制器选项。
 */
export interface LuaRuntimeSpaceControllerOptions {
    /**
     * Optional endpoint override.
     * 可选端点覆盖。
     */
    endpoint: string | null;
    /**
     * Whether the runtime may spawn a controller.
     * 运行时是否允许启动控制器。
     */
    auto_spawn: boolean;
    /**
     * Optional executable path.
     * 可选可执行文件路径。
     */
    executable_path: string | null;
    /**
     * Controller process mode.
     * 控制器进程模式。
     */
    process_mode: "service" | "managed";
    /**
     * Minimum controller uptime in seconds.
     * 控制器最小存活秒数。
     */
    minimum_uptime_secs: number;
    /**
     * Controller idle timeout in seconds.
     * 控制器空闲超时秒数。
     */
    idle_timeout_secs: number;
    /**
     * Default controller lease TTL in seconds.
     * 默认控制器租约 TTL 秒数。
     */
    default_lease_ttl_secs: number;
    /**
     * Connection timeout in seconds.
     * 连接超时秒数。
     */
    connect_timeout_secs: number;
    /**
     * Startup timeout in seconds.
     * 启动等待超时秒数。
     */
    startup_timeout_secs: number;
    /**
     * Startup retry interval in milliseconds.
     * 启动重试间隔毫秒数。
     */
    startup_retry_interval_ms: number;
    /**
     * Lease renew interval in seconds.
     * 租约续约间隔秒数。
     */
    lease_renew_interval_secs: number;
}
/**
 * Host options forwarded to LuaSkills engine creation.
 * 转发给 LuaSkills 引擎创建流程的宿主选项。
 */
export interface LuaRuntimeHostOptions {
    /**
     * Temporary directory used by runtime helpers.
     * 运行时辅助功能使用的临时目录。
     */
    temp_dir: string | null;
    /**
     * Optional resources directory.
     * 可选资源目录。
     */
    resources_dir: string | null;
    /**
     * Optional Lua packages directory.
     * 可选 Lua 包目录。
     */
    lua_packages_dir: string | null;
    /**
     * Optional host-provided tool root.
     * 可选宿主工具根目录。
     */
    host_provided_tool_root: string | null;
    /**
     * Optional host-provided Lua root.
     * 可选宿主 Lua 根目录。
     */
    host_provided_lua_root: string | null;
    /**
     * Optional host-provided native FFI root.
     * 可选宿主原生 FFI 根目录。
     */
    host_provided_ffi_root: string | null;
    /**
     * Optional download cache root.
     * 可选下载缓存根目录。
     */
    download_cache_root: string | null;
    /**
     * Dependency sibling directory name.
     * 依赖兄弟目录名称。
     */
    dependency_dir_name: string;
    /**
     * State sibling directory name.
     * 状态兄弟目录名称。
     */
    state_dir_name: string;
    /**
     * Database sibling directory name.
     * 数据库兄弟目录名称。
     */
    database_dir_name: string;
    /**
     * Optional unified skill config file path.
     * 可选统一 skill 配置文件路径。
     */
    skill_config_file_path: string | null;
    /**
     * Whether network downloads are allowed.
     * 是否允许网络下载。
     */
    allow_network_download: boolean;
    /**
     * Optional GitHub web base URL.
     * 可选 GitHub Web 基址。
     */
    github_base_url: string | null;
    /**
     * Optional GitHub API base URL.
     * 可选 GitHub API 基址。
     */
    github_api_base_url: string | null;
    /**
     * Optional SQLite library path.
     * 可选 SQLite 动态库路径。
     */
    sqlite_library_path: string | null;
    /**
     * SQLite provider mode.
     * SQLite provider 模式。
     */
    sqlite_provider_mode: "dynamic_library" | "host_callback" | "space_controller";
    /**
     * SQLite callback mode.
     * SQLite 回调模式。
     */
    sqlite_callback_mode: "standard" | "json";
    /**
     * Optional LanceDB library path.
     * 可选 LanceDB 动态库路径。
     */
    lancedb_library_path: string | null;
    /**
     * LanceDB provider mode.
     * LanceDB provider 模式。
     */
    lancedb_provider_mode: "dynamic_library" | "host_callback" | "space_controller";
    /**
     * LanceDB callback mode.
     * LanceDB 回调模式。
     */
    lancedb_callback_mode: "standard" | "json";
    /**
     * Shared space controller options.
     * 共享空间控制器选项。
     */
    space_controller: LuaRuntimeSpaceControllerOptions;
    /**
     * Optional cache configuration object.
     * 可选缓存配置对象。
     */
    cache_config: JsonValue | null;
    /**
     * Optional isolated run-lua pool configuration.
     * 可选隔离 run-lua 池配置。
     */
    runlua_pool_config: LuaRuntimeRunLuaPoolConfig | null;
    /**
     * Host-reserved canonical entry names.
     * 宿主保留的 canonical 入口名称。
     */
    reserved_entry_names: string[];
    /**
     * Host-forced ignored skill ids.
     * 宿主强制忽略的 skill id。
     */
    ignored_skill_ids: string[];
    /**
     * Runtime capability toggles.
     * 运行时能力开关。
     */
    capabilities: LuaRuntimeCapabilityOptions;
}
/**
 * Engine creation options accepted by the JSON FFI.
 * JSON FFI 接受的引擎创建选项。
 */
export interface LuaEngineOptions {
    /**
     * Main VM pool config.
     * 主虚拟机池配置。
     */
    pool_config: LuaVmPoolConfig;
    /**
     * Host runtime options.
     * 宿主运行时选项。
     */
    host_options: LuaRuntimeHostOptions;
}
/**
 * Invocation context injected into call_skill and run_lua.
 * 注入 call_skill 与 run_lua 的调用上下文。
 */
export interface LuaInvocationContext {
    /**
     * Optional request context object.
     * 可选请求上下文对象。
     */
    request_context?: JsonValue;
    /**
     * Client budget JSON object.
     * 客户端预算 JSON 对象。
     */
    client_budget?: JsonValue;
    /**
     * Tool config JSON object.
     * 工具配置 JSON 对象。
     */
    tool_config?: JsonValue;
}
/**
 * Runtime entry descriptor returned by listEntries.
 * listEntries 返回的运行时入口描述。
 */
export interface RuntimeEntryDescriptor {
    /**
     * Canonical tool name.
     * canonical 工具名称。
     */
    canonical_name: string;
    /**
     * Owning skill id.
     * 所属 skill id。
     */
    skill_id: string;
    /**
     * Local entry name.
     * 本地入口名称。
     */
    local_name: string;
    /**
     * Owning root name.
     * 所属 root 名称。
     */
    root_name: string;
    /**
     * Physical skill directory.
     * 物理 skill 目录。
     */
    skill_dir: string;
    /**
     * Entry description.
     * 入口描述。
     */
    description: string;
    /**
     * Entry parameter descriptors.
     * 入口参数描述。
     */
    parameters: RuntimeEntryParameterDescriptor[];
}
/**
 * Runtime entry parameter descriptor.
 * 运行时入口参数描述。
 */
export interface RuntimeEntryParameterDescriptor {
    /**
     * Parameter name.
     * 参数名称。
     */
    name: string;
    /**
     * Parameter type.
     * 参数类型。
     */
    param_type: string;
    /**
     * Parameter description.
     * 参数描述。
     */
    description: string;
    /**
     * Whether this parameter is required.
     * 当前参数是否必填。
     */
    required: boolean;
}
/**
 * Runtime invocation result returned by callSkill.
 * callSkill 返回的运行时调用结果。
 */
export interface RuntimeInvocationResult {
    /**
     * Textual content returned by the skill.
     * skill 返回的文本内容。
     */
    content: string;
    /**
     * Overflow mode encoded by the runtime.
     * 运行时编码的溢出模式。
     */
    overflow_mode: "Truncate" | "Page" | null;
    /**
     * Optional template hint.
     * 可选模板提示。
     */
    template_hint: string | null;
    /**
     * Content size in bytes.
     * 内容字节数。
     */
    content_bytes: number;
    /**
     * Content line count.
     * 内容行数。
     */
    content_lines: number;
}
/**
 * Skill install or update request.
 * skill 安装或更新请求。
 */
export interface SkillInstallRequest {
    /**
     * Optional explicit skill id.
     * 可选显式 skill id。
     */
    skill_id?: string | null;
    /**
     * Optional source locator.
     * 可选来源定位。
     */
    source?: string | null;
    /**
     * Managed source type.
     * 受管来源类型。
     */
    source_type?: SkillInstallSourceType | `${SkillInstallSourceType}`;
}
/**
 * Skill uninstall options.
 * skill 卸载选项。
 */
export interface SkillUninstallOptions {
    /**
     * Whether SQLite data should be removed.
     * 是否删除 SQLite 数据。
     */
    remove_sqlite?: boolean;
    /**
     * Whether LanceDB data should be removed.
     * 是否删除 LanceDB 数据。
     */
    remove_lancedb?: boolean;
}
/**
 * FFI version result returned by the JSON bridge.
 * JSON 桥返回的 FFI 版本结果。
 */
export interface FfiVersionResult {
    /**
     * Crate-derived FFI version string.
     * 从 crate 派生的 FFI 版本字符串。
     */
    ffi_version: string;
    /**
     * Stable protocol family name.
     * 稳定协议族名称。
     */
    protocol: string;
}
/**
 * FFI self-description result returned by the JSON bridge.
 * JSON 桥返回的 FFI 自描述结果。
 */
export interface FfiDescribeResult {
    /**
     * Crate-derived FFI version string.
     * 从 crate 派生的 FFI 版本字符串。
     */
    ffi_version: string;
    /**
     * Exported JSON FFI function names.
     * 已导出的 JSON FFI 函数名称。
     */
    exported_functions: string[];
}
/**
 * Engine handle result returned by engine creation.
 * 引擎创建返回的句柄结果。
 */
export interface EngineHandleResult {
    /**
     * Stable numeric engine id stored inside the FFI registry.
     * 存放在 FFI 注册表中的稳定数值引擎标识。
     */
    engine_id: number;
}
/**
 * Boolean value wrapper returned by query helpers.
 * 查询辅助接口返回的布尔值包装。
 */
export interface BooleanResult {
    /**
     * Query boolean value.
     * 查询布尔值。
     */
    value: boolean;
}
/**
 * Optional skill-id wrapper returned by tool-name resolution.
 * 工具名称解析返回的可选 skill id 包装。
 */
export interface OptionalSkillNameResult {
    /**
     * Optional owning skill id.
     * 可选所属 skill id。
     */
    skill_id?: string | null;
}
/**
 * Generic lifecycle acknowledgement returned by load and reload operations.
 * load 与 reload 操作返回的通用生命周期确认。
 */
export interface RuntimeAckResult {
    /**
     * Whether the engine finished loading roots.
     * 引擎是否完成 root 加载。
     */
    loaded?: boolean;
    /**
     * Whether the engine finished reloading roots.
     * 引擎是否完成 root 重载。
     */
    reloaded?: boolean;
    /**
     * Whether the engine handle was released.
     * 引擎句柄是否已释放。
     */
    freed?: boolean;
    /**
     * Whether one skill was disabled.
     * 是否已有一个 skill 被停用。
     */
    disabled?: boolean;
    /**
     * Whether one skill was enabled.
     * 是否已有一个 skill 被启用。
     */
    enabled?: boolean;
}
/**
 * Single flattened skill-config record.
 * 单条扁平化 skill 配置记录。
 */
export interface SkillConfigEntry {
    /**
     * Owning skill id.
     * 所属 skill id。
     */
    skill_id: string;
    /**
     * Config key under the skill namespace.
     * skill 命名空间下的配置键。
     */
    key: string;
    /**
     * String config value.
     * 字符串配置值。
     */
    value: string;
}
/**
 * Skill-config lookup result.
 * skill 配置查找结果。
 */
export interface SkillConfigGetResult {
    /**
     * Whether the value exists.
     * 值是否存在。
     */
    found: boolean;
    /**
     * Queried skill id.
     * 被查询的 skill id。
     */
    skill_id: string;
    /**
     * Queried config key.
     * 被查询的配置键。
     */
    key: string;
    /**
     * Optional string config value.
     * 可选字符串配置值。
     */
    value?: string | null;
}
/**
 * Skill-config mutation result.
 * skill 配置变更结果。
 */
export interface SkillConfigMutationResult {
    /**
     * Mutation action name.
     * 变更动作名称。
     */
    action: "set" | "delete" | string;
    /**
     * Touched skill id.
     * 被触及的 skill id。
     */
    skill_id: string;
    /**
     * Touched config key.
     * 被触及的配置键。
     */
    key: string;
    /**
     * Optional value returned by set.
     * set 返回的可选值。
     */
    value?: string | null;
    /**
     * Optional delete flag returned by delete.
     * delete 返回的可选删除标记。
     */
    deleted?: boolean | null;
}
/**
 * Runtime help node summary.
 * 运行时帮助节点摘要。
 */
export interface RuntimeHelpNodeDescriptor {
    /**
     * Stable flow name.
     * 稳定流程名称。
     */
    flow_name: string;
    /**
     * Short help description.
     * 简短帮助说明。
     */
    description: string;
    /**
     * Related canonical runtime entries.
     * 关联的 canonical 运行时入口。
     */
    related_entries: string[];
    /**
     * Whether this node is the main help node.
     * 当前节点是否为主帮助节点。
     */
    is_main: boolean;
}
/**
 * Runtime help tree summary for one skill.
 * 单个 skill 的运行时帮助树摘要。
 */
export interface RuntimeSkillHelpDescriptor {
    /**
     * Owning skill id.
     * 所属 skill id。
     */
    skill_id: string;
    /**
     * Human-readable skill name.
     * 人类可读 skill 名称。
     */
    skill_name: string;
    /**
     * Skill package version.
     * skill 包版本。
     */
    skill_version: string;
    /**
     * Owning root name.
     * 所属 root 名称。
     */
    root_name: string;
    /**
     * Physical skill directory.
     * 物理 skill 目录。
     */
    skill_dir: string;
    /**
     * Main help node.
     * 主帮助节点。
     */
    main: RuntimeHelpNodeDescriptor;
    /**
     * Additional flow help nodes.
     * 额外流程帮助节点。
     */
    flows: RuntimeHelpNodeDescriptor[];
}
/**
 * Rendered runtime help detail for one flow.
 * 单个流程渲染后的运行时帮助详情。
 */
export interface RuntimeHelpDetail extends RuntimeHelpNodeDescriptor {
    /**
     * Owning skill id.
     * 所属 skill id。
     */
    skill_id: string;
    /**
     * Human-readable skill name.
     * 人类可读 skill 名称。
     */
    skill_name: string;
    /**
     * Skill package version.
     * skill 包版本。
     */
    skill_version: string;
    /**
     * Owning root name.
     * 所属 root 名称。
     */
    root_name: string;
    /**
     * Physical skill directory.
     * 物理 skill 目录。
     */
    skill_dir: string;
    /**
     * Rendered content type.
     * 渲染后的内容类型。
     */
    content_type: string;
    /**
     * Rendered help content.
     * 渲染后的帮助正文。
     */
    content: string;
}
/**
 * Managed install or update result returned by lifecycle operations.
 * 生命周期操作返回的受管安装或更新结果。
 */
export interface SkillApplyResult {
    /**
     * Target skill id.
     * 目标 skill id。
     */
    skill_id: string;
    /**
     * High-level operation status.
     * 高层操作状态。
     */
    status: string;
    /**
     * Human-readable result message.
     * 人类可读结果消息。
     */
    message: string;
    /**
     * Optional involved version.
     * 可选涉及版本。
     */
    version?: string | null;
    /**
     * Optional managed source type.
     * 可选受管来源类型。
     */
    source_type?: SkillInstallSourceType | `${SkillInstallSourceType}` | null;
    /**
     * Optional stable source locator.
     * 可选稳定来源定位。
     */
    source_locator?: string | null;
}
/**
 * Skill uninstall result returned by lifecycle operations.
 * 生命周期操作返回的 skill 卸载结果。
 */
export interface SkillUninstallResult {
    /**
     * Target skill id.
     * 目标 skill id。
     */
    skill_id: string;
    /**
     * Whether the skill package directory was removed.
     * skill 包目录是否被删除。
     */
    skill_removed: boolean;
    /**
     * Whether the SQLite database directory was removed.
     * SQLite 数据库目录是否被删除。
     */
    sqlite_removed: boolean;
    /**
     * Whether the LanceDB database directory was removed.
     * LanceDB 数据库目录是否被删除。
     */
    lancedb_removed: boolean;
    /**
     * Whether the SQLite database directory was retained.
     * SQLite 数据库目录是否被保留。
     */
    sqlite_retained: boolean;
    /**
     * Whether the LanceDB database directory was retained.
     * LanceDB 数据库目录是否被保留。
     */
    lancedb_retained: boolean;
    /**
     * Human-readable result message.
     * 人类可读结果消息。
     */
    message: string;
}
/**
 * Options accepted by SDK lifecycle wrappers.
 * SDK 生命周期封装接受的选项。
 */
export interface SkillLifecycleOptions {
    /**
     * Optional explicit target root.
     * 可选显式目标 root。
     */
    targetRoot?: RuntimeSkillRoot;
    /**
     * Optional host-injected authority for system entrypoints.
     * system 入口使用的可选宿主注入权限。
     */
    authority?: Authority | `${Authority}`;
}
/**
 * Runtime-root helper creation options.
 * runtime-root 辅助创建选项。
 */
export interface RuntimeRootsOptions {
    /**
     * Shared runtime root directory.
     * 共享 runtime root 目录。
     */
    runtimeRoot: string;
    /**
     * Whether PROJECT should be included.
     * 是否包含 PROJECT。
     */
    includeProject?: boolean;
    /**
     * Whether USER should be included.
     * 是否包含 USER。
     */
    includeUser?: boolean;
    /**
     * Directory name used for ROOT skills.
     * ROOT skills 使用的目录名。
     */
    rootSkillsDirName?: string;
    /**
     * Directory name used for PROJECT skills.
     * PROJECT skills 使用的目录名。
     */
    projectSkillsDirName?: string;
    /**
     * Directory name used for USER skills.
     * USER skills 使用的目录名。
     */
    userSkillsDirName?: string;
}
/**
 * SDK client creation options.
 * SDK 客户端创建选项。
 */
export interface LuaSkillsClientOptions extends LuaSkillsSdkOptions {
    /**
     * Shared runtime root used to derive default host paths.
     * 用于派生默认宿主路径的共享 runtime root。
     */
    runtimeRoot?: string;
    /**
     * Fully explicit engine options; when present SDK defaults are skipped.
     * 完整显式引擎选项；存在时跳过 SDK 默认值。
     */
    engineOptions?: LuaEngineOptions;
    /**
     * Partial host option overrides merged over SDK defaults.
     * 覆盖 SDK 默认值的部分宿主选项。
     */
    hostOptions?: Partial<LuaRuntimeHostOptions>;
    /**
     * Partial VM pool overrides merged over SDK defaults.
     * 覆盖 SDK 默认值的部分虚拟机池选项。
     */
    poolConfig?: Partial<LuaVmPoolConfig>;
    /**
     * Whether the SDK should create the default runtime directories.
     * SDK 是否应创建默认运行时目录。
     */
    ensureRuntimeLayout?: boolean;
}
/**
 * Common SDK creation options.
 * 通用 SDK 创建选项。
 */
export interface LuaSkillsSdkOptions {
    /**
     * Explicit dynamic library path.
     * 显式动态库路径。
     */
    libraryPath?: string;
}
