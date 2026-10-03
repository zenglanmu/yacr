# Concept 界面重设计（2026-10-03）

设计依据为 `ui-spec/ui-desc.md` 和四张 concept 图；图中图纸/尺寸是参考内容，不作为产品假数据。

## 里程碑

1. 配置与纯布局契约：`cad-app::viewer_config`，逻辑像素 720/1200 断点，短屏紧凑布局、
   安全区扣除、48px 触控、原子配置更新、纯画布强制隐藏、恢复组件偏好。
2. Slint 外观与组合：浅银色 chrome、蓝色选中、深色 CAD 画布；桌面侧栏与底部布局/状态，
   移动标题/底部工具入口与抽屉，紧凑布局收起 Ribbon。
3. Linux lavapipe 核心/离屏主回归；最后一次 WASM + Playwright 第二层集成。

## 配置边界（必须明确）

当前协议只实现 `schemaVersion`、`ui.preset/layout/components` 的布局子集。
不支持的配置字段拒绝而非忽略；`features/view/interaction`、工具分组/排序/placement、
宿主允许的用户偏好合并、配置事件与浏览器公开 setConfig 接口仍未闭环。
这不是 ui-desc 完整配置协议验收；既有命令层 Work/Viewer 授权继续有效。

## 验证原则

用户本轮明确解除原生 Slint 编译限制并授权 sudo apt 安装开发依赖，使用 Slint
官方 `FemtoVGWGPURenderer` 自定义离屏平台，Linux wgpu/lavapipe 作为 UI 主反馈路径。
不运行 Android 模拟器、不安装窗口系统；WASM headless 只做最后集成抽查。
软件 GPU 不代表真实 GPU/真机。

里程碑 1 已执行：`cargo test -p cad-app viewer_config --offline`：3 项合成配置契约通过。
原生 UI 初次编译失败于缺少 pkg-config/fontconfig 开发依赖；失败记录不算通过。
随后系统依赖安装成功。里程碑 1 完整核心门禁：fmt、严格 clippy、架构/fixture/workflows/i18n、
wasm 全 workspace lib 检查通过；核心串行 968 passed / 0 failed / 1 ignored（合成，lavapipe）。
未设置真实 DWG 环境变量，按样本跳过的用例不计真实图纸执行证据。
