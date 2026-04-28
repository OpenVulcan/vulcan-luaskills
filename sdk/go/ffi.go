//go:build cgo

package luaskills

/*
#cgo LDFLAGS: -lluaskills
#include <stdint.h>
#include <stdlib.h>

typedef struct FfiBorrowedBuffer {
    uint8_t *ptr;
    size_t len;
} FfiBorrowedBuffer;

typedef struct FfiOwnedBuffer {
    uint8_t *ptr;
    size_t len;
} FfiOwnedBuffer;

void luaskills_ffi_buffer_free(FfiOwnedBuffer value);
FfiOwnedBuffer luaskills_ffi_version_json(void);
FfiOwnedBuffer luaskills_ffi_describe_json(void);
FfiOwnedBuffer luaskills_ffi_engine_new_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_engine_free_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_load_from_dirs_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_load_from_roots_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_reload_from_dirs_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_reload_from_roots_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_list_entries_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_list_skill_help_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_render_skill_help_detail_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_prompt_argument_completions_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_is_skill_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_skill_name_for_tool_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_skill_config_list_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_skill_config_get_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_skill_config_set_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_skill_config_delete_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_call_skill_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_run_lua_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_disable_skill_in_dirs_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_disable_skill_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_system_disable_skill_in_dirs_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_system_disable_skill_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_enable_skill_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_system_enable_skill_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_uninstall_skill_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_system_uninstall_skill_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_install_skill_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_system_install_skill_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_update_skill_json(FfiBorrowedBuffer input_json);
FfiOwnedBuffer luaskills_ffi_system_update_skill_json(FfiBorrowedBuffer input_json);
*/
import "C"

import (
	"encoding/json"
	"fmt"
	"unsafe"
)

// jsonEnvelope is the standard response wrapper returned by public JSON FFI functions.
// jsonEnvelope 是公共 JSON FFI 函数返回的标准响应包络。
type jsonEnvelope struct {
	OK     bool            `json:"ok"`
	Result json.RawMessage `json:"result"`
	Error  string          `json:"error"`
}

// Version queries the public JSON FFI version without creating an engine.
// Version 不创建引擎并查询公共 JSON FFI 版本。
func Version() (map[string]any, error) {
	var result map[string]any
	if err := callJSONNoInput("luaskills_ffi_version_json", &result); err != nil {
		return nil, err
	}
	return result, nil
}

// Describe queries the public JSON FFI self-description without creating an engine.
// Describe 不创建引擎并查询公共 JSON FFI 自描述。
func Describe() (map[string]any, error) {
	var result map[string]any
	if err := callJSONNoInput("luaskills_ffi_describe_json", &result); err != nil {
		return nil, err
	}
	return result, nil
}

// callJSONNoInput calls one JSON FFI function that does not accept input.
// callJSONNoInput 调用一个不接收输入的 JSON FFI 函数。
func callJSONNoInput(functionName string, out any) error {
	var buffer C.FfiOwnedBuffer
	switch functionName {
	case "luaskills_ffi_version_json":
		buffer = C.luaskills_ffi_version_json()
	case "luaskills_ffi_describe_json":
		buffer = C.luaskills_ffi_describe_json()
	default:
		return fmt.Errorf("unsupported JSON FFI no-input function: %s", functionName)
	}
	return decodeJSONEnvelope(functionName, buffer, out)
}

// callJSON calls one JSON FFI function with one JSON-serializable payload.
// callJSON 使用一个可 JSON 序列化载荷调用单个 JSON FFI 函数。
func callJSON(functionName string, payload any, out any) error {
	payloadBytes, err := json.Marshal(payload)
	if err != nil {
		return err
	}
	borrowed, storage := makeBorrowedBuffer(payloadBytes)
	if storage != nil {
		defer C.free(storage)
	}
	var buffer C.FfiOwnedBuffer
	switch functionName {
	case "luaskills_ffi_engine_new_json":
		buffer = C.luaskills_ffi_engine_new_json(borrowed)
	case "luaskills_ffi_engine_free_json":
		buffer = C.luaskills_ffi_engine_free_json(borrowed)
	case "luaskills_ffi_load_from_dirs_json":
		buffer = C.luaskills_ffi_load_from_dirs_json(borrowed)
	case "luaskills_ffi_load_from_roots_json":
		buffer = C.luaskills_ffi_load_from_roots_json(borrowed)
	case "luaskills_ffi_reload_from_dirs_json":
		buffer = C.luaskills_ffi_reload_from_dirs_json(borrowed)
	case "luaskills_ffi_reload_from_roots_json":
		buffer = C.luaskills_ffi_reload_from_roots_json(borrowed)
	case "luaskills_ffi_list_entries_json":
		buffer = C.luaskills_ffi_list_entries_json(borrowed)
	case "luaskills_ffi_list_skill_help_json":
		buffer = C.luaskills_ffi_list_skill_help_json(borrowed)
	case "luaskills_ffi_render_skill_help_detail_json":
		buffer = C.luaskills_ffi_render_skill_help_detail_json(borrowed)
	case "luaskills_ffi_prompt_argument_completions_json":
		buffer = C.luaskills_ffi_prompt_argument_completions_json(borrowed)
	case "luaskills_ffi_is_skill_json":
		buffer = C.luaskills_ffi_is_skill_json(borrowed)
	case "luaskills_ffi_skill_name_for_tool_json":
		buffer = C.luaskills_ffi_skill_name_for_tool_json(borrowed)
	case "luaskills_ffi_skill_config_list_json":
		buffer = C.luaskills_ffi_skill_config_list_json(borrowed)
	case "luaskills_ffi_skill_config_get_json":
		buffer = C.luaskills_ffi_skill_config_get_json(borrowed)
	case "luaskills_ffi_skill_config_set_json":
		buffer = C.luaskills_ffi_skill_config_set_json(borrowed)
	case "luaskills_ffi_skill_config_delete_json":
		buffer = C.luaskills_ffi_skill_config_delete_json(borrowed)
	case "luaskills_ffi_call_skill_json":
		buffer = C.luaskills_ffi_call_skill_json(borrowed)
	case "luaskills_ffi_run_lua_json":
		buffer = C.luaskills_ffi_run_lua_json(borrowed)
	case "luaskills_ffi_disable_skill_in_dirs_json":
		buffer = C.luaskills_ffi_disable_skill_in_dirs_json(borrowed)
	case "luaskills_ffi_disable_skill_json":
		buffer = C.luaskills_ffi_disable_skill_json(borrowed)
	case "luaskills_ffi_system_disable_skill_in_dirs_json":
		buffer = C.luaskills_ffi_system_disable_skill_in_dirs_json(borrowed)
	case "luaskills_ffi_system_disable_skill_json":
		buffer = C.luaskills_ffi_system_disable_skill_json(borrowed)
	case "luaskills_ffi_enable_skill_json":
		buffer = C.luaskills_ffi_enable_skill_json(borrowed)
	case "luaskills_ffi_system_enable_skill_json":
		buffer = C.luaskills_ffi_system_enable_skill_json(borrowed)
	case "luaskills_ffi_uninstall_skill_json":
		buffer = C.luaskills_ffi_uninstall_skill_json(borrowed)
	case "luaskills_ffi_system_uninstall_skill_json":
		buffer = C.luaskills_ffi_system_uninstall_skill_json(borrowed)
	case "luaskills_ffi_install_skill_json":
		buffer = C.luaskills_ffi_install_skill_json(borrowed)
	case "luaskills_ffi_system_install_skill_json":
		buffer = C.luaskills_ffi_system_install_skill_json(borrowed)
	case "luaskills_ffi_update_skill_json":
		buffer = C.luaskills_ffi_update_skill_json(borrowed)
	case "luaskills_ffi_system_update_skill_json":
		buffer = C.luaskills_ffi_system_update_skill_json(borrowed)
	default:
		return fmt.Errorf("unsupported JSON FFI function: %s", functionName)
	}
	return decodeJSONEnvelope(functionName, buffer, out)
}

// makeBorrowedBuffer allocates one temporary C buffer for one JSON FFI request.
// makeBorrowedBuffer 为单个 JSON FFI 请求分配一个临时 C 缓冲。
func makeBorrowedBuffer(payload []byte) (C.FfiBorrowedBuffer, unsafe.Pointer) {
	if len(payload) == 0 {
		return C.FfiBorrowedBuffer{}, nil
	}
	storage := C.CBytes(payload)
	return C.FfiBorrowedBuffer{
		ptr: (*C.uint8_t)(storage),
		len: C.size_t(len(payload)),
	}, storage
}

// decodeJSONEnvelope decodes one owned response envelope and releases the native buffer.
// decodeJSONEnvelope 解码单个拥有型响应包络并释放原生缓冲。
func decodeJSONEnvelope(functionName string, buffer C.FfiOwnedBuffer, out any) error {
	defer C.luaskills_ffi_buffer_free(buffer)
	text := ""
	if buffer.ptr != nil && buffer.len > 0 {
		text = string(C.GoBytes(unsafe.Pointer(buffer.ptr), C.int(buffer.len)))
	}
	var envelope jsonEnvelope
	if err := json.Unmarshal([]byte(text), &envelope); err != nil {
		return err
	}
	if !envelope.OK {
		if envelope.Error == "" {
			envelope.Error = "unknown LuaSkills FFI error"
		}
		return fmt.Errorf("%s: %s", functionName, envelope.Error)
	}
	if out == nil {
		return nil
	}
	return json.Unmarshal(envelope.Result, out)
}
