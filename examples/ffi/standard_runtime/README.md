# Standard Runtime FFI Fixture

## 1. 目录作用

这个目录提供一份**专门给标准 ABI 示例使用**的最小运行时夹具。

它的目标只有一个：

- 让 [Python demo](/D:/projects/luaskills/examples/ffi/python/demo.py)
- 让 [Python lifecycle demo](/D:/projects/luaskills/examples/ffi/python/lifecycle_demo.py)
- 让 [Python query demo](/D:/projects/luaskills/examples/ffi/python/query_demo.py)
- 让 [Go demo](/D:/projects/luaskills/examples/ffi/go/demo.go)
- 让 [Go lifecycle demo](/D:/projects/luaskills/examples/ffi/go/lifecycle_demo/main.go)
- 让 [Go query demo](/D:/projects/luaskills/examples/ffi/go/query_demo/main.go)
- 让 [TypeScript demo](/D:/projects/luaskills/examples/ffi/typescript/demo.ts)
- 让 [TypeScript lifecycle demo](/D:/projects/luaskills/examples/ffi/typescript/lifecycle_demo.ts)
- 让 [TypeScript query demo](/D:/projects/luaskills/examples/ffi/typescript/query_demo.ts)
- 让 [C demo](/D:/projects/luaskills/examples/ffi/c/demo.c)

都能稳定演示：

- `engine_new`
- `load_from_roots`
- `list_entries`
- `call_skill`
- `run_lua`
- `disable_skill / enable_skill`
- `is_skill / skill_name_for_tool / prompt_argument_completions`
- 结构化结果读取

## 2. 夹具内容

当前夹具内置一个最小 skill：

- `demo-standard-ffi-skill`

它包含一个入口：

- `ping`

因此标准 ABI 示例在默认情况下应至少能读到一条 entry。

同时它的 `ping` 入口会稳定返回：

- `standard-ffi-demo:ok`
- 或 `standard-ffi-demo:<note>`

这让标准 ABI 示例可以继续演示：

- `FfiBorrowedBuffer` 形式的参数输入
- `call_skill` 的结构化结果读取
- `run_lua` 的 JSON 结果读取

## 3. 目录定位

这个目录和下面两个目录职责不同：

- [demo_runtime](/D:/projects/luaskills/examples/ffi/demo_runtime/README.md)
  - 用于动态安装与调用烟测
- [host_provider_demo](/D:/projects/luaskills/examples/ffi/host_provider_demo/README.md)
  - 用于宿主数据库 callback / provider 接管演示

而 `standard_runtime` 只负责：

- 给标准 ABI 示例提供稳定、最小、无额外宿主依赖的 entry 夹具

## 4. 建议查看顺序

如果您想快速理解当前标准 ABI 示例矩阵，建议按下面的顺序看：

1. 先看 [C demo](/D:/projects/luaskills/examples/ffi/c/demo.c)
   - 理解最底层标准 ABI 的最短闭环
2. 再看任意一门动态语言的主示例
   - [Python demo](/D:/projects/luaskills/examples/ffi/python/demo.py)
   - [Go demo](/D:/projects/luaskills/examples/ffi/go/demo.go)
   - [TypeScript demo](/D:/projects/luaskills/examples/ffi/typescript/demo.ts)
3. 再按专题补看：
   - 生命周期切换：
     - [Python lifecycle demo](/D:/projects/luaskills/examples/ffi/python/lifecycle_demo.py)
     - [Go lifecycle demo](/D:/projects/luaskills/examples/ffi/go/lifecycle_demo/main.go)
     - [TypeScript lifecycle demo](/D:/projects/luaskills/examples/ffi/typescript/lifecycle_demo.ts)
   - 查询辅助接口：
     - [Python query demo](/D:/projects/luaskills/examples/ffi/python/query_demo.py)
     - [Go query demo](/D:/projects/luaskills/examples/ffi/go/query_demo/main.go)
     - [TypeScript query demo](/D:/projects/luaskills/examples/ffi/typescript/query_demo.ts)

这样看，最容易把“主调用链”、“生命周期链路”和“查询辅助链路”三类示例区分开。
