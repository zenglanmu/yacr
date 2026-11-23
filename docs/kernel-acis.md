# ACIS 实体离散（F15：已实现一个可复现的受限子集）

本文件描述 `cad-kernel-adapter` 的对外契约，以及当前**真实**支持的 ACIS
（SAT/SAB）离散子集。默认直通实现仍保持诚实的 `Unsupported`；只有显式选用
`BrepTessellator` 且输入落在下述子集内时才会产出真实网格。

“已实现”只指：对**本仓库可复现的合成 SAT/SAB**，在文档化的曲面/曲线类型上
产出非空、闭合（或带有精确退化报告）的网格。它**不**声明对 AutoCAD /
Tianzheng / TSSD 等第三方实体的兼容性。

## 1. 边界

`cad-kernel-adapter` 是工作区中**唯一**允许持有内核/ACIS 交换数据的 crate。
中性 `BrepData`（纯数据、无 ACIS 句柄）自 ADR-0036 起定义在 `cad-domain`，由本
crate 重导出，因为 `cad-db` 只允许依赖 `cad-domain`，而实体几何必须能被数据库
拥有、校验、变换与包围盒计算。跨边界只传递纯数据：

* `TessellationRequest` / `TessellationResult`（原始契约）；
* `SolidExchange`（见 §2）与其中的中性 `BrepData`（`cad_domain::brep`）。

`cad-connector-acadrust` 是唯一依赖 acadrust 的 crate；它把 SAT/SAB 解析为
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
    pub tolerance: Option<f64>,           // 长度；None = 源未给出，不臆造
}

pub struct BrepFace {
    pub id: u32,
    pub surface: BrepSurface, // Plane | Cylinder | Cone | Sphere | Torus | Nurbs | Unsupported
    pub reversed: bool,
    pub loops: Vec<BrepLoop>,  // BrepCurve: Line | Circle | Arc | Unsupported
    pub tolerance: Option<f64>, // 长度；record 没有逐面容差槽位
    pub domain: Option<[[f64; 2]; 2]>, // 面自身参数里的 (u, v) 矩形；曲面面必须有
}
```

* `BrepSurface::Nurbs { u_knots, v_knots, points }`（P2.1b，2026-11-17）是**form 曲面**：齐次控制
  系数 `[x·w, y·w, z·w, w]`、**隐式次数**（`knots.len() - count - 1`）、knots 按生产者原样、
  **无周期性标志**（闭合是控制网的性质）。它是几何，不是被证明的解析类型；解析 label 仍优先于
  同名 form（`Cylinder`/`Cone`/`Sphere`/`Torus` 的 label 就是几何）。record 的结构限制
  （每方向 2…4096 控制点、次数 1…16、`knots.len() == count + degree + 1`、权重 `>= 1e-12`）
  不在类型上断言，只在回写中性体时校验（唯一能分辨“畸形网”和“不寻常网”的消费方）。

* `BrepCurve::Arc{center, normal, u_dir, radius, start, end}` 是**开**圆弧（绕 `normal`
  逆时针，扫掠 ∈ `(0,2π)`，两端是点），不是整圆；映射、方向推导与容差规则见 §4/§11。
* `BrepData.tolerance` / `BrepFace.tolerance` 是**模型单位长度**的 `Option<f64>`：`None` 表示源
  没给出（不是 0、也不是默认容差）。组合 `BrepPlacement` **不**改容差；`record::bake` 是**唯一**
  的缩放点——它按放置的比例**一次**缩放 body 与 face 容差（不再有第二处乘算）。

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
  洞边界可为直线、完整圆或**圆弧**（`BrepCurve::Arc`，见下）。
* **完整圆**（`ellipse-curve` 且 `ratio == 1` 且首尾顶点相同）。
* **圆弧**（`BrepCurve::Arc{center, normal, u_dir, radius, start, end}`）：读入时是
  `ellipse-curve` 且 `ratio == 1` 的**开边**——两个顶点都在同一圆上（半径**与**平面，
  容差 `1e-9·max(1, r)`）；绕 `normal` **逆时针**，`start`/`end` 是点。**跨度与方向都是推导的**：
  ACIS 的边参数**可以跨 seam 回绕**，所以跨度取 `(end_param − start_param).rem_euclid(2π)`
  （把回绕值读成 `|end − start|` 会把弧静默镜像到**补弧**上——已有回归测试钉住）。非半弧由跨度
  唯一决定方向；**半弧**两个候选跨度同量级，按 ACIS 的逆时针约定取 `about_normal`（按构造胜出），
  不是靠量级猜测。跨度 `<= 1e-9` 是独立的
  `BrepCurve::Unsupported{type_key: "arc-without-span"}`（**不是** `"partial-circle"`）：
  acadrust 的 **v400** 升版转换器会把边参数补 `0.0`，所以 **v400 SAT 文件里的 Arc 边不可导入**。
  圆弧边界的**平面**面可 lift 并按**弦近似**离散（报弦高上界，`Precision::Approximate`，
  绝不标 `Analytic`）；回写 record 时写成开圆（`Curve::Circle` 标签 + 精确有理二次 NURBS 形式 +
  角度 `interval` + 两个端点顶点，UV pcurve 是该 NURBS 在面 `(u, v)` 里的精确投影）；
  同一条圆弧的两个 use 折叠为**一条** record edge（按圆心/半径/中点匹配点集）。
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
* 精度：纯直线平面 → `Precision::Analytic`；含曲面或**圆弧边界** →
  `Approximate { error_bound }`（弦高上界由采样段数与曲率算出；圆弧是弦近似，
  **不得**标 `Analytic`）。

## 5. 明确 Unsupported（返回缺面，不伪造）

* `cone-surface` 且 `cos(half_angle) == 0`（零锥角退化为平面），或
  `sin(half_angle) == 0` 且无法按圆柱处理者。
* 椭圆（`ratio != 1`）；**圆弧边界的曲面侧面**（半圆柱壁、圆角槽侧壁等被圆弧裁剪的圆柱/
  圆锥面）——注意这与 **lift** 无关：自 **P2.1b（2026-11-17）** 起这类面**能写回 record 并参与
  内核计算**，但**本构建没有画出它的网格器**——侧面条带要求“两个整圆”，被剪裁的侧壁由离散器按名
  报缺面（`cylinder boundary is not a full circle`），整条 body 为 `Partial`。**平面面**里的圆弧
  （外环或洞）已支持（§4）。
* NURBS 面（`BrepSurface::Nurbs`）：**载体存在、lift 可用，但没有网格器**——按名拒绝并点名
  `solidcore-tess`（`nurbs-face: no NURBS mesher in this build (solidcore-tess, phase P2.1c)`），
  该面进 `missing_faces`，绝不伪造网格。
* 其它未建模曲面（spline 边界曲线、mesh 曲面等）。
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

`cad-connector-acadrust`：

* `solid_exchange_from_entity(&EntityType) -> Option<SolidExchange>`：对
  `3DSOLID` / `REGION` / `BODY` / `SURFACE` 取 `AcisData`，解析成功则返回
  `Brep(SolidExchange)`；解析失败保留原始 `Sat`/`Sab` 字节。
* `acis_exchange(&AcisData) -> SolidExchange`、`sat_to_brep(&SatDocument)`、
  `sab_to_brep(&[u8])` 为可单测的底层入口。
* 导入时**不保留**原始 SAT/SAB 字节，但保留 lift 后的中性 B-Rep：实体以
  `SemanticGeometry::Body3d` 发出（`exact` + `display`，ADR-0036）。`display` 是中性显示缓存
  （网格 + 边线 + 弦高）；**SAT/SAB 源字节仍不保留** ⇒ 不能按原字节往返；导出改由保留的精确
  B-Rep **重建** ACIS 载荷（P3a 的 DXF 内联 SAT 文本 / P3b 的 DWG SAB，子集外的体退回 POLYFACE
  并逐个记 `acis.*`，见 §11）。无法 lift 时 `exact = Unavailable`，`3DSOLID` 同时降为 `Unknown`，
  不伪造实体。完整性：平面精确体可 `Complete`，曲面体 `Partial`（解析曲面按参数采样；NURBS 面
  无网格器 ⇒ 具名缺面并点名 `solidcore-tess`）；见 `docs/compatibility.md`「ACIS 实体状态」。
* **容差**：SAT 的 `body`/`lump`/`shell`/`face`/`loop`/`coedge`/`edge`/`vertex`/`curve` 记录集
  经本读取器使用的类型化访问器**不带**逐体/逐面/逐边/逐顶点几何容差，所以 lift 报
  `BrepData.tolerance = None`、`BrepFace.tolerance = None` —— **“未指定”，绝不是一个编造的数**。
  SAT **头**里的三个数字是文档级模型器设置（acadrust 写空间分辨率 `10`，当成 10 mm 交容差会
  放松之后每一次内核调用），因此刻意不采用。

acadrust API 使用：`entities::acis::{SatDocument, SabReader, SatFace, SatLoop,
SatCoedge, SatEdge, SatVertex, SatPoint, SatPlaneSurface, SatConeSurface,
SatSphereSurface, SatTorusSurface, SatEllipseCurve, Sense}`，以及
`AcisData::{parse, is_binary, sat_data, sab_data}` 与
`SatDocument::{placement, resolve, records}`。

## 8. 显示路径

**已接线（2026-10-21；2026-11-16 改为一级 body）**：导入调用 `solid_exchange_from_entity` +
内核无 stamp 入口 `tessellate_exchange`，把 `TessellationMesh` 封装成
`SemanticGeometry::Body3d`（`exact` 来自 `SolidExchange::Brep`，`display` 带网格/边线/弦高）。
`DisplayRepresentation::from_tessellation` 仍只有测试调用者（导入不经该转换器）。
容差 = 包围盒对角线的 0.1 %；完整性按 `docs/compatibility.md`「ACIS 实体状态」的规则给出
（精确平面体可 `Complete`，曲面体 `Partial`）。

转换器本身的行为（供接线时使用）：`DisplayRepresentation::from_tessellation(result, source,
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

截头圆锥（两个完整圆环的圆锥侧面）由单元测试覆盖，未单独提供夹具。**圆弧**
（`BrepCurve::Arc`，P2.1a）也没有夹具，由单元/契约测试覆盖：圆弧边界的平面面采样环在接缝处
精确闭合、四分之一圆盘面积对 `π/4` 在 1e-5 弦高预算内 0.1 %、半弧的 bulge 有界、跨 seam
跨度的回绕不镜像到补弧、半弧的 CCW 约定、`arc-without-span` 的拒绝，以及**含圆弧的全平面闭合壳
被内核以 status `1001` 拒绝**（`solid.kernel_failed`，退化体而不是能力缺口）。

覆盖：立方体三角形数/面积/闭合性、球面容差单调性、空载荷、未解析句柄、
未知交换、取消、超预算、无法离散的面 → `Partial` + `kernel.missing_face`、
全不支持 → `Unsupported(kernel.unsupported_surface)`、importer 的 SAT/SAB
往返与中性提升、representation 的 kernel-mesh 转换。

## 10. 诚实边界

* 夹具由 acadrust 写出/手写，**不是**第三方图纸；不构成互操作证据。
* 默认内核仍为 `Unsupported`；本文件不把 `BrepTessellator` 的可达子集等同于
  F15 整体完成。
* 未列于 §4 的曲面/曲线/退化情形一律按 §5 缺面上报。

## 11. 写回（P3a/P3b 已实现子集）

本节描述**导出**方向：`cad-connector-acadrust` 的 `export::acis` 用中性 `BrepData`
构建 acadrust `SatDocument`，再由 `DxfWriter` 写成 DXF `3DSOLID` 的内联 ACIS SAT
文本（AC1024），或用 `SabWriter` 从**同一份** `SatDocument` 生成**二进制 SAB**、
经 `Solid3D::from_sab` 挂到 DWG `3DSOLID` 上（P3b，2026-11-17；由上游 `DwgWriter`
放进 AC1032 的 AcDs 段）。它是本仓库唯一的 ACIS 写出路径；`BODY`/`REGION`/`SURFACE`
仍不写回。

**可达子集（闭集，两种格式相同）**

* 恰好**一个** shell，且该 shell 有面（多壳 = 空腔/多 lump，拒绝）；
* 每个面都由 `BrepSurface::Plane` 支撑（曲面/未知面拒绝）；
* 环的每一段都是 `BrepCurve::Line`（圆/圆弧/椭圆/样条段拒绝 —— **圆弧 lift 是导入方向**
  的 P2.1a 能力，写出仍是直线子集）；
* 每条几何边恰好被**两个** coedge 使用（开放壳、非流形拒绝），且两个面沿该边
  走向相反、同一环内不重复使用同一条边；
* 每个面的**外环**（绝对面积最大者）绕向须与面朝向一致（`forward` 面绕平面法向
  CCW、`reversed` 面 CW），面的净带符号面积在朝向侧为正 —— 单个面被写反即拒绝；
  内环（洞）按构造反向绕；
* 放置只能是**真相似变换**（正交列 + 正行列式：允许旋转/均匀缩放/平移，拒绝
  剪切与镜像），并**烘焙进点与曲面框架**，不写 `transform` 记录；
* 所有几何有限；焊缝容差 = 体对角线 × 1e-9（用于把同一几何点/边只写一次）。

**曲面 lift（P2.1b part 2，2026-11-17）：曲面体重新进入内核**

> 这一节说的是**内核 record 方向**（`BrepData → record → Body::from_record`），不是 ACIS 导出：
> ACIS 写出仍是下面的平面子集，P2.1b 没有扩大它。

P2.1a 的边界（“曲面侧面按名拒绝”）已经不成立。写入方向现在不再只写平面，而写：

1. **解析 label**（`Cylinder`/`Cone`/`Sphere`/`Torus`）——label 就是几何，读回时优先于 form；
2. `solidcore::eval::form::surface` 给出的**精确有理 NURBS form**（knots/控制点来自内核的闭式解，
   不是拟合）；**knots 按每个角度方向 π/2 重标**（见下面的偏差 2）；
3. form 参数下的 **`(u, v)` box**；曲面面**必须**声明，未声明 ⇒ 具名 `solid.lift_unsupported`
   （写方**不发明**参数盒）；
4. 每个 use 的**精确 pcurve**（`solidcore_ops::pcurve::pcurves_within`）——只接受闭式解，
   **绝不拟合**；未命中是新的稳定码 **`solid.pcurve_not_exact`**。
   占位/拟合 pcurve 会让面积积分返回 0（ADR-0036 §7 已实测），所以宁可拒绝。

完整圆边界（`BrepCurve::Circle`）现在也写出：`Curve::Circle` label + 整圈 form + 一个 seam 顶点；
闭合边没有第二个顶点可读 `forward`，方向由共享该圆的**平面面**（其 loop 绕自身法向逆时针）解析，
两个非平面面共用同一整圆时按名拒绝。球/环面的 seam loop 由表面自己的 1 条经线 / 1 条管圆 + 1 条
外赤道重建（读方向一直丢弃它们，这是几何重建而不是发明）。被**剪裁**的侧面（弧 + 母线）按字面
映射，因此**能 lift**；**完整圆锥**（`radius_top = 0`）同样 lift——读回的退化 apex 环被写成内核
record 自己的 apex band（**一条母线用两次**、apex 作其对端顶点，不是新几何），于是 `MASSPROP`
报 `V = πr²h/3`、`A = πr² + πr√(h²+r²)`、形心在 `h/4`（`a_full_cone_measures_its_apex_band`）；
**不是只有截头圆锥能 lift**。

**精确性是怎么被证明的（不是“内核三个样本”）**：pcurve 的闭式解在内核里说 **label 参数**，而
record 存的是**面的 form 参数**，两者只在边界落在共享的**四分之一网格**上时才逐项一致。因此写方：

- 把圆柱/圆锥的**纬圆**（边界圆）先**重标到面自己的 frame 上**（`curved::align_curved_frames`，
  只移动参数零点、圆本身不动）；直接给角度区间做 `eval::form::on_frame` 位移会破坏“pcurve 区间
  必须等于边的 `interval`”，整圈 pcurve 不再覆盖 form 的 knot span，被 record 以
  “pcurve interval differs from its edge's” 拒绝（已实测）；
- 用 `verify_pcurves` 逐个 use 在 **6 个样本（含 4 个内点）** 上比对 `pcurve(t)` 的面点与
  `edge(t)` 的点，按 `solidcore` 的 **1e-9 相对**（`1e-9·(1+max|坐标|)`）判“精确”；
  未命中 ⇒ `solid.pcurve_not_exact`，诊断点名 **face / loop / use**、实测 gap 与原因；
- **偏离网格的裁剪**（例如用旋转过的盒切圆柱）没有精确闭式 pcurve，直接拒绝
  `solid.pcurve_not_exact` 并写明“an exact closed form exists only where the boundary shares the
  face's quarter grid”，而不是把 record 的结构性报错（“geometry disagrees with its topology”）
  漏给用户。**平面**面不受影响：它的圆弧退回弧自己的 knots，仍能 lift
  （`an_arc_off_the_quarter_grid_still_lifts_a_plane_body`）。

**能力位**：`ModelerCapabilities::nurbs = true`（登记字符串 `nurbs.faces`），并且**承载**——
布尔 / 质量属性 / 倒角任一操作数含非平面面时该位必需；声明 `false` 的模型器直接答
`solid.nurbs_not_supported`（不是静默尝试，也不是泛化错误）。它是 family 级声明：声明 `true`
仍可能对单个操作数返回 `Unsupported`。**优先级是族在前**：族本身未声明的请求答
`solid.unsupported`，即使操作数带曲面也**不**报 `solid.nurbs_not_supported`——否则会指向一个
改能力位也修不好的原因。

**两处偏差（实测逼出来的，不是实现偏好）**：

1. **写入的 box 由面自身边界推出**，不复制生产者的 `BrepFace.domain`。生产者 box 在**生产者自己的
   form 参数**里（球是 `[0,1]²`、圆柱是弧度），交给读 label 参数的 `eval::form` 会得到一个小片
   并拒绝每个球。中性 box 的**存在**仍是生产者“这是被剪裁片”的声明。
   `label_box` 是**包围盒**（by construction 比裁剪片宽：u 取整圈、v 取边界范围），**绝不可收紧**：
   球的 seam 是**零面积 bigon**，全靠 u 盒是整圈、两极跳变把第二个 use 放在一个**周期**之外才成立；
   收紧盒子会把两个 use 叠在一处，内核直接拒绝该面。
2. **`eval::form` 的 knots 按 π/2 重标——这只是换算单位，不是把 form 参数变成角度**。
   `eval::form` 把弧铺在四分之一网格上，它的参数 `q + quarter_u(ψ)` 在四分之一圈内**是非线性的**
   （缩放后参数 `0.25` 对应 `0.357 rad`，不是 `0.393 rad`）；重标后**一圈 = `2π`、四分之一 =
   π/2**，于是 box 端点、解析曲面的 `Domain::period`（`2π`）与 seam 跳变同单位，落在四分之一线上的
   边界正好压在 knot 上。不重标时单位对不上（圆柱 seam 落在 `u = 2π` 而端圆在 `u = 4`，球面经线
   起点偏离边 0.65 mm）。**任何代码都不得把 form 参数当 label 的角度使用**；pcurve 的精确性靠的是
   上一条的 `align_curved_frames` + `verify_pcurves`，不是这次重标。

**仍未做（不要读成“曲面已完全可用”）**：NURBS **面显示**没有网格器——中性离散器按名拒绝并点名
`solidcore-tess`（P2.1c），该面进 `missing_faces`，body 是 `Partial`；**被剪裁的侧面**能 lift、
能测量，但条带要求两个整圆，所以画不出那一面（`cylinder boundary is not a full circle`）。
**lift 成功 ≠ 显示完整**。几何之外的曲面**编辑**（圆角、曲面边倒角、抽壳）仍是 P2.1c/P2.2。

**圆弧退化体的旧事实（仍成立，但不再是“转换缺口”）**：圆弧只落在**一个**平面里，所以“含圆弧的
全平面闭合壳”只能是**零厚度圆盘**，内核对它给出 status **`1001`** → `solid.kernel_failed`；
真实含圆弧的实体总有曲面邻居，自 P2.1b 起那个邻居能 lift 了。这类平面的显示仍是 `Partial`
（弦近似）：默认形状相对弦高（对角线 1e-3）下四分之一圆盘的**面积**与闭式值相差约 2 %，
网格只报弦高上界与 `Precision::Approximate`，**绝不**标 `Analytic`。

**容差（模型单位长度 + record 侧）**：容差是**模型单位长度**，不是角度、也不是无单位 epsilon。
组合 `BrepPlacement`（`composed_with`）**不改**它——`cad-db` 与 `cad-geometry` 的变换测试钉住
“相似变换后仍是同一模型长度”；`record::bake` 是**唯一**的缩放点，按 placement 的比例把 body 与
face 容差**各乘一次**（曾经“变换时缩放 + bake 时再缩放”的双重缩放已修）。非均匀变换仍把 body
降级为 `BodyExact::Unavailable`。

写 record 时 `Brep.tolerance` 取：body 有声明值就用它；body 未声明（`None`）则取**已声明面容差的
最大值**；两者都没有才用调用方的默认 `1e-7`——那是**我们**的默认值，**不是**生产者的值。若 body
声明了而任一已声明面容差与它相对偏差超过 `1e-6`，按名拒绝 `solid.lift_unsupported`（诊断点名
face id 与两个数）；非有限或非正的已声明面容差同样拒绝。该 body 容差值复制到每条
`Edge.tolerance` / `Vertex.tolerance`。`from_record` 把正的有限 `Brep.tolerance` 读回
`BrepData.tolerance`，面级恒 `None`（pinned record **没有**逐面容差槽位）。

**稳定拒绝码**（导出报告逐条记录，同时退回显示缓存）

> 注意区分方向：`solid.pcurve_not_exact` 与 `solid.nurbs_not_supported` 是**写回内核 / 求值**
> 方向的码，不属于导出表；导出始终只写平面子集（可达子集见上）。这条 record 方向的具名拒绝清单
> （全部 `Unsupported`，诊断带 face/loop/use 或具体原因）：
> `solid.pcurve_not_exact`（某个 use 没有精确 pcurve——要么闭式解未命中，要么**裁剪落在
> 四分之一网格之外**）；`solid.lift_unsupported` **也**用于：label 不在 `parameter_scale`
> 覆盖的四种解析族内、曲面 form 无法重标（不是 NURBS）、**圆弧边没有有理 NURBS form 可投影进面**
> （`an arc edge has no rational NURBS form to project into the face`）、同一整圆与一个**不包含它**的
> 平面面共用（法向不平行——含垂直；垂直曾被误读成“反向遍历”）、调用方给的 seam 点不在该圆的
> 参考半径上；`solid.nurbs_not_supported` 只用于“族已声明、但模型器没声明 `nurbs` 却收到曲面
> 操作数”。**族优先**：族本身未声明的请求答 `solid.unsupported`，即使带曲面也不报
> `solid.nurbs_not_supported`。

| 码 | 含义 |
|---|---|
| `acis.curved_face_not_written` | 非平面面（圆柱/圆锥/球/环面/NURBS/未知）；**P2.1b 的曲面 lift 不扩大导出子集**，能 lift 的曲面体照样走此码 |
| `acis.non_line_curve_not_written` | 非直线边界段 |
| `acis.no_shell_not_written` | 无壳 / 空壳 |
| `acis.multi_shell_not_written` | 多壳（空腔、多 lump） |
| `acis.placement_not_written` | 剪切/非均匀缩放/镜像/非有限放置 |
| `acis.invalid_topology_not_written` | 开放/非流形边、面朝向反转、退化环、面框架非正交 |

表格中的码在 **DXF 与 DWG 两种格式下相同**：拒绝是**体**的属性，不是容器的属性
（退回显示缓存：POLYFACE / LINE / POLYLINE3D）。**没有** SAB 专用拒绝码 ——
`SabWriter::write` 返回 `Vec<u8>`（不失败），其输入是本仓库已 `validate()` 的同一份
`SatDocument`；一个不可达的码不写进文档。

**DXF 版本（文档级选择，仅 DXF）**：`3DSOLID` 的 ACIS 载荷只在 AC1027 之前内联为 SAT
文本；AC1027+ 由上游 writer 改写成 `ACDSDATA` 里的 SAB。因此**只要文件含任一
`3DSOLID`**（模型空间或已写出的块定义内），整个 DXF 按 **AC1024 (R2010)** 写出，
并记 note `export.note.acis_sat_text_version`（SAVEAS 状态行同义）。这条约束是
文档级的，未来任何需要 AC1027+ 的实体特性都必须重新审视 SAT 文本路径。**DWG 不受此
约束**：DWG 保持 **AC1032**，`3DSOLID` 的载荷由上游 writer 放进 AcDs（`AcDsPrototype_1b`）
段，记 note `export.note.acis_sab_dwg`（SAVEAS 状态行 `save.exported_acis_sab[_partial]`）。
**干净的变体目前不可达**：写出 ACIS 总会丢弃显示缓存，报告恒为 `Partial`，只有
`save.exported_acis_sab_partial` / `save.exported_acis_sat_partial` 会真正出现。

**证据边界**：DXF 往返只经「本仓库构建 → acadrust `SatWriter` → acadrust
`SatDocument::parse` → 本仓库 `sat_to_brep`」，DWG 往返是「本仓库构建 → `SabWriter` →
acadrust `DwgWriter`（AcDs）→ acadrust `DwgReader` → `SabReader` → 本仓库 `sat_to_brep`」；
两者都覆盖盒/楔/棱锥/带方孔面的拓扑计数、曲面参数精确相等、几何 ≤ 1e-9·对角线、
`BrepTessellator` 体积 ≤ 0.1%；真实 solidcore BOX 经 DXF 存盘→再导入得到精确体、
体积 1±1e-3。**没有第三方 CAD 读取器验证过这些字节**（AutoCAD/ODA/BricsCAD 均未运行），
也没有 vendor/像素/非 Linux 证据；AcDs 的字节布局来自上游实现与其自读器，不是 Autodesk
规范验证。SAB 的载荷头是 **ACIS 7.0（700，非 ASM）**（`SatDocument::new_body()` 是本仓库
自己的构建器），ACAD 2013+ 原生体通常带 21800 ASM 头，**vendor 接受性未验证、不声明**。
**第二个未验证的 vendor 风险**：每个 AC1027+ `3DSOLID` 还带一个 `acis_revision` 块，而我们把它
**置零**写出（`has_guid = false`、全零 GUID）——`Solid3D::from_sab` 设 `AcisRevision::default()`，
`write_solid3d` 又无条件写该块。acadrust 自己的文档说该块在写出时必须保留、省略会破坏实体流，
文件会在 ACAD/TrueView/BricsCAD 里打不开；我们**没有**可保留的源 revision，而且**没有测试能
抓到**（往返只走 acadrust 的宽松读取器）。这也是**未验证的 vendor 接受性风险**，证据边界不变：
只有 acadrust 自读往返。
