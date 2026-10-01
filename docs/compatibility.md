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
| CIRCLE / ARC / ELLIPSE | 实现 | 实现(保留 normal/OCS) | 实现 | 实现 | 距离实现；面积限共面环 |
| SPLINE | 实现 | 近似(缺 knots/weights 语义) | 近似离散 | 实现 | 近似(标记 tessellated) |
| POINT | 实现 | 实现 | 实现 | 实现 | 部分 |
| SOLID / TRACE / 3DFACE | 实现 | 网格(四边形→三角) | 网格 | 实现 | 不支持 |
| INSERT / 块 | 实现(定义与实例分离) | 实现 | 实例展开块几何(嵌套有界，保留 InstancePath) | 实现 | 不支持 |
| TEXT / MTEXT | 实现(保留字体名/字高/旋转) | 实现 | 整形为线段：outline(TTF/OTF/WOFF)+SHX(shapes/unifont/bigfont)，缺失字体走回退链 | 同上 | 不支持 |
| HATCH | 实现 | 部分(椭圆/样条边界近似) | **不支持**(落 Opaque，无表示) | **不支持** | 不支持 |
| DIMENSION | 实现 | 经匿名块引用 | 展开匿名块几何(线/箭头/文字)；无块名时 Partial | 经展开几何(AABB) | 不支持 |
| MESH / PolyfaceMesh | 部分 | 网格契约 | 网格 | 实现 | 不支持 |
| 3DSOLID / BODY / REGION / SURFACE (ACIS) | Opaque(未解码) | 交换契约 | 未实现 | 未实现 | 不支持 |
| 天正/探索者代理 | 实现(仅公开缓存记录) | 仅 FillOff/UnicodeText 有证据 | 依解码结果 | 实现 | 标记缓存几何 |
| 布局 / 视口 | 实现(矩形裁剪/比例) | 复杂裁剪标记部分 | 未装配纸空间渲染 | 部分 | 纸空间测量显式禁用 |

## 三维

- 观察（轨道/标准视图/正交-透视）：数据结构与命令存在（`StandardView`、
  `Projection`），**渲染未实现**。
- 直接网格（Mesh/3DFACE）：数据与离散实现，着色/法向可用于 GPU，**未运行**。
- ACIS 曲面离散：`cad-kernel-adapter` 为契约（`SolidTessellator`），
  **未实现**，返回 NotImplemented。

## 平台

- Android：可编译并打包 APK（见 `docs/validation.md`）；**未真机运行**。
- Web：无 Wasm 导出/JS 宿主，**不可运行**；WebGL2/WebGPU 均未验证。
- iOS/Linux/macOS/Windows：仅 `cad-platform` 抽象，**无宿主**。

## 已明确的不支持项

字体/图标等资源授权未核查；TTF/OTF/WOFF 与 SHX（shapes/unifont/bigfont）文本已整形为
线段绘制，缺失字体按回退链替代（源码见 `docs/fonts.md`）；TEXT 对齐、复杂文字整形、
动态块求值、注释性缩放、复杂视口裁剪、代理无缓存几何均按样本标记为未支持或未验证，
不会报告为完整图纸。
