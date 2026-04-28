package main

import (
	"errors"
	"fmt"
	"log"

	luaskills "github.com/LuaSkills/luaskills/sdk/go"
)

// main shows the current Go provider-callback bridge boundary.
// main 展示当前 Go provider callback 桥接边界。
func main() {
	err := luaskills.SetSQLiteProviderJSONCallback(func(request any) (any, error) {
		return map[string]any{"ok": true, "request": request}, nil
	})
	if errors.Is(err, luaskills.ErrProviderCallbacksRequireHostBridge) {
		fmt.Println("Go provider callbacks require a host-owned cgo callback bridge.")
		return
	}
	if err != nil {
		log.Fatal(err)
	}
	defer luaskills.ClearSQLiteProviderJSONCallback()
}
