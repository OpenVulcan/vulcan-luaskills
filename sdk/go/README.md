# LuaSkills Go SDK

Go SDK，用于通过公共 `_json` FFI 集成 LuaSkills 运行时。

当前 SDK 使用 cgo 调用 `luaskills` 动态库导出的 JSON FFI 函数，封装 engine 生命周期、root helper、查询、调用、skill-config、普通管理面与 system 管理面。

## 安装

```bash
go get github.com/LuaSkills/luaskills/sdk/go
```

使用时需要让 cgo 能找到 `luaskills` 链接库，并让运行时能找到动态库。

Windows 示例：

```powershell
$env:CGO_ENABLED = "1"
$env:CGO_LDFLAGS = "-LD:\projects\vulcan-luaskills\target\debug"
$env:PATH = "D:\projects\vulcan-luaskills\target\debug;$env:PATH"
```

Linux / macOS 示例：

```bash
export CGO_ENABLED=1
export CGO_LDFLAGS="-L/path/to/luaskills"
export LD_LIBRARY_PATH="/path/to/luaskills:${LD_LIBRARY_PATH}"
```

## 基础用法

```go
package main

import (
	"fmt"

	luaskills "github.com/LuaSkills/luaskills/sdk/go"
)

func main() {
	runtimeRoot := "D:/runtime/luaskills"
	roots := luaskills.StandardRoots(runtimeRoot)

	client, err := luaskills.NewClient(luaskills.ClientOptions{
		RuntimeRoot:         runtimeRoot,
		EnsureRuntimeLayout: true,
	})
	if err != nil {
		panic(err)
	}
	defer client.Close()

	if _, err := client.LoadFromRoots(roots); err != nil {
		panic(err)
	}

	entries, err := client.ListEntries(luaskills.AuthorityDelegatedTool)
	if err != nil {
		panic(err)
	}

	result, err := client.CallSkill("demo-standard-ffi-skill-ping", map[string]any{
		"note": "go-sdk",
	}, nil)
	if err != nil {
		panic(err)
	}

	fmt.Println(entries)
	fmt.Println(result.Content)
}
```

## 权限与边界

- 查询类接口建议默认使用 `AuthorityDelegatedTool`，因此不会返回 `ROOT` skills。
- `AuthoritySystem` 只表示可管理 ROOT 层，不表示可绕过 ROOT 同名占用规则。
- `CallSkill` 与 `RunLua` 是运行时执行面，不作为 ROOT 可见性过滤。
- skill-config 按 `skill_id + key` 读写，只有 Lua skill 实际读取配置时才影响行为。
- 如果宿主不希望用户执行任意 Lua，不应直接暴露 `RunLua`。

## JSON Provider Callback

Go SDK 当前覆盖公共 `_json` FFI 主链，但不在包内直接注册进程级 provider callback。原因是 Go 的 C 回调需要宿主拥有明确的 cgo callback bridge、线程模型和全局生命周期管理；SDK 先提供显式 API 边界：

```go
err := luaskills.SetSQLiteProviderJSONCallback(func(request any) (any, error) {
	return map[string]any{"ok": true, "request": request}, nil
})
```

当前该 API 会返回 `ErrProviderCallbacksRequireHostBridge`。正式 Go 宿主如果需要 `host_callback + json`，建议在宿主工程内实现受控 cgo callback bridge，或先通过 TypeScript / Python SDK 接 JSON callback。示例见 `examples/provider_callback/main.go`。

## Runtime 资产规划

Go SDK 提供与 TypeScript / Python SDK 相同的发布资产命名模型。宿主可以先生成 manifest，再决定由自己的安装器下载或复用 TypeScript / Python CLI：

```go
manifest, err := luaskills.BuildRuntimeInstallManifest(luaskills.RuntimeInstallOptions{
	RuntimeRoot:         "D:/runtime/luaskills",
	Database:            luaskills.RuntimeDatabaseVldbDirect,
	SkipLuaSkillsFFI:    false,
})
if err != nil {
	panic(err)
}

hostOptions := luaskills.HostOptionsFromRuntimeManifest(manifest)
```

`DefaultHostOptions(runtimeRoot)` / `NewClient` 会自动读取 `runtimeRoot/resources/luaskills-sdk-runtime-manifest.json` 并合入 `host_options_patch`；上面的显式读取适合宿主自定义安装器或需要审计 manifest 内容的场景。

数据库模式固定为：

- `RuntimeDatabaseNone`：不安装数据库 provider。
- `RuntimeDatabaseVldbController`：使用 `vldb-controller-{version}-{target}`，对应 `space_controller`。
- `RuntimeDatabaseVldbDirect`：使用 `vldb-sqlite-lib-{version}-{target}` 与 `vldb-lancedb-lib-{version}-{target}`，对应 `dynamic_library`。
- `RuntimeDatabaseHostCallback`：由宿主提供 JSON callback。

## 验证

源码环境可运行：

```powershell
$env:CGO_ENABLED = "1"
$env:CGO_LDFLAGS = "-LD:\projects\vulcan-luaskills\target\debug"
$env:PATH = "D:\projects\vulcan-luaskills\target\debug;$env:PATH"
go test ./sdk/go
```
