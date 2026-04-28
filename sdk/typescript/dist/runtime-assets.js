import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { existsSync, readFileSync } from "node:fs";
import { chmod, cp, mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, relative, resolve } from "node:path";
import { spawn } from "node:child_process";
/**
 * Default LuaSkills release tag used by SDK runtime installation.
 * SDK 运行时安装使用的默认 LuaSkills 发布标签。
 */
export const DEFAULT_LUASKILLS_VERSION = "v0.2.2";
/**
 * Default vldb-controller release tag used by SDK runtime installation.
 * SDK 运行时安装使用的默认 vldb-controller 发布标签。
 */
export const DEFAULT_VLDB_CONTROLLER_VERSION = "v0.2.1";
/**
 * Default vldb-sqlite release tag used by SDK runtime installation.
 * SDK 运行时安装使用的默认 vldb-sqlite 发布标签。
 */
export const DEFAULT_VLDB_SQLITE_VERSION = "v0.1.5";
/**
 * Default vldb-lancedb release tag used by SDK runtime installation.
 * SDK 运行时安装使用的默认 vldb-lancedb 发布标签。
 */
export const DEFAULT_VLDB_LANCEDB_VERSION = "v0.1.5";
/**
 * Manifest file name written into the runtime resources directory.
 * 写入 runtime resources 目录的清单文件名。
 */
export const RUNTIME_MANIFEST_FILE_NAME = "luaskills-sdk-runtime-manifest.json";
/**
 * Database integration preset selected by SDK users.
 * SDK 用户选择的数据库集成预设。
 */
export var RuntimeDatabasePreset;
(function (RuntimeDatabasePreset) {
    /**
     * Do not install or configure database providers.
     * 不安装也不配置数据库 provider。
     */
    RuntimeDatabasePreset["None"] = "none";
    /**
     * Use the shared vldb-controller executable through space_controller mode.
     * 通过 space_controller 模式使用共享 vldb-controller 可执行文件。
     */
    RuntimeDatabasePreset["VldbController"] = "vldb-controller";
    /**
     * Use vldb-sqlite-lib and vldb-lancedb-lib dynamic libraries directly.
     * 直接使用 vldb-sqlite-lib 与 vldb-lancedb-lib 动态库。
     */
    RuntimeDatabasePreset["VldbDirect"] = "vldb-direct";
    /**
     * Let the host provide JSON callbacks instead of native VLDB assets.
     * 由宿主提供 JSON callback，而不是安装原生 VLDB 资产。
     */
    RuntimeDatabasePreset["HostCallback"] = "host-callback";
})(RuntimeDatabasePreset || (RuntimeDatabasePreset = {}));
/**
 * Return the runtime platform target for the current Node.js process.
 * 返回当前 Node.js 进程对应的运行时平台目标。
 */
export function resolveRuntimePlatformTarget(platform = process.platform, arch = process.arch) {
    if (platform === "win32" && arch === "x64") {
        return {
            platform_key: "windows-x64",
            target_triple: "x86_64-pc-windows-msvc",
            archive_ext: ".zip",
            controller_binary_name: "vldb-controller.exe",
            dynamic_library_ext: ".dll",
            luaskills_library_name: "luaskills.dll",
            sqlite_library_name: "vldb_sqlite.dll",
            lancedb_library_name: "vldb_lancedb.dll",
        };
    }
    if (platform === "darwin" && arch === "x64") {
        return darwinTarget("x86_64", "macos-x64");
    }
    if (platform === "darwin" && arch === "arm64") {
        return darwinTarget("aarch64", "macos-arm64");
    }
    if (platform === "linux" && arch === "x64") {
        return linuxTarget("x86_64", "linux-x64");
    }
    if (platform === "linux" && arch === "arm64") {
        return linuxTarget("aarch64", "linux-arm64");
    }
    throw new Error(`Unsupported runtime platform: ${platform}/${arch}`);
}
/**
 * Build one deterministic runtime installation manifest.
 * 构造一个确定性的运行时安装清单。
 */
export function buildRuntimeInstallManifest(options) {
    const runtimeRoot = resolve(options.runtimeRoot);
    const database = normalizeDatabasePreset(options.database ?? RuntimeDatabasePreset.None);
    const platform = resolveRuntimePlatformTarget();
    const assets = buildRuntimeAssetDescriptors({ ...options, database, runtimeRoot }, platform);
    return {
        schema_version: 1,
        generated_at: new Date().toISOString(),
        runtime_root: runtimeRoot,
        database_mode: database,
        platform,
        assets,
        host_options_patch: buildHostOptionsPatch(runtimeRoot, database, platform, assets),
    };
}
/**
 * Install native runtime assets and write the shared manifest.
 * 安装原生运行时资产并写入共享清单。
 */
export async function installRuntimeAssets(options) {
    const manifest = buildRuntimeInstallManifest(options);
    await ensureRuntimeDirectories(manifest.runtime_root);
    const temporaryRoot = join(tmpdir(), `luaskills-runtime-assets-${process.pid}-${Date.now()}`);
    await mkdir(temporaryRoot, { recursive: true });
    try {
        for (const asset of manifest.assets) {
            await installOneAsset(manifest.runtime_root, asset, temporaryRoot, manifest.platform);
        }
        const refreshedManifest = refreshHostOptionsPatch(manifest);
        await writeRuntimeInstallManifest(refreshedManifest);
        return refreshedManifest;
    }
    finally {
        await rm(temporaryRoot, { recursive: true, force: true });
    }
}
/**
 * Write one runtime install manifest into the runtime resources directory.
 * 将单个运行时安装清单写入 runtime resources 目录。
 */
export async function writeRuntimeInstallManifest(manifest) {
    const manifestPath = runtimeManifestPath(manifest.runtime_root);
    await mkdir(resolve(manifest.runtime_root, "resources"), { recursive: true });
    await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
    return manifestPath;
}
/**
 * Load one runtime install manifest from the runtime resources directory.
 * 从 runtime resources 目录加载单个运行时安装清单。
 */
export async function loadRuntimeInstallManifest(runtimeRoot) {
    const manifestPath = runtimeManifestPath(runtimeRoot);
    try {
        const raw = await readFile(manifestPath, "utf8");
        return JSON.parse(raw);
    }
    catch (error) {
        if (error.code === "ENOENT") {
            return null;
        }
        throw error;
    }
}
/**
 * Load one runtime install manifest synchronously when SDK defaults need it.
 * 在 SDK 默认值需要时同步加载单个运行时安装清单。
 */
export function loadRuntimeInstallManifestSync(runtimeRoot) {
    const manifestPath = runtimeManifestPath(runtimeRoot);
    try {
        return JSON.parse(readFileSync(manifestPath, "utf8"));
    }
    catch (error) {
        if (error.code === "ENOENT") {
            return null;
        }
        throw error;
    }
}
/**
 * Return the absolute runtime manifest path for one runtime root.
 * 返回单个 runtime root 对应的绝对运行时清单路径。
 */
export function runtimeManifestPath(runtimeRoot) {
    return resolve(runtimeRoot, "resources", RUNTIME_MANIFEST_FILE_NAME);
}
/**
 * Convert one runtime manifest into host option overrides.
 * 将单个运行时清单转换为宿主选项覆盖。
 */
export function hostOptionsFromRuntimeManifest(manifest) {
    return { ...manifest.host_options_patch };
}
/**
 * Resolve an installed LuaSkills dynamic library from one runtime root.
 * 从单个 runtime root 解析已安装的 LuaSkills 动态库。
 */
export function resolveLuaSkillsLibraryPathFromRuntime(runtimeRoot, platform = resolveRuntimePlatformTarget()) {
    const libsDir = resolve(runtimeRoot, "libs");
    const candidates = luaSkillsLibraryCandidates(platform);
    for (const candidate of candidates) {
        const candidatePath = resolve(libsDir, candidate);
        if (existsSync(candidatePath)) {
            return candidatePath;
        }
    }
    return null;
}
/**
 * Normalize one database preset string.
 * 归一化单个数据库预设字符串。
 */
export function normalizeDatabasePreset(value) {
    if (value === RuntimeDatabasePreset.None) {
        return RuntimeDatabasePreset.None;
    }
    if (value === RuntimeDatabasePreset.VldbController) {
        return RuntimeDatabasePreset.VldbController;
    }
    if (value === RuntimeDatabasePreset.VldbDirect) {
        return RuntimeDatabasePreset.VldbDirect;
    }
    if (value === RuntimeDatabasePreset.HostCallback) {
        return RuntimeDatabasePreset.HostCallback;
    }
    throw new Error(`Unsupported database preset: ${value}`);
}
/**
 * Build one macOS runtime platform descriptor.
 * 构造单个 macOS 运行时平台描述。
 */
function darwinTarget(archPrefix, platformKey) {
    return {
        platform_key: platformKey,
        target_triple: `${archPrefix}-apple-darwin`,
        archive_ext: ".tar.gz",
        controller_binary_name: "vldb-controller",
        dynamic_library_ext: ".dylib",
        luaskills_library_name: "libluaskills.dylib",
        sqlite_library_name: "libvldb_sqlite.dylib",
        lancedb_library_name: "libvldb_lancedb.dylib",
    };
}
/**
 * Build one Linux runtime platform descriptor.
 * 构造单个 Linux 运行时平台描述。
 */
function linuxTarget(archPrefix, platformKey) {
    return {
        platform_key: platformKey,
        target_triple: `${archPrefix}-unknown-linux-gnu`,
        archive_ext: ".tar.gz",
        controller_binary_name: "vldb-controller",
        dynamic_library_ext: ".so",
        luaskills_library_name: "libluaskills.so",
        sqlite_library_name: "libvldb_sqlite.so",
        lancedb_library_name: "libvldb_lancedb.so",
    };
}
/**
 * Build every asset descriptor required by one manifest.
 * 构造单个清单所需的全部资产描述。
 */
function buildRuntimeAssetDescriptors(options, platform) {
    const assets = [];
    if (options.includeLuaSkillsFfi ?? true) {
        const assetName = `luaskills-ffi-sdk-${platform.platform_key}.tar.gz`;
        assets.push(releaseAsset("luaskills_ffi", options.luaskillsRepo ?? "LuaSkills/luaskills", options.luaskillsVersion ?? DEFAULT_LUASKILLS_VERSION, assetName, `libs/${platform.luaskills_library_name}`));
    }
    if (options.database === RuntimeDatabasePreset.VldbController) {
        const assetName = `vldb-controller-${options.vldbControllerVersion ?? DEFAULT_VLDB_CONTROLLER_VERSION}-${platform.target_triple}${platform.archive_ext}`;
        assets.push(releaseAsset("vldb_controller", options.vldbControllerRepo ?? "OpenVulcan/vldb-controller", options.vldbControllerVersion ?? DEFAULT_VLDB_CONTROLLER_VERSION, assetName, `bin/${platform.controller_binary_name}`));
    }
    if (options.database === RuntimeDatabasePreset.VldbDirect) {
        const sqliteAsset = `vldb-sqlite-lib-${options.vldbSqliteVersion ?? DEFAULT_VLDB_SQLITE_VERSION}-${platform.target_triple}${platform.archive_ext}`;
        const lancedbAsset = `vldb-lancedb-lib-${options.vldbLancedbVersion ?? DEFAULT_VLDB_LANCEDB_VERSION}-${platform.target_triple}${platform.archive_ext}`;
        assets.push(releaseAsset("vldb_sqlite_lib", options.vldbSqliteRepo ?? "OpenVulcan/vldb-sqlite", options.vldbSqliteVersion ?? DEFAULT_VLDB_SQLITE_VERSION, sqliteAsset, `libs/${platform.sqlite_library_name}`));
        assets.push(releaseAsset("vldb_lancedb_lib", options.vldbLancedbRepo ?? "OpenVulcan/vldb-lancedb", options.vldbLancedbVersion ?? DEFAULT_VLDB_LANCEDB_VERSION, lancedbAsset, `libs/${platform.lancedb_library_name}`));
    }
    return assets;
}
/**
 * Build one release asset descriptor from exact naming inputs.
 * 从精确命名输入构造单个发布资产描述。
 */
function releaseAsset(role, repository, version, assetName, installedPath) {
    const encodedAssetName = encodeURIComponent(assetName);
    const baseUrl = `https://github.com/${repository}/releases/download/${version}/${encodedAssetName}`;
    return {
        role,
        repository,
        version,
        asset_name: assetName,
        sha256_asset_name: `${assetName}.sha256`,
        download_url: baseUrl,
        sha256_url: `${baseUrl}.sha256`,
        installed_path: installedPath,
    };
}
/**
 * Return candidate LuaSkills dynamic library names for one platform.
 * 返回单个平台对应的 LuaSkills 动态库候选名称。
 */
function luaSkillsLibraryCandidates(platform) {
    const names = [platform.luaskills_library_name];
    if (platform.dynamic_library_ext === ".dll") {
        names.push("libluaskills.dll");
    }
    else if (platform.dynamic_library_ext === ".dylib") {
        names.push("luaskills.dylib");
    }
    else {
        names.push("luaskills.so");
    }
    return [...new Set(names)];
}
/**
 * Build host option overrides for one database mode.
 * 为单个数据库模式构造宿主选项覆盖。
 */
function buildHostOptionsPatch(runtimeRoot, database, platform, assets) {
    if (database === RuntimeDatabasePreset.HostCallback) {
        return {
            sqlite_provider_mode: "host_callback",
            sqlite_callback_mode: "json",
            lancedb_provider_mode: "host_callback",
            lancedb_callback_mode: "json",
        };
    }
    if (database === RuntimeDatabasePreset.VldbController) {
        return {
            sqlite_provider_mode: "space_controller",
            lancedb_provider_mode: "space_controller",
            space_controller: {
                endpoint: null,
                auto_spawn: true,
                executable_path: resolve(runtimeRoot, "bin", platform.controller_binary_name),
                process_mode: "managed",
                minimum_uptime_secs: 300,
                idle_timeout_secs: 900,
                default_lease_ttl_secs: 120,
                connect_timeout_secs: 5,
                startup_timeout_secs: 15,
                startup_retry_interval_ms: 250,
                lease_renew_interval_secs: 30,
            },
        };
    }
    if (database === RuntimeDatabasePreset.VldbDirect) {
        return {
            sqlite_library_path: resolveInstalledAsset(runtimeRoot, assets, "vldb_sqlite_lib"),
            sqlite_provider_mode: "dynamic_library",
            lancedb_library_path: resolveInstalledAsset(runtimeRoot, assets, "vldb_lancedb_lib"),
            lancedb_provider_mode: "dynamic_library",
        };
    }
    return {};
}
/**
 * Resolve the absolute path for one installed asset role.
 * 解析单个已安装资产角色对应的绝对路径。
 */
function resolveInstalledAsset(runtimeRoot, assets, role) {
    const asset = assets.find((candidate) => candidate.role === role);
    return asset?.installed_path ? resolve(runtimeRoot, asset.installed_path) : null;
}
/**
 * Convert one absolute installed path into a manifest-relative path.
 * 将单个绝对安装路径转换为清单相对路径。
 */
function relativeInstalledPath(runtimeRoot, installedPath) {
    return relative(resolve(runtimeRoot), installedPath).replace(/\\/g, "/");
}
/**
 * Ensure runtime directories used by SDK-managed assets exist.
 * 确保 SDK 管理资产使用的 runtime 目录存在。
 */
async function ensureRuntimeDirectories(runtimeRoot) {
    await mkdir(resolve(runtimeRoot, "bin"), { recursive: true });
    await mkdir(resolve(runtimeRoot, "libs"), { recursive: true });
    await mkdir(resolve(runtimeRoot, "include"), { recursive: true });
    await mkdir(resolve(runtimeRoot, "licenses"), { recursive: true });
    await mkdir(resolve(runtimeRoot, "resources"), { recursive: true });
}
/**
 * Download, verify, extract, and install one asset.
 * 下载、校验、解压并安装单个资产。
 */
async function installOneAsset(runtimeRoot, asset, temporaryRoot, platform) {
    const assetDirectory = join(temporaryRoot, asset.role);
    const archivePath = join(assetDirectory, asset.asset_name);
    const extractDirectory = join(assetDirectory, "extract");
    await mkdir(assetDirectory, { recursive: true });
    const sha256Text = await downloadText(asset.sha256_url);
    await downloadFile(asset.download_url, archivePath);
    await verifySha256(archivePath, sha256Text);
    await mkdir(extractDirectory, { recursive: true });
    await extractArchive(archivePath, extractDirectory);
    if (asset.role === "luaskills_ffi") {
        await installLuaSkillsFfi(runtimeRoot, extractDirectory, platform, asset);
    }
    else if (asset.role === "vldb_controller") {
        await installController(runtimeRoot, extractDirectory, platform, asset);
    }
    else if (asset.role === "vldb_sqlite_lib") {
        await installDynamicLibrary(runtimeRoot, extractDirectory, platform, "sqlite", asset);
    }
    else if (asset.role === "vldb_lancedb_lib") {
        await installDynamicLibrary(runtimeRoot, extractDirectory, platform, "lancedb", asset);
    }
}
/**
 * Download one UTF-8 text file.
 * 下载单个 UTF-8 文本文件。
 */
async function downloadText(url) {
    const response = await fetch(url);
    if (!response.ok) {
        throw new Error(`Failed to download ${url}: ${response.status} ${response.statusText}`);
    }
    return response.text();
}
/**
 * Download one binary file to disk.
 * 将单个二进制文件下载到磁盘。
 */
async function downloadFile(url, destination) {
    const response = await fetch(url);
    if (!response.ok) {
        throw new Error(`Failed to download ${url}: ${response.status} ${response.statusText}`);
    }
    const buffer = Buffer.from(await response.arrayBuffer());
    await writeFile(destination, buffer);
}
/**
 * Verify one downloaded archive against a SHA-256 sidecar.
 * 使用 SHA-256 旁路文件校验单个已下载归档。
 */
async function verifySha256(filePath, sha256Text) {
    const expectedHash = sha256Text.trim().split(/\s+/)[0]?.toLowerCase();
    if (!expectedHash) {
        throw new Error(`Invalid SHA-256 sidecar for ${filePath}`);
    }
    const actualHash = await fileSha256(filePath);
    if (actualHash !== expectedHash) {
        throw new Error(`SHA-256 mismatch for ${filePath}: expected ${expectedHash}, got ${actualHash}`);
    }
}
/**
 * Compute the SHA-256 hash for one file.
 * 计算单个文件的 SHA-256 哈希。
 */
async function fileSha256(filePath) {
    const hash = createHash("sha256");
    await new Promise((resolvePromise, rejectPromise) => {
        const stream = createReadStream(filePath);
        stream.on("data", (chunk) => hash.update(chunk));
        stream.on("error", rejectPromise);
        stream.on("end", resolvePromise);
    });
    return hash.digest("hex");
}
/**
 * Extract one archive with the platform tar implementation.
 * 使用平台 tar 实现解压单个归档。
 */
async function extractArchive(archivePath, destination) {
    await runProcess("tar", ["-xf", archivePath, "-C", destination]);
}
/**
 * Install a LuaSkills FFI SDK archive into runtime include/libs/licenses directories.
 * 将 LuaSkills FFI SDK 归档安装到 runtime include/libs/licenses 目录。
 */
async function installLuaSkillsFfi(runtimeRoot, extractDirectory, platform, asset) {
    await copyDirectoryIfPresent(join(extractDirectory, "include"), resolve(runtimeRoot, "include"));
    await copyDirectoryIfPresent(join(extractDirectory, "lib"), resolve(runtimeRoot, "libs"));
    await copyDirectoryIfPresent(join(extractDirectory, "licenses"), resolve(runtimeRoot, "licenses", "luaskills-ffi"));
    const installedPath = resolveLuaSkillsLibraryPathFromRuntime(runtimeRoot, platform);
    if (!installedPath) {
        throw new Error(`LuaSkills dynamic library was not found after installing ${asset.asset_name}`);
    }
    asset.installed_path = relativeInstalledPath(runtimeRoot, installedPath);
}
/**
 * Install vldb-controller into the runtime bin directory.
 * 将 vldb-controller 安装到 runtime bin 目录。
 */
async function installController(runtimeRoot, extractDirectory, platform, asset) {
    const source = await findFile(extractDirectory, (candidate) => candidate === platform.controller_binary_name);
    if (!source) {
        throw new Error(`${platform.controller_binary_name} was not found in ${asset.asset_name}`);
    }
    const destination = resolve(runtimeRoot, "bin", platform.controller_binary_name);
    await cp(source, destination, { force: true });
    await chmod(destination, 0o755).catch(() => undefined);
    asset.installed_path = `bin/${platform.controller_binary_name}`;
}
/**
 * Install one VLDB dynamic library into the runtime libs directory.
 * 将单个 VLDB 动态库安装到 runtime libs 目录。
 */
async function installDynamicLibrary(runtimeRoot, extractDirectory, platform, nameHint, asset) {
    const source = await findFile(extractDirectory, (candidate) => candidate.endsWith(platform.dynamic_library_ext) && candidate.toLowerCase().includes(nameHint));
    if (!source) {
        throw new Error(`Dynamic library for ${asset.role} was not found in ${asset.asset_name}`);
    }
    const destinationName = source.split(/[\\/]/).pop() ?? (nameHint === "sqlite" ? platform.sqlite_library_name : platform.lancedb_library_name);
    const destination = resolve(runtimeRoot, "libs", destinationName);
    await cp(source, destination, { force: true });
    asset.installed_path = `libs/${destinationName}`;
}
/**
 * Copy one directory only when it exists.
 * 仅在目录存在时复制单个目录。
 */
async function copyDirectoryIfPresent(source, destination) {
    try {
        await cp(source, destination, { recursive: true, force: true });
    }
    catch (error) {
        if (error.code !== "ENOENT") {
            throw error;
        }
    }
}
/**
 * Find one file under a directory by base-name predicate.
 * 根据基础文件名谓词在目录下查找单个文件。
 */
async function findFile(root, predicate) {
    const entries = await readdir(root, { withFileTypes: true });
    for (const entry of entries) {
        const fullPath = join(root, entry.name);
        if (entry.isDirectory()) {
            const nested = await findFile(fullPath, predicate);
            if (nested) {
                return nested;
            }
        }
        else if (entry.isFile() && predicate(entry.name)) {
            return fullPath;
        }
    }
    return null;
}
/**
 * Run one child process and reject when it fails.
 * 运行单个子进程，并在失败时拒绝。
 */
async function runProcess(command, args) {
    await new Promise((resolvePromise, rejectPromise) => {
        const child = spawn(command, args, { stdio: "inherit" });
        child.on("error", rejectPromise);
        child.on("exit", (code) => {
            if (code === 0) {
                resolvePromise();
            }
            else {
                rejectPromise(new Error(`${command} exited with code ${code}`));
            }
        });
    });
}
/**
 * Refresh host option paths after extraction may have discovered exact library names.
 * 在解压发现精确动态库名称后刷新宿主选项路径。
 */
function refreshHostOptionsPatch(manifest) {
    return {
        ...manifest,
        host_options_patch: buildHostOptionsPatch(manifest.runtime_root, normalizeDatabasePreset(manifest.database_mode), manifest.platform, manifest.assets),
    };
}
//# sourceMappingURL=runtime-assets.js.map