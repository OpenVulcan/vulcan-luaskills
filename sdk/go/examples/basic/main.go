package main

import (
	"fmt"
	"log"

	luaskills "github.com/LuaSkills/luaskills/sdk/go"
)

// main prints the LuaSkills JSON FFI version through the Go SDK.
// main 通过 Go SDK 输出 LuaSkills JSON FFI 版本。
func main() {
	version, err := luaskills.Version()
	if err != nil {
		log.Fatal(err)
	}
	fmt.Println(version)
}
