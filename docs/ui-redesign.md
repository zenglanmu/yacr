# Concept 界面重设计（2026-10-03）

> 后续轮（2026-10-03）：按用户要求把外观从“浅银色 concept”改为 **AutoCAD 风格深色**
> （`ui/theme.slint`：深灰 chrome/panel、浅色文字、蓝色选中、近黑模型空间；并设
> `Palette.color-scheme = dark` 让 std-widgets 的命令栏/下拉/按钮/进度条也变深）。
> 布局结构与规格描述（Ribbon、图层/属性面板、浮动工具栏、命令输入区、模型/布局标签、
> 状态栏）不变。`concept_offscreen` 的“chrome 是浅色”断言相应改为“chrome 为深色”。
> 下面里程碑 2 的“浅银色 chrome”为历史描述，不再代表当前外观。

设计依据为 `ui-spec/ui-desc.md` 和四张 concept 图；图中图纸/尺寸是参考内容，不作为产品假数据。

## 里程碑

1. 配置与纯布局契约：`cad-app::viewer_config`，逻辑像素 720/1200 断点，短屏紧凑布局、
   安全区扣除、48px 触控、原子配置更新、纯画布强制隐藏、恢复组件偏好。
2. Slint 外观与组合：浅银色 chrome、蓝色选中、深色 CAD 画布；桌面侧栏与底部布局/状态，
   移动标题/底部工具入口与抽屉，紧凑布局收起 Ribbon。
3. Linux lavapipe 核心/离屏主回归；最后一次 WASM + Playwright 第二层集成。

## 配置边界（必须明确）

本轮闭环：`ViewerConfig` 完整协议类型（`ui`/`features`/`view`/`interaction`）、
稳定 `CommandId` 白名单、ribbon 分组/排序、placement、`initiallyOpen`、命令覆盖、
宿主允许的用户偏好合并（clamp 而非报错）、配置变更事件与 revision、有效配置查询、
Slint 分开的图层/属性显隐与命令可见性。数据驱动：配置不携带任何脚本/回调。

仍未闭环（不宣称完整验收）：
- `canvasOnly` 之外的精简预设对 ribbon 的实际重排仍是布局子集；`minimal` 语义待细化。
- 自定义 ribbon 分组已落地：`ui.components.ribbon.tabs[].groups[].commands[]` 现真正渲染为
  选项卡/分组/命令按钮并按 `commandVisibility` 过滤，点击映射到真实命令（未支持的合法
  命令显式报 `ribbon.command_unsupported`，不静默成功）。**未声明自定义 tabs 时内置 5 标签
  面板逐字节不变**。分组支持 `display`（`iconAndLabel`|`iconOnly`|`labelOnly`），每项按此
  渲染；组内命令超过 6 个时以内联溢出列表保留全部命令（**浮动锚定弹层仍未做**）。
- 可配置 ribbon 命令的派发缺口已收窄：`view.reset` 现派发真实 `ResetView`；其余 8 个
  `Unsupported`（`view.orbit`/`view.standard`/`backend.switch`/`annotation.delete|select|
  visibility`/`layer.toggle`/`layout.switch`）因缺少明确目标或手势而**显式分理由**
  （`ribbon.command_needs_target`/`ribbon.command_needs_gesture`），不伪造目标。
- 用户偏好不得重新启用被预设/能力禁用的命令：`ui.commandOverrides.<id>.visible` 现与
  `features.*`、`ui.components.*.visible` 一样按“只能关不能开”钳制（此前可越权打开
  `canvasOnly` 隐藏的命令），新增回归测试。
- `view.overlays.*` 已端到端门控绘制：`selectionHighlight`/`annotations`/`snapHints`
  控制选择高亮、已提交批注、预览光标十字与**真实对象捕捉标记**（`cad_measure::SnapKind`
  形状）；`axes`/`grid` 首次产生真实世界坐标参考
  几何（按图纸 bounds 生成、1/2/5 步长约 10 格、行数有上限、无 bounds 时显式诊断
  不伪造）。内置默认 `axes=true`、`grid=false`，与 `ui-spec/ui-desc.md` 示例和 AutoCAD
  一致。宿主在每次状态漏斗读取有效配置推送，切换只重建瞬态叠加层、不动底图；桌面
  状态栏另有 axes/grid/snapHints 实时开关（经配置存储重推）。捕捉标记的宿主喂入
  （`CadView::set_snap_hints`）已接线，并由 Linux/Web 状态漏斗从真实 `drawing_pick_items`
  候选喂入（`snap_candidates_near`；无光标时清空，实例变换烘焙、不可精确变换者显式跳过）。
- `interaction.pointer/touch/keyboardShortcuts` 已逐项门控输入：共享 Slint 适配器在
  事件时早退指针/滚轮/画布拾取与拾取映射；命令别名仅在 `keyboardShortcuts` 为真时
  展开（完整命令名仍可用）；Web 触控 wasm 入口在 `touch` 为假时空操作。门控在 Rust
  侧，尚未把三个标志作为 Slint 属性推送（UI 外观不因门控而变）。已知边界：画布拾取
  回调为指针与触控共用，若 `pointer=false` 而 `touch=true`，触控拾取也会被关闭；分离
  需要独立的触控拾取回调，尚未实现。
- 原生宿主已从磁盘读取 `$XDG_CONFIG_HOME/yacr/config.json`（完整 `ViewerConfig`）与
  `preferences.json`（`allowedPaths` 投影），并提供 `--config`/`--preferences` 覆盖；
  无 XDG/HOME 时显式不持久化，解析失败保留默认并报告。用户偏好的交互式改写入
  口仍未在原生 UI 中出现（当前仅通过宿主 API 应用后持久化）。
- 既有命令层 Work/Viewer 授权继续有效，与配置 `features` 是两套独立门控。

`UiHandle::set_config/set_config_json/update_config_json/apply_user_preference_json/effective_config_json`
已消费完整协议；`initiallyOpen`、分开的图层/属性显隐已接线。

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

## 里程碑 2：真实 Slint 原生离屏

新增 `cad-ui-slint::offscreen` 与 `scripts/check-ui-native.sh`：官方 FemtoVG/wgpu 自定义
无窗口平台，官方 snapshot 纹理读回；不使用重画的 HTML/图片替代 Slint。
共享 CAD 桥在相同设备提交，真实适应后的合成数据库进入画布。新图标为本项目绘制 SVG。

截图已抽查：`/tmp/opencode/yacr-concept-round4/` 的 desktop、mobile、mobile-tools、
mobile-layers；测试另产出 compact、narrow（320px）、canvas-only。
软件 Vulkan：llvmpipe / Mesa 26.0.8 / LLVM 21.1.8；单次渲染测试 11.66 秒（不含编译）。
真实指针事件证明打开与测量命令正确派发，布局/纯画布切换保留相机和底图 Arc。
UI 单元合成测试 **76 passed**；离屏集成 **1 passed**。
里程碑 2 最终门禁：扩大到 Linux UI 的严格 clippy、fmt、架构/fixture/workflows/i18n
（164 keys）、全 workspace wasm lib 检查通过；Linux 核心+UI 串行 **1045 passed / 0 failed /
1 ignored**。JS 模块契约 **28 passed**（不含 wasm/GPU）。最终本轮原生截图
`/tmp/opencode/yacr-concept-m2/`，离屏集成 11.85 秒。
首次包含编译的 120 秒命令超时保留；旧宽屏相机未 fit 的截图不算视觉通过。

设计未伪造：系统窗口按钮、DWG 保存、罗盘/坐标、标尺数值和图层颜色色块。
真实样本/真实 GPU/Android 真机及完整 ViewerConfig 仍不在本轮已验收范围。

## 里程碑 3：浏览器集成

首轮 `scripts/check-web-ui.mjs` 在 Chromium 153.0.8010.12 / SwiftShader WebGL2 通过：
模块全部 HTTP 200、导航 CAD 像素变化 0.266%、双语与持久偏好、349 字节空批注 sidecar
导出/回导，无控制台/page 错误。截图发现初始相机仍按整壳 fit 的裁切问题；随后修正 Web
启动只按真实 CAD 内容矩形 fit 一次，运行时布局调整仍保留相机。首轮截图留存，不作为完整视觉通过。

最终重跑：`/tmp/opencode/yacr-concept-final-web/integration.png` 已人工抽查，合成图边界
完整位于 CAD 区域；导航变化 0.417%，语言/偏好/空批注往返与错误检查再次通过。
release wasm 15,985,060 bytes，SHA-256
`2d60a7d5aa6e99af205d1e3ee063391f660debc49d58457435818e86c7f83dee`。
构建使用 `WITH_FONTS=0`：只带授权 UI 字体，未验证第三方 CAD 字体与真实 DWG。

最终门禁再次执行通过：fmt、Linux 核心+UI 严格 clippy、Web wasm 严格 clippy、四项 Python
门禁、全 workspace wasm lib check；Linux 合成串行 1045 passed / 0 failed / 1 ignored。
原生最终目录 `/tmp/opencode/yacr-concept-final-native/`，离屏集成 11.63 秒。
浏览器本轮为桌面抽查；移动在原生离屏验证，未宣称浏览器移动/真机完成。
