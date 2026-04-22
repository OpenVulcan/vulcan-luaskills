# Standard Runtime FFI Fixture

## 1. 目录作用

这个目录提供一份**专门给标准 ABI 示例使用**的最小运行时夹具。

它的目标只有一个：

- 让 [Python demo](/D:/projects/vulcan-luaskills/examples/ffi/python/demo.py)
- 让 [Go demo](/D:/projects/vulcan-luaskills/examples/ffi/go/demo.go)
- 让 [TypeScript demo](/D:/projects/vulcan-luaskills/examples/ffi/typescript/demo.ts)
- 让 [C demo](/D:/projects/vulcan-luaskills/examples/ffi/c/demo.c)

都能稳定演示：

- `engine_new`
- `load_from_roots`
- `list_entries`
- 结构化结果读取

## 2. 夹具内容

当前夹具内置一个最小 skill：

- `demo-standard-ffi-skill`

它包含一个入口：

- `ping`

因此标准 ABI 示例在默认情况下应至少能读到一条 entry。

## 3. 目录定位

这个目录和下面两个目录职责不同：

- [demo_runtime](/D:/projects/vulcan-luaskills/examples/ffi/demo_runtime/README.md)
  - 用于动态安装与调用烟测
- [host_provider_demo](/D:/projects/vulcan-luaskills/examples/ffi/host_provider_demo/README.md)
  - 用于宿主数据库 callback / provider 接管演示

而 `standard_runtime` 只负责：

- 给标准 ABI 示例提供稳定、最小、无额外宿主依赖的 entry 夹具
