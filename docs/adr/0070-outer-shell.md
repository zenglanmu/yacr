# ADR-0070：外壳（Outer Shell）

- 状态：**已接受（决策，2026-11-27）**；**已实现（2026-11-27，闭合网格体并集 + 启动前校验；无真实 GUI/GPU 证据）**
- 关联：ADR-0061（网格内核/布尔）、`UNION` 的网格路径、ADR-0067/0068/0069（同属“工具不再 pending”）

## 背景

SketchUp 的 Outer Shell 把选中的实体（组/组件/实体）合并成一个**只有外表面**的对象：内部面被
去掉，结果是一个实体。本仓库已有：

- 精确体与网格体两条布尔 seam（ADR-0061）：网格操作数经 `MeshModeler`（manifold）在 worker 上
  做 `UNION`/`SUBTRACT`/`INTERSECT`，`dom-canvas` 同步执行并受 `mesh.sync_budget`；
- 结果作为一行 `AcDbSubDMesh` 提交，一个事务/一步撤销（`boolean_meshes`）；
- `inspect_topology_welded` 可在命令线程判定闭合/非流形（显示网格常带重复顶点，必须焊接后判定）。

对外壳来说，闭合流形体的“只留外表面”就是**并集**，无需第二套内核。

## 决策

### D1 `OUTERSHELL` = 网格并集 + 更严格的输入

- 新命令 `CommandId::OuterShell`（selector `OUTERSHELL`，无 payload，选择即输入）。
- **仅网格体**：`SemanticGeometry::Mesh` 或导入的 `Body3d { Unknown, no exact }` 显示网格；精确实体
  按 `outershell.not_mesh` 拒绝，并提示改用 `UNION`（绝不静默三角化精确实体）。
- 逐个操作数在**启动 worker 之前**用 `inspect_topology_welded` 判定拓扑：
  - `OpenShell` → `outershell.open_shell`（带边界边数）；
  - `NonManifold` → `outershell.non_manifold`（带非流形边数）。
- 其余启动前拒绝：`outershell.needs_two`（<2 个不同实体）、`outershell.sub_element_not_supported`
  （子元素/实例路径）、`outershell.layer_mismatch` / `outershell.space_mismatch`、
  `outershell.budget`（输入三角形总数超过 1 000 000 的安全阀）。没有注入 modeler 的载体复用
  `unsupported-runtime: mesh-modeler`；Viewer 是 `PermissionDenied`。
- 合法的输入交给 `boolean_meshes(command, Union, …)`：与 `UNION` 完全同一作业/提交路径，
  结果一行 `AcDbSubDMesh`，一个事务/一步撤销；`dom-canvas` 的 `mesh.sync_budget` 仍然适用。

### D2 UI

概念域工具栏/菜单的 Outer Shell 从 pending 变成 `command: 'OUTERSHELL'` 的真实命令；catalog 增加
`OUTERSHELL` 行；`SELECTION_REQUIREMENTS` 要求至少两个对象（与 UNION 相同的本地提示，主机仍按名
拒绝每一种输入错误）。命令提示明确“只用闭合网格体”。

## 不在本 ADR 范围

- 逐体 shell 偏移（真正的“加厚成壳”）、保留内部空腔、删除内部面后再缝合的独立实现；
- 精确实体的外壳运算（本仓库对精确体只有 `UNION`，不做 tessellation）；
- 递归嵌套组/组件的自动展开选择、材质/UV 继承；
- 真实 GUI/GPU 交互与其它载体运行时。

## 后果

- 新增一个命令与一次选择前置校验；**无格式档位变化**（结果就是普通网格行）。
- 开放壳/非流形在命令线程就拒绝，不占用 worker，也不让内核报错变成后台失败；
- 与 `UNION` 的差异被明确限定在“仅网格体 + 启动前拓扑校验”，不是第二套内核语义。
