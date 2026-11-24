# ADR 0055：块属性模型（ATTDEF / ATTRIB / 属性编辑与提取）

- 状态：**Accepted（2026-11-24）**。模型/schema 实现中；命令状态在对应实现与契约测试落地前仍是
  `部分实现`（具名拒绝/只读报告）。
- 关联：[ADR-0037](0037-ymodel-schema-and-persistence.md)（`.ymodel` 版本与表）、
  [ADR-0031](0031-acadrust-vendored-patch-policy.md)（acadrust 补丁台账）、
  [ADR-0043](0043-parametric-dependency.md)（表达式/参数，属性默认值可复用）、
  [ADR-0053](0053-application-domains.md)（cad2d 面板摆放）、
  [ADR-0054](0054-2d-constraint-solver.md)（同轮放开的二维约束；不共享模型）。
- 影响：`cad-domain`（属性类型/语义）、`cad-db`（块成员、INSERT 行、验证/变换/边界/快照）、
  `cad-store-zip`（minor 14）、`cad-connector-acadrust`（DXF/DWG 属性导入/导出）、
  `cad-representation`（属性值绘制、ATTDISP）、`cad-app`/`cad-ui-core`/`apps/desktop-ui`
  （命令与面板）。

## 背景

acadrust 0.6.8 已经建模 `AttributeDefinition`、`AttributeEntity` 和 `Insert.attributes`；当前
connector 把 ATTDEF/ATTRIB 拍平成 `Text`，`cad-db` 没有属性定义/值模型，`ATTDEF`/`ATTEDIT`/
`ATTEXT`/`BATTMAN`/`ATTSYNC`/`ATTREDEF`/`BURST` 等因此只能具名拒绝或只读报告。这一族是二维
绘图的高频能力，也是数据提取/表格块单元格的前置模型。

## 决策

### D1 — 定义是块成员实体，值是 INSERT 行内数据

- `ATTDEF` 继续作为 `BlockDefinition.entities` 里的成员实体，新增
  `SemanticGeometry::AttributeDefinition(Box<AttributeDefinition>)`；块表/`BlockRow`/`BlockPatch`
  不加字段，保持 `cad-db` 单一权威。
- `SemanticGeometry::Insert` 增加 `attributes: Vec<AttributeValue>`（缺省 `[]`），值按块局部坐标
  存储，随 INSERT 的变换/复制/阵列/嵌套自然传播。
- 不引入“ATTRIB 作为第一类实体 + owner_insert”的第二条写路径：那会让移动/复制/删除/撤销
  变成两行写入，任何漏写都会静默失去同步。

### D2 — 定义与值的字段

- `AttributeDefinition { tag, prompt, default_value, flags, order, text, source_handle }`；
  `flags` 覆盖 invisible/constant/verify/preset/locked_position/annotative。
- `AttributeValue { tag, value, flags, text, layer, appearance, source_handle }`；tag 匹配按
  ASCII 大小写不敏感，顺序由定义的 `order` 决定；未匹配的值保留并报告，不静默丢弃。
- `AttributeText` 镜像 `Text` 的排版字段并增加 `oblique`；`as_text()` 复用现有文本通道做边界/
  导出/BURST。
- 重复 tag 在导入时允许（保真）并由写命令拒绝；空 tag、非有限位置、非正字高/宽度因子、
  未知样式按稳定码拒绝。

### D3 — 变更范围与撤销

- 定义编辑脏 `ModelKey::Block(id)` 与 `TableKind::Blocks`；值编辑是普通 INSERT 行更新
  （`ChangeMask::GEOMETRY`）。两者都是一条事务/一次 revision/一步撤销。
- ATTSYNC/ATTREDEF 等“定义改动传播到引用”的路径必须**显式**写每个引用 INSERT 的值，
  不依赖显示缓存失效；不允许静默留下旧值。

### D4 — `.ymodel`：INSERT 可选 `attributes`，minor 14

- `cad:Insert` 增加可选 `attributes` 字段（无值省略；旧档缺失 = `[]`），新增
  `FieldKind::AttributeValues`；`cad:AttributeDefinition` 类/载荷承载定义。
- `FORMAT_MINOR` 升到 **14**；`document_minor()` 的 block-attribute 档必须检查在
  `application_domain`（13）之前；`DocumentSnapshot::has_block_attributes()` 决定该档。
- 不新增 archive 表；`tables/blocks.json` 逐字节兼容，定义在块元素分片、值在 INSERT 行。
- 旧读新按 `ymodel.unsupported_minor` 拒绝；新读旧 = 无属性。

### D5 — DXF/DWG：acadrust 原生属性，不拍平

- 导入：ATTDEF → `AttributeDefinition`；INSERT 的 `attributes` 按逆变换落到块局部坐标；
  空间 ATTRIB（无 owner）保持 Text 回退并记 `import.orphan_attrib`。不重复绘制定义的默认文字。
- 导出：INSERT 写 `attributes`（acadrust 已写 `66`/ATTRIB/SEQEND）；ATTDEF 走普通块成员写出；
  无法表达的引用（镜像/非平面爆炸等）按名报告，不静默丢。
- 需要 acadrust 补丁时只允许 acadrust（ADR-0031），固定版本/rev 并登记
  `docs/acadrust-patches.md`；本 ADR 不预设补丁。

### D6 — 显示与 ATTDISP

- 实例展开时按定义顺序绘制值文本；`invisible` 值随 ATTDISP 模式（Normal/On/Off）显隐，
  `constant` 定义即使 ATTDISP Off 也绘制；定义本身不作为一个可绘制实体重复画。
- ATTDISP 是显示设置（SceneVersion 参与重建），不写库、不升修订、不入历史；出图按 Normal。
- 属性值编辑走普通实体更新/ChangeSet；片段数变化时回退整批重建，不静默部分更新。

### D7 — 命令与 UI

- 本 ADR 覆盖：`ATTDEF`/`-ATTDEF`、`ATTEDIT`/`-ATTEDIT`、`EATTEDIT`、`ATTIPEDIT`、`GATTE`、
  `ATTSYNC`、`ATTREDEF`、`ATTDISP`、`BATTORDER`、`BATTMAN`、`ATTIN`、`ATTOUT`、`ATTEXT`/
  `-ATTEXT`、`EATTEXT`/`-EATTEXT`、`BURST`。
- 命令按“创建定义 / 编辑值 / 同步重定义 / 导入导出 / 报告面板 / 显示 / 分解”分组，共享一套
  属性值编辑核心；未支持选项按名拒绝，不空成功。
- UI：`ATTDEF`/`ATTSYNC`/`BATTMAN`/`BATTORDER` 进 Block Definition 面板；`ATTEXT`/`EATTEXT`/
  `ATTOUT`/`ATTIN` 进 Insert → Linking；`ATTDISP` 进 View → Display；属性值可在 Properties
  面板以只读行显示，编辑走命令/面板。未实现前面板保持 `pending`。

## 非目标

- 动态块参数/动作/查找表（另一模型）。
- 字段表达式求值（`FIELD`/`UPDATEFIELD` 按 ADR 之外的范围另行决策；属性默认值只保留原串）。
- 完整 AutoCAD 属性对话框/选项顺序/提示文案（全局范围）。
- 自动把属性值写进表格块单元格（`TINSERT` 后续）。

## 实施分期

1. **M1 模型/schema**：`cad-domain` 类型、`SemanticGeometry` 变体、`cad-db` 验证/变换/边界/
   默认值助手、ymodel minor 14、快照/往返测试。
2. **M2 connector + renderer**：DXF/DWG 导入/导出、实例展开绘制、ATTDISP、ChangeSet 失效。
3. **M3 命令**：ATTDEF/ATTEDIT/ATTEXT/BATTMAN/ATTSYNC/ATTREDEF/ATTDISP/BURST 等 typed 路径，
   导入/导出 codec，一次事务/一步撤销。
4. **M4 UI**：面板/属性行/i18n/fixture，`autocad-structure` 只移除真实完成的面板。

## 证据边界

以契约测试为主（domain/db/ymodel/DXF/命令/渲染设置）；真实 DWG 需 `YACR_TEST_DWG` 显式运行，
未运行写 **NOT RUN**。无 GUI/GPU/像素/真机结论时不得推广。
