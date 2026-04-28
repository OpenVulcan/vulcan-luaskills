import type { JsonValue, LuaSkillsSdkOptions } from "./types.js";
/**
 * Host-side JSON provider callback implemented by SDK callers.
 * 由 SDK 调用方实现的宿主侧 JSON provider callback。
 */
export type JsonProviderCallback = (request: JsonValue) => JsonValue;
/**
 * Error thrown when a LuaSkills FFI call returns an error envelope.
 * LuaSkills FFI 调用返回错误包络时抛出的错误。
 */
export declare class LuaSkillsError extends Error {
    /**
     * Name of the FFI function that failed.
     * 失败的 FFI 函数名称。
     */
    readonly functionName: string;
    /**
     * Create one SDK error from an FFI function name and runtime message.
     * 基于 FFI 函数名称与运行时消息创建一个 SDK 错误。
     */
    constructor(functionName: string, message: string);
}
/**
 * Low-level JSON FFI bridge used by higher-level SDK clients.
 * 高层 SDK 客户端使用的底层 JSON FFI 桥。
 */
export declare class LuaSkillsJsonFfi {
    /**
     * Loaded koffi dynamic library handle.
     * 已加载的 koffi 动态库句柄。
     */
    private readonly library;
    /**
     * Resolved dynamic library path used by this bridge.
     * 当前桥接使用的已解析动态库路径。
     */
    private readonly libraryPath;
    /**
     * Koffi type descriptor for owned buffers.
     * 拥有型缓冲的 koffi 类型描述。
     */
    private readonly ownedBufferType;
    /**
     * Koffi type descriptor for borrowed buffers.
     * 借用型缓冲的 koffi 类型描述。
     */
    private readonly borrowedBufferType;
    /**
     * Koffi callback type descriptor for JSON provider callbacks.
     * JSON provider callback 的 koffi 回调类型描述。
     */
    private readonly jsonProviderCallbackType;
    /**
     * Native buffer-free function exported by LuaSkills.
     * LuaSkills 导出的原生缓冲释放函数。
     */
    private readonly freeBuffer;
    /**
     * Native buffer clone helper used by provider callback returns.
     * provider callback 返回值使用的原生缓冲克隆辅助函数。
     */
    private readonly cloneBuffer;
    /**
     * Unique owner token used to protect native callback slot cleanup.
     * 用于保护原生 callback 槽位清理的唯一 owner 令牌。
     */
    private readonly providerOwnerToken;
    /**
     * Create one loaded JSON FFI bridge.
     * 创建一个已加载的 JSON FFI 桥。
     */
    constructor(options?: LuaSkillsSdkOptions);
    /**
     * Call a JSON FFI entrypoint that does not accept input.
     * 调用一个不接收输入的 JSON FFI 入口。
     */
    callJsonNoInput<T>(functionName: string): T;
    /**
     * Call a JSON FFI entrypoint with one JSON payload.
     * 使用一个 JSON 载荷调用 JSON FFI 入口。
     */
    callJson<T>(functionName: string, payload: JsonValue | Record<string, unknown>): T;
    /**
     * Register or clear the SQLite JSON provider callback.
     * 注册或清理 SQLite JSON provider callback。
     */
    setSqliteProviderJsonCallback(callback: JsonProviderCallback | null): void;
    /**
     * Register or clear the LanceDB JSON provider callback.
     * 注册或清理 LanceDB JSON provider callback。
     */
    setLanceDbProviderJsonCallback(callback: JsonProviderCallback | null): void;
    /**
     * Clear the SQLite JSON provider callback slot.
     * 清理 SQLite JSON provider callback 槽位。
     */
    clearSqliteProviderJsonCallback(): void;
    /**
     * Clear the LanceDB JSON provider callback slot.
     * 清理 LanceDB JSON provider callback 槽位。
     */
    clearLanceDbProviderJsonCallback(): void;
    /**
     * Decode one owned FFI buffer into a typed JSON envelope and free it.
     * 将一个拥有型 FFI 缓冲解码为类型化 JSON 包络并释放它。
     */
    private decodeEnvelope;
    /**
     * Read one owned FFI buffer into UTF-8 text without freeing it.
     * 将一个拥有型 FFI 缓冲读取为 UTF-8 文本但不释放它。
     */
    private readOwnedBuffer;
    /**
     * Register or clear one concrete JSON provider callback slot.
     * 注册或清理一个具体 JSON provider callback 槽位。
     */
    private setJsonProviderCallback;
    /**
     * Call one provider callback setter and surface any native error.
     * 调用单个 provider callback setter 并暴露原生错误。
     */
    private callProviderSetter;
    /**
     * Parse one borrowed JSON buffer passed by the native provider bridge.
     * 解析原生 provider 桥传入的单个借用 JSON 缓冲。
     */
    private parseBorrowedJson;
    /**
     * Clone one JavaScript-owned payload into one native owned buffer output.
     * 将单个 JavaScript 拥有的载荷克隆到一个原生拥有型缓冲输出。
     */
    private cloneOwnedBuffer;
    /**
     * Build a process-local provider slot key for one library path and provider kind.
     * 为单个动态库路径和 provider 类型构造进程内 provider 槽位键。
     */
    private jsonProviderSlotKey;
}
/**
 * Resolve the dynamic library path from options or environment variables.
 * 从选项或环境变量解析动态库路径。
 */
export declare function resolveLibraryPath(explicitPath?: string): string;
