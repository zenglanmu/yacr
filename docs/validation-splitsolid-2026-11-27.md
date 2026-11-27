# 分割（Split，`SPLITSOLID`）（2026-11-27）

`SPLITSOLID` 从 pending 变成真实命令：第一个选中体是目标、其余是切割体；worker 先并集切割体，
再把目标分成**外部**（`target − ∪cutters`）与**内部**（`target ∩ ∪cutters`）两块同时插入，切割体
保留。设计见 [ADR-0072](adr/0072-split-solid.md)。无格式档位变化。

## 1. 已接线

- **命令**：`CommandId::SplitSolid`（`SPLITSOLID`，无 payload，选择即输入）；Work-only。
- **作业**：`MeshJobKind::Split` 在 worker 上做 `Union`（切割体）→ `Subtract`（外部）→
  `Intersect`（内部）；两块都必须闭合（`mesh.not_closed`），任一为空是
  `splitsolid.no_intersection` / `splitsolid.empty_piece`（作业失败、不写文档）。
- **提交**：删除目标、插入两块（各继承目标的 layer/space/draw order/显示状态），切割体行保留；
  一次事务/一步撤销；结果仍是 `AcDbSubDMesh`。
- **启动前拒绝**：`splitsolid.needs_two`、`sub_element_not_supported`、`space_mismatch`、
  `not_mesh`（精确实体提示 `SUBTRACT`）、`open_shell`、`non_manifold`、`budget`（>1M 三角形）、
  `unsupported-runtime: mesh-modeler`、Viewer `PermissionDenied`；`dom-canvas` 仍受
  `mesh.sync_budget`。
- **UI**：概念域工具栏/Draw 菜单 Split 不再是 pending（`command: 'SPLITSOLID'` + 提示键），
  catalog 增加 `SPLITSOLID`，选择前置要求 ≥2 个对象。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked` | PASS（**3512 passed**，含 split 3 个 + host 双块/保留切割体 1 个） |
| `cargo test -p cad-host --lib --locked` | PASS（**499 passed**，含 `tests::split_solid` 边界拒绝） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**975 passed**，含 `tests_splitsolid` 2 个） |
| `(cd apps/desktop-ui && npm test)` | PASS（tsc --noEmit + node --test：**2128 passed**，Split 不再是 pending） |
| `cargo clippy -p cad-app -p cad-host -p cad-ui-core --all-targets --locked -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` | PASS（2995 files） |
| `python3 scripts/check-i18n.py` | PASS（3707 keys） |
| `python3 scripts/check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI/GPU 像素证据；布尔折叠/双产物用 fake modeler 的作业契约与真实 host 提交
契约，真实 manifold 内核/大模型性能与其它载体运行时 **NOT VERIFIED**；空侧拒绝（no_intersection）
由作业代码路径保证，未用真实内核样本覆盖。

## 3. NOT VERIFIED / 不在范围

- 切割体自身被分割、精确实体 Split；
- 结果归组/材质继承/按体积命名；
- 真实 GUI/GPU 交互与其它载体运行时验收。
