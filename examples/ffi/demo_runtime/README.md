# FFI Demo Runtime

## 1. 目录作用

这个目录提供一套最小可运行的 LuaSkills FFI 演示环境，用于：

- 创建空运行时目录
- 通过 FFI 创建引擎
- 动态安装 `OpenVulcan/luaskills-demo-skill`
- 调用 `luaskills-demo-skill-demo-status`
- 输出 success 级别的烟测结果

## 2. 目录结构

- [D:\projects\vulcan-luaskills\examples\ffi\demo_runtime\runtime_root](/D:/projects/vulcan-luaskills/examples/ffi/demo_runtime/runtime_root)
  - 演示使用的空运行时根目录
- [D:\projects\vulcan-luaskills\examples\ffi\demo_runtime\run_python_install_demo.py](/D:/projects/vulcan-luaskills/examples/ffi/demo_runtime/run_python_install_demo.py)
  - 可直接运行的 Python FFI 安装与调用烟测脚本

## 3. 前置条件

运行前需要准备：

1. 已构建好 `vulcan-luaskills` 动态库
2. 设置环境变量 `VULCAN_LUASKILLS_LIB`
3. 当前网络可访问 GitHub Release

## 4. 运行方式

```powershell
python .\examples\ffi\demo_runtime\run_python_install_demo.py
```

运行成功后，脚本会：

1. 清理 demo skill 在当前 runtime root 下的旧安装痕迹
2. 创建引擎
3. 加载空 roots
4. 动态安装 `OpenVulcan/luaskills-demo-skill`
5. 调用 `luaskills-demo-skill-demo-status`
6. 校验返回结果
7. 输出 success

## 5. 说明

当前 smoke demo 选择 Python 是因为：

- ctypes 接入成本最低
- JSON FFI 调试体验最好
- 便于快速验证 install / call 主链

Go 和 TypeScript 示例仍保留在：

- [D:\projects\vulcan-luaskills\examples\ffi\go\demo.go](/D:/projects/vulcan-luaskills/examples/ffi/go/demo.go)
- [D:\projects\vulcan-luaskills\examples\ffi\typescript\demo.ts](/D:/projects/vulcan-luaskills/examples/ffi/typescript/demo.ts)

它们当前主要用于说明最小标准 FFI 对接方式。
