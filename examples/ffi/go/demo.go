package main

/*
#cgo CFLAGS: -I../../../include
#include <stdlib.h>
#include "vulcan_luaskills_ffi.h"
*/
import "C"

import (
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"unsafe"
)

// mustOK raises one panic when the standard FFI call reports failure.
// mustOK 在标准 FFI 调用报告失败时抛出一个 panic。
func mustOK(status C.int32_t, errorOut C.FfiOwnedBuffer) {
	if status == 0 {
		return
	}
	var message string
	if errorOut.ptr != nil && errorOut.len > 0 {
		message = string(C.GoBytes(unsafe.Pointer(errorOut.ptr), C.int(errorOut.len)))
	}
	C.vulcan_luaskills_ffi_buffer_free(errorOut)
	if message == "" {
		message = "Unknown FFI error"
	}
	panic(message)
}

// demoRuntimeRoot resolves the shared demo runtime root under examples/ffi/demo_runtime.
// demoRuntimeRoot 解析位于 examples/ffi/demo_runtime 下的共享演示运行时根目录。
func demoRuntimeRoot() string {
	_, currentFile, _, ok := runtime.Caller(0)
	if !ok {
		panic("failed to resolve demo.go path")
	}
	return filepath.Join(filepath.Dir(filepath.Dir(currentFile)), "demo_runtime", "runtime_root")
}

// ensureDemoRuntimeLayout creates the shared demo runtime directory layout when it is missing.
// ensureDemoRuntimeLayout 在缺失时创建共享演示运行时目录结构。
func ensureDemoRuntimeLayout(root string) {
	for _, relativePath := range []string{
		"skills",
		"dependencies",
		"state",
		"databases",
		"temp",
		"resources",
		"lua_packages",
		filepath.Join("bin", "tools"),
		"libs",
	} {
		if err := os.MkdirAll(filepath.Join(root, relativePath), 0o755); err != nil {
			panic(err)
		}
	}
}

// main demonstrates one version query and one engine create/free roundtrip.
// main 演示一次版本查询以及一次引擎创建与释放往返调用。
func main() {
	var version C.FfiOwnedBuffer
	var errorOut C.FfiOwnedBuffer
	mustOK(C.vulcan_luaskills_ffi_version(&version, &errorOut), errorOut)
	fmt.Println("Version:", string(C.GoBytes(unsafe.Pointer(version.ptr), C.int(version.len))))
	C.vulcan_luaskills_ffi_buffer_free(version)

	root := demoRuntimeRoot()
	ensureDemoRuntimeLayout(root)
	host := C.FfiLuaRuntimeHostOptions{
		temp_dir:                       C.CString(filepath.ToSlash(filepath.Join(root, "temp"))),
		resources_dir:                  C.CString(filepath.ToSlash(filepath.Join(root, "resources"))),
		lua_packages_dir:               C.CString(filepath.ToSlash(filepath.Join(root, "lua_packages"))),
		luaexec_program:                nil,
		host_provided_tool_root:        C.CString(filepath.ToSlash(filepath.Join(root, "bin", "tools"))),
		host_provided_lua_root:         C.CString(filepath.ToSlash(filepath.Join(root, "lua_packages"))),
		host_provided_ffi_root:         C.CString(filepath.ToSlash(filepath.Join(root, "libs"))),
		download_cache_root:            C.CString(filepath.ToSlash(filepath.Join(root, "temp", "downloads"))),
		dependency_dir_name:            C.CString("dependencies"),
		state_dir_name:                 C.CString("state"),
		database_dir_name:              C.CString("databases"),
		protected_skill_ids:            nil,
		protected_skill_ids_len:        0,
		allow_network_download:         0,
		github_base_url:                nil,
		github_api_base_url:            nil,
		sqlite_library_path:            nil,
		sqlite_provider_mode:           0,
		sqlite_callback_mode:           0,
		lancedb_library_path:           nil,
		lancedb_provider_mode:          0,
		lancedb_callback_mode:          0,
		cache_config:                   nil,
		reserved_entry_names:           nil,
		reserved_entry_names_len:       0,
		enable_skill_management_bridge: 0,
	}
	defer C.free(unsafe.Pointer(host.temp_dir))
	defer C.free(unsafe.Pointer(host.resources_dir))
	defer C.free(unsafe.Pointer(host.lua_packages_dir))
	defer C.free(unsafe.Pointer(host.host_provided_tool_root))
	defer C.free(unsafe.Pointer(host.host_provided_lua_root))
	defer C.free(unsafe.Pointer(host.host_provided_ffi_root))
	defer C.free(unsafe.Pointer(host.download_cache_root))
	defer C.free(unsafe.Pointer(host.dependency_dir_name))
	defer C.free(unsafe.Pointer(host.state_dir_name))
	defer C.free(unsafe.Pointer(host.database_dir_name))

	options := C.FfiLuaEngineOptions{
		pool: C.FfiLuaVmPoolConfig{
			min_size:      1,
			max_size:      1,
			idle_ttl_secs: 30,
		},
		host: host,
	}

	var engineID C.uint64_t
	errorOut = C.FfiOwnedBuffer{}
	mustOK(C.vulcan_luaskills_ffi_engine_new(&options, &engineID, &errorOut), errorOut)
	fmt.Println("Engine created:", uint64(engineID))

	errorOut = C.FfiOwnedBuffer{}
	mustOK(C.vulcan_luaskills_ffi_engine_free(engineID, &errorOut), errorOut)
	fmt.Println("Engine freed")
}
