package luaskills

import "errors"

// JSONProviderCallback is the host function shape used by JSON provider callbacks.
// JSONProviderCallback 是 JSON provider callback 使用的宿主函数形状。
type JSONProviderCallback func(request any) (any, error)

// ErrProviderCallbacksRequireHostBridge explains why Go does not register callbacks directly yet.
// ErrProviderCallbacksRequireHostBridge 说明 Go 当前为何尚不直接注册 callback。
var ErrProviderCallbacksRequireHostBridge = errors.New("luaskills Go SDK provider callbacks require a host-owned cgo callback bridge")

// SetSQLiteProviderJSONCallback reports the required host bridge for SQLite JSON callbacks.
// SetSQLiteProviderJSONCallback 报告 SQLite JSON callback 所需的宿主桥接。
func SetSQLiteProviderJSONCallback(callback JSONProviderCallback) error {
	return ErrProviderCallbacksRequireHostBridge
}

// ClearSQLiteProviderJSONCallback reports the required host bridge for clearing SQLite callbacks.
// ClearSQLiteProviderJSONCallback 报告清理 SQLite callback 所需的宿主桥接。
func ClearSQLiteProviderJSONCallback() error {
	return SetSQLiteProviderJSONCallback(nil)
}

// SetLanceDBProviderJSONCallback reports the required host bridge for LanceDB JSON callbacks.
// SetLanceDBProviderJSONCallback 报告 LanceDB JSON callback 所需的宿主桥接。
func SetLanceDBProviderJSONCallback(callback JSONProviderCallback) error {
	return ErrProviderCallbacksRequireHostBridge
}

// ClearLanceDBProviderJSONCallback reports the required host bridge for clearing LanceDB callbacks.
// ClearLanceDBProviderJSONCallback 报告清理 LanceDB callback 所需的宿主桥接。
func ClearLanceDBProviderJSONCallback() error {
	return SetLanceDBProviderJSONCallback(nil)
}
