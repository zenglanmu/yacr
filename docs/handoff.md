# 后续 agent 接手入口

1. 阅读规范、AGENTS.md、requirements.md、architecture.md 与最近 Git diff。
   此次恢复中发现并发实现与提交，结束时可能仍有其他实现工作；本文描述的是框架交接，
   请以当前源码和最新构建证据确认状态，不覆盖陌生变更。
2. 运行 README 的 check/test/architecture 命令；不要删除陌生的已有实现。
3. 用 `rg 'pending\(' crates apps` 查找显式占位；文件路径与字符串标识对应实施单元。
4. Android 的 Slint + wgpu 组合已验证到打包层（ADR 0002、docs/validation.md）；
   本轮进一步在无头模拟器（KVM + SwiftShader，x86_64）实际安装、启动、渲染并验证
   画布平移/适应（docs/validation-android.md）。**真机仍未运行**。Web 已可运行：
   wasm 静态产物在无头 Chromium 以 WebGL2 通过加载/导航/双语（docs/validation-web.md）；
   **WebGPU 与真实 GPU 未验证**。
5. 逐项实现 Builder/事务/历史 → 导入/基础语义 → 显示/索引/场景 → 应用/工具 → 宿主闭环；
   不把这个依赖顺序当作排期，也不绕过优先 GPU/UI 验证。
6. 每次移除占位同步补齐规范测试、能力表、构建说明、Git 提交。

## 本轮状态（compact，2026-10-03）

**修复与 Pages 发布轮（最新，2026-10-03）**：继续修复 Web 第二触点/touchcancel 不取消
shell 绘制捕获、Web/Android 成功换图纸后残留旧取点、TRIM 使用固定世界拾取容差的问题。
新增 `check-web-drawing-safety.mjs`，真实浏览器证明第二触点（未移动）、touchcancel、换图纸
之后确认不写库；TRIM 复用共享屏幕像素拾取容差。核心 **963/0/1 ignored**、JS **28 passed**，
跨平台编译/严格 clippy/架构/i18n/fixtures/workflows 通过。

发布候选 `/tmp/opencode/yacr-pages-release/` 含完整 99 个同源 CAD 字体；四种工具、输入安全、
桌面 UI 通过。mobile 三场景独立进程复跑通过（`/tmp/opencode/yacr-pages-mobile/`），
此前超时记录不删除；最终 Ribbon 触控复验仍超时，**不宣称最终 Ribbon 回归通过**。
修复提交 `01f9f2b` 已推送 main 并部署至已有 `yacr-examples` 的生产分支。
生产地址 `https://yacr-examples.pages.dev/`（自定义域名 `yacr-examples.snakeheartgo.top`）；
两地址 wasm 哈希匹配验证包。线上桌面 UI、LINE/CIRCLE、输入取消安全通过；MOVE/TRIM
线上截图超时，本地像素通过，**不能宣称所有线上绘制回归通过**。详情见 validation-web §11。
Android 本轮仍只有编译门，未新增 APK/真机/WebGPU 证据。

**绘制/编辑合并后主控验收（历史）**：基于 `39077ec` 的数据库写事务、应用命令、
UI 工具及 Web/Android sink 合并结果，完成核心与无头 Web 验证；没有另启 subagent。

- 修复命令后渲染仍持有旧底图 Arc：两宿主状态漏斗通过 `CadView::sync_drawing`
  发布当前数据库快照，创建/编辑/撤销/重做可见；导航与叠加保持原 Arc，不重导入。
- 修复绘制主指针同时进入宿主导航/选择：MOVE 取空白锚点不再清空选择，取消不残留
  意外选择高亮。绘制 sink 返回真实命令错误，失败保留捕获参数，不冒充提交成功。
- 新增 `scripts/check-web-drawing.mjs`：真实 Slint 命令栏取点/确认，经宿主与事务到
  WebGL2；LINE/CIRCLE/MOVE/TRIM 四场景均改变底图像素，撤销恢复相同 CAD 像素哈希；
  LINE/CIRCLE 确认前与取消不新增实体，重做恢复实体计数。每种工具独立进程串行运行。
- 核心串行 **963 passed / 0 failed / 1 ignored**；JS **27 passed**；fmt、核心与 app-web
  严格 clippy、架构、i18n（159 keys）、fixture/workflow、wasm workspace lib、UI/Web wasm
  测试编译、Android aarch64 测试编译通过。UI/宿主 Rust 测试为**编译而非执行**。
- 本地无头 Chromium/SwiftShader：四工具、overlay、ribbon（1280/390/320px）、桌面 UI
  通过；证据 `/tmp/opencode/yacr-drawing-final/`、`/tmp/opencode/yacr-draw-final-*`。
  `check-web-mobile.mjs` 两次超时，本轮**未通过**；连续截图停滞记录在 validation-web §9。
  本轮未部署生产、未运行 Android APK/真机/WebGPU，测试 bundle 不含 CAD 字体。

详情：`docs/drawing-edit.md` §6、`docs/validation-web.md` §9。绘制/编辑是受控内存库
子集，不支持保存修改后的 DWG；不能据此宣布规范的“未来底图编辑”完整产品验收。

**UI 宿主接线轮 2（历史，iterations 2–4）**：Ribbon 文档的宿主接线与渲染叠加全部闭环，
并继续补齐审计项。四个迭代的顺序合入均通过主控验证：

- **叠加层宿主接线**：`CadView::set_selection_highlight/set_measurement_preview/
  set_annotation_preview` 已由 Web 与 Android 的状态漏斗在每次命令/拾取/打开/确认取消后
  推送；选择变化复用底图与批注 Arc。
- **选择高亮端到端修复**：修复 Web 画布点击永不可选（`!was_dragging` 门把每次点击都当
  拖动拒绝）；现在单击命中即派发 `Select`，无头截图证明圆被高亮且清空回到基线。
- **测量存为批注（F06/F07）**：`CommandId::SaveMeasurementAsAnnotation`，一次事务/一次
  撤销，无记录时 `InvalidInput`；UI 按钮「存为批注」仅在确认记录存在时可用。
- **查看/工作模式（U02）**：`CommandId::SetMode` + 命令栏开关，权限仍在命令层强制。
- **窄屏命令栏**：手机上模式开关移入可展开行，320px 不再挤压命令输入框。
- **异步导入（F01）**：核心快照 + `ImportProgressUiState`；Web 在 wasm（无线程）只推真实
  终态并诚实说明，Android 走真实 `std::thread` worker + 100ms 轮询 + 取消；面板无伪造进度。
- **可访问性（U12）/ 状态重叠（U08）**：`aria-live=polite` 人类摘要区域、`role=application`
  可聚焦画布、`host-state` 就绪时隐藏且不再双重播报。
- **U07**：Android 暴露 `set_surface_size` 入口（Activity 回调仍为显式未接钩子）。

证据：核心串行 **830 passed / 0 failed / 1 ignored**；JS 契约 **26 passed**；i18n
**151 keys**；架构、fixture manifest、workflows、Android aarch64 检查全通过；无头
Chromium（SwiftShader/WebGL2）ribbon/UI/mobile/overlay 四套脚本在 1280×800、390×844
DPR3、320×740 DPR2 通过，并直接对 **Cloudflare Pages 生产 URL 重跑通过**。新增
`scripts/check-web-overlay.mjs` 端到端证明选择高亮改变像素并清空回基线。仍开放：真机、
WebGPU/真实 GPU、Android Activity resize/SAF。（绘制/编辑后续进展见上方主控验收。）

**UI 宿主接线轮（iteration 1）**：Ribbon 文档里"宿主连接器（本轮范围外）"与"预览几何尚未
接线"两项已落地并端到端验证。三个并行 workstream 已合入 main：

- **Web 宿主**：`browser/state_push.rs::push_panel_state` 成为唯一状态漏斗，在每次
  命令/打开/批注变更/启动恢复后推送历史可用性（undo+redo 独立）、测量、图层+有序 id、
  属性、批注+有序 id、布局、诊断抽屉（来自真实 `ImportReport`）；安装
  `WebCanvasPickMapper`（逻辑像素→世界点，退化输入返回 None）；无工具时的轻触经
  `pick_at_screen` 派发 `Select`（空命中＝清空选择），不再静默。新增
  `diagnostics_report_json`。空态/多值文案全部来自目录。
- **渲染叠加**：`cad-app::render_scene::overlay` 新增选择高亮（按 `drawing_pick_items`
  展开 INSERT，未解析引用显式诊断不伪造）与工具预览（测量/批注 rubber-band + 捕捉点
  + 矩形/椭圆）批次；`CadView::set_selection_highlight/set_measurement_preview/
  set_annotation_preview` 独立 `overlay_revision`，选择变化复用底图与批注 Arc。
  `prepare_shared_with_overlays` 为向后兼容的新入口。高亮 `draw_order` 900_000，
  预览 2_000_000，位于底图与批注之间/之上。
- **Android + Web 手势**：Android 接入同一状态漏斗 + `AndroidCanvasPickMapper` +
  tap/drag 判定（`InputPolicy`）+ `apply_surface_size`（横竖屏不改相机中心；Activity
  resize 回调仍为显式未接钩子）；修复 `web/host/touch.js` 手指数变化时重定基准会跳变
  的缺陷，`node --test scripts/test-web-touch.mjs` 6 passed。

证据：核心串行 **819 passed / 0 failed / 1 ignored**；wasm lib check、clippy、
架构、i18n（132 keys）、fixture manifest、Android aarch64 check 全通过；无头 Chromium
（SwiftShader/WebGL2）ribbon/UI/mobile 三套脚本在 1280×800、390×844 DPR3、320×740
DPR2 通过，并直接对 **Cloudflare Pages 生产 URL 重跑通过**。截图
`/tmp/opencode/yacr-ribbon-final/`、`/tmp/opencode/yacr-ui-final.png`。仍开放：真机、
WebGPU/真实 GPU、Android Activity resize/SAF、INSERT 根选择高亮（仅叶子高亮）。
详见 `docs/ribbon-ui.md`、`docs/ui.md`、`docs/panels.md`。

**Ribbon UI 布局轮**：保留 Slint/shared-wgpu，界面拆成 app/ribbon/button/command-bar/
canvas/panels，桌面 Ribbon 与命令栏可收展；手机默认仅底部命令栏，TOOLS 打开工具。
手机 48px 按钮，真实 Slint 文件选择器导入 DWG、双指缩放与拖动已接应用命令。
未接绘制/编辑菜单禁用且显式标注，交接详见 `docs/ribbon-ui.md`。独立无 Slint CAD
显示 wasm 尚未提取，当前 app-web.wasm 不算独立引擎，由下一阶段实施。

**渲染桥职责重构**：继续采用 Slint/shared Device/Queue。CPU controller 移至
`cad-app/render_scene`，GPU runtime 与 Slint presenter 分离；字体 revision、
上传成功后发布、底图/批注组更新、画面脏标记、纹理身份和设备生命周期已接线。
`IncomingDocument=None` 明确表示关闭文档。详见 `docs/bridge-runtime.md`。
最终核心串行测试 875 passed / 0 failed / 1 ignored；新增 GPU 契约实际运行。
桌面及 320/390px 高 DPI 无头 WebGL2 回归通过，UI-only 语言切换不增加 CAD 绘制数。
原生 Slint 测试仍受环境阻塞；wasm 测试仅编译。后台 CPU 准备、分批上传预算和真实
设备丢失恢复尚未验收。并行核心 GPU 测试一次驱动 SIGSEGV，串行重跑通过，不能隐去。

**宿主结构拆分**：Web Rust 宿主拆为 documents/annotations/fonts/input/persistence，
JS 宿主拆为 i18n/files/renderer/runtime；Slint 桥分离 scene/camera/tests，既有接口和
13 项桥测试保留。核心 870 passed、新增 JS 契约 5 passed、Web release 与两次本地
Playwright 通过（含模块 HTTP 200、批注下载/空 sidecar 回导与状态保留）。CI 与部署
检查包含所有 JS 模块。结构与环境阻塞见 `docs/code-structure.md`、
`docs/validation-web.md` §6；原生 Slint 测试未执行完成，不能当作通过。

四个并行 workstream 已合入 main 并验证：

- **Android 运行闭环**：x86_64 release APK 在无头模拟器安装/启动/渲染；修复画布输入
  未接线（现在单指拖动平移、滚轮/捏合缩放）、初始状态文案、后端日志；像素 diff 证明
  平移生效。证据 `docs/validation-android.md`（含截图/日志）。
- **Web 运行闭环**：修复桥固定 WebGPU、Slint 缺 `renderer-femtovg-wgpu`、wasm 轮询
  误判设备丢失、WebGL2 MSAA present 失败；`web-dist/` 在无头 Chromium 以 WebGL2 通过
  加载/导航/双语，语言偏好持久化；B29 冒烟脚本修复。证据 `docs/validation-web.md`。
- **3D / 纸空间宿主接线**：bridge 按 `SpaceSelection` 与视图模式分派 `render`/`render_3d`，
  UI 提供 2D/3D、投影、标准视图、拖动轨道与布局选择；退化/不支持视图显式诊断。
  宿主编排者已补 `CadView::sync_session`（空间+相机+模式）。`docs/view-3d.md`。
- **透明度与代理**：acadrust `Transparency`（ByLayer/ByObject/ByBlock）经
  `DisplayFragment.alpha` 进入透明管线；代理保留全部片段与真实来源/精度。软件 Vulkan
  透明合成测试通过。`docs/render-order.md`、`docs/proxy-support.md`。

核心测试 **573 passed / 0 failed**；Wasm、Android target、fmt、clippy(0)、架构、
`check-i18n.py`（116 keys）、`check-fixture-manifest.py`、`check-workflows.py` 全通过；
`cad-render-wgpu` 在 lavapipe 下 47 passed（含透明合成）。

**集成轮 2（核心 CAD，Linux 构建）**：曲线升级为真实 NURBS + 椭圆 OCS 法向 +
仿射真椭圆 + 解析交点（`docs/curve-geometry.md`）；F15 ACIS 打通
acadrust SAT/SAB → 中性 B-rep → 平面/球/柱/环面子集离散（`docs/kernel-acis.md`，
合成夹具入 manifest）；纸空间 4 角视口/正确比例 + 空间感知测量（`docs/layouts.md`、
`docs/measure.md`）；对象捕捉六类 + HATCH 多环含孔洞填充。核心测试 **667 passed /
0 failed**。

**集成轮 3（核心 CAD，Linux 构建）**：实体颜色（ByObject/ByLayer/ByBlock，ACI/RGB）与
线宽（mm）进入渲染（`docs/entity-style.md`，线宽显式不绘制）；MTEXT 格式 run 解析与
整形（`docs/mtext.md`，堆叠分数/颜色/装饰为显式 Partial）；导入 4 角纸空间视口/完整变换/
INSERT/OCS/SOLID（闭合 round-2 `docs/layouts.md` §3.1）；网格面 `SubElementId` 拾取 +
选择高亮叠加（`docs/picking-3d.md`）。核心测试 **735 passed / 0 failed**，软件 Vulkan
**50 passed**；fmt/clippy/架构/i18n/fixtures/wasm `--lib` 全通过（仍只构建 Linux 核心，
未构建 Android/Web）。仍开放：ACIS 真实授权样本与锥面/带环球面、LINETYPE 虚线、渐变
HATCH、动态块求值、注释性缩放、打印/出图、异步可取消导入与进度、性能基准与预算实测。

**集成轮 4（核心 CAD，Linux 构建）**：LINETYPE 虚线端到端（`docs/entity-style.md`）；
纸空间布局出图到 PNG + `PLOTSETTINGS` 导入（`docs/plot.md`）；性能预算
（`cpu_bytes`/`queued_tasks`/`upload_bytes_per_frame`）真实计费 + 可复现 benchmark
（`docs/performance.md`）；渐变 HATCH 逐顶点颜色烘焙（`docs/hatch-gradient.md`）；
异步可取消导入（进度 + 过期结果丢弃，F01）（`docs/import-async.md`）。集成测试
**824 passed / 0 failed**（lavapipe），fmt/clippy/架构/i18n/fixtures/wasm `--lib` 全通过。
仍开放：矢量出图（PDF/HPGL/CTB）、复杂/嵌入形状线型、宿主异步导入面板接线、
手机内存/FPS 实测。

**集成轮 5（核心 CAD，Linux 构建）**：动态块可见性状态读取与切换（GEOMETRY 增量，
`docs/dynamic-blocks.md`）；ACIS 锥面（完整圆锥 + 截头圆锥）与环形圆环面离散，修复
截头圆锥母线缝合在环长不等时夹紧末点导致的非流形开边（`docs/kernel-acis.md`）；
注释性缩放：`Scale` 表 + `CANNOSCALE` 导入、TEXT/MTEXT 按活动比例缩放并支持
按比例位置覆盖，非文本/非法比例显式 `Partial`（`docs/annotative-scaling.md`）。集成测试
**870 passed / 0 failed**（lavapipe），fmt/clippy(0)/架构/i18n/fixtures/wasm `--lib` 全通过。
仍开放：真实授权 ACIS 样本、带环球面、非圆椭圆/样条面、矢量出图、复杂线型形状、
宿主比例切换与异步导入面板接线、真机内存/FPS 实测。

仍开放（受环境/外部依赖限制）：**F15 真实 ACIS 离散**（需内核 + SAT/SAB 解析器 + 授权样本）；
**真机**（模拟器 SwiftShader 不等同真机）；**WebGPU / 真实 GPU / 移动与桌面浏览器矩阵**；
Android surface 尺寸/安全区（U07）、SAF、量测/批注拾取与面板状态推送；自托管 GPU/Android
runner；**授权 DWG/字体/黄金图**（`fixtures/manifest` 仍无授权样本）；MultiLeader；
复杂文字整形；自动保存/崩溃恢复保留策略。详见各功能 `docs/*.md` 的"未完成"。

## 已定义，但尚需设计审查

- RenderTarget/HostTexture 当前仅为自有数字 token；共享 Device/Queue 的拥有者、
  token 生命周期与安全访问方式须在平台合成 ADR 中固定，不可凭 token 猜测 GPU 对象。
- AnnotationId(u128) 只表达 UUID 位宽；生成/碰撞防护由宿主服务和数据库校验完成。
  JSON UUID 编码、时间格式、extension 原始 JSON 校验尚未实现。
- FingerprintPolicy 的显式策略不意味着导入已经能自动建立锚点；fingerprint 计算需分段/异步。
- CommandPayload 的类型校验、防止 ID 与 payload 不匹配、事务权限与撤销能力仍需接线。
  当前 Application 提前检查模式与文档身份，但 Work 模式执行仍返回 NotImplemented。
- RepresentationProvider 的字体/样式上下文目前只提供最小配置，需要扩充自有只读 views。
- DB 中已有基础实现应保留并复查几何边界/块实例/验证完整性，不代表工业语义已验收。
- 数据库/几何已有基础真实实现与合成测试，代理也已有有界 framing/记录回放代码。
  这些是在恢复过程中保留的新增实现；需审查实际支持范围，不能据此宣称真实 DWG/工业兼容。
- 扩展点包含 Importer/Tool/CommandHandler 等；注册/优先级/冲突策略需统一实现。
- PropertyProvider、字体 shaping、缓存容量、Worker 二进制传输、Journal codec、schema migrations
  都需要完整接线和验收，不允许以现有结构体宣称闭环。

## 接线验收

业务不调用 draw；批注确认一次事务，取消零事务；UI 回填不触发重复命令；
revision 不连续重建；GPU 重建不改数据库；设备失败和退出保护未保存批注；
局部失败保留对象级报告；未知源单位显示图纸单位。Android/Web 真机与浏览器分开记录。
