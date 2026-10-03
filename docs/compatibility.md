# 兼容性与能力矩阵

**没有任何已验收的 DWG 版本、实体或平台。** `fixtures/manifest` 现含合成夹具与一个
开源 QCAD `flange` 样本（`Partial`，`fixtures/dxf/qcad-flange/`），因此下表的
「实现」仅表示代码路径存在并有契约测试，**不代表对真实图纸兼容**。构建证据
不等于兼容性（规范 §1、§11.4）。2026-10-01 已对 11 份公开样本执行导入端到端
（AC1014–AC1032，见 `docs/validation.md`），仍未构成验收。

## 实体能力（读取 / 语义 / 绘制 / 拾取 / 测量）

2026-10-03 新增 DXF ASCII/二进制导入入口，共用 DWG 语义/数据库转换，只有合成 LINE
与截断拒绝针对性契约，不宣称复杂 DXF 兼容。旧式 SHX 字号修复与网络 DWG 对比见
`docs/ui-dxf-text-fixes.md`；原字体缺失的回退整形保留 Partial。

| 对象 | 读取 | 语义 | 绘制 | 拾取 | 测量 |
|---|---|---|---|---|---|
| LINE | 实现 | 实现 | 实现(线) | 实现(AABB) | 实现(2D/3D) |
| LWPOLYLINE/POLYLINE(bulge) | 实现 | 实现 | 实现(含 bulge) | 实现 | 折线长度实现 |
| CIRCLE / ARC / ELLIPSE | 实现 | 实现(保留 normal/OCS)；非均匀仿射重解为真椭圆 | 实现 | 实现 | 距离实现；面积限共面环 |
| SPLINE | 实现 | 实现(保留 degree/knots/weights，真实 NURBS) | 弦高自适应离散 | 实现 | 解析/采样(与显示 LOD 解耦) |
| POINT | 实现 | 实现 | 实现 | 实现 | 部分 |
| SOLID / TRACE / 3DFACE | 实现 | 网格(四边形→三角) | 网格 | 实现 | 不支持 |
| INSERT / 块 | 实现(定义与实例分离) | 实现 | 实例展开块几何(嵌套有界，保留 InstancePath) | 实现 | 不支持 |
| TEXT / MTEXT | 实现(保留字体名/字高/旋转) | 实现 | 整形为线段：outline(TTF/OTF/WOFF)+SHX(shapes/unifont/bigfont)，缺失字体走回退链 | 同上 | 不支持 |
| HATCH | 实现 | 部分(椭圆/样条边界近似) | 边界环 + 多环实心填充(含孔洞，偶奇) / 图案线；超预算/自交降 Partial | 经边界(AABB) | 不支持 |
| DIMENSION | 实现 | 经匿名块引用 | 展开匿名块几何(线/箭头/文字)；无块名时 Partial | 经展开几何(AABB) | 不支持 |
| MESH / PolyfaceMesh | 部分 | 网格契约 | 网格 | 实现 | 不支持 |
- 3DSOLID / BODY / REGION / SURFACE (ACIS) | 实现(acadrust `entities::acis` 解析) | 部分(平面/球/柱/环面/锥面子集) | 子集离散：闭合 `Success`，否则 `Partial`；带环球面/非圆椭圆/样条面 `Unsupported` | 经网格 | 不支持(近似标记) |
| TEXT/MTEXT 格式 | 实现(格式 run 解析) | 部分(堆叠分数/颜色/装饰/行对齐为显式 Partial) | 按 run 整形/换行 | — | — |
| 实体颜色/线宽 | 实现(ByObject/ByLayer/ByBlock，ACI/RGB) | 实现(颜色)；线宽仅携带 | 颜色进入 shader；线宽**显式不绘制** | — | — |
| LINETYPE 虚线 | 实现(名称/表/线型比例) | 实现(ByObject/ByLayer/ByBlock) | 按弧长细分 dash/gap；复杂线型仅 dash、显式 Partial | — | — |
| 渐变 HATCH | 实现(gradient_color) | 部分(LINEAR/SPHERICAL/CYLINDER 精确；其余显式 Partial) | 逐顶点颜色烘焙，裁剪于边界 | 经边界 | 不支持 |
| 出图(打印) | 实现(PLOTSETTINGS/内嵌 Layout) | 实现(纸张/边距/比例/旋转) | 光栅 PNG(host readback)；无矢量/CTB | — | — |
| 异步导入 | 实现(进度阶段/取消) | 实现(唯一任务、过期丢弃) | — | — | — |
| 天正/探索者代理 | 实现(仅公开缓存记录) | 仅 FillOff/UnicodeText 有证据 | 依解码结果 | 实现 | 标记缓存几何 |
| 布局 / 视口 | 实现(矩形裁剪/比例) | 复杂裁剪标记部分 | 未装配纸空间渲染 | 部分 | 纸空间测量显式禁用 |

## Linux 宿主（2026-10-03 新增，主验收入口）

`apps/app-linux` 共用桌面与官方 Slint/wgpu 离屏宿主，Linux release App 成为第一验收标准。
软件 Vulkan 合成 smoke、窗口系统、真实 GPU、真实 DWG 结论独立，当前窗口/真实 GPU 未验收。
文件打开/侧车通过显式路径参数；无文件选择器/恢复决策/后台导入/Trim 取点时显式 Unsupported，
不会默默 discard 未保存批注。实现及本轮运行证据见 `docs/linux-app.md`，不更改实体兼容结论。

## 三维

- 观察（轨道/标准视图/正交-透视）：`cad-app` 的相机/投影/标准视图/轨道命令与 Slint
  桥已接线，`BeforeRendering` 按当前空间与视图模式选择 `render`(2D) 或 `render_3d`
  (`Camera3d`)，UI 提供 2D/3D、投影、标准视图与拖动轨道（`docs/view-3d.md`）。
  **真实设备上的 3D 出图仍未验证**（模拟器 SwiftShader 只覆盖 2D；软件 Vulkan 的
  网格测试见 `docs/validation.md`）。
- 直接网格（Mesh/3DFACE）：数据、法向、深度、绘制顺序与透明通道已实现；透明合成有
  软件 Vulkan（lavapipe）无头测试。真实 GPU 行为未验收。
- 底图透明度：importer 读取 acadrust `Transparency`（`ByLayer`/`ByObject`/`ByBlock`）
  → `DisplayFragment.alpha` → `RenderBatch.alpha`，由渲染器分类为不透明/透明/不可见
  （`docs/render-order.md`）；plot-style 表 alpha 未应用（acadrust 未暴露解析值）。
- ACIS 曲面离散：`cad-kernel-adapter` 为契约（`SolidTessellator`），**未实现**，
  返回 `Unsupported`/`NotImplemented`（无内核、无 SAT/SAB 解析器、无授权样本）。

## 平台

- Android：aarch64 可编译并打包 APK；x86_64 release APK 已在无头模拟器（KVM +
  SwiftShader）**实际安装、启动、渲染并验证画布平移/缩放**（
  `docs/validation-android.md`）。真机仍 **未运行**；SAF、surface 尺寸/安全区、
  量测/批注拾取、面板状态推送未接线。
- Web：wasm 静态产物可构建，`web-dist/` 在无头 Chromium（Chrome for Testing 153，
  SwiftShader）以 **WebGL2 实际运行**，加载、导航与语言切换通过
  （`docs/validation-web.md`）。WebGPU 路径、真实 GPU、移动/桌面浏览器矩阵 **未运行**。
- iOS/Linux/macOS/Windows：仅 `cad-platform` 抽象，**无宿主**。

## 已明确的不支持项

字体/图标等资源授权未核查；TTF/OTF/WOFF 与 SHX（shapes/unifont/bigfont）文本已整形为
线段绘制，缺失字体按回退链替代（源码见 `docs/fonts.md`）；TEXT 对齐、复杂文字整形、
动态块参数/夹点求值、复杂视口裁剪、代理无缓存几何均按样本标记为未支持或未验证，
不会报告为完整图纸。出图仅光栅 PNG（无矢量 PDF/HPGL/CTB）；线宽的复杂/嵌入形状段
仍按显式 `Partial`/未实现报告，不计入完成。动态块**可见性状态**读取/切换与
**注释性缩放（TEXT/MTEXT）**已实现，但动态块参数/夹点编辑、非文本注释性类型仍为
显式 `Partial`，且宿主比例切换 UI 未接线。
