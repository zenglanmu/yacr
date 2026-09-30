# 兼容性与能力矩阵

当前没有已验收产品平台、DWG 版本或实体。构建证据不等于兼容性。

| 对象 | 读取 | 语义 | 绘制 | 拾取 | 测量 |
|---|---|---|---|---|---|
| LINE、LWPOLYLINE/POLYLINE(bulge)、CIRCLE、ARC、ELLIPSE、SPLINE、POINT | 未实现 | 契约 | 未实现 | 未实现 | 未实现 |
| INSERT/块、TEXT/MTEXT | 未实现 | 基础契约 | 未实现 | 未实现 | 未实现 |
| SOLID/TRACE、HATCH、DIMENSION | 未实现 | opaque 预留 | 未实现 | 未实现 | 未实现 |
| 3DFACE、PolyfaceMesh、Mesh、3D 折线 | 未实现 | 网格/折线契约 | 未实现 | 未实现 | 未实现 |
| 3DSOLID、BODY、REGION、SURFACE | 未实现 | 交换数据契约 | 未实现 | 未实现 | 未实现 |
| 天正/探索者代理 | 未实现 | 代理契约 | 未实现 | 未实现 | 未实现 |
| 基本布局/复杂视口/动态块 | 未实现 | 部分结构预留 | 未实现 | 未实现 | 未实现 |

Android/Web 优先；iOS/Linux/macOS/Windows 只保留平台抽象，不提供虚构宿主。
Slint shell 未编译；WebGL2/WebGPU 未运行。字体授权与 SHX/BigFont 实际支持未核查。
acadrust 0.5.5 是未修改的正式依赖；当前 Importer 没有调用解析器。
