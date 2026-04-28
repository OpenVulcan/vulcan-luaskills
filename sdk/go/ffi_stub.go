//go:build !cgo

package luaskills

import "fmt"

// Version reports that the Go SDK needs cgo when the package is built without cgo.
// Version 在未启用 cgo 构建包时报告 Go SDK 需要 cgo。
func Version() (map[string]any, error) {
	return nil, errCgoRequired()
}

// Describe reports that the Go SDK needs cgo when the package is built without cgo.
// Describe 在未启用 cgo 构建包时报告 Go SDK 需要 cgo。
func Describe() (map[string]any, error) {
	return nil, errCgoRequired()
}

// callJSON reports that JSON FFI calls require a cgo-enabled build.
// callJSON 报告 JSON FFI 调用需要启用 cgo 的构建。
func callJSON(functionName string, payload any, out any) error {
	return errCgoRequired()
}

// callJSONNoInput reports that JSON FFI calls require a cgo-enabled build.
// callJSONNoInput 报告 JSON FFI 调用需要启用 cgo 的构建。
func callJSONNoInput(functionName string, out any) error {
	return errCgoRequired()
}

// errCgoRequired builds the standard no-cgo error message.
// errCgoRequired 构造标准 no-cgo 错误消息。
func errCgoRequired() error {
	return fmt.Errorf("luaskills Go SDK requires CGO_ENABLED=1 and a C compiler to call the LuaSkills JSON FFI")
}
