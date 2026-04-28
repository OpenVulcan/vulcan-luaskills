import koffi from "koffi";
import { existsSync } from "node:fs";
import { resolve } from "node:path";
/**
 * Shared JSON provider slot registry for this Node.js process.
 * 当前 Node.js 进程内共享的 JSON provider 槽位注册表。
 */
const JSON_PROVIDER_SLOTS = new Map();
/**
 * Shared Koffi type descriptor for LuaSkills owned buffers.
 * LuaSkills 拥有型缓冲的共享 Koffi 类型描述。
 */
const OWNED_BUFFER_TYPE = koffi.struct("FfiOwnedBuffer", {
    ptr: "void *",
    len: "size_t",
});
/**
 * Shared Koffi type descriptor for LuaSkills borrowed buffers.
 * LuaSkills 借用型缓冲的共享 Koffi 类型描述。
 */
const BORROWED_BUFFER_TYPE = koffi.struct("FfiBorrowedBuffer", {
    ptr: "void *",
    len: "size_t",
});
/**
 * Shared Koffi callback type descriptor for JSON provider callbacks.
 * JSON provider callback 的共享 Koffi 回调类型描述。
 */
const JSON_PROVIDER_CALLBACK_TYPE = koffi.proto("int32_t FfiJsonProviderCallback(FfiBorrowedBuffer request_json, void *user_data, FfiOwnedBuffer *response_out, FfiOwnedBuffer *error_out)");
/**
 * Error thrown when a LuaSkills FFI call returns an error envelope.
 * LuaSkills FFI 调用返回错误包络时抛出的错误。
 */
export class LuaSkillsError extends Error {
    /**
     * Name of the FFI function that failed.
     * 失败的 FFI 函数名称。
     */
    functionName;
    /**
     * Create one SDK error from an FFI function name and runtime message.
     * 基于 FFI 函数名称与运行时消息创建一个 SDK 错误。
     */
    constructor(functionName, message) {
        super(`${functionName}: ${message}`);
        this.name = "LuaSkillsError";
        this.functionName = functionName;
    }
}
/**
 * Low-level JSON FFI bridge used by higher-level SDK clients.
 * 高层 SDK 客户端使用的底层 JSON FFI 桥。
 */
export class LuaSkillsJsonFfi {
    /**
     * Loaded koffi dynamic library handle.
     * 已加载的 koffi 动态库句柄。
     */
    library;
    /**
     * Resolved dynamic library path used by this bridge.
     * 当前桥接使用的已解析动态库路径。
     */
    libraryPath;
    /**
     * Koffi type descriptor for owned buffers.
     * 拥有型缓冲的 koffi 类型描述。
     */
    ownedBufferType;
    /**
     * Koffi type descriptor for borrowed buffers.
     * 借用型缓冲的 koffi 类型描述。
     */
    borrowedBufferType;
    /**
     * Koffi callback type descriptor for JSON provider callbacks.
     * JSON provider callback 的 koffi 回调类型描述。
     */
    jsonProviderCallbackType;
    /**
     * Native buffer-free function exported by LuaSkills.
     * LuaSkills 导出的原生缓冲释放函数。
     */
    freeBuffer;
    /**
     * Native buffer clone helper used by provider callback returns.
     * provider callback 返回值使用的原生缓冲克隆辅助函数。
     */
    cloneBuffer;
    /**
     * Unique owner token used to protect native callback slot cleanup.
     * 用于保护原生 callback 槽位清理的唯一 owner 令牌。
     */
    providerOwnerToken = Symbol("LuaSkillsJsonFfiProviderOwner");
    /**
     * Create one loaded JSON FFI bridge.
     * 创建一个已加载的 JSON FFI 桥。
     */
    constructor(options = {}) {
        const libraryPath = resolveLibraryPath(options.libraryPath);
        this.libraryPath = libraryPath;
        this.library = koffi.load(libraryPath);
        this.ownedBufferType = OWNED_BUFFER_TYPE;
        this.borrowedBufferType = BORROWED_BUFFER_TYPE;
        this.jsonProviderCallbackType = JSON_PROVIDER_CALLBACK_TYPE;
        this.freeBuffer = this.library.func("void luaskills_ffi_buffer_free(FfiOwnedBuffer value)");
        this.cloneBuffer = this.library.func("int32_t luaskills_ffi_buffer_clone(const void *value, size_t len, FfiOwnedBuffer *buffer_out, _Out_ FfiOwnedBuffer *error_out)");
    }
    /**
     * Call a JSON FFI entrypoint that does not accept input.
     * 调用一个不接收输入的 JSON FFI 入口。
     */
    callJsonNoInput(functionName) {
        const fn = this.library.func(`FfiOwnedBuffer ${functionName}()`);
        const output = fn();
        return this.decodeEnvelope(functionName, output);
    }
    /**
     * Call a JSON FFI entrypoint with one JSON payload.
     * 使用一个 JSON 载荷调用 JSON FFI 入口。
     */
    callJson(functionName, payload) {
        const fn = this.library.func(`FfiOwnedBuffer ${functionName}(FfiBorrowedBuffer input_json)`);
        const text = JSON.stringify(payload);
        const bytes = Buffer.from(text, "utf8");
        const input = {
            ptr: bytes.length > 0 ? bytes : null,
            len: bytes.length,
        };
        const output = fn(input);
        return this.decodeEnvelope(functionName, output);
    }
    /**
     * Register or clear the SQLite JSON provider callback.
     * 注册或清理 SQLite JSON provider callback。
     */
    setSqliteProviderJsonCallback(callback) {
        this.setJsonProviderCallback("sqlite", "luaskills_ffi_set_sqlite_provider_json_callback", callback);
    }
    /**
     * Register or clear the LanceDB JSON provider callback.
     * 注册或清理 LanceDB JSON provider callback。
     */
    setLanceDbProviderJsonCallback(callback) {
        this.setJsonProviderCallback("lancedb", "luaskills_ffi_set_lancedb_provider_json_callback", callback);
    }
    /**
     * Clear the SQLite JSON provider callback slot.
     * 清理 SQLite JSON provider callback 槽位。
     */
    clearSqliteProviderJsonCallback() {
        this.setSqliteProviderJsonCallback(null);
    }
    /**
     * Clear the LanceDB JSON provider callback slot.
     * 清理 LanceDB JSON provider callback 槽位。
     */
    clearLanceDbProviderJsonCallback() {
        this.setLanceDbProviderJsonCallback(null);
    }
    /**
     * Decode one owned FFI buffer into a typed JSON envelope and free it.
     * 将一个拥有型 FFI 缓冲解码为类型化 JSON 包络并释放它。
     */
    decodeEnvelope(functionName, output) {
        const text = this.readOwnedBuffer(output);
        this.freeBuffer(output);
        const envelope = JSON.parse(text);
        if (!envelope.ok) {
            throw new LuaSkillsError(functionName, envelope.error ?? "Unknown LuaSkills FFI error");
        }
        return envelope.result;
    }
    /**
     * Read one owned FFI buffer into UTF-8 text without freeing it.
     * 将一个拥有型 FFI 缓冲读取为 UTF-8 文本但不释放它。
     */
    readOwnedBuffer(output) {
        if (!output.ptr || Number(output.len) === 0) {
            return "";
        }
        return Buffer.from(koffi.view(output.ptr, Number(output.len))).toString("utf8");
    }
    /**
     * Register or clear one concrete JSON provider callback slot.
     * 注册或清理一个具体 JSON provider callback 槽位。
     */
    setJsonProviderCallback(kind, functionName, callback) {
        const slotKey = this.jsonProviderSlotKey(kind);
        const previousSlot = JSON_PROVIDER_SLOTS.get(slotKey);
        if (!callback) {
            if (previousSlot?.ownerToken !== this.providerOwnerToken) {
                return;
            }
            this.callProviderSetter(functionName, null);
            koffi.unregister(previousSlot.registeredCallback);
            JSON_PROVIDER_SLOTS.delete(slotKey);
            return;
        }
        const registeredCallback = koffi.register((requestJson, _userData, responseOut, errorOut) => {
            try {
                const request = this.parseBorrowedJson(requestJson);
                const response = callback(request);
                this.cloneOwnedBuffer(serializeProviderJson(response), responseOut);
                return 0;
            }
            catch (error) {
                try {
                    this.cloneOwnedBuffer(Buffer.from(errorMessage(error), "utf8"), errorOut);
                }
                catch {
                    // Callback boundaries must not throw into C.
                    // callback 边界不能向 C 层抛出异常。
                }
                return 1;
            }
        }, koffi.pointer(this.jsonProviderCallbackType));
        try {
            this.callProviderSetter(functionName, registeredCallback);
        }
        catch (error) {
            koffi.unregister(registeredCallback);
            throw error;
        }
        if (previousSlot) {
            koffi.unregister(previousSlot.registeredCallback);
        }
        JSON_PROVIDER_SLOTS.set(slotKey, {
            libraryPath: this.libraryPath,
            ownerToken: this.providerOwnerToken,
            registeredCallback,
        });
    }
    /**
     * Call one provider callback setter and surface any native error.
     * 调用单个 provider callback setter 并暴露原生错误。
     */
    callProviderSetter(functionName, callback) {
        const setter = this.library.func(`int32_t ${functionName}(FfiJsonProviderCallback *callback, void *user_data, _Out_ FfiOwnedBuffer *error_out)`);
        const errorOut = {};
        const status = setter(callback, null, errorOut);
        if (status === 0) {
            if (errorOut.ptr) {
                this.freeBuffer(errorOut);
            }
            return;
        }
        const message = this.readOwnedBuffer(errorOut) || "Unknown provider callback registration error";
        if (errorOut.ptr) {
            this.freeBuffer(errorOut);
        }
        throw new LuaSkillsError(functionName, message);
    }
    /**
     * Parse one borrowed JSON buffer passed by the native provider bridge.
     * 解析原生 provider 桥传入的单个借用 JSON 缓冲。
     */
    parseBorrowedJson(input) {
        if (!input.ptr || Number(input.len) === 0) {
            return null;
        }
        return JSON.parse(Buffer.from(koffi.view(input.ptr, Number(input.len))).toString("utf8"));
    }
    /**
     * Clone one JavaScript-owned payload into one native owned buffer output.
     * 将单个 JavaScript 拥有的载荷克隆到一个原生拥有型缓冲输出。
     */
    cloneOwnedBuffer(payload, bufferOut) {
        const errorOut = {};
        const status = this.cloneBuffer(payload.length > 0 ? payload : null, payload.length, bufferOut, errorOut);
        if (status === 0) {
            return;
        }
        const message = this.readOwnedBuffer(errorOut) || "Unknown buffer clone error";
        if (errorOut.ptr) {
            this.freeBuffer(errorOut);
        }
        throw new LuaSkillsError("luaskills_ffi_buffer_clone", message);
    }
    /**
     * Build a process-local provider slot key for one library path and provider kind.
     * 为单个动态库路径和 provider 类型构造进程内 provider 槽位键。
     */
    jsonProviderSlotKey(kind) {
        return `${this.libraryPath}:${kind}`;
    }
}
/**
 * Serialize one provider callback response into UTF-8 JSON bytes.
 * 将单个 provider callback 响应序列化为 UTF-8 JSON 字节。
 */
function serializeProviderJson(value) {
    return Buffer.from(JSON.stringify(value ?? null), "utf8");
}
/**
 * Convert an unknown thrown value into a stable error string.
 * 将未知抛出值转换为稳定错误字符串。
 */
function errorMessage(error) {
    return error instanceof Error ? error.message : String(error);
}
/**
 * Resolve the dynamic library path from options or environment variables.
 * 从选项或环境变量解析动态库路径。
 */
export function resolveLibraryPath(explicitPath) {
    const selectedPath = explicitPath ?? process.env.LUASKILLS_LIB;
    if (!selectedPath) {
        throw new Error("LuaSkills library path is required; pass libraryPath or set LUASKILLS_LIB");
    }
    const absolutePath = resolve(selectedPath);
    if (!existsSync(absolutePath)) {
        throw new Error(`LuaSkills library not found: ${absolutePath}`);
    }
    return absolutePath;
}
//# sourceMappingURL=ffi.js.map