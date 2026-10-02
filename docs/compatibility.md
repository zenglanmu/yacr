# 兼容性与能力矩阵

**没有任何已验收的 DWG 版本、实体或平台。** `fixtures/manifest` 为空，因此下表的
「实现」仅表示代码路径存在并有合成契约测试，**不代表对真实图纸兼容**。构建证据
不等于兼容性（规范 §1、§11.4）。2026-10-01 已对 11 份公开样本执行导入端到端
（AC1014–AC1032，见 `docs/validation.md`），但样本未授权入库，仍不构成验收。

## 实体能力（读取 / 语义 / 绘制 / 拾取 / 测量）

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
| 3DSOLID / BODY / REGION / SURFACE (ACIS) | 实现(acadrust `entities::acis` 解析) | 部分(平面/球/柱/无环球面子集) | 子集离散：闭合 `Success`，否则 `Partial`；锥面/带环球面/非圆椭圆/样条面 `Unsupported` | 经网格 | 不支持(近似标记) |
| 天正/探索者代理 | 实现(仅公开缓存记录) | 仅 FillOff/UnicodeText 有证据 | 依解码结果 | 实现 | 标记缓存几何 |
| 布局 / 视口 | 实现(矩形裁剪/比例) | 复杂裁剪标记部分 | 未装配纸空间渲染 | 部分 | 纸空间测量显式禁用 |

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
动态块求值、注释性缩放、复杂视口裁剪、代理无缓存几何均按样本标记为未支持或未验证，
不会报告为完整图纸。
