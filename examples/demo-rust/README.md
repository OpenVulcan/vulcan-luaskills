# LuaSkills Rust Demo

该目录是非 FFI 模式 demo：宿主直接通过 Rust crate 引用 `vulcan-luaskills`，不经过 C ABI 动态库。

运行前可先拉取依赖：

```powershell
.\examples\demo-rust\run.ps1 -Fetch all
```

或在类 Unix 环境：

```bash
bash examples/demo-rust/run.sh all
```

该 demo 使用 `examples/ffi/standard_runtime/runtime_root` 作为共享运行根，只是宿主接入方式从 FFI 改为 Rust 直连。
