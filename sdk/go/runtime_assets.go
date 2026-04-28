package luaskills

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"time"
)

// DefaultLuaSkillsVersion is the release tag used by SDK runtime installation.
// DefaultLuaSkillsVersion 是 SDK 运行时安装使用的 LuaSkills 发布标签。
const DefaultLuaSkillsVersion = "v0.2.2"

// DefaultVldbControllerVersion is the release tag used by SDK runtime installation.
// DefaultVldbControllerVersion 是 SDK 运行时安装使用的 vldb-controller 发布标签。
const DefaultVldbControllerVersion = "v0.2.1"

// DefaultVldbSQLiteVersion is the release tag used by SDK runtime installation.
// DefaultVldbSQLiteVersion 是 SDK 运行时安装使用的 vldb-sqlite 发布标签。
const DefaultVldbSQLiteVersion = "v0.1.5"

// DefaultVldbLanceDBVersion is the release tag used by SDK runtime installation.
// DefaultVldbLanceDBVersion 是 SDK 运行时安装使用的 vldb-lancedb 发布标签。
const DefaultVldbLanceDBVersion = "v0.1.5"

// RuntimeManifestFileName is the manifest name stored under runtime resources.
// RuntimeManifestFileName 是存放在 runtime resources 下的清单文件名。
const RuntimeManifestFileName = "luaskills-sdk-runtime-manifest.json"

// RuntimeDatabasePreset is one SDK-level database integration mode.
// RuntimeDatabasePreset 是单个 SDK 级数据库集成模式。
type RuntimeDatabasePreset string

const (
	// RuntimeDatabaseNone does not install or configure database providers.
	// RuntimeDatabaseNone 不安装也不配置数据库 provider。
	RuntimeDatabaseNone RuntimeDatabasePreset = "none"
	// RuntimeDatabaseVldbController uses vldb-controller through space_controller mode.
	// RuntimeDatabaseVldbController 通过 space_controller 模式使用 vldb-controller。
	RuntimeDatabaseVldbController RuntimeDatabasePreset = "vldb-controller"
	// RuntimeDatabaseVldbDirect uses vldb-sqlite-lib and vldb-lancedb-lib directly.
	// RuntimeDatabaseVldbDirect 直接使用 vldb-sqlite-lib 与 vldb-lancedb-lib。
	RuntimeDatabaseVldbDirect RuntimeDatabasePreset = "vldb-direct"
	// RuntimeDatabaseHostCallback lets the host provide JSON callbacks.
	// RuntimeDatabaseHostCallback 由宿主提供 JSON callback。
	RuntimeDatabaseHostCallback RuntimeDatabasePreset = "host-callback"
)

// RuntimeAssetRole is a logical role for one release asset.
// RuntimeAssetRole 是单个发布资产的逻辑角色。
type RuntimeAssetRole string

const (
	// RuntimeAssetLuaSkillsFFI identifies the LuaSkills FFI SDK archive.
	// RuntimeAssetLuaSkillsFFI 标识 LuaSkills FFI SDK 归档。
	RuntimeAssetLuaSkillsFFI RuntimeAssetRole = "luaskills_ffi"
	// RuntimeAssetVldbController identifies the vldb-controller executable archive.
	// RuntimeAssetVldbController 标识 vldb-controller 可执行文件归档。
	RuntimeAssetVldbController RuntimeAssetRole = "vldb_controller"
	// RuntimeAssetVldbSQLiteLib identifies the vldb-sqlite dynamic library archive.
	// RuntimeAssetVldbSQLiteLib 标识 vldb-sqlite 动态库归档。
	RuntimeAssetVldbSQLiteLib RuntimeAssetRole = "vldb_sqlite_lib"
	// RuntimeAssetVldbLanceDBLib identifies the vldb-lancedb dynamic library archive.
	// RuntimeAssetVldbLanceDBLib 标识 vldb-lancedb 动态库归档。
	RuntimeAssetVldbLanceDBLib RuntimeAssetRole = "vldb_lancedb_lib"
)

// RuntimePlatformTarget describes release asset naming for one platform.
// RuntimePlatformTarget 描述单个平台的发布资产命名。
type RuntimePlatformTarget struct {
	// PlatformKey is the LuaSkills platform key used by FFI SDK archives.
	// PlatformKey 是 FFI SDK 归档使用的 LuaSkills 平台标识。
	PlatformKey string `json:"platform_key"`
	// TargetTriple is the Rust-style target triple used by VLDB archives.
	// TargetTriple 是 VLDB 归档使用的 Rust 风格 target triple。
	TargetTriple string `json:"target_triple"`
	// ArchiveExt is the archive extension used by this platform.
	// ArchiveExt 是当前平台使用的归档扩展名。
	ArchiveExt string `json:"archive_ext"`
	// ControllerBinaryName is the vldb-controller executable file name.
	// ControllerBinaryName 是 vldb-controller 可执行文件名。
	ControllerBinaryName string `json:"controller_binary_name"`
	// DynamicLibraryExt is the dynamic library extension used by this platform.
	// DynamicLibraryExt 是当前平台使用的动态库扩展名。
	DynamicLibraryExt string `json:"dynamic_library_ext"`
	// LuaSkillsLibraryName is the expected installed LuaSkills library name.
	// LuaSkillsLibraryName 是预期安装后的 LuaSkills 动态库名称。
	LuaSkillsLibraryName string `json:"luaskills_library_name"`
	// SQLiteLibraryName is the expected installed SQLite library name.
	// SQLiteLibraryName 是预期安装后的 SQLite 动态库名称。
	SQLiteLibraryName string `json:"sqlite_library_name"`
	// LanceDBLibraryName is the expected installed LanceDB library name.
	// LanceDBLibraryName 是预期安装后的 LanceDB 动态库名称。
	LanceDBLibraryName string `json:"lancedb_library_name"`
}

// RuntimeAssetDescriptor describes one GitHub Release asset.
// RuntimeAssetDescriptor 描述单个 GitHub Release 资产。
type RuntimeAssetDescriptor struct {
	// Role is the logical asset role.
	// Role 是逻辑资产角色。
	Role RuntimeAssetRole `json:"role"`
	// Repository is the GitHub repository in owner/name form.
	// Repository 是 owner/name 形式的 GitHub 仓库。
	Repository string `json:"repository"`
	// Version is the release tag used by this asset.
	// Version 是当前资产使用的发布标签。
	Version string `json:"version"`
	// AssetName is the exact release asset file name.
	// AssetName 是精确的发布资产文件名。
	AssetName string `json:"asset_name"`
	// SHA256AssetName is the exact SHA-256 sidecar asset file name.
	// SHA256AssetName 是精确的 SHA-256 旁路资产文件名。
	SHA256AssetName string `json:"sha256_asset_name"`
	// DownloadURL is the browser download URL for the archive.
	// DownloadURL 是归档的浏览器下载地址。
	DownloadURL string `json:"download_url"`
	// SHA256URL is the browser download URL for the SHA-256 sidecar.
	// SHA256URL 是 SHA-256 旁路文件的浏览器下载地址。
	SHA256URL string `json:"sha256_url"`
	// InstalledPath is the relative installed executable or library path.
	// InstalledPath 是已安装可执行文件或动态库的相对路径。
	InstalledPath *string `json:"installed_path"`
}

// RuntimeInstallManifest is the shared SDK runtime installation manifest.
// RuntimeInstallManifest 是共享 SDK 运行时安装清单。
type RuntimeInstallManifest struct {
	// SchemaVersion is the manifest schema version.
	// SchemaVersion 是清单结构版本。
	SchemaVersion int `json:"schema_version"`
	// GeneratedAt is the UTC timestamp when the manifest was generated.
	// GeneratedAt 是清单生成时的 UTC 时间戳。
	GeneratedAt string `json:"generated_at"`
	// RuntimeRoot is the runtime root represented by the manifest.
	// RuntimeRoot 是清单表示的 runtime root。
	RuntimeRoot string `json:"runtime_root"`
	// DatabaseMode is the selected database integration mode.
	// DatabaseMode 是选中的数据库集成模式。
	DatabaseMode RuntimeDatabasePreset `json:"database_mode"`
	// Platform is the platform target used by manifest assets.
	// Platform 是清单资产使用的平台目标。
	Platform RuntimePlatformTarget `json:"platform"`
	// Assets are required by the selected runtime mode.
	// Assets 是选中运行时模式所需的资产。
	Assets []RuntimeAssetDescriptor `json:"assets"`
	// HostOptionsPatch is derived from installed runtime assets.
	// HostOptionsPatch 是从已安装运行时资产派生的宿主选项补丁。
	HostOptionsPatch map[string]any `json:"host_options_patch"`
}

// RuntimeInstallOptions controls runtime asset planning.
// RuntimeInstallOptions 控制运行时资产规划。
type RuntimeInstallOptions struct {
	// RuntimeRoot receives native assets and the manifest.
	// RuntimeRoot 接收原生资产与清单。
	RuntimeRoot string
	// Database selects the SDK-level database integration mode.
	// Database 选择 SDK 级数据库集成模式。
	Database RuntimeDatabasePreset
	// LuaSkillsVersion is the LuaSkills release tag.
	// LuaSkillsVersion 是 LuaSkills 发布标签。
	LuaSkillsVersion string
	// VldbControllerVersion is the vldb-controller release tag.
	// VldbControllerVersion 是 vldb-controller 发布标签。
	VldbControllerVersion string
	// VldbSQLiteVersion is the vldb-sqlite release tag.
	// VldbSQLiteVersion 是 vldb-sqlite 发布标签。
	VldbSQLiteVersion string
	// VldbLanceDBVersion is the vldb-lancedb release tag.
	// VldbLanceDBVersion 是 vldb-lancedb 发布标签。
	VldbLanceDBVersion string
	// SkipLuaSkillsFFI omits the LuaSkills FFI SDK archive from the manifest.
	// SkipLuaSkillsFFI 从清单中省略 LuaSkills FFI SDK 归档。
	SkipLuaSkillsFFI bool
	// LuaSkillsRepo is the GitHub repository that publishes LuaSkills assets.
	// LuaSkillsRepo 是发布 LuaSkills 资产的 GitHub 仓库。
	LuaSkillsRepo string
	// VldbControllerRepo is the GitHub repository that publishes vldb-controller assets.
	// VldbControllerRepo 是发布 vldb-controller 资产的 GitHub 仓库。
	VldbControllerRepo string
	// VldbSQLiteRepo is the GitHub repository that publishes vldb-sqlite assets.
	// VldbSQLiteRepo 是发布 vldb-sqlite 资产的 GitHub 仓库。
	VldbSQLiteRepo string
	// VldbLanceDBRepo is the GitHub repository that publishes vldb-lancedb assets.
	// VldbLanceDBRepo 是发布 vldb-lancedb 资产的 GitHub 仓库。
	VldbLanceDBRepo string
}

// ResolveRuntimePlatformTarget returns the release target for the current Go process.
// ResolveRuntimePlatformTarget 返回当前 Go 进程对应的发布目标。
func ResolveRuntimePlatformTarget() (RuntimePlatformTarget, error) {
	return ResolveRuntimePlatformTargetFor(runtime.GOOS, runtime.GOARCH)
}

// ResolveRuntimePlatformTargetFor returns the release target for explicit platform values.
// ResolveRuntimePlatformTargetFor 返回显式平台值对应的发布目标。
func ResolveRuntimePlatformTargetFor(goos string, goarch string) (RuntimePlatformTarget, error) {
	if goos == "windows" && goarch == "amd64" {
		return RuntimePlatformTarget{
			PlatformKey:          "windows-x64",
			TargetTriple:         "x86_64-pc-windows-msvc",
			ArchiveExt:           ".zip",
			ControllerBinaryName: "vldb-controller.exe",
			DynamicLibraryExt:    ".dll",
			LuaSkillsLibraryName: "luaskills.dll",
			SQLiteLibraryName:    "vldb_sqlite.dll",
			LanceDBLibraryName:   "vldb_lancedb.dll",
		}, nil
	}
	if goos == "darwin" && goarch == "amd64" {
		return darwinRuntimeTarget("x86_64", "macos-x64"), nil
	}
	if goos == "darwin" && goarch == "arm64" {
		return darwinRuntimeTarget("aarch64", "macos-arm64"), nil
	}
	if goos == "linux" && goarch == "amd64" {
		return linuxRuntimeTarget("x86_64", "linux-x64"), nil
	}
	if goos == "linux" && goarch == "arm64" {
		return linuxRuntimeTarget("aarch64", "linux-arm64"), nil
	}
	return RuntimePlatformTarget{}, fmt.Errorf("unsupported runtime platform: %s/%s", goos, goarch)
}

// BuildRuntimeInstallManifest builds one deterministic runtime installation manifest.
// BuildRuntimeInstallManifest 构造一个确定性的运行时安装清单。
func BuildRuntimeInstallManifest(options RuntimeInstallOptions) (*RuntimeInstallManifest, error) {
	target, err := ResolveRuntimePlatformTarget()
	if err != nil {
		return nil, err
	}
	normalized := normalizeRuntimeInstallOptions(options)
	assets := buildRuntimeAssetDescriptors(normalized, target)
	manifest := &RuntimeInstallManifest{
		SchemaVersion:    1,
		GeneratedAt:      time.Now().UTC().Format(time.RFC3339),
		RuntimeRoot:      normalizePath(normalized.RuntimeRoot),
		DatabaseMode:     normalized.Database,
		Platform:         target,
		Assets:           assets,
		HostOptionsPatch: buildRuntimeHostOptionsPatch(normalized.RuntimeRoot, normalized.Database, target, assets),
	}
	return manifest, nil
}

// HostOptionsFromRuntimeManifest converts one runtime manifest into host option overrides.
// HostOptionsFromRuntimeManifest 将单个运行时清单转换为宿主选项覆盖。
func HostOptionsFromRuntimeManifest(manifest *RuntimeInstallManifest) map[string]any {
	if manifest == nil {
		return map[string]any{}
	}
	return mergeMaps(map[string]any{}, manifest.HostOptionsPatch)
}

// LoadRuntimeInstallManifest reads one SDK runtime install manifest from a runtime root.
// LoadRuntimeInstallManifest 从单个 runtime root 读取 SDK 运行时安装清单。
func LoadRuntimeInstallManifest(runtimeRoot string) (*RuntimeInstallManifest, error) {
	manifestPath := RuntimeManifestPath(runtimeRoot)
	raw, err := os.ReadFile(manifestPath)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}
	var manifest RuntimeInstallManifest
	if err := json.Unmarshal(raw, &manifest); err != nil {
		return nil, err
	}
	return &manifest, nil
}

// RuntimeManifestPath returns the expected manifest path under one runtime root.
// RuntimeManifestPath 返回单个 runtime root 下的预期清单路径。
func RuntimeManifestPath(runtimeRoot string) string {
	return filepath.Join(runtimeRoot, "resources", RuntimeManifestFileName)
}

// normalizeRuntimeInstallOptions fills default release repositories and versions.
// normalizeRuntimeInstallOptions 填充默认发布仓库与版本。
func normalizeRuntimeInstallOptions(options RuntimeInstallOptions) RuntimeInstallOptions {
	if options.Database == "" {
		options.Database = RuntimeDatabaseNone
	}
	if options.LuaSkillsVersion == "" {
		options.LuaSkillsVersion = DefaultLuaSkillsVersion
	}
	if options.VldbControllerVersion == "" {
		options.VldbControllerVersion = DefaultVldbControllerVersion
	}
	if options.VldbSQLiteVersion == "" {
		options.VldbSQLiteVersion = DefaultVldbSQLiteVersion
	}
	if options.VldbLanceDBVersion == "" {
		options.VldbLanceDBVersion = DefaultVldbLanceDBVersion
	}
	if options.LuaSkillsRepo == "" {
		options.LuaSkillsRepo = "LuaSkills/luaskills"
	}
	if options.VldbControllerRepo == "" {
		options.VldbControllerRepo = "OpenVulcan/vldb-controller"
	}
	if options.VldbSQLiteRepo == "" {
		options.VldbSQLiteRepo = "OpenVulcan/vldb-sqlite"
	}
	if options.VldbLanceDBRepo == "" {
		options.VldbLanceDBRepo = "OpenVulcan/vldb-lancedb"
	}
	return options
}

// darwinRuntimeTarget builds one macOS runtime platform descriptor.
// darwinRuntimeTarget 构造单个 macOS 运行时平台描述。
func darwinRuntimeTarget(archPrefix string, platformKey string) RuntimePlatformTarget {
	return RuntimePlatformTarget{
		PlatformKey:          platformKey,
		TargetTriple:         archPrefix + "-apple-darwin",
		ArchiveExt:           ".tar.gz",
		ControllerBinaryName: "vldb-controller",
		DynamicLibraryExt:    ".dylib",
		LuaSkillsLibraryName: "libluaskills.dylib",
		SQLiteLibraryName:    "libvldb_sqlite.dylib",
		LanceDBLibraryName:   "libvldb_lancedb.dylib",
	}
}

// linuxRuntimeTarget builds one Linux runtime platform descriptor.
// linuxRuntimeTarget 构造单个 Linux 运行时平台描述。
func linuxRuntimeTarget(archPrefix string, platformKey string) RuntimePlatformTarget {
	return RuntimePlatformTarget{
		PlatformKey:          platformKey,
		TargetTriple:         archPrefix + "-unknown-linux-gnu",
		ArchiveExt:           ".tar.gz",
		ControllerBinaryName: "vldb-controller",
		DynamicLibraryExt:    ".so",
		LuaSkillsLibraryName: "libluaskills.so",
		SQLiteLibraryName:    "libvldb_sqlite.so",
		LanceDBLibraryName:   "libvldb_lancedb.so",
	}
}

// buildRuntimeAssetDescriptors builds every asset required by one manifest.
// buildRuntimeAssetDescriptors 构造单个清单所需的全部资产。
func buildRuntimeAssetDescriptors(options RuntimeInstallOptions, target RuntimePlatformTarget) []RuntimeAssetDescriptor {
	assets := []RuntimeAssetDescriptor{}
	if !options.SkipLuaSkillsFFI {
		assetName := fmt.Sprintf("luaskills-ffi-sdk-%s.tar.gz", target.PlatformKey)
		assets = append(assets, releaseRuntimeAsset(RuntimeAssetLuaSkillsFFI, options.LuaSkillsRepo, options.LuaSkillsVersion, assetName, stringPtr("libs/"+target.LuaSkillsLibraryName)))
	}
	if options.Database == RuntimeDatabaseVldbController {
		assetName := fmt.Sprintf("vldb-controller-%s-%s%s", options.VldbControllerVersion, target.TargetTriple, target.ArchiveExt)
		assets = append(assets, releaseRuntimeAsset(RuntimeAssetVldbController, options.VldbControllerRepo, options.VldbControllerVersion, assetName, stringPtr("bin/"+target.ControllerBinaryName)))
	}
	if options.Database == RuntimeDatabaseVldbDirect {
		sqliteAsset := fmt.Sprintf("vldb-sqlite-lib-%s-%s%s", options.VldbSQLiteVersion, target.TargetTriple, target.ArchiveExt)
		lancedbAsset := fmt.Sprintf("vldb-lancedb-lib-%s-%s%s", options.VldbLanceDBVersion, target.TargetTriple, target.ArchiveExt)
		assets = append(assets, releaseRuntimeAsset(RuntimeAssetVldbSQLiteLib, options.VldbSQLiteRepo, options.VldbSQLiteVersion, sqliteAsset, stringPtr("libs/"+target.SQLiteLibraryName)))
		assets = append(assets, releaseRuntimeAsset(RuntimeAssetVldbLanceDBLib, options.VldbLanceDBRepo, options.VldbLanceDBVersion, lancedbAsset, stringPtr("libs/"+target.LanceDBLibraryName)))
	}
	return assets
}

// releaseRuntimeAsset builds one release asset descriptor from exact naming inputs.
// releaseRuntimeAsset 从精确命名输入构造单个发布资产描述。
func releaseRuntimeAsset(role RuntimeAssetRole, repository string, version string, assetName string, installedPath *string) RuntimeAssetDescriptor {
	baseURL := fmt.Sprintf("https://github.com/%s/releases/download/%s/%s", repository, version, assetName)
	return RuntimeAssetDescriptor{
		Role:            role,
		Repository:      repository,
		Version:         version,
		AssetName:       assetName,
		SHA256AssetName: assetName + ".sha256",
		DownloadURL:     baseURL,
		SHA256URL:       baseURL + ".sha256",
		InstalledPath:   installedPath,
	}
}

// buildRuntimeHostOptionsPatch builds host option overrides for one database mode.
// buildRuntimeHostOptionsPatch 为单个数据库模式构造宿主选项覆盖。
func buildRuntimeHostOptionsPatch(runtimeRoot string, database RuntimeDatabasePreset, target RuntimePlatformTarget, assets []RuntimeAssetDescriptor) map[string]any {
	root := normalizePath(runtimeRoot)
	if database == RuntimeDatabaseHostCallback {
		return map[string]any{
			"sqlite_provider_mode":  "host_callback",
			"sqlite_callback_mode":  "json",
			"lancedb_provider_mode": "host_callback",
			"lancedb_callback_mode": "json",
		}
	}
	if database == RuntimeDatabaseVldbController {
		return map[string]any{
			"sqlite_provider_mode":  "space_controller",
			"lancedb_provider_mode": "space_controller",
			"space_controller": map[string]any{
				"endpoint":                  nil,
				"auto_spawn":                true,
				"executable_path":           normalizePath(filepath.Join(root, "bin", target.ControllerBinaryName)),
				"process_mode":              "managed",
				"minimum_uptime_secs":       300,
				"idle_timeout_secs":         900,
				"default_lease_ttl_secs":    120,
				"connect_timeout_secs":      5,
				"startup_timeout_secs":      15,
				"startup_retry_interval_ms": 250,
				"lease_renew_interval_secs": 30,
			},
		}
	}
	if database == RuntimeDatabaseVldbDirect {
		return map[string]any{
			"sqlite_library_path":   resolveRuntimeInstalledAsset(root, assets, RuntimeAssetVldbSQLiteLib),
			"sqlite_provider_mode":  "dynamic_library",
			"lancedb_library_path":  resolveRuntimeInstalledAsset(root, assets, RuntimeAssetVldbLanceDBLib),
			"lancedb_provider_mode": "dynamic_library",
		}
	}
	return map[string]any{}
}

// resolveRuntimeInstalledAsset resolves the absolute installed path for one asset role.
// resolveRuntimeInstalledAsset 解析单个资产角色的绝对安装路径。
func resolveRuntimeInstalledAsset(runtimeRoot string, assets []RuntimeAssetDescriptor, role RuntimeAssetRole) any {
	for _, asset := range assets {
		if asset.Role == role && asset.InstalledPath != nil {
			return normalizePath(filepath.Join(runtimeRoot, *asset.InstalledPath))
		}
	}
	return nil
}

// stringPtr returns a pointer to one string literal.
// stringPtr 返回单个字符串字面量的指针。
func stringPtr(value string) *string {
	return &value
}
