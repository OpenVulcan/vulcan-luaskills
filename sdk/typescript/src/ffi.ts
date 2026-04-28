import koffi from "koffi";
import { existsSync } from "node:fs";
import { resolve } from "node:path";
import type { JsonValue, LuaSkillsSdkOptions } from "./types.js";

/**
 * Owned buffer shape returned by the LuaSkills JSON FFI.
 * LuaSkills JSON FFI 返回的拥有型缓冲结构。
 */
interface FfiOwnedBuffer {
  /**
   * Native pointer exposed by koffi as an opaque pointer value.
   * koffi 以不透明指针值形式暴露的原生指针。
   */
  ptr: unknown | null;
  /**
   * Byte length of the pointed buffer.
   * 指针缓冲区的字节长度。
   */
  len: number | bigint;
}

/**
 * Borrowed buffer shape passed into the LuaSkills JSON FFI.
 * 传入 LuaSkills JSON FFI 的借用型缓冲结构。
 */
interface FfiBorrowedBuffer {
  /**
   * Native pointer backed by a live Node Buffer.
   * 由存活 Node Buffer 支撑的原生指针。
   */
  ptr: Buffer | null;
  /**
   * Byte length of the borrowed payload.
   * 借用载荷的字节长度。
   */
  len: number;
}

/**
 * Standard JSON response envelope produced by the Rust FFI layer.
 * Rust FFI 层生成的标准 JSON 响应包络。
 */
interface FfiJsonEnvelope<T> {
  /**
   * Whether the FFI call succeeded.
   * FFI 调用是否成功。
   */
  ok: boolean;
  /**
   * Successful result payload.
   * 成功结果载荷。
   */
  result?: T;
  /**
   * Error message returned by the runtime.
   * 运行时返回的错误消息。
   */
  error?: string;
}

/**
 * Function shape used by JSON FFI entrypoints that accept one borrowed JSON buffer.
 * 接收单个借用 JSON 缓冲的 JSON FFI 入口函数形状。
 */
type JsonInputFunction = (input: FfiBorrowedBuffer) => FfiOwnedBuffer;

/**
 * Function shape used by JSON FFI entrypoints that do not need input.
 * 不需要输入的 JSON FFI 入口函数形状。
 */
type JsonNoInputFunction = () => FfiOwnedBuffer;

/**
 * Host-side JSON provider callback implemented by SDK callers.
 * 由 SDK 调用方实现的宿主侧 JSON provider callback。
 */
export type JsonProviderCallback = (request: JsonValue) => JsonValue;

/**
 * Function shape used by luaskills_ffi_buffer_clone.
 * luaskills_ffi_buffer_clone 使用的函数形状。
 */
type BufferCloneFunction = (
  value: Buffer | null,
  len: number,
  bufferOut: unknown,
  errorOut: FfiOwnedBuffer,
) => number;

/**
 * Function shape used by JSON provider callback registration entrypoints.
 * JSON provider callback 注册入口使用的函数形状。
 */
type JsonProviderSetterFunction = (
  callback: koffi.IKoffiRegisteredCallback | null,
  userData: null,
  errorOut: FfiOwnedBuffer,
) => number;

/**
 * Native provider callback slot names managed by this bridge.
 * 当前桥接管理的原生 provider callback 槽位名称。
 */
type JsonProviderKind = "sqlite" | "lancedb";

/**
 * Module-level provider callback slot state matching native process-wide slots.
 * 与原生进程级槽位对齐的模块级 provider callback 槽位状态。
 */
interface JsonProviderSlotState {
  /**
   * Resolved library path owning the native slot.
   * 持有原生槽位的已解析动态库路径。
   */
  libraryPath: string;
  /**
   * SDK instance token that owns the current native callback registration.
   * 持有当前原生 callback 注册的 SDK 实例令牌。
   */
  ownerToken: symbol;
  /**
   * Registered Koffi callback kept alive while the native slot points to it.
   * 原生槽位指向期间保持存活的已注册 Koffi callback。
   */
  registeredCallback: koffi.IKoffiRegisteredCallback;
}

/**
 * Shared JSON provider slot registry for this Node.js process.
 * 当前 Node.js 进程内共享的 JSON provider 槽位注册表。
 */
const JSON_PROVIDER_SLOTS = new Map<string, JsonProviderSlotState>();

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
const JSON_PROVIDER_CALLBACK_TYPE = koffi.proto(
  "int32_t FfiJsonProviderCallback(FfiBorrowedBuffer request_json, void *user_data, FfiOwnedBuffer *response_out, FfiOwnedBuffer *error_out)",
);

/**
 * Error thrown when a LuaSkills FFI call returns an error envelope.
 * LuaSkills FFI 调用返回错误包络时抛出的错误。
 */
export class LuaSkillsError extends Error {
  /**
   * Name of the FFI function that failed.
   * 失败的 FFI 函数名称。
   */
  readonly functionName: string;

  /**
   * Create one SDK error from an FFI function name and runtime message.
   * 基于 FFI 函数名称与运行时消息创建一个 SDK 错误。
   */
  constructor(functionName: string, message: string) {
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
  private readonly library: ReturnType<typeof koffi.load>;

  /**
   * Resolved dynamic library path used by this bridge.
   * 当前桥接使用的已解析动态库路径。
   */
  private readonly libraryPath: string;

  /**
   * Koffi type descriptor for owned buffers.
   * 拥有型缓冲的 koffi 类型描述。
   */
  private readonly ownedBufferType: koffi.IKoffiCType;

  /**
   * Koffi type descriptor for borrowed buffers.
   * 借用型缓冲的 koffi 类型描述。
   */
  private readonly borrowedBufferType: koffi.IKoffiCType;

  /**
   * Koffi callback type descriptor for JSON provider callbacks.
   * JSON provider callback 的 koffi 回调类型描述。
   */
  private readonly jsonProviderCallbackType: koffi.IKoffiCType;

  /**
   * Native buffer-free function exported by LuaSkills.
   * LuaSkills 导出的原生缓冲释放函数。
   */
  private readonly freeBuffer: (value: FfiOwnedBuffer) => void;

  /**
   * Native buffer clone helper used by provider callback returns.
   * provider callback 返回值使用的原生缓冲克隆辅助函数。
   */
  private readonly cloneBuffer: BufferCloneFunction;

  /**
   * Unique owner token used to protect native callback slot cleanup.
   * 用于保护原生 callback 槽位清理的唯一 owner 令牌。
   */
  private readonly providerOwnerToken = Symbol("LuaSkillsJsonFfiProviderOwner");

  /**
   * Create one loaded JSON FFI bridge.
   * 创建一个已加载的 JSON FFI 桥。
   */
  constructor(options: LuaSkillsSdkOptions = {}) {
    const libraryPath = resolveLibraryPath(options.libraryPath);
    this.libraryPath = libraryPath;
    this.library = koffi.load(libraryPath);
    this.ownedBufferType = OWNED_BUFFER_TYPE;
    this.borrowedBufferType = BORROWED_BUFFER_TYPE;
    this.jsonProviderCallbackType = JSON_PROVIDER_CALLBACK_TYPE;
    this.freeBuffer = this.library.func("void luaskills_ffi_buffer_free(FfiOwnedBuffer value)") as (
      value: FfiOwnedBuffer,
    ) => void;
    this.cloneBuffer = this.library.func(
      "int32_t luaskills_ffi_buffer_clone(const void *value, size_t len, FfiOwnedBuffer *buffer_out, _Out_ FfiOwnedBuffer *error_out)",
    ) as BufferCloneFunction;
  }

  /**
   * Call a JSON FFI entrypoint that does not accept input.
   * 调用一个不接收输入的 JSON FFI 入口。
   */
  callJsonNoInput<T>(functionName: string): T {
    const fn = this.library.func(`FfiOwnedBuffer ${functionName}()`) as JsonNoInputFunction;
    const output = fn();
    return this.decodeEnvelope<T>(functionName, output);
  }

  /**
   * Call a JSON FFI entrypoint with one JSON payload.
   * 使用一个 JSON 载荷调用 JSON FFI 入口。
   */
  callJson<T>(functionName: string, payload: JsonValue | Record<string, unknown>): T {
    const fn = this.library.func(`FfiOwnedBuffer ${functionName}(FfiBorrowedBuffer input_json)`) as JsonInputFunction;
    const text = JSON.stringify(payload);
    const bytes = Buffer.from(text, "utf8");
    const input: FfiBorrowedBuffer = {
      ptr: bytes.length > 0 ? bytes : null,
      len: bytes.length,
    };
    const output = fn(input);
    return this.decodeEnvelope<T>(functionName, output);
  }

  /**
   * Register or clear the SQLite JSON provider callback.
   * 注册或清理 SQLite JSON provider callback。
   */
  setSqliteProviderJsonCallback(callback: JsonProviderCallback | null): void {
    this.setJsonProviderCallback(
      "sqlite",
      "luaskills_ffi_set_sqlite_provider_json_callback",
      callback,
    );
  }

  /**
   * Register or clear the LanceDB JSON provider callback.
   * 注册或清理 LanceDB JSON provider callback。
   */
  setLanceDbProviderJsonCallback(callback: JsonProviderCallback | null): void {
    this.setJsonProviderCallback(
      "lancedb",
      "luaskills_ffi_set_lancedb_provider_json_callback",
      callback,
    );
  }

  /**
   * Clear the SQLite JSON provider callback slot.
   * 清理 SQLite JSON provider callback 槽位。
   */
  clearSqliteProviderJsonCallback(): void {
    this.setSqliteProviderJsonCallback(null);
  }

  /**
   * Clear the LanceDB JSON provider callback slot.
   * 清理 LanceDB JSON provider callback 槽位。
   */
  clearLanceDbProviderJsonCallback(): void {
    this.setLanceDbProviderJsonCallback(null);
  }

  /**
   * Decode one owned FFI buffer into a typed JSON envelope and free it.
   * 将一个拥有型 FFI 缓冲解码为类型化 JSON 包络并释放它。
   */
  private decodeEnvelope<T>(functionName: string, output: FfiOwnedBuffer): T {
    const text = this.readOwnedBuffer(output);
    this.freeBuffer(output);
    const envelope = JSON.parse(text) as FfiJsonEnvelope<T>;
    if (!envelope.ok) {
      throw new LuaSkillsError(functionName, envelope.error ?? "Unknown LuaSkills FFI error");
    }
    return envelope.result as T;
  }

  /**
   * Read one owned FFI buffer into UTF-8 text without freeing it.
   * 将一个拥有型 FFI 缓冲读取为 UTF-8 文本但不释放它。
   */
  private readOwnedBuffer(output: FfiOwnedBuffer): string {
    if (!output.ptr || Number(output.len) === 0) {
      return "";
    }
    return Buffer.from(koffi.view(output.ptr, Number(output.len))).toString("utf8");
  }

  /**
   * Register or clear one concrete JSON provider callback slot.
   * 注册或清理一个具体 JSON provider callback 槽位。
   */
  private setJsonProviderCallback(
    kind: JsonProviderKind,
    functionName: string,
    callback: JsonProviderCallback | null,
  ): void {
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

    const registeredCallback = koffi.register(
      (requestJson: FfiBorrowedBuffer, _userData: unknown, responseOut: unknown, errorOut: unknown): number => {
        try {
          const request = this.parseBorrowedJson(requestJson);
          const response = callback(request);
          this.cloneOwnedBuffer(serializeProviderJson(response), responseOut);
          return 0;
        } catch (error) {
          try {
            this.cloneOwnedBuffer(Buffer.from(errorMessage(error), "utf8"), errorOut);
          } catch {
            // Callback boundaries must not throw into C.
            // callback 边界不能向 C 层抛出异常。
          }
          return 1;
        }
      },
      koffi.pointer(this.jsonProviderCallbackType),
    );

    try {
      this.callProviderSetter(functionName, registeredCallback);
    } catch (error) {
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
  private callProviderSetter(functionName: string, callback: koffi.IKoffiRegisteredCallback | null): void {
    const setter = this.library.func(
      `int32_t ${functionName}(FfiJsonProviderCallback *callback, void *user_data, _Out_ FfiOwnedBuffer *error_out)`,
    ) as JsonProviderSetterFunction;
    const errorOut = {} as FfiOwnedBuffer;
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
  private parseBorrowedJson(input: FfiBorrowedBuffer): JsonValue {
    if (!input.ptr || Number(input.len) === 0) {
      return null;
    }
    return JSON.parse(Buffer.from(koffi.view(input.ptr, Number(input.len))).toString("utf8")) as JsonValue;
  }

  /**
   * Clone one JavaScript-owned payload into one native owned buffer output.
   * 将单个 JavaScript 拥有的载荷克隆到一个原生拥有型缓冲输出。
   */
  private cloneOwnedBuffer(payload: Buffer, bufferOut: unknown): void {
    const errorOut = {} as FfiOwnedBuffer;
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
  private jsonProviderSlotKey(kind: JsonProviderKind): string {
    return `${this.libraryPath}:${kind}`;
  }
}

/**
 * Serialize one provider callback response into UTF-8 JSON bytes.
 * 将单个 provider callback 响应序列化为 UTF-8 JSON 字节。
 */
function serializeProviderJson(value: JsonValue | undefined): Buffer {
  return Buffer.from(JSON.stringify(value ?? null), "utf8");
}

/**
 * Convert an unknown thrown value into a stable error string.
 * 将未知抛出值转换为稳定错误字符串。
 */
function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * Resolve the dynamic library path from options or environment variables.
 * 从选项或环境变量解析动态库路径。
 */
export function resolveLibraryPath(explicitPath?: string): string {
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
