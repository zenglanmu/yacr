# ADR-0081：逐面材质（Per-face materials）

- 状态：**已接受（决策，2026-11-27）**；**已实现（2026-11-27，命令 + 材质表 `faces` + 渲染拆分；无真实 GUI/GPU 证据）**
- 关联：ADR-0062（文档材质）、pick 的 `SelectionRef.sub_element`、`Mesh.face_sources`、ADR-0079（Materials 托盘）

## 背景

材质表此前只有**整实体**赋值：`PAINT <name>` 拒绝任何 `sub_element`，渲染时一个实体只有一种
材质色。SketchUp 的核心用法是给网格的单个面涂不同材质。本构建的 pick 已经能返回网格面的
`SubElementId`（`Mesh.face_sources` 的 source key），点击选择会保留它，因此命令层与渲染层缺的
只是“按面存”和“按面画”。

## 决策

### D1 表格式（minor 25）

- `MaterialDocument` 增加 `face_assignments: BTreeMap<(EntityId, String), MaterialSeq>`；
  每个材质的行可以带 `"faces": [{"entity": <id>, "face": "<source key>"}]`。
- 没有 `faces` 的行保持 minor 19；非空 `faces` 把归档升到 **minor 25**
  （`FORMAT_MINOR = 25`）。
- 校验：face key trim 非空、≤256 字符、无控制字符；最多 65 536 个面赋值；材质 seq 必须存在；
  同一 `(entity, face)` 不能出现两次；载入时实体必须存在（与整实体赋值同一条规则）。
  face key 是否仍在网格里**不**在载入时校验——网格编辑可以删面。

### D2 PAINT

- 选中 `sub_element` 时：实例路径必须为空；实体必须是 `SemanticGeometry::Mesh` 或带显示网格的
  `Body3d`；key 必须出现在 `mesh.face_sources` 中（否则 `paint.face_unknown`），然后只写该面。
- 同一实体同时以整实体和面被选中是 `paint.mixed_targets`（不猜测优先级）；非网格子元素是
  `paint.face_not_paintable`。
- 一次涂刷仍是**一次** `tables/materials.json` 写入、一次 revision、一步撤销；无变化仍是
  `paint.no_change`。`MATERIALS` 报告分别统计整实体与面涂刷。

### D3 渲染

- 渲染批次没有逐三角形颜色通道，因此 `cad-representation` 在展开时按面材质把网格**拆成多个
  fragment**（每个 fragment 本来就带一个具体颜色/alpha）：未涂面保留实体解析色，涂过的面用其
  材质色，透明材质乘实体 alpha。
- 拆分保留顶点属性（法线/顶点色/UV/纹理/PBR）；`MeshWithStoredEdges` 的存储边只挂在第一个
  fragment，其余组不重复推导边界线。
- **陈旧 face key**（网格编辑删了面）不重映射到别的三角形；展开上报
  `representation.material_face_inert` 并保持该面按实体颜色绘制。

## 不在本 ADR 范围

- 材质编辑器（改名/改色/删除）、贴图/纹理材质、材质库/拖放；
- 逐面选择高亮/框选面、面级 UV/纹理、按材质选择；
- 真实 GUI/GPU 像素证据与其它载体运行时。

## 后果

- 材质表新增面赋值与档位 25；旧表仍按 19 读写。
- `PAINT` 与 `MATERIALS` 报告覆盖面；渲染器只在存在面涂刷时多拆 fragment，普通实体路径不变。
- Materials 仍为 `partial`：编辑器/贴图/材质库未建模，但“逐面材质”不再是缺口。
