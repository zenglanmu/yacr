# ADR-0071：修剪（Trim）

- 状态：**已接受（决策，2026-11-27）**；**已实现（2026-11-27，减法折叠 + 保留刀具；无真实 GUI/GPU 证据）**
- 关联：ADR-0061（网格内核/布尔）、ADR-0070（外壳）、`SUBTRACT` 的网格路径

## 背景

SketchUp Solid Tools 的 **Trim** 用刀具体修剪目标体，并**同时保留刀具**（不同于 Subtract 只留
结果）。本仓库已有：

- 精确体与网格体两条布尔 seam（ADR-0061）；网格路径由 `MeshModeler` 在 worker 上折叠
  `UNION`/`SUBTRACT`/`INTERSECT`；
- 一次提交把**所有**操作数替换为一行结果（`boolean_meshes`）；
- 显示网格常带重复顶点，因此闭合判定必须焊接后再做（`inspect_topology_welded`）。

Trim 的几何与 `SUBTRACT` 完全相同，差异只在**提交**：只删除目标、保留刀具。所以不需要第二套
内核或第二种几何语义，只需要把“保留哪些操作数”写进作业计划。

## 决策

### D1 一个新作业种类 `Trim`，而不是新内核路径

- 选择即输入：**第一个选中的体是目标**，其余每个选中体是刀具（顺序显式，文档与提示都写明）。
- 新命令 `CommandId::TrimSolid`（selector `TRIMSOLID`，无 payload）。
- `MeshJobKind::Trim` 在 worker 上与 `Boolean(Subtract)` **同一条折叠**（`target − cutter…`），
  因此内核调用、取消、预算与结果类型都完全复用。
- 提交时（`host::mesh::commit_mesh_job`）：
  - 仍校验每个操作数（含刀具）与开始时完全一致，任一变化都按 stale 丢弃；
  - 只删除 `operands[0]`（目标），插入结果行；
  - 刀具行不进 delete patch，撤销时也不会被“恢复”成另一份副本；
  - 结果继承**目标**的 layer/space/draw order 与显示状态；历史记录 = 目标删除 patch + 结果插入
    patch，仍是一个事务/一步撤销。
- 结果仍是一行 `AcDbSubDMesh`（无 `face_sources`/UV/材质）。

### D2 启动前校验（都不启动 worker、不写文档）

- `<2` 个不同实体 → `trimsolid.needs_two`；
- 子元素/实例路径 → `trimsolid.sub_element_not_supported`；
- 与目标不同空间 → `trimsolid.space_mismatch`（图层不要求一致：结果在目标图层，刀具保留自己的）；
- 精确实体 → `trimsolid.not_mesh`（提示用 `SUBTRACT`，绝不三角化精确体）；其它非网格体同样拒绝；
- 焊接后拓扑：开放壳 → `trimsolid.open_shell`（带边界边数）、非流形 → `trimsolid.non_manifold`；
- 输入三角形总数 >1 000 000 → `trimsolid.budget`；
- 没有注入 modeler 的载体 → `unsupported-runtime: mesh-modeler`；Viewer → `PermissionDenied`；
- `dom-canvas` 的同步执行仍受 `mesh.sync_budget`。

### D3 UI

概念域工具栏/菜单的 Trim 从 pending 变成 `command: 'TRIMSOLID'` 的真实命令（提示键说明“第一个是
目标、其余是刀具、刀具保留”）；catalog 增加 `TRIMSOLID` 行，`SELECTION_REQUIREMENTS` 要求 ≥2 个
对象。

## 不在本 ADR 范围

- 精确实体的 Trim（本仓库只对网格体做，精确实体请用 `SUBTRACT`）；
- 刀具的材质/分组继承、把结果自动并入刀具所在组；
- 多目标同时修剪的独立语义（本实现是“一个目标、多个刀具”的折叠）；
- 逐体 shell 偏移/空腔、真实 GUI/GPU 交互与其它载体运行时。

## 后果

- 新增一个命令、一个作业种类和一处提交分支；**无格式档位变化**（结果就是普通网格行）。
- “刀具保留”由作业种类唯一决定，命令层、worker 与提交层不会各自猜一份；
- 刀具被并发编辑会让结果按 stale 丢弃（保守），而不是写入与用户所见不一致的修剪体。
