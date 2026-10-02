# ACIS 实体离散（F15：已实现一个可复现的受限子集）

本文件描述 `cad-kernel-adapter` 的对外契约，以及当前**真实**支持的 ACIS
（SAT/SAB）离散子集。默认直通实现仍保持诚实的 `Unsupported`；只有显式选用
`BrepTessellator` 且输入落在下述子集内时才会产出真实网格。

“已实现”只指：对**本仓库可复现的合成 SAT/SAB**，在文档化的曲面/曲线类型上
产出非空、闭合（或带有精确退化报告）的网格。它**不**声明对 AutoCAD /
Tianzheng / TSSD 等第三方实体的兼容性。

## 1. 边界

`cad-kernel-adapter` 是工作区中**唯一**允许持有内核/ACIS 交换数据的 crate。
跨边界只传递纯数据：

* `TessellationRequest` / `TessellationResult`（原始契约）；
* `SolidExchange`（见 §2）与其中的中性 `BrepData`。

`cad-import-acadrust` 是唯一依赖 acadrust 的 crate；它把 SAT/SAB 解析为
`BrepData`，此后再无 acadrust 类型越过内核 seam。`cad-kernel-adapter` 自身
**不**依赖 acadrust，也不解析 SAT/SAB 字节。

## 2. 交换类型

```rust
pub enum SolidExchange {
    Sat(Vec<u8>),        // 原始 SAT 文本（解析失败/未解析时保留）
    Sab(Vec<u8>),        // 原始 SAB 二进制（同上）
    Brep(BrepData),      // importer 解析后的中性 B-rep（真实网格路径）
    Unsupported { type_key: String, data: Vec<u8> },
}

pub struct BrepData {
    pub shells: Vec<BrepShell>,
    pub placement: Option<BrepPlacement>, // world = scale·(p·M) + t
}

pub struct BrepFace {
    pub id: u32,
    pub surface: BrepSurface, // Plane | Sphere | Torus | Cylinder | Unsupported
    pub reversed: bool,
    pub loops: Vec<BrepLoop>,  // Line | Circle | Unsupported
}
```

* `BrepData` 是**中性纯数据**：没有 acadrust 句柄、解析器或曲面对象。
* 未支持的曲面/曲线以 `BrepSurface::Unsupported` / `BrepCurve::Unsupported`
  建模，绝不以近似网格顶替。
* 原始 `Sat`/`Sab` 字节保留用于溯源；默认内核不解析它们。

## 3. 两个离散器

| 实现 | 行为 |
|---|---|
| `NoKernelTessellator`（默认，别名 `PendingTessellator`） | 对任何非空载荷返回 `Unsupported`，绝不伪造网格 |
| `BrepTessellator`（显式选用） | 对 `SolidExchange::Brep` 求值文档化子集；其余返回 `Unsupported` |

`BrepTessellator` 对原始 `Sat`/`Sab` 仍返回 `Unsupported(kernel.no_acis_kernel)`：
解析只允许发生在 importer。

## 4. 已支持子集（真实网格）

曲面/边界：

* **平面面**：直线边界多边形（凸/凹）与内环（洞）三角化；
  洞边界可为直线或完整圆。
* **完整圆**（`ellipse-curve` 且 `ratio == 1` 且首尾顶点相同）。
* **球面**：无环（`first_loop == NULL`）、覆盖完整参数域的单面。
* **圆环面（torus）**：同上，完整参数域单面；外/内主方向的环带分别采样。
* **圆柱侧面**：`cone-surface` 且 `sin(half_angle) == 0`，由两个完整圆环围成。
* **圆锥侧面**（`cone-surface` 且 `sin(half_angle) != 0`）：
  * **完整圆锥**：一个底面圆环 + 顶点奇点环，扇形三角化到顶点；
  * **截头圆锥**（frustum）：两个完整圆环，沿母线 zipper 缝合；两环
    采样点数不同时仍按角序合并，闭合处回绕到起点而非夹紧到末点。

离散细节：

* 平面：投影到面平面按 signed-area 区分外环/洞，桥接洞后 ear-clip，三角形
  仅使用源环顶点（不会产生 T 形接点）。
* 球/环/柱：按自然参数化采样；采样段数同时满足线性弦高与角度伺服，
  **随容差单调**（容差更小 → 面片不少于、误差不高于）。
* 网格顶点按位置焊接后检测边流形性：非“恰好两个面共享”的边以
  `kernel.open_edge` 精确上报。
* 精度：纯平面 → `Precision::Analytic`；含曲面 → `Approximate { error_bound }`
  （弦高上界由采样段数与曲率算出）。

## 5. 明确 Unsupported（返回缺面，不伪造）

* `cone-surface` 且 `cos(half_angle) == 0`（零锥角退化为平面），或
  `sin(half_angle) == 0` 且无法按圆柱处理者。
* 部分圆弧 / 椭圆（`ratio != 1` / 首尾顶点不同）。
* NURBS / spline / mesh 等未建模曲面。
* 带修剪环（trimming loops）的球面或环面。
* 圆锥侧面不是由一个底面圆 + 顶点、或两个完整圆围成（例如带修剪）。
* 无曲线记录的退化边（例如圆锥顶点奇点边）。
* 当整个 B-rep 无任何可离散面且失败原因均为“不支持”时，整体返回
  `Unsupported(kernel.unsupported_surface)`，而不是空网格。

## 6. 结果与稳定诊断码

保持“没有成功但空网格”的约束：

```rust
TessellationOutcome::Success   { geometry, diagnostics }
TessellationOutcome::Partial   { geometry, degradation, diagnostics }
TessellationOutcome::Unsupported { reason }
TessellationOutcome::Failed     { diagnostics }
```

`Partial` 的 `degradation` 显式记录 `missing_faces` / `open_edges` /
`dropped_shells`（壳内无任何面成功时上报）。

| 码 | 含义 |
|---|---|
| `kernel.no_acis_kernel` | 默认内核不解析 SAT/SAB 字节 |
| `kernel.unsupported_exchange` | 无法分类的交换载荷 |
| `kernel.unsupported_surface` | 中性 B-rep 含本构建无法求值的曲面/曲线 |
| `kernel.empty_geometry` | 载荷为空字节，或 B-rep 无可离散面 |
| `kernel.missing_handle` | 几何句柄无法解析 |
| `kernel.invalid_tolerance` | 伺服容差非有限/非正 |
| `kernel.invalid_budget` | 每实体预算非法（如为 0） |
| `kernel.budget_exceeded` | 产出的网格超出请求预算（不静默截断） |
| `kernel.missing_face` | 源实体某个面未能离散 |
| `kernel.open_edge` | 边界边未闭合/非流形 |
| `kernel.dropped_shell` | 因缺面而丢弃的壳/体 |
| `kernel.cancelled` | 请求在产出前被取消（`CadError::Cancelled`） |

## 7. importer 数据路径

`cad-import-acadrust`：

* `solid_exchange_from_entity(&EntityType) -> Option<SolidExchange>`：对
  `3DSOLID` / `REGION` / `BODY` / `SURFACE` 取 `AcisData`，解析成功则返回
  `Brep(SolidExchange)`；解析失败保留原始 `Sat`/`Sab` 字节。
* `acis_exchange(&AcisData) -> SolidExchange`、`sat_to_brep(&SatDocument)`、
  `sab_to_brep(&[u8])` 为可单测的底层入口。
* 导入时实体的 `SemanticGeometry::Opaque.payload` 现在保留原始 SAT/SAB 字节
  （version 1 = SAT，2 = SAB）作为溯源；显示支持状态仍为 `Unchecked`，
  除非显式走上表 §3 的 `BrepTessellator`。

acadrust API 使用：`entities::acis::{SatDocument, SabReader, SatFace, SatLoop,
SatCoedge, SatEdge, SatVertex, SatPoint, SatPlaneSurface, SatConeSurface,
SatSphereSurface, SatTorusSurface, SatEllipseCurve, Sense}`，以及
`AcisData::{parse, is_binary, sat_data, sab_data}` 与
`SatDocument::{placement, resolve, records}`。

## 8. 显示路径

`cad-representation::DisplayRepresentation::from_tessellation(result, source,
alpha)` 把 `TessellationResult` 转为显示图元：

* 有网格 → `DisplayPrimitive::Mesh`，`GeometrySource::KernelMesh`，
  `precision_for_kernel_mesh` 直接采用内核报告的上界；
* `Unsupported`/`Failed` → `Completeness::Missing` + `kernel.unsupported`
  诊断，绝不产生“空网格成功”。

`precision_for_source(KernelMesh)` 不再声称 `Analytic`（仅凭标签无法知道误差），
改为 `Approximate { error_bound: None }`。

## 9. 夹具与测试

夹具（`fixtures/acis/`，全部标注为 synthetic、本仓库自撰，见 `fixtures/manifest`）：

| 夹具 | 预期 |
|---|---|
| `cube.sat` | `Success`，12 三角形，面积 24，闭合 |
| `box-with-square-hole.sat` | `Success`，10 面（含内环洞），面积 392，闭合 |
| `cylinder.sat` | `Success`（两圆盖 + 圆柱侧面），带弦高上界 |
| `sphere.sat` | `Success`，单面无环 |
| `cone.sat` | `Success`（底面圆盖 + 圆锥侧面扇形到顶点），闭合 |
| `torus.sat` | `Success`（完整环形圆环面），闭合 |

截头圆锥（两个完整圆环的圆锥侧面）由单元测试覆盖，未单独提供夹具。

覆盖：立方体三角形数/面积/闭合性、球面容差单调性、空载荷、未解析句柄、
未知交换、取消、超预算、无法离散的面 → `Partial` + `kernel.missing_face`、
全不支持 → `Unsupported(kernel.unsupported_surface)`、importer 的 SAT/SAB
往返与中性提升、representation 的 kernel-mesh 转换。

## 10. 诚实边界

* 夹具由 acadrust 写出/手写，**不是**第三方图纸；不构成互操作证据。
* 默认内核仍为 `Unsupported`；本文件不把 `BrepTessellator` 的可达子集等同于
  F15 整体完成。
* 未列于 §4 的曲面/曲线/退化情形一律按 §5 缺面上报。
