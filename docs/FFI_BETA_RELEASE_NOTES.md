# luaskills FFI Beta 发布说明

## 1. 文档定位

本文档用于说明当前 `v0.1.x / beta` 阶段，`luaskills` FFI 对外发布时最应该让接入方知道的事情。

它不是完整 API 参考，也不是完整设计文档。  
如果您需要更细的参数、内存和接口说明，请继续阅读：

- [FFI_INTEGRATION_GUIDE.md](FFI_INTEGRATION_GUIDE.md)
- [FFI_HOST_CHECKLIST.md](FFI_HOST_CHECKLIST.md)
- [HOST_DATABASE_PROVIDER_GUIDE.md](HOST_DATABASE_PROVIDER_GUIDE.md)

## 2. 当前版本应如何理解

当前 `v0.1.x / beta` 阶段，建议把发布面理解成下面三层：

- Rust API
  - 主集成方式
  - 最适合 Rust 宿主直接接入
- 标准 C ABI
  - 低层正式宿主契约
  - 适合 C / C++ / Go 这类能稳定处理结构体和 out 指针的宿主
- 公共 `_json` FFI
  - 高层易用公共接口
  - 适合 Python / Node.js / TypeScript / 动态脚本环境

一句话总结：

- Rust 宿主优先 Rust API
- 低层正式跨语言宿主优先标准 C ABI
- 动态语言与快速集成优先公共 `_json` FFI

## 3. 这次 beta 阶段已经完成的关键收敛

当前 FFI 已经完成了一轮直接收敛，重点包括：

- 标准 C ABI 与公共 `_json` FFI 已明确分层，不再混成一套模糊交付物
- 标准 C ABI 的错误通道已统一收敛到 `FfiOwnedBuffer *error_out`
- `_json` FFI 的输入输出已统一到 `FfiBorrowedBuffer / FfiOwnedBuffer`
- 标准 provider callback 与 JSON provider callback 的 buffer 规则已统一
- 标准结构化结果的大量文本字段已收敛到 `FfiOwnedBuffer`
- 标准 C ABI 头文件与公共 `_json` FFI 头文件已经拆分
- 标准 C ABI 的 Python / Go / TypeScript 示例矩阵已经补齐主调用链、生命周期链和查询辅助链
- 标准 C ABI 的 C 示例当前覆盖最短主调用链，用于演示底层宿主接法
- 外部宿主可以直接使用：
  - [FFI_HOST_CHECKLIST.md](FFI_HOST_CHECKLIST.md)
  - [examples/ffi/standard_runtime/README.md](../examples/ffi/standard_runtime/README.md)
  快速完成 beta 联调前自检

## 4. 当前 beta 发布边界

接入方需要明确下面这些边界：

- 当前版本更适合作为**受控宿主集成接口**
- 当前版本的主集成方式仍然是 Rust 直连
- FFI 主要服务于非 Rust 宿主和跨语言桥接
- FFI 是低层 ABI，不承诺“误用后仍然安全”
- 当前运行时默认把 skill 当作受信代码看待
- 本阶段不提供 Lua skill 沙箱安全承诺

也就是说，这个 beta 版本适合：

- 官方宿主
- 合作方宿主
- 能遵守文档和示例约束的外部接入方

但不适合被理解成：

- 完全零学习成本的开放 SDK
- 误用后仍能完全兜底的安全边界

## 5. 对接方最需要注意的强约束

如果只记几条，请优先记下面这些：

- callback 必须先注册，再创建 engine
- callback 不能跨 C ABI 抛异常
- 同一线程内，不支持重入同一个 engine
- `FfiOwnedBuffer` 必须通过 `luaskills_ffi_buffer_free` 释放
- 结构化结果必须通过专用 free 函数释放
- 宿主不能把自己分配的字符串或缓冲伪装成交给 luaskills 回收
- 远端 controller 场景必须关闭 `auto_spawn`

## 6. 推荐的外部接入顺序

如果宿主正在做第一次 beta 联调，推荐按这个顺序推进：

1. 跑通 `version -> engine_new -> load_from_roots -> list_entries -> call_skill -> run_lua -> engine_free`
2. 再补 `disable_skill / enable_skill`
3. 再补 `is_skill / skill_name_for_tool / prompt_argument_completions`
4. 最后再接：
   - `install / update / uninstall`
   - provider callback
   - `space_controller`

这样做的目的，是把“主调用链稳定性”和“扩展能力接入”分开验证。

接入 `install / update / uninstall` 前，宿主应先固定 skill root 层级策略：正式对外语义为 `ROOT -> PROJECT -> USER`，启动或加载时必须传入 `ROOT` root。普通用户管理面只操作实际存在的 `PROJECT` / `USER`，`ROOT` 级调整只通过注入 `System` authority 的 system tools 或受控 system updater 执行；若 system tools、查询或 prompt completion 被封装给普通 tools，应注入 `DelegatedTool`，且 system install 缺少 `ROOT` 时应失败，不应回退到普通层。运行时调用工具面向当前已激活 skill，不作为 root 管理权限边界。

## 7. 推荐查看的示例

如果宿主需要最短路径上手，直接按下面选：

- 标准 C ABI 最短闭环：
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

## 8. 发布给社区时建议怎么表述

如果后续需要对外发 beta 版本说明，建议直接按下面这个口径表达：

- 当前版本已经具备 Rust 直连主链与 FFI beta 宿主接入能力
- 标准 C ABI 和公共 `_json` FFI 已完成基础分层
- 当前版本更适合受控宿主集成与 beta 联调
- 正式长期 ABI 仍会在后续版本继续收敛
- 欢迎社区围绕：
  - 宿主接入体验
  - 示例可用性
  - 文档清晰度
  - ABI 一致性
  提供反馈

## 9. 当前一句话结论

`v0.1.x / beta` 阶段的 `luaskills` 已经具备“可联调、可示例化、可受控发布”的 FFI 基础，但它的定位仍然是 beta 宿主接入面，而不是完全收敛后的终版 ABI。
