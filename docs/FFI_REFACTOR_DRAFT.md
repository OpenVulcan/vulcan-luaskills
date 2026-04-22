# vulcan-luaskills FFI 收敛改造草案

## 1. 文档定位

本文档不是并行 `v2` 设计，而是当前 `v0.1.x / beta` 阶段对**现有 FFI** 的直接收敛改造草案。

当前判断依据如下：

- 还没有任何第三方正式接入当前 FFI
- 当前主集成路径仍然是 Rust 直连
- 当前最需要解决的是 FFI 契约不够产品化，而不是维护多套并行 ABI

因此，本草案的核心原则是：

**直接调整现有 FFI，使其逐步演进为将来长期维护的正式 ABI。**

## 2. 改造边界

本轮改造必须遵守以下边界：

1. 不影响当前 Rust 主集成方式
2. 不重写当前 Rust runtime 主流程
3. 优先收敛 callback ABI、所有权模型、错误模型
4. 允许在 `beta / v0.1.x` 阶段做必要的破坏性 ABI 调整
5. 保留标准 C ABI 与公共 JSON FFI 两层接口，但明确两者职责边界

## 3. 当前 FFI 的主要问题

### 3.1 所有权模型过度依赖文档记忆

当前接口大量依赖：

- `char *`
- `uint8_t *`
- 多个 free 函数

这导致宿主需要额外记住：

- 哪些值由 luaskills 分配
- 哪些值只是借用
- 哪些值需要用哪个 free 函数释放

### 3.2 callback ABI 过度分裂

当前 callback 同时存在：

- JSON callback
- SQLite 标准 callback
- LanceDB 标准 callback

但三者在输入、输出、错误表达上并不统一，增加了接入与维护成本。

### 3.3 错误模型分散

当前错误表达分散在：

- `i32` 状态码
- `char **error_out`
- JSON 包络中的 `ok/error`

长期看不利于绑定生成器、动态语言桥接和社区接入。

## 4. 收敛方向

### 4.1 显式 buffer 化

后续 FFI 应优先采用显式缓冲结构：

- `FfiBorrowedBuffer`
- `FfiOwnedBuffer`

目的：

- 降低对 NUL 终止字符串的依赖
- 统一文本与二进制返回通道
- 减少 `char **` 与 `uint8_t ** + len` 的分裂

### 4.2 callback 优先重构

当前最值得优先收敛的不是所有 FFI 入口，而是 callback 相关 ABI。

原因：

- callback 是最容易误用的边界
- callback 是宿主接入成本最高的部分
- callback 重构不会影响 Rust 主集成方式

### 4.3 保留 engine_id 模型

当前 `engine_id: u64` 已经足够清晰，应继续保留。

不建议当前阶段引入：

- `void *engine_handle`
- 复杂 opaque runtime 对象

### 4.4 分层交付而不是二选一

当前更合理的对外交付方式不是删掉 JSON FFI，而是明确分层：

- 标准 C ABI 负责低层正式契约
- 公共 JSON FFI 负责动态语言和快速集成

也就是说，后续收敛目标不是“只剩一种接口”，而是：

- 让标准 C ABI 更稳定
- 让公共 JSON FFI 更易用
- 让两层交付物、头文件和文档边界更清晰

## 5. 当前阶段已经确定的收敛原则

1. callback 应先注册，再创建 engine
2. callback 不允许跨 ABI 抛异常
3. 同线程不支持对同一 engine 的重入
4. callback 返回值应逐步统一到 buffer 模型
5. 返回文本必须是合法 UTF-8
6. JSON 载荷必须是合法 JSON 文本

## 6. 分阶段推进建议

### 第一阶段

- 收紧 unsafe 边界
- 固化所有权规则
- 固化文档和头文件契约

### 第二阶段

- 重构 provider callback ABI
- 引入 `FfiBorrowedBuffer` / `FfiOwnedBuffer`
- 减少 `char **error_out` 的使用面

### 第三阶段

- 进一步统一普通 FFI 入口的 buffer / status 模型
- 重写官方接入样例
- 为未来 `v1.0.0` 的正式 ABI 定稿做准备

## 7. 当前推进状态

截至当前这轮改造，已经完成以下收敛项：

1. provider callback 输出统一改成 `FfiOwnedBuffer`
2. JSON provider callback 输入统一改成 `FfiBorrowedBuffer`
3. 普通 `_json` FFI 请求输入统一改成 `FfiBorrowedBuffer`
4. 普通 `_json` FFI 返回值统一改成 `FfiOwnedBuffer`
5. 标准 SQLite / LanceDB provider 请求中的 `input_json` 也已改成 `FfiBorrowedBuffer`
6. 头文件、Python smoke demo、host-provider demo、对接文档已同步到新 ABI
7. 标准头文件与公共 JSON FFI 头文件已经开始拆分，避免交付面继续混淆
8. 标准 `FfiRuntimeInvocationResult`、`FfiSkillApplyResult`、`FfiSkillUninstallResult` 的文本字段也开始收敛到 `FfiOwnedBuffer`
9. 标准 `FfiRuntimeEntryParameterDescriptor`、`FfiRuntimeEntryDescriptor`、`FfiRuntimeHelpNodeDescriptor`、`FfiRuntimeSkillHelpDescriptor`、`FfiRuntimeHelpDetail` 的单值文本字段也开始收敛到 `FfiOwnedBuffer`
10. 标准 `FfiStringArray` 与 `related_entries` 这类数组文本通道也开始收敛到 `FfiOwnedBuffer` 元素数组
11. 标准 C / Python / Go / TypeScript 示例已经同步覆盖 `load_from_roots + list_entries` 的结构化结果读取
12. 标准 FFI 已补充入口列表、帮助详情与帮助树结果的嵌套 `FfiOwnedBuffer` 回归测试
13. 已新增 `standard_runtime` 最小夹具目录，避免标准 ABI 示例依赖 host-provider 或动态安装场景

当前还没有完成的重点项主要是：

1. 是否进一步统一普通标准 FFI 入口的错误 / 输出模型
2. 是否减少 `char **error_out` 在标准主接口中的使用面
3. 是否为后续社区接入补更多最小宿主样例与迁移说明

## 8. 当前结论

当前 FFI 的正确演进方式不是：

- 保留旧 ABI 再并行维护一个未来 `v2`

而是：

- 直接利用当前没有第三方包袱的窗口期
- 把现有 FFI 逐步收敛成正式产品 ABI

也就是说，本文档讨论的不是“下一代接口”，而是：

**现有接口如何在 `beta / v0.1.x` 阶段被直接整理为未来正式接口。**
