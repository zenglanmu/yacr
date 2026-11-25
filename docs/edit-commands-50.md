# 50 个已接线的编辑命令/功能（2026-11-25）

> 口径：**已接线** = 命令可被词汇表接纳、走 `cad-db` 事务写入（或是有界的只读/选择操作）、
> 拒绝是具名的、并有契约测试或真实内核测试。`Partial` 表示只支持有界子集，边界写在
> `docs/compatibility.md` 的同一行。**未列出的命令不算支持**（见文末清单）。
>
> 计数只含**编辑/建模**操作：绘制（LINE/RECTANG/…）与 3D 图元创建（BOX/CYLINDER/…）不计入
> 这 50；它们本身已实现，但不属于“编辑命令”。

| # | 命令/功能 | 状态 | 边界摘要 |
|---|---|---|---|
| 1 | `MOVE` | 实现 | 平移选择集；typed `moveDistance` |
| 2 | `COPY` | 实现 | 复制选择集；多副本 |
| 3 | `ROTATE` | 实现 | 基点 + 角度；typed `rotateAngle` |
| 4 | `SCALE` | 实现 | 基点 + 参考/目标；typed `scaleFactor` |
| 5 | `MIRROR` | 实现 | 轴 + `MIRRTEXT`/擦除选项 |
| 6 | `ALIGN` | 实现 | 1–3 对点 + 可选缩放 |
| 7 | `STRETCH` | 实现 | 矩形/多边形/套索 crossing window |
| 8 | `REVERSE` | Partial | 曲线/折线方向反转；边界见兼容表 |
| 9 | `OFFSET` | 实现 | 距离/Through、Multiple、图层、`OFFSETGAPTYPE` |
| 10 | `TRIM` | 实现 | 边界裁剪；曲线/填充边界子集 |
| 11 | `EXTEND` | Partial | 延伸到最近边界；边界子集 |
| 12 | `JOIN` | 实现 | 共端曲线合并；闭合环 |
| 13 | `BREAK` | 实现 | 两点断开 |
| 14 | `BREAKATPOINT` | 实现 | 单点断开 |
| 15 | `LENGTHEN` | 实现 | `DE`/`DY`/`P`/`T` |
| 16 | `FILLET` | 实现 | 两线/多段线内顶点圆角 |
| 17 | `CHAMFER` | 实现 | 两线/多段线内顶点倒角 |
| 18 | `ARRAYRECT` | 实现 | 行列/间距/填充角/旋转项 |
| 19 | `ARRAYPOLAR` | 实现 | 项数/填充角/旋转项 |
| 20 | `ARRAYPATH` | Partial | 等弧长沿路径；路径形状有界 |
| 21 | `NCOPY` | Partial | 块成员按世界变换复制为顶层实体 |
| 22 | `COPYTOLAYER` | Partial | 复制到目标层，原实体保留 |
| 23 | `ERASE` | 实现 | 删除选择集；`OOPS` 可恢复 |
| 24 | `OOPS` | 实现 | 恢复最近一次 `ERASE` |
| 25 | `OVERKILL` | 实现 | 重复/共线线段清理 |
| 26 | `EXPLODE` | Partial | 块/多段线一级分解 |
| 27 | `XPLODE` | 实现 | 块分解 + 属性/图层/线型继承 |
| 28 | `FLATTEN` | Partial | 支持实体的 z 归零；新增子集 |
| 29 | `CHPROP` | 实现 | layer/color/linetype/lineweight/transparency |
| 30 | `MATCHPROP` | 实现 | 源实体属性复制到选择集 |
| 31 | `SETBYLAYER` | 实现 | ByLayer 属性归位 |
| 32 | `DRAWORDER` | 实现 | 前/后/上/下移动 |
| 33 | `CHSPACE` | 实现 | 模型/图纸空间移动 |
| 34 | `GROUP` / `-GROUP` | 实现 | 持久编组表（minor 17） |
| 35 | `UNGROUP` | 实现 | 移除组名，保留实体 |
| 36 | `BLOCK` | 实现 | 从选择集定义块 |
| 37 | `INSERT` | 实现 | 插入块参照（含属性值通道） |
| 38 | `TEXTEDIT` | 实现 | 改写选中文本 |
| 39 | `SCALETEXT` | 实现 | 文本高度缩放 |
| 40 | `TXT2MTXT` | 实现 | 文本合并为多行文本 |
| 41 | `PEDIT` | Partial | 全选项行 + 顶点编辑；无逐段锥形宽度 |
| 42 | `HATCHEDIT` | Partial | 图案/角度/间距/原点/边界子集 |
| 43 | `QSELECT` | Partial | 属性/运算符/Include/Exclude/Append |
| 44 | `FILTER` | Partial | 逻辑树 + 相对运算符；无命名过滤器 |
| 45 | `EXTRUDE` | 实现（2026-11-25） | 闭合平面轮廓（LINE 闭环/闭合直线 LWPOLYLINE/单个 CIRCLE）→ 新精确实体；±Z；轮廓保留 |
| 46 | `PRESSPULL` | 实现（2026-11-25） | 一个精确实体的一个面，带符号 `pressPullDistance`，原地改写 |
| 47 | `UNION` | 实现 | ≥2 精确体后台布尔并 |
| 48 | `SUBTRACT` | 实现 | 首个减其余 |
| 49 | `INTERSECT` | 实现 | ≥2 精确体交集 |
| 50 | `CHAMFEREDGE` | 实现 | 单边直线倒角，`chamferDistanceA/B` |

## 这 50 之外的“已实现”（不计入，因为属于绘制/创建/只读）

- 绘制：`LINE`、`RECTANG`、`CIRCLE`（含 2P/3P/D）、`ARC`、`ELLIPSE`、`POLYGON`（内接/外切）、
  `PLINE`、`SPLINE`、`POINT`、`XLINE`、`RAY`、`TEXT`、`MTEXT`、`WIPEOUT`、`BOUNDARY`。
- 3D 创建：`BOX`、`WEDGE`、`PYRAMID`（含 `PYRAMIDNGON` 形式）、`CYLINDER`、`SPHERE`、`CONE`、
  `TORUS`。
- 只读/查询：`MASSPROP`、`LIST`、`DIST`、`AREA`、`SELECTCOUNT`、`STATUS`。

## 明确**未**支持（不得计入 50）

`REVOLVE`、`SWEEP`、`LOFT`、`SLICE`、`THICKEN`、`POLYSOLID`、`IMPRINT`、`OFFSETEDGE`、
`FILLETEDGE`、`EDGE`、`MIRROR3D`、`ROTATE3D`、`SURF*`、`MESH*`、`FOLLOWME`、`SECTIONPLANE`、
`CAMERA`、`MATERIALS`、`SCENES`、`SHADOWS`、`FOG` 仍以稳定具名原因拒绝或不在词汇表内。
`EXTRUDE` 的挖孔/多环、拔模、部分圆弧壁，`PRESSPULL` 的多面/拖拽幅度，以及 `FOLLOWME`
的 3D 路径模型都**未实现**，边界见 `docs/compatibility.md` 与
`docs/plan-atomic-api-and-measurements.md`。

## 证据口径

- 每行的完整行为与拒绝在 `docs/compatibility.md` 的同一命令行；
- 契约测试分布在 `crates/cad-app/src/tests/drawing/` 与
  `crates/cad-ui-core/src/session/tests/`；
- `EXTRUDE` 的真实内核证据是 `crates/cad-kernel-solidcore/tests/extrude_profile.rs`；
  `PRESSPULL` 的是同目录的 `offset_face.rs`；
- 本表是**命令接线**清单，不是 GPU/像素/真机/性能验收：这些仍 **NOT VERIFIED**。
