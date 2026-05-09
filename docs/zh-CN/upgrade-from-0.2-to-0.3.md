# LuaSkills `0.2` 到 `0.3` 升级说明

本文面向已经使用 `LuaSkills 0.2.x` 的宿主、SDK 使用方和发布维护者，说明升级到 `0.3.x` 后需要关注的变化、是否需要改代码，以及新的发布与运行时资产边界。

## 1. 适用范围

本文覆盖以下升级路径：

- `luaskills` crate / FFI / demo：`0.2.x -> 0.3.x`
- `luaskills-packages`：独立拆分后的 `0.1.x` 协议线
- TypeScript / Python / Go SDK：升级到 `0.3.x`

本文默认当前稳定组合为：

- `LuaSkills/luaskills`：`0.4.0`
- `LuaSkills/luaskills-packages`：`0.1.6`

## 2. 升级结论

先说结论：

1. **Rust 直连 crate 的宿主，大多数情况下只需要把依赖升到 `0.3`。**
2. **如果你自己处理 runtime 资产下载、FFI 安装脚本或 demo 依赖脚本，就需要同步适配 `luaskills-packages` 独立发布。**
3. **如果你仍在使用旧的 directory-style roots 或旧版 runtime-session 兼容路径，就必须切换到 `0.3` 的正式接口。**
4. **如果你使用 packaged runtime，`0.3` 现在会强校验 `luaskills-packages` 元数据文件；缺失会直接报错，不再默默继续。**

## 3. `0.3` 的核心变化

### 3.1 `luaskills-packages` 从主仓库拆分

`0.2.x` 时代，Lua 运行时依赖、LuaRocks 包清单、原生依赖打包和 runtime 资产大多由主仓库一并产出。

从 `0.3.x` 开始，职责边界变成：

| 组件 | 负责内容 |
| --- | --- |
| `LuaSkills/luaskills-packages` | `lua-runtime-packages-*`、`lua-deps-*`、Lua 包清单、help 元数据、license 元数据 |
| `LuaSkills/luaskills` | crate、本体动态库、`luaskills-ffi-sdk-*`、demo 资产 |
| SDK | 组合下载 core 资产与 packages 资产 |

这意味着主仓库发布面已经收缩，**`lua-runtime-*` 和 `lua-deps-*` 不再由 `LuaSkills/luaskills` 发布**。

### 3.2 runtime 资产结构变成双层来源

新的运行时资产来源如下：

| 资产 | 来源仓库 |
| --- | --- |
| `luaskills-ffi-sdk-{platform}.tar.gz` | `LuaSkills/luaskills` |
| `lua-runtime-packages-{platform}.tar.gz` | `LuaSkills/luaskills-packages` |
| `lua-deps-{platform}.tar.gz` | `LuaSkills/luaskills-packages` |

如果你之前默认认为“只要下载主仓库 release 就包含完整 Lua runtime”，这一点在 `0.3` 已经不成立。

### 3.3 packaged runtime 新增强校验

`0.3` 对 packaged runtime 增加了 `luaskills-packages` 元数据校验。正式打包的 runtime 现在要求至少包含：

- `resources/lua-runtime-manifest.json`
- `resources/luaskills-packages-manifest.json`
- `resources/luaskills-packages/install-manifest.json`
- `resources/luaskills-packages/lua_packages.txt`
- `resources/luaskills-packages/platform-support.json`
- `resources/luaskills-packages/THIRD_PARTY_LICENSES.json`
- `resources/luaskills-packages/THIRD_PARTY_NOTICES.md`
- `resources/luaskills-packages/help/index.json`
- `resources/luaskills-packages/help/packages`
- `resources/luaskills-packages/help/modules`
- `licenses/luaskills-packages/index.json`

如果这是一个 packaged runtime，但这些文件缺失，`0.3` 会直接报错，而不是继续容忍不完整的包结构。

### 3.4 旧兼容接口已收口

`0.3` 已明确按“只支持最新协议、尽量不保留兼容兜底”的方向收口，主要包括：

- authority-bound runtime-session 不再静默回退到公共旧入口
- legacy directory-style roots API 已移除，统一收口到 roots / root chain
- 主发布链不再负责本地编译 Lua deps，再由 demo 或 SDK 从主仓库拿完整 runtime

## 4. Rust 直连宿主是否需要改代码

### 4.1 大多数 Rust 宿主只需要升级依赖版本

如果你的宿主是直接依赖 Rust crate，而且已经按正式接口使用：

- `LuaEngine`
- `RuntimeSkillRoot`
- `load_from_roots(...)`

那么升级通常只需要：

```toml
[dependencies]
luaskills = "0.3"
```

这类宿主一般**不需要重写主调用流程**。

### 4.2 需要改代码的情况

下面几类场景需要调整：

#### 情况 A：你自己反序列化宿主能力配置 JSON

`LuaRuntimeCapabilityOptions` 在 `0.3` 中要求显式处理 `enable_managed_io_compat`。如果你自己维护配置 JSON，就应该显式带上这个字段，而不是依赖旧兼容默认。

#### 情况 B：你直接构造 `LuaRuntimeCapabilityOptions` 字面量

如果你不是用 `Default::default()`，而是自己写 struct literal，那么需要把 `enable_managed_io_compat` 一并写出。

#### 情况 C：你仍在用旧的 directory-style roots 语义

`0.3` 已经收口到 `RuntimeSkillRoot + load_from_roots / reload_from_roots`。如果你还保留旧的 directory-style wrapper，就应尽快切到正式 roots 模型。

#### 情况 D：你自己组装 packaged runtime

如果你不是直接使用官方发布的 runtime 资产，而是自己打包、复制或裁剪运行时目录，那么必须确保新加入的 `luaskills-packages` 元数据目录完整。否则 `0.3` 在加载 packaged runtime 时会直接拒绝初始化。

## 5. FFI / SDK / 安装脚本侧需要关注什么

### 5.1 SDK 不再只依赖主仓库 release

SDK 现在默认采用“core + packages”双来源：

- core 资产跟随 `luaskills` / SDK 自身版本
- packages 资产跟随兼容协议线

当前稳定策略是：

- core：`0.3.x`
- packages：`0.1.x`

SDK 在没有显式指定 patch 版本时，会按 `0.1` 协议线自动解析 `LuaSkills/luaskills-packages` 当前最新 patch。

### 5.2 demo / runtime 拉取脚本语义变化

`0.3` 的 `fetch_runtime_deps.ps1` / `fetch_runtime_deps.sh` 已经改成组合拉取：

- `LuaSkills/luaskills` 的 `luaskills-ffi-sdk-*`
- `LuaSkills/luaskills-packages` 的 `lua-runtime-packages-*`

同时 `install_lua_deps.ps1` / `install_lua_deps.sh` 的预编译依赖下载源也已经切到 `LuaSkills/luaskills-packages`。

如果你在外部仓库复制过旧脚本，或者自己魔改过下载逻辑，这一块需要同步。

## 6. 发布流程变化

推荐的统一发布顺序已经变成：

1. 先发布 `LuaSkills/luaskills-packages`
2. 再发布 `LuaSkills/luaskills`
3. 再发布 TypeScript SDK
4. 再发布 Python SDK
5. 再发布 Go SDK
6. 最后运行各 SDK 仓库的 examples release

这样可以保证：

- packages 资产已经存在
- 主仓库 demo 与 SDK 默认下载链不会指向还没发布的 assets
- examples release 用到的 runtime 资产都是可解析的最终版本

## 7. 对接方升级检查清单

建议按下面的顺序自检：

### Rust 直连 crate

- [ ] `Cargo.toml` 已升级到 `luaskills = "0.3"`
- [ ] 仍然使用 `RuntimeSkillRoot + load_from_roots(...)`
- [ ] 若自定义宿主能力配置，已显式处理 `enable_managed_io_compat`

### FFI / SDK / demo

- [ ] 不再假设主仓库 release 自带完整 `lua-runtime-*`
- [ ] 已接受 `lua-runtime-packages-*` 与 `lua-deps-*` 来自 `luaskills-packages`
- [ ] 若使用 packaged runtime，已检查 `resources/luaskills-packages*` 元数据完整

### 发布维护者

- [ ] 已按 `luaskills-packages -> luaskills -> SDKs -> examples` 顺序发布
- [ ] SDK 默认 packages 协议线已设为 `0.1`
- [ ] 未再保留旧的 directory-style roots / runtime-session 兼容回退说明

## 8. 常见问题

### 升级到 `0.3` 后，Rust 宿主一定要改业务代码吗？

不一定。大多数按正式 Rust API 接入的宿主，只需要升级依赖版本和少量配置字段。

### 为什么 `luaskills` 主仓库 release 里看不到旧的 `lua-runtime-*`？

因为从 `0.3` 开始，这部分资产已经拆到 `LuaSkills/luaskills-packages` 独立发布。

### 为什么 packaged runtime 在 `0.3` 会直接报缺文件？

因为 `0.3` 新增了 `luaskills-packages` 元数据强校验，避免不完整 runtime 被悄悄装起来后在更后面的阶段出问题。

## 9. 推荐阅读

- [中文首页](../../README.zh-CN.md)
- [中文文档入口](index.md)
- [FFI 宿主接入检查清单](ffi/host-checklist.md)
- [FFI 对接文档](ffi/integration-guide.md)
- [Lua Skill 开发手册](skill-development.md)
