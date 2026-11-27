# ADR-0064：柔化边线（Soften Edges）

- 状态：**已接受（决策，2026-11-27）**；**已实现（2026-11-27，per-entity 显示属性 +
  `SOFTENEDGES` + 渲染过滤 + `.ymodel` minor 21 + 托盘面板；无真实 GUI/GPU 证据）**
- 关联：ADR-0037（`.ymodel`）、ADR-0045（网格线框是表面边界）、ADR-0049（导入边作为实体显示边）、
  ADR-0062/0063（同属“文档显示属性 + 命令 + 面板”）

## 背景

SketchUp 的 Soften Edges 把相邻面夹角很小的边从“硬边”变成柔化边：边不再画出来（可再选
Smooth normals 融合明暗）。本仓库的 3D 体显示有两套线框来源：

- `BodyDisplay::edges`（存储的边界/特征边折线，SKP 导入与精确体都有）：渲染时作为
  `MeshWithStoredEdges` 的 `edges`，是实体的线框权威；
- 没有存储边的三角网格：显示按“表面边界”（奇数使用边）推导，内部三角对角线不画。

没有持久化的“哪些边是柔化的”模型，也没有命令能隐藏边。

## 决策

### D1 每实体一个柔化角度（不是逐边集合）

新增 `EntityRenderAttributes::soften: Option<f64>`（度，`0..=180`）：

- `None` = 未柔化（全部硬边）；
- `Some(angle)` = 隐藏“相邻两个显示三角形法向夹角 ≤ angle”的显示边。

角度而非边集合，是因为边身份由几何派生（`mesh:edge:<bits>`）且随编辑失效；角度对几何修改保持
意义，也和 SketchUp 的 “Soften coplanar” 语义一致。显示属性随 `update_entity_display` 写入，
与颜色/透明度/线宽同一条显示状态路径。

### D2 `SOFTENEDGES [<angle>]`

- 只作用于**当前选择**的三维体（`SemanticGeometry::Body3d`）；整实体、顶层，块成员/子元素按
  `soften.not_addressable` 拒绝，非三维体按 `soften.not_a_body` 拒绝；
- 裸命令用默认 **20°**；`SOFTENEDGES 0` 只隐藏完全共面的边（SketchUp 的 0° 语义）；
- 一次写入 = 一次 revision = 一步撤销；历史 patch 携带 before/after 显示状态，撤销精确还原；
  已持有同一角度的选择按 `soften.unchanged` 不写；
- 角度非有限或超出 `0..=180` 是 `soften.angle_invalid`；Viewer 模式按 `PermissionDenied` 拒绝。

### D3 渲染过滤

`cad-representation` 的体展开在 `soften` 存在时：

- 把存储边按端点与显示三角形的无向边精确匹配；只在**恰好两个相邻三角形**且法向夹角
  ≤ 阈值时隐藏；边界边（一个三角形）、非流形边（≥3）和无法匹配的段保持可见——柔化绝不隐藏
  无法解释的轮廓；
- 输出仍是 `MeshWithStoredEdges`，新增 `derive_when_empty: bool`：柔化体为 `false`，于是“所有边
  都被柔化”时只画填充、不回退到三角网线框（旧的空边回退只用于 legacy/手写片段）；
- `VSCURRENT WIREFRAME` 走同一过滤：全部柔化时线框为空（不报缺失几何）；
- 没有存储边的三角网格仍走旧的表面边界显示（本来就没有内部边可柔化）；软法线/明暗融合
  **不在本 ADR 范围**。

### D4 持久化

`render_attributes.soften` 只在设置时写入元素行；`FORMAT_MINOR` 20 → **21**，无柔化文档保持
原档位。解码拒绝非有限/越界角度，`render_attributes` 的未知键仍按严格规则拒绝。

### D5 托盘面板

概念域托盘的 Soften Edges 面板从 pending 变成真实面板：角度输入（0..=180，本地校验）+
“应用”按钮 dispatch `SOFTENEDGES <angle>`；所有语义仍在命令里。

## 不在本 ADR 范围

- Smooth normals / 明暗融合（SketchUp 的另一个开关）；
- 逐边手工柔化/取消柔化、边选择器、Soft Edge 显示模式；
- 场景保存柔化设置（场景当前不含显示属性快照）；
- 真实 GUI/GPU/像素证据与其它载体运行时。

## 后果

- 元素行多一个可选键、格式多一个档位；普通文档零成本。
- `DisplayPrimitive::MeshWithStoredEdges` 多一个字段，所有构造点显式给出（历史行为 = `true`），
  场景缓存按它决定是否回退。
- 柔化只在“边与显示三角形端点精确一致”时生效；导入体通常如此，程序生成的细分网格可能不匹配，
  此时边保持可见（保守、不猜）。
