# ADR-0062：文档材质（SketchUp Paint Bucket）

- 状态：**已接受（决策，2026-11-27）**；**已实现（2026-11-27，命令层 + 文档表 + 渲染解析 +
  `.ymodel` minor 19；无真实 GUI/GPU 证据）**
- 关联：ADR-0037（`.ymodel` 格式）、ADR-0041（保存/导出）、ADR-0053（应用域；SketchUp 对应
  `concept`）、ADR-0055（块属性，同属“文档级表 + 元素引用”的存储形状）、ADR-0040（资源，
  纹理不在本 ADR 范围）

## 背景

`MATERIALS` 此前是具名拒绝（`materials.unsupported`），`concept` 域的 Paint Bucket 工具与
Materials 托盘面板都是 pending，渲染只认实体自身的 `ByLayer`/显式颜色/图层透明度。SketchUp 的
Paint Bucket 是这个产品最基础的可视化操作之一：一个命名材质（颜色 + 透明度）被涂到对象上，
颜色立即出现在画面里，并随文档保存。

本轮之前的约束：

- `DbEntity` 没有材质字段；全仓有 662 处 `DbEntity { … }` 字面量，加字段的改动面过大；
- 网格已有 `texture`/`pbr` 通道（导入器填），但它们不是用户可涂的命名材质；
- 需求唯一权威要求未支持能力显式建模，不允许“涂了但没画出来”或“保存后丢失”。

## 决策

### D1 文档材质表 + 每实体一个赋值

`cad-domain::material::MaterialDocument` 持有：

- `Material { seq, name, color: [u8;3] (sRGB), transparency: 0..=1 }`，表内 `seq` 标识，
  名称大小写不敏感唯一（trimmed、非空、≤64 字符、无控制字符），上限 **256** 行；
- `assignments: BTreeMap<EntityId, MaterialSeq>`：每个实体最多一个材质。

赋值放在**表里**而不是 `DbEntity` 字段：加字段会迫使 662 处字面量与所有导入器改写，而材质是
一个稀疏的文档级外观覆盖，正好是表的形状。实体删除时在**唯一删除点** `remove_entity` 清掉赋值，
序列化/加载再校验一次（未知实体/未知材质 → 加载拒绝）。

### D2 内置调色板 + 首次涂刷落文档

`BUILTIN_MATERIALS` 是 12 个扁平内置色（Red…Glass，Glass 透明度 0.6），是**我们自己的**调色板，
不复制任何厂商材质库、图标或纹理。`PAINT <name>` 先在文档表中解析，未命中再查内置表；命中内置
时把它**复制成文档行**再赋值（`MaterialDocument::ensure`），因此归档只存真正用到的材质。
同名的文档行优先于内置行，且重复涂同名材质**不**静默改写已有颜色。

### D3 命令面

- `PAINT <name>`：typed-argument（`PAINT_SELECTOR` + `paint_grammar`，与 `VIEWGO` 同一机制），
  目标是**当前选择**（整实体、顶层；块成员/子元素按 `paint.not_addressable` 拒绝，INSERT 按
  `paint.insert_not_paintable` 拒绝而不是假装涂上）。一次表写入 = 一次 revision = 一步撤销；
  无变化按 `paint.no_change` 拒绝。
- `MATERIALS`：只读报告（文档材质 + 涂刷数量 + 内置调色板名称），即命令级 Materials Browser；
  从 `RefusalId` 毕业（311 → 310 个具名拒绝），Viewer 合法。
- 概念域托盘 Materials 面板是真面板：内置色块点击 dispatch `PAINT <name>`，另有“材质报告”按钮。

### D4 渲染

`cad-representation` 的展开在解析 `ByLayer`/显式颜色后查 `database.materials().assignment(entity.id)`：
命中则用材质的 sRGB 颜色替换颜色、把 alpha 乘以 `1 - transparency`，并清掉 `color_unresolved`。
赋值是文档数据，所以下次展开自动生效，无需写实体、无需重建底图。

### D5 持久化与撤销

- `tables/materials.json`（一行一个材质，`entities` 内联该材质涂刷的实体 id；`transparency`
  为 0 时省略；严格解码：未知键、重复名称/序列/实体、越界颜色/透明度、赋值到不存在的材质都拒绝）。
- `FORMAT_MINOR` 18 → **19**；无材质行的文档保持原 minor；`cad-store-zip` 的 `document_minor`
  把 materials 放在最高档，读侧 18 及以下照常打开（只是没有涂刷）。
- 撤销/重做：`cad-history` 新增 `material_patches` 车道，携带整份 `MaterialDocument` 的
  before/after（小表，无需逐行 patch）；`cad-app` 的 replay 走同一 `apply_material_document`。

## 不在本 ADR 范围

- **面级赋值**（SketchUp 涂某个面）：网格仍是一个实体，`face_sources` 键上的材质未建模；
- **纹理/图片材质**：`Mesh.texture`/`pbr` 保持导入器语义，文档材质只有扁平色；
- **INSERT 级联**：涂块引用不会级联到成员（按名拒绝）；
- **材质创建/编辑命令**（改名、改色、删除、透明度 UI）：当前只有内置调色板 + 首次涂刷落行；
- 真实 GUI/GPU/像素证据与其它载体运行证据。

## 后果

- 文档多一张小表与一个格式档位；不涂刷的文档零成本。
- 撤销/重做车道增加一个字段，所有 `DrawingHistoryStep` 字面量显式列出（机械改动）。
- 删除实体时清理赋值；加载时拒绝悬空赋值，因此不会出现“涂了但实体已删”的隐藏行。
- `MATERIALS` 不再是拒绝，AutoCAD 覆盖文档对应行要同步改；拒绝总数 311 → 310。
