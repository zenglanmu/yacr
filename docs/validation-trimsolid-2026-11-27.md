# 修剪（Trim，`TRIMSOLID`）（2026-11-27）

`TRIMSOLID` 从 pending 变成真实命令：第一个选中体是目标、其余是刀具；经既有 manifold 布尔做
`Subtract` 折叠，**只替换目标并保留刀具**（一个事务/一步撤销）。设计见
[ADR-0071](adr/0071-trim-solid.md)。无格式档位变化。

## 1. 已接线

- **命令**：`CommandId::TrimSolid`（`TRIMSOLID`，无 payload，选择即输入）；Work-only。
- **作业**：`MeshJobKind::Trim` 在 worker 上复用 `Boolean(Subtract)` 折叠（`target − cutter…`）；
  提交只删除 `operands[0]`、插入结果，刀具行原样保留；结果继承目标的 layer/space/draw order/
  显示状态；仍是一行 `AcDbSubDMesh`。
- **启动前拒绝**：`trimsolid.needs_two`、`sub_element_not_supported`、`space_mismatch`、
  `not_mesh`（精确实体提示 `SUBTRACT`）、`open_shell`、`non_manifold`、`budget`（>1M 三角形）、
  `unsupported-runtime: mesh-modeler`、Viewer `PermissionDenied`；`dom-canvas` 仍受
  `mesh.sync_budget`。
- **UI**：概念域工具栏/Draw 菜单 Trim 不再是 pending（`command: 'TRIMSOLID'` + 提示键），
  catalog 增加 `TRIMSOLID`，选择前置要求 ≥2 个对象。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked` | PASS（**3508 passed**，含 trimsolid 3 个 + host 保留刀具提交 1 个） |
| `cargo test -p cad-host --lib --locked` | PASS（**498 passed**，含 `tests::trim_solid` 边界拒绝） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**973 passed**，含 `tests_trimsolid` 2 个） |
| `(cd apps/desktop-ui && npm test)` | PASS（tsc --noEmit + node --test：**2128 passed**，Trim 不再是 pending） |
| `cargo clippy -p cad-app -p cad-host -p cad-ui-core --all-targets --locked -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` | PASS（2993 files） |
| `python3 scripts/check-i18n.py` | PASS（3704 keys） |
| `python3 scripts/check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI/GPU 像素证据；折叠/保留刀具用 fake modeler 的作业与真实 host 提交契约，
真实 manifold 内核/大模型性能与其它载体运行时 **NOT VERIFIED**。

## 3. NOT VERIFIED / 不在范围

- 精确实体的 Trim（请用 `SUBTRACT`）；
- 刀具材质/分组继承、结果自动并入组、多目标独立修剪；
- 逐体 shell 偏移/空腔；
- 真实 GUI/GPU 交互与其它载体运行时验收。
