# ADR-0072：分割（Split）

- 状态：**已接受（决策，2026-11-27）**；**已实现（2026-11-27，双结果分割；无真实 GUI/GPU 证据）**
- 关联：ADR-0061（网格内核/布尔）、ADR-0070（外壳）、ADR-0071（修剪）

## 背景

SketchUp Solid Tools 的 **Split** 用一个/多个实体切开目标体，得到**多块**结果（切面两侧都保留）。
本仓库已有网格布尔（`UNION`/`SUBTRACT`/`INTERSECT`）与一次提交的作业/撤销路径；Trim 已证明“只替换
目标、保留刀具”的提交形态（ADR-0071）。Split 与 Trim 的差别只有两点：

1. 需要**两块**结果：外部 `target − ∪cutters` 与内部 `target ∩ ∪cutters`；
2. 切割体可能是多个，必须先并集，否则 `∩ c1 ∩ c2` 不是 `target ∩ (c1 ∪ c2)`。

## 决策

### D1 新作业种类 `Split`，两块结果同一次提交

- 新命令 `CommandId::SplitSolid`（selector `SPLITSOLID`，无 payload，选择即输入）。
- 选择顺序：**第一个选中体是目标**，其余每个选中体是切割体（显式规则，提示与文档都写明）。
- `MeshJobKind::Split` 在 worker 上：
  1. 把切割体按 `Union` 折叠成 `U`（只有一个切割体时就是它本身）；
  2. `outside = Subtract(target, U)`；
  3. `inside = Intersect(target, U)`。
- 两块都必须通过既有的“闭合 2-流形”产物闸门（`mesh.not_closed` 拒绝非闭合结果）。
  任一为空是具名拒绝：`splitsolid.no_intersection`（内部为空，切割体没碰到目标）/
  `splitsolid.empty_piece`（外部为空，切割体覆盖了整个目标）。
- 作业产物携带第二个网格（`second: Option<Box<(MeshData, MeshTopologyReport)>>`），提交时：
  删除目标、插入**外部**与**内部**两行（各继承目标的 layer/space/draw order/显示状态），
  切割体行原样保留；历史记录 = 目标删除 patch + 两个插入 patch，一个事务/一步撤销。
- 结果仍是 `AcDbSubDMesh`（无 `face_sources`/UV/材质）。

### D2 启动前校验（都不启动 worker、不写文档）

与 Trim 相同的形状，用 `splitsolid.*`：`needs_two`（<2 个不同实体）、
`sub_element_not_supported`、`space_mismatch`、`not_mesh`（精确实体提示用 `SUBTRACT`）、
`open_shell` / `non_manifold`（焊接后拓扑）、`budget`（输入 >1 000 000 三角形）、无 modeler 的
`unsupported-runtime: mesh-modeler`、Viewer 的 `PermissionDenied`；`dom-canvas` 仍受
`mesh.sync_budget`。

### D3 UI

概念域工具栏/菜单的 Split 从 pending 变成 `command: 'SPLITSOLID'` 的真实命令（提示键说明“第一个是
目标、其余是切割体、切割体保留、得到两块”）；catalog 增加 `SPLITSOLID` 行，
`SELECTION_REQUIREMENTS` 要求 ≥2 个对象。

## 不在本 ADR 范围

- 切割体自身也被分割（本实现只分割目标）；
- 精确实体的 Split（本仓库只对闭合网格体做）；
- 结果自动归组/合并材质、按体积命名两块；
- 真实 GUI/GPU 交互与其它载体运行时。

## 后果

- 新增一个命令、一个作业种类、一个双产物提交分支；**无格式档位变化**（两块都是普通网格行）。
- 双产物只存在于作业/提交的内部类型，`MeshJobPoll::Done` 以一个首选对象 + 可选第二对象表达，
  现有单产物调用点不受影响；
- 切割体不参与删除/撤销补丁，因此撤销不会复制出第二份切割体；切割体被并发编辑会让结果按 stale
  丢弃（保守）。
