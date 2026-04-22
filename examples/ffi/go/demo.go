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
		message = readOwnedBufferText(errorOut)
	}
	C.vulcan_luaskills_ffi_buffer_free(errorOut)
	if message == "" {
		message = "Unknown FFI error"
	}
	panic(message)
}

// readOwnedBufferText reads one nested UTF-8 owned buffer without freeing it immediately.
// readOwnedBufferText 读取一个嵌套 UTF-8 拥有型缓冲但不立即释放。
func readOwnedBufferText(buffer C.FfiOwnedBuffer) string {
	if buffer.ptr == nil || buffer.len == 0 {
		return ""
	}
	return string(C.GoBytes(unsafe.Pointer(buffer.ptr), C.int(buffer.len)))
}

// standardFixtureRuntimeRoot resolves the dedicated standard-ABI fixture runtime root.
// standardFixtureRuntimeRoot 解析标准 ABI 专用夹具运行时根目录。
func standardFixtureRuntimeRoot() string {
	_, currentFile, _, ok := runtime.Caller(0)
	if !ok {
		panic("failed to resolve demo.go path")
	}
	return filepath.Join(filepath.Dir(filepath.Dir(currentFile)), "standard_runtime", "runtime_root")
}

// ensureStandardFixtureLayout creates the shared fixture runtime layout when it is missing.
// ensureStandardFixtureLayout 在缺失时创建共享夹具运行时目录结构。
func ensureStandardFixtureLayout(root string) {
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

// main demonstrates version, engine lifecycle, root loading, and one structured entry-list read.
// main 演示版本查询、引擎生命周期、根链加载以及一次结构化入口列表读取。
func main() {
	var version C.FfiOwnedBuffer
	var errorOut C.FfiOwnedBuffer
	mustOK(C.vulcan_luaskills_ffi_version(&version, &errorOut), errorOut)
	fmt.Println("Version:", readOwnedBufferText(version))
	C.vulcan_luaskills_ffi_buffer_free(version)

	root := standardFixtureRuntimeRoot()
	ensureStandardFixtureLayout(root)
	host := C.FfiLuaRuntimeHostOptions{
		temp_dir:                         C.CString(filepath.ToSlash(filepath.Join(root, "temp"))),
		resources_dir:                    C.CString(filepath.ToSlash(filepath.Join(root, "resources"))),
		lua_packages_dir:                 C.CString(filepath.ToSlash(filepath.Join(root, "lua_packages"))),
		luaexec_program:                  nil,
		host_provided_tool_root:          C.CString(filepath.ToSlash(filepath.Join(root, "bin", "tools"))),
		host_provided_lua_root:           C.CString(filepath.ToSlash(filepath.Join(root, "lua_packages"))),
		host_provided_ffi_root:           C.CString(filepath.ToSlash(filepath.Join(root, "libs"))),
		download_cache_root:              C.CString(filepath.ToSlash(filepath.Join(root, "temp", "downloads"))),
		dependency_dir_name:              C.CString("dependencies"),
		state_dir_name:                   C.CString("state"),
		database_dir_name:                C.CString("databases"),
		protected_skill_ids:              nil,
		protected_skill_ids_len:          0,
		allow_network_download:           0,
		github_base_url:                  nil,
		github_api_base_url:              nil,
		sqlite_library_path:              nil,
		sqlite_provider_mode:             0,
		sqlite_callback_mode:             0,
		lancedb_library_path:             nil,
		lancedb_provider_mode:            0,
		lancedb_callback_mode:            0,
		space_controller_endpoint:        nil,
		space_controller_auto_spawn:      0,
		space_controller_executable_path: nil,
		space_controller_process_mode:    0,
		cache_config:                     nil,
		reserved_entry_names:             nil,
		reserved_entry_names_len:         0,
		enable_skill_management_bridge:   0,
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

	rootName := C.CString("ROOT")
	skillsDir := C.CString(filepath.ToSlash(filepath.Join(root, "skills")))
	defer C.free(unsafe.Pointer(rootName))
	defer C.free(unsafe.Pointer(skillsDir))
	skillRoots := []C.FfiRuntimeSkillRoot{
		{
			name:       rootName,
			skills_dir: skillsDir,
		},
	}
	errorOut = C.FfiOwnedBuffer{}
	mustOK(
		C.vulcan_luaskills_ffi_load_from_roots(
			engineID,
			(*C.FfiRuntimeSkillRoot)(unsafe.Pointer(&skillRoots[0])),
			C.size_t(len(skillRoots)),
			&errorOut,
		),
		errorOut,
	)
	fmt.Println("Loaded roots from:", filepath.ToSlash(filepath.Join(root, "skills")))

	var entryList *C.FfiRuntimeEntryDescriptorList
	errorOut = C.FfiOwnedBuffer{}
	mustOK(C.vulcan_luaskills_ffi_list_entries(engineID, &entryList, &errorOut), errorOut)
	if entryList != nil {
		defer C.vulcan_luaskills_ffi_entry_list_free(entryList)
		entrySlice := unsafe.Slice(entryList.items, int(entryList.len))
		fmt.Println("Entry count:", len(entrySlice))
		if len(entrySlice) > 0 {
			firstEntry := entrySlice[0]
			fmt.Println("First canonical entry:", readOwnedBufferText(firstEntry.canonical_name))
			fmt.Println("First entry skill id:", readOwnedBufferText(firstEntry.skill_id))
			fmt.Println("First entry description:", readOwnedBufferText(firstEntry.description))
			parameterSlice := unsafe.Slice(firstEntry.parameters, int(firstEntry.parameters_len))
			fmt.Println("First entry parameter count:", len(parameterSlice))
			if len(parameterSlice) > 0 {
				firstParameter := parameterSlice[0]
				fmt.Println("First parameter name:", readOwnedBufferText(firstParameter.name))
				fmt.Println("First parameter type:", readOwnedBufferText(firstParameter.param_type))
				fmt.Println("First parameter required:", firstParameter.required != 0)
			}
		} else {
			fmt.Println("No entries were returned by the current fixture root.")
		}
	}

	errorOut = C.FfiOwnedBuffer{}
	mustOK(C.vulcan_luaskills_ffi_engine_free(engineID, &errorOut), errorOut)
	fmt.Println("Engine freed")
}
