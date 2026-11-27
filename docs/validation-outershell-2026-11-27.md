# 外壳（Outer Shell）（2026-11-27）

`OUTERSHELL` 从 pending 变成真实命令：把所选**闭合网格体**经既有 manifold 布尔路径合并成一个
`AcDbSubDMesh` 外壳（一个事务/一步撤销）。设计见 [ADR-0070](adr/0070-outer-shell.md)。无格式档位
变化。

## 1. 已接线

- **命令**：`CommandId::OuterShell`（selector `OUTERSHELL`，无 payload，选择即输入）；Work-only，
  Viewer 拒绝。
- **启动前校验**：<2 个不同实体、子元素/实例路径、图层/空间不一致、非网格（含精确实体，提示改用
  `UNION`）、开放壳（`outershell.open_shell`，焊接后边界边计数）、非流形
  （`outershell.non_manifold`）、输入三角形 >1 000 000（`outershell.budget`）；无 modeler 的载体是
  `unsupported-runtime: mesh-modeler`。
- **作业/提交**：合法输入交给 `boolean_meshes(Union)`——与 `UNION` 同一条 worker 与提交路径，
  结果一行 `AcDbSubDMesh`、一个事务/一步撤销；`dom-canvas` 的 `mesh.sync_budget` 仍然适用。
- **UI**：概念域工具栏/Draw 菜单 Outer Shell 不再是 pending（`command: 'OUTERSHELL'` + 提示键），
  catalog 增加 `OUTERSHELL`，`SELECTION_REQUIREMENTS` 要求 ≥2 个对象。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked` | PASS（**3504 passed**，含 outer_shell 5 个 + host 提交 1 个） |
| `cargo test -p cad-host --lib --locked` | PASS（**497 passed**，含 `tests::outer_shell` 的 host 边界拒绝） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**971 passed**，含 `tests_outershell` 2 个） |
| `(cd apps/desktop-ui && npm test)` | PASS（tsc --noEmit + node --test：**2128 passed**，Outer Shell 不再是 pending） |
| `cargo clippy -p cad-app -p cad-host -p cad-ui-core --all-targets --locked -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` | PASS（2991 files） |
| `python3 scripts/check-i18n.py` | PASS（3701 keys） |
| `python3 scripts/check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI/GPU 像素证据；网格并集用 fake modeler 的契约测试与 host 提交契约，真实
manifold 内核/大模型性能与其它载体运行时 **NOT VERIFIED**。

## 3. NOT VERIFIED / 不在范围

- 逐体 shell 偏移/加厚、保留内部空腔；
- 精确实体的外壳运算（本仓库只提供 `UNION`，不三角化精确体）；
- 嵌套组/组件自动展开、材质/UV 继承；
- 真实 GUI/GPU 交互与其它载体运行时验收。
