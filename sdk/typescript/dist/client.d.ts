import { LuaSkillsJsonFfi } from "./ffi.js";
import { Authority, type FfiDescribeResult, type FfiVersionResult, type JsonValue, type LuaEngineOptions, type LuaInvocationContext, type LuaRuntimeHostOptions, type LuaRuntimeSpaceControllerOptions, type LuaSkillsClientOptions, type LuaSkillsSdkOptions, type LuaVmPoolConfig, type RuntimeAckResult, type RuntimeEntryDescriptor, type RuntimeHelpDetail, type RuntimeInvocationResult, type RuntimeSkillHelpDescriptor, type RuntimeSkillRoot, type SkillApplyResult, type SkillConfigEntry, type SkillConfigGetResult, type SkillConfigMutationResult, type SkillInstallRequest, type SkillLifecycleOptions, type SkillUninstallOptions, type SkillUninstallResult } from "./types.js";
/**
 * Options accepted by runtime help rendering.
 * 运行时帮助渲染接受的选项。
 */
export interface RenderHelpOptions {
    /**
     * Host-injected query authority.
     * 宿主注入的查询权限。
     */
    authority?: Authority | `${Authority}`;
    /**
     * Optional request context forwarded to help rendering.
     * 转发给帮助渲染的可选请求上下文。
     */
    requestContext?: JsonValue;
}
/**
 * High-level LuaSkills SDK client over the public JSON FFI surface.
 * 基于公共 JSON FFI 表面的高级 LuaSkills SDK 客户端。
 */
export declare class LuaSkillsClient {
    /**
     * Low-level JSON FFI bridge used by this client.
     * 当前客户端使用的底层 JSON FFI 桥。
     */
    readonly ffi: LuaSkillsJsonFfi;
    /**
     * Stable numeric engine handle stored inside the native FFI registry.
     * 存放在原生 FFI 注册表中的稳定数值引擎句柄。
     */
    readonly engineId: number;
    /**
     * Skill-config API namespace.
     * skill 配置 API 命名空间。
     */
    readonly config: SkillConfigClient;
    /**
     * Ordinary Skills-plane management API namespace.
     * 普通 Skills plane 管理 API 命名空间。
     */
    readonly skills: SkillManagementClient;
    /**
     * Whether the native engine handle has already been released.
     * 原生引擎句柄是否已经被释放。
     */
    private closed;
    /**
     * Create one SDK client around an already-created engine id.
     * 围绕已创建的 engine id 创建一个 SDK 客户端。
     */
    private constructor();
    /**
     * Create one native LuaSkills engine and wrap it in a high-level SDK client.
     * 创建一个原生 LuaSkills 引擎并封装为高级 SDK 客户端。
     */
    static create(options?: LuaSkillsClientOptions): LuaSkillsClient;
    /**
     * Query the JSON FFI version without creating a runtime engine.
     * 不创建运行时引擎并查询 JSON FFI 版本。
     */
    static version(options?: LuaSkillsSdkOptions): FfiVersionResult;
    /**
     * Query the JSON FFI self-description without creating a runtime engine.
     * 不创建运行时引擎并查询 JSON FFI 自描述。
     */
    static describe(options?: LuaSkillsSdkOptions): FfiDescribeResult;
    /**
     * Return one system-management namespace bound to a host-injected authority.
     * 返回绑定到宿主注入权限的 system 管理命名空间。
     */
    system(authority?: Authority | `${Authority}`): SystemSkillManagementClient;
    /**
     * Query the JSON FFI version through the current low-level bridge.
     * 通过当前底层桥查询 JSON FFI 版本。
     */
    version(): FfiVersionResult;
    /**
     * Query the JSON FFI self-description through the current low-level bridge.
     * 通过当前底层桥查询 JSON FFI 自描述。
     */
    describe(): FfiDescribeResult;
    /**
     * Load skills from legacy directory-style root options.
     * 从旧目录风格 root 选项加载 skills。
     */
    loadFromDirs(baseDir: string, overrideDir?: string | null): RuntimeAckResult;
    /**
     * Load skills from the formal ordered root chain.
     * 从正式有序 root 链加载 skills。
     */
    loadFromRoots(skillRoots: RuntimeSkillRoot[]): RuntimeAckResult;
    /**
     * Reload skills from legacy directory-style root options.
     * 从旧目录风格 root 选项重载 skills。
     */
    reloadFromDirs(baseDir: string, overrideDir?: string | null): RuntimeAckResult;
    /**
     * Reload skills from the formal ordered root chain.
     * 从正式有序 root 链重载 skills。
     */
    reloadFromRoots(skillRoots: RuntimeSkillRoot[]): RuntimeAckResult;
    /**
     * List runtime entries visible to the selected authority.
     * 列出指定权限可见的运行时入口。
     */
    listEntries(authority?: Authority | `${Authority}`): RuntimeEntryDescriptor[];
    /**
     * List runtime help trees visible to the selected authority.
     * 列出指定权限可见的运行时帮助树。
     */
    listSkillHelp(authority?: Authority | `${Authority}`): RuntimeSkillHelpDescriptor[];
    /**
     * Render one help flow detail visible to the selected authority.
     * 渲染指定权限可见的单个帮助流程详情。
     */
    renderSkillHelpDetail(skillId: string, flowName?: string, options?: RenderHelpOptions): RuntimeHelpDetail | null;
    /**
     * Query prompt argument completions visible to the selected authority.
     * 查询指定权限可见的 prompt 参数补全项。
     */
    promptArgumentCompletions(promptName: string, argumentName: string, authority?: Authority | `${Authority}`): string[] | null;
    /**
     * Return whether one canonical tool name is visible as a skill entry for the selected authority.
     * 返回指定 canonical 工具名对所选权限是否可见为 skill 入口。
     */
    isSkill(toolName: string, authority?: Authority | `${Authority}`): boolean;
    /**
     * Resolve the owning skill id for one visible canonical tool name.
     * 解析单个可见 canonical 工具名称所属的 skill id。
     */
    skillNameForTool(toolName: string, authority?: Authority | `${Authority}`): string | null;
    /**
     * Call one active skill entry by canonical tool name.
     * 按 canonical 工具名称调用单个已激活 skill 入口。
     */
    callSkill(toolName: string, args?: JsonValue, invocationContext?: LuaInvocationContext): RuntimeInvocationResult;
    /**
     * Execute one inline Lua snippet against the active runtime.
     * 针对当前活动运行时执行单段内联 Lua。
     */
    runLua<T = JsonValue>(code: string, args?: JsonValue, invocationContext?: LuaInvocationContext): T;
    /**
     * Release the native engine handle.
     * 释放原生引擎句柄。
     */
    close(): RuntimeAckResult | null;
    /**
     * Assert that the client still owns a live native engine handle.
     * 断言当前客户端仍持有存活的原生引擎句柄。
     */
    private assertOpen;
}
/**
 * Skill-config namespace backed by the unified runtime config store.
 * 基于统一运行时配置存储的 skill 配置命名空间。
 */
export declare class SkillConfigClient {
    private readonly client;
    /**
     * Create one skill-config namespace for a parent SDK client.
     * 为父级 SDK 客户端创建一个 skill 配置命名空间。
     */
    constructor(client: LuaSkillsClient);
    /**
     * List flattened config records, optionally limited to one skill id.
     * 列出扁平化配置记录，并可选限制到单个 skill id。
     */
    list(skillId?: string): SkillConfigEntry[];
    /**
     * Get one config value by skill id and key.
     * 按 skill id 与 key 获取单个配置值。
     */
    get(skillId: string, key: string): SkillConfigGetResult;
    /**
     * Set one config value by skill id and key.
     * 按 skill id 与 key 设置单个配置值。
     */
    set(skillId: string, key: string, value: string): SkillConfigMutationResult;
    /**
     * Delete one config value by skill id and key.
     * 按 skill id 与 key 删除单个配置值。
     */
    delete(skillId: string, key: string): SkillConfigMutationResult;
}
/**
 * Ordinary and system lifecycle namespace over the JSON FFI management entrypoints.
 * 覆盖 JSON FFI 管理入口的普通与 system 生命周期命名空间。
 */
export declare class SkillManagementClient {
    protected readonly client: LuaSkillsClient;
    private readonly systemPlane;
    private readonly authority;
    /**
     * Create one lifecycle namespace for a parent SDK client.
     * 为父级 SDK 客户端创建一个生命周期命名空间。
     */
    constructor(client: LuaSkillsClient, systemPlane: boolean, authority?: Authority | `${Authority}`);
    /**
     * Disable one skill through formal root-chain lifecycle state.
     * 通过正式 root 链生命周期状态停用单个 skill。
     */
    disable(skillRoots: RuntimeSkillRoot[], skillId: string, reason?: string | null): RuntimeAckResult;
    /**
     * Disable one skill through legacy directory-style roots.
     * 通过旧目录风格 roots 停用单个 skill。
     */
    disableInDirs(baseDir: string, skillId: string, overrideDir?: string | null, reason?: string | null): RuntimeAckResult;
    /**
     * Enable one skill through formal root-chain lifecycle state.
     * 通过正式 root 链生命周期状态启用单个 skill。
     */
    enable(skillRoots: RuntimeSkillRoot[], skillId: string): RuntimeAckResult;
    /**
     * Uninstall one skill and optionally clean its databases.
     * 卸载单个 skill，并可选清理其数据库。
     */
    uninstall(skillRoots: RuntimeSkillRoot[], skillId: string, options?: SkillUninstallOptions, lifecycleOptions?: SkillLifecycleOptions): SkillUninstallResult;
    /**
     * Install one managed skill through the current lifecycle namespace.
     * 通过当前生命周期命名空间安装单个受管 skill。
     */
    install(skillRoots: RuntimeSkillRoot[], request: SkillInstallRequest, lifecycleOptions?: SkillLifecycleOptions): SkillApplyResult;
    /**
     * Update one managed skill through the current lifecycle namespace.
     * 通过当前生命周期命名空间更新单个受管 skill。
     */
    update(skillRoots: RuntimeSkillRoot[], request: SkillInstallRequest, lifecycleOptions?: SkillLifecycleOptions): SkillApplyResult;
    /**
     * Build the concrete JSON FFI function name for the current namespace.
     * 为当前命名空间构造具体 JSON FFI 函数名称。
     */
    private functionName;
    /**
     * Build the authority payload required by system JSON FFI entrypoints.
     * 构造 system JSON FFI 入口要求的权限载荷。
     */
    private authorityPayload;
}
/**
 * System lifecycle namespace with host-injected authority.
 * 携带宿主注入权限的 system 生命周期命名空间。
 */
export declare class SystemSkillManagementClient extends SkillManagementClient {
    /**
     * Create one system lifecycle namespace for a parent SDK client.
     * 为父级 SDK 客户端创建一个 system 生命周期命名空间。
     */
    constructor(client: LuaSkillsClient, authority: Authority | `${Authority}`);
}
/**
 * Build complete engine options from SDK defaults and caller overrides.
 * 基于 SDK 默认值和调用方覆盖构造完整引擎选项。
 */
export declare function createEngineOptions(options?: LuaSkillsClientOptions): LuaEngineOptions;
/**
 * Return the SDK default VM pool configuration.
 * 返回 SDK 默认虚拟机池配置。
 */
export declare function defaultPoolConfig(): LuaVmPoolConfig;
/**
 * Return the SDK default host options for one runtime root.
 * 返回单个 runtime root 对应的 SDK 默认宿主选项。
 */
export declare function defaultHostOptions(runtimeRoot: string): LuaRuntimeHostOptions;
/**
 * Return the SDK default space-controller options.
 * 返回 SDK 默认 space-controller 选项。
 */
export declare function defaultSpaceControllerOptions(): LuaRuntimeSpaceControllerOptions;
