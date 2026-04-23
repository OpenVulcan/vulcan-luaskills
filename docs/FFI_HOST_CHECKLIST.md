# vulcan-luaskills FFI 宿主接入检查清单

## 1. 这份清单的用途

这份清单不是完整设计说明，也不是 API 逐项参考。  
它的目标只有一个：

- 让宿主在第一次接入 `vulcan-luaskills` FFI 时，能按最短路径完成自检

如果您需要完整背景说明，请继续阅读：

- [FFI_INTEGRATION_GUIDE.md](FFI_INTEGRATION_GUIDE.md)
- [HOST_DATABASE_PROVIDER_GUIDE.md](HOST_DATABASE_PROVIDER_GUIDE.md)

## 2. 先选接入面

在真正写宿主代码之前，先确定这一步：

- 如果宿主本身是 Rust：
  - 优先直接接 Rust API
- 如果宿主是 C / C++ / Go / 其他能稳定处理结构体和 out 指针的语言：
  - 优先接标准 C ABI
- 如果宿主是 Python / Node.js / TypeScript / 动态脚本环境：
  - 优先接公共 `_json` FFI
- 如果宿主需要“稳定主链 + 快速调试链”：
  - 可以混合使用
  - 标准 C ABI 负责主链
  - 公共 `_json` FFI 负责快速桥接和动态调试

## 3. 启动前检查

在 `engine_new` 之前，先确认这些条件：

- 已经准备好宿主运行时目录：
  - `temp`
  - `resources`
  - `lua_packages`
  - `dependencies`
  - `state`
  - `databases`
- 已经决定数据库 provider 模式：
  - `dynamic_library`
  - `host_callback`
  - `space_controller`
- 如果要用 callback：
  - callback 必须先注册，再创建 engine
- 如果要用 `space_controller`：
  - 已确认 `endpoint / auto_spawn / executable_path / process_mode`
- 如果连接远端 controller：
  - 必须关闭 `auto_spawn`

## 4. 标准创建顺序

第一次接入最推荐按这个顺序实现：

1. `version`
2. `engine_new`
3. `load_from_roots`
4. `list_entries`
5. `call_skill`
6. `run_lua`
7. `engine_free`

如果这条链还没跑通，不建议先去接：

- `install / update / uninstall`
- 数据库 provider callback
- `space_controller`

## 5. 生命周期与查询辅助的第二阶段顺序

基础调用链打通后，再按这个顺序往下补：

1. `disable_skill / enable_skill`
2. `is_skill`
3. `skill_name_for_tool`
4. `prompt_argument_completions`
5. `list_skill_help`
6. `render_skill_help_detail`

这样更容易定位问题，不会把“运行时主链问题”和“辅助接口问题”混在一起。

## 6. 内存释放检查

这是最容易误用的部分，建议逐项对照：

- 标准 C ABI 接口失败信息：
  - 通过 `FfiOwnedBuffer error_out` 返回
  - 读取后必须 `vulcan_luaskills_ffi_buffer_free`
- 标准 C ABI 接口的单值文本输出：
  - 例如 `version_out` / `skill_id_out` / `result_json_out`
  - 也应按 `FfiOwnedBuffer` 读取与释放
- 结构化结果：
  - 不能手动释放内部字段
  - 必须调用结构体专用 free 函数
- 字符串数组：
  - 必须调用 `vulcan_luaskills_ffi_string_array_free`
- 裸字符串辅助函数：
  - `vulcan_luaskills_ffi_string_free` 只能释放 **luaskills 自己分配** 的字符串

一句话规则：

- 单值文本看 `FfiOwnedBuffer`
- 结构体结果看专用 free
- 不要自己猜该释放什么

## 7. 指针与缓冲规则

宿主在传参时要特别确认：

- `FfiBorrowedBuffer.ptr` 在调用期间必须有效
- `len > 0` 时，`ptr` 不能为 null
- 不能把宿主自己的内存伪装成 `FfiOwnedBuffer`
- 不能把宿主自己的字符串交给 `vulcan_luaskills_ffi_string_free`

## 8. 回调与线程规则

如果宿主要接 callback，请对照下面几条：

- callback 必须在 `engine_new` 前注册
- callback 不能跨 C ABI 抛异常
- 同一线程内，不支持在一个 engine 调用尚未返回时再次重入同一个 engine
- 如果一个进程里需要多套 callback 逻辑：
  - 应分别创建不同 engine
  - 不要指望在 engine 创建后再切换全局 callback

## 9. 标准 C ABI 与公共 `_json` FFI 的最短判断

如果还在犹豫该走哪条路，直接按下面判断：

- 想要更稳定的底层契约：
  - 走标准 C ABI
- 想更快接进 Python / Node / TypeScript：
  - 走公共 `_json` FFI
- 想以后接更多语言绑定：
  - 先把标准 C ABI 跑通
- 想快速验证功能闭环：
  - 先跑公共 `_json` FFI 或 Python 示例

## 10. 示例入口速查

按目标直接选示例：

- 最短标准 ABI 闭环：
  - [examples/ffi/c/demo.c](../examples/ffi/c/demo.c)
  - [examples/ffi/python/demo.py](../examples/ffi/python/demo.py)
  - [examples/ffi/go/demo.go](../examples/ffi/go/demo.go)
  - [examples/ffi/typescript/demo.ts](../examples/ffi/typescript/demo.ts)
- 生命周期切换：
  - [examples/ffi/python/lifecycle_demo.py](../examples/ffi/python/lifecycle_demo.py)
  - [examples/ffi/go/lifecycle_demo/main.go](../examples/ffi/go/lifecycle_demo/main.go)
  - [examples/ffi/typescript/lifecycle_demo.ts](../examples/ffi/typescript/lifecycle_demo.ts)
- 查询辅助接口：
  - [examples/ffi/python/query_demo.py](../examples/ffi/python/query_demo.py)
  - [examples/ffi/go/query_demo/main.go](../examples/ffi/go/query_demo/main.go)
  - [examples/ffi/typescript/query_demo.ts](../examples/ffi/typescript/query_demo.ts)
- 标准 ABI 共用夹具：
  - [examples/ffi/standard_runtime/README.md](../examples/ffi/standard_runtime/README.md)
- 动态安装烟测：
  - [examples/ffi/demo_runtime/README.md](../examples/ffi/demo_runtime/README.md)
- 宿主 provider 接管：
  - [examples/ffi/host_provider_demo/README.md](../examples/ffi/host_provider_demo/README.md)

## 11. 发布前最小自测

如果宿主准备进入 beta 联调，至少确认下面这些项目都通过：

- `engine_new -> load_from_roots -> list_entries -> call_skill -> run_lua -> engine_free`
- `disable_skill / enable_skill` 能反映到运行时视图
- `is_skill / skill_name_for_tool / prompt_argument_completions` 返回符合预期
- 所有 `error_out` 都能被正确读取和释放
- 所有结构化结果都通过专用 free 回收
- callback 场景下没有跨 ABI 异常
- callback 场景下没有同线程重入

只要这组检查全部通过，宿主接入通常就已经具备 beta 联调基础。
