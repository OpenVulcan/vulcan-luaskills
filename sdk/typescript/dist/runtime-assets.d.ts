import type { LuaRuntimeHostOptions } from "./types.js";
/**
 * Default LuaSkills release tag used by SDK runtime installation.
 * SDK 运行时安装使用的默认 LuaSkills 发布标签。
 */
export declare const DEFAULT_LUASKILLS_VERSION = "v0.2.2";
/**
 * Default vldb-controller release tag used by SDK runtime installation.
 * SDK 运行时安装使用的默认 vldb-controller 发布标签。
 */
export declare const DEFAULT_VLDB_CONTROLLER_VERSION = "v0.2.1";
/**
 * Default vldb-sqlite release tag used by SDK runtime installation.
 * SDK 运行时安装使用的默认 vldb-sqlite 发布标签。
 */
export declare const DEFAULT_VLDB_SQLITE_VERSION = "v0.1.5";
/**
 * Default vldb-lancedb release tag used by SDK runtime installation.
 * SDK 运行时安装使用的默认 vldb-lancedb 发布标签。
 */
export declare const DEFAULT_VLDB_LANCEDB_VERSION = "v0.1.5";
/**
 * Manifest file name written into the runtime resources directory.
 * 写入 runtime resources 目录的清单文件名。
 */
export declare const RUNTIME_MANIFEST_FILE_NAME = "luaskills-sdk-runtime-manifest.json";
/**
 * Database integration preset selected by SDK users.
 * SDK 用户选择的数据库集成预设。
 */
export declare enum RuntimeDatabasePreset {
    /**
     * Do not install or configure database providers.
     * 不安装也不配置数据库 provider。
     */
    None = "none",
    /**
     * Use the shared vldb-controller executable through space_controller mode.
     * 通过 space_controller 模式使用共享 vldb-controller 可执行文件。
     */
    VldbController = "vldb-controller",
    /**
     * Use vldb-sqlite-lib and vldb-lancedb-lib dynamic libraries directly.
     * 直接使用 vldb-sqlite-lib 与 vldb-lancedb-lib 动态库。
     */
    VldbDirect = "vldb-direct",
    /**
     * Let the host provide JSON callbacks instead of native VLDB assets.
     * 由宿主提供 JSON callback，而不是安装原生 VLDB 资产。
     */
    HostCallback = "host-callback"
}
/**
 * Runtime asset role inside one installation manifest.
 * 安装清单中的运行时资产角色。
 */
export type RuntimeAssetRole = "luaskills_ffi" | "vldb_controller" | "vldb_sqlite_lib" | "vldb_lancedb_lib";
/**
 * Supported platform descriptor used by release asset names.
 * 发布资产命名使用的受支持平台描述。
 */
export interface RuntimePlatformTarget {
    /**
     * LuaSkills platform key used by luaskills-ffi-sdk archives.
     * luaskills-ffi-sdk 归档使用的 LuaSkills 平台标识。
     */
    platform_key: string;
    /**
     * Rust-style target triple used by VLDB release archives.
     * VLDB 发布归档使用的 Rust 风格 target triple。
     */
    target_triple: string;
    /**
     * Archive extension used by this platform.
     * 当前平台使用的归档扩展名。
     */
    archive_ext: ".tar.gz" | ".zip";
    /**
     * vldb-controller executable file name inside the archive.
     * 归档内的 vldb-controller 可执行文件名。
     */
    controller_binary_name: string;
    /**
     * Dynamic library file extension used by this platform.
     * 当前平台使用的动态库文件扩展名。
     */
    dynamic_library_ext: ".dll" | ".so" | ".dylib";
    /**
     * Expected LuaSkills dynamic library file name after installation.
     * 安装后的预期 LuaSkills 动态库文件名。
     */
    luaskills_library_name: string;
    /**
     * Expected SQLite dynamic library file name after installation.
     * 安装后的预期 SQLite 动态库文件名。
     */
    sqlite_library_name: string;
    /**
     * Expected LanceDB dynamic library file name after installation.
     * 安装后的预期 LanceDB 动态库文件名。
     */
    lancedb_library_name: string;
}
/**
 * One GitHub Release asset needed by an SDK runtime installation.
 * SDK 运行时安装所需的单个 GitHub Release 资产。
 */
export interface RuntimeAssetDescriptor {
    /**
     * Logical asset role.
     * 逻辑资产角色。
     */
    role: RuntimeAssetRole;
    /**
     * GitHub repository in owner/name form.
     * owner/name 形式的 GitHub 仓库。
     */
    repository: string;
    /**
     * Release tag used by this asset.
     * 当前资产使用的发布标签。
     */
    version: string;
    /**
     * Exact release asset file name.
     * 精确的发布资产文件名。
     */
    asset_name: string;
    /**
     * Exact SHA-256 sidecar asset file name.
     * 精确的 SHA-256 旁路校验资产文件名。
     */
    sha256_asset_name: string;
    /**
     * Browser download URL for the archive.
     * 归档的浏览器下载地址。
     */
    download_url: string;
    /**
     * Browser download URL for the SHA-256 sidecar.
     * SHA-256 旁路文件的浏览器下载地址。
     */
    sha256_url: string;
    /**
     * Relative path where the installed executable or library should live.
     * 已安装可执行文件或动态库应位于的相对路径。
     */
    installed_path: string | null;
}
/**
 * Runtime installation manifest shared by all SDK languages.
 * 所有 SDK 语言共享的运行时安装清单。
 */
export interface RuntimeInstallManifest {
    /**
     * Manifest schema version.
     * 清单结构版本。
     */
    schema_version: 1;
    /**
     * ISO timestamp when the manifest was generated.
     * 生成清单时的 ISO 时间戳。
     */
    generated_at: string;
    /**
     * Absolute runtime root represented by the manifest.
     * 清单表示的绝对 runtime root。
     */
    runtime_root: string;
    /**
     * Selected database integration mode.
     * 选中的数据库集成模式。
     */
    database_mode: RuntimeDatabasePreset | `${RuntimeDatabasePreset}`;
    /**
     * Platform target used by every asset in this manifest.
     * 当前清单中所有资产使用的平台目标。
     */
    platform: RuntimePlatformTarget;
    /**
     * Assets required by the selected mode.
     * 选中模式所需的资产列表。
     */
    assets: RuntimeAssetDescriptor[];
    /**
     * Host option patch derived from installed runtime assets.
     * 从已安装运行时资产派生的宿主选项补丁。
     */
    host_options_patch: Partial<LuaRuntimeHostOptions>;
}
/**
 * Options used to build or install one SDK runtime asset set.
 * 构造或安装一组 SDK 运行时资产使用的选项。
 */
export interface RuntimeInstallOptions {
    /**
     * Runtime root that receives native assets and the manifest.
     * 接收原生资产与清单的 runtime root。
     */
    runtimeRoot: string;
    /**
     * Selected database integration mode.
     * 选中的数据库集成模式。
     */
    database?: RuntimeDatabasePreset | `${RuntimeDatabasePreset}`;
    /**
     * LuaSkills release tag.
     * LuaSkills 发布标签。
     */
    luaskillsVersion?: string;
    /**
     * vldb-controller release tag.
     * vldb-controller 发布标签。
     */
    vldbControllerVersion?: string;
    /**
     * vldb-sqlite release tag.
     * vldb-sqlite 发布标签。
     */
    vldbSqliteVersion?: string;
    /**
     * vldb-lancedb release tag.
     * vldb-lancedb 发布标签。
     */
    vldbLancedbVersion?: string;
    /**
     * Whether the LuaSkills FFI SDK archive should be included.
     * 是否包含 LuaSkills FFI SDK 归档。
     */
    includeLuaSkillsFfi?: boolean;
    /**
     * GitHub repository that publishes LuaSkills assets.
     * 发布 LuaSkills 资产的 GitHub 仓库。
     */
    luaskillsRepo?: string;
    /**
     * GitHub repository that publishes vldb-controller assets.
     * 发布 vldb-controller 资产的 GitHub 仓库。
     */
    vldbControllerRepo?: string;
    /**
     * GitHub repository that publishes vldb-sqlite assets.
     * 发布 vldb-sqlite 资产的 GitHub 仓库。
     */
    vldbSqliteRepo?: string;
    /**
     * GitHub repository that publishes vldb-lancedb assets.
     * 发布 vldb-lancedb 资产的 GitHub 仓库。
     */
    vldbLancedbRepo?: string;
}
/**
 * Return the runtime platform target for the current Node.js process.
 * 返回当前 Node.js 进程对应的运行时平台目标。
 */
export declare function resolveRuntimePlatformTarget(platform?: NodeJS.Platform, arch?: NodeJS.Architecture): RuntimePlatformTarget;
/**
 * Build one deterministic runtime installation manifest.
 * 构造一个确定性的运行时安装清单。
 */
export declare function buildRuntimeInstallManifest(options: RuntimeInstallOptions): RuntimeInstallManifest;
/**
 * Install native runtime assets and write the shared manifest.
 * 安装原生运行时资产并写入共享清单。
 */
export declare function installRuntimeAssets(options: RuntimeInstallOptions): Promise<RuntimeInstallManifest>;
/**
 * Write one runtime install manifest into the runtime resources directory.
 * 将单个运行时安装清单写入 runtime resources 目录。
 */
export declare function writeRuntimeInstallManifest(manifest: RuntimeInstallManifest): Promise<string>;
/**
 * Load one runtime install manifest from the runtime resources directory.
 * 从 runtime resources 目录加载单个运行时安装清单。
 */
export declare function loadRuntimeInstallManifest(runtimeRoot: string): Promise<RuntimeInstallManifest | null>;
/**
 * Load one runtime install manifest synchronously when SDK defaults need it.
 * 在 SDK 默认值需要时同步加载单个运行时安装清单。
 */
export declare function loadRuntimeInstallManifestSync(runtimeRoot: string): RuntimeInstallManifest | null;
/**
 * Return the absolute runtime manifest path for one runtime root.
 * 返回单个 runtime root 对应的绝对运行时清单路径。
 */
export declare function runtimeManifestPath(runtimeRoot: string): string;
/**
 * Convert one runtime manifest into host option overrides.
 * 将单个运行时清单转换为宿主选项覆盖。
 */
export declare function hostOptionsFromRuntimeManifest(manifest: RuntimeInstallManifest): Partial<LuaRuntimeHostOptions>;
/**
 * Resolve an installed LuaSkills dynamic library from one runtime root.
 * 从单个 runtime root 解析已安装的 LuaSkills 动态库。
 */
export declare function resolveLuaSkillsLibraryPathFromRuntime(runtimeRoot: string, platform?: RuntimePlatformTarget): string | null;
/**
 * Normalize one database preset string.
 * 归一化单个数据库预设字符串。
 */
export declare function normalizeDatabasePreset(value: RuntimeDatabasePreset | `${RuntimeDatabasePreset}`): RuntimeDatabasePreset;
