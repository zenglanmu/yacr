# 量角器（Protractor）（2026-11-27）

Protractor 从 pending 变成**真实三点角度量具**：先点顶点、再各点一条射线，结果走既有
`Angle3Points` 测量记录与角度格式化（只读、不写文档）；guide 线创建未建模，`PROTRACTOR GUIDE`
是具名拒绝。设计见 [ADR-0068](adr/0068-protractor.md)。无新命令、无格式变化。

## 1. 已接线

- `MeasurementToolKind::Protractor`（`ALL` 5 → 6，末尾追加），`MEASUREMENT_SELECTORS` 增加
  `("PROTRACTOR", Protractor)`：无 `CommandId`，由 `cad_commands` 广告，路由到
  `BeginMeasurement(Protractor)`。
- 捕获顺序 = SketchUp 的顶点 → 第一条射线 → 第二条射线；`MeasurementToolKind::engine_points`
  把捕获顺序适配到引擎的 `(ray, vertex, ray)`（记录 `inputs` 为引擎顺序，预览保持捕获顺序）。
- 状态行分步提示（ASCII）；结果仍走 `measure.result` 诊断与 `format_angle`。
- `PROTRACTOR GUIDE` → `protractor.guide_not_implemented`（不启动捕获、不写文档）；
  其它参数 → `unsupported CAD command`。
- 概念域工具栏/Draw 菜单 Protractor 不再是 pending（`command: 'PROTRACTOR'` + 提示键）；
  catalog 增加 `PROTRACTOR`（`measure.kind.protractor`）。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked` | PASS（**3491 passed**，含 Protractor 顶点优先/90° 记录/引擎顺序契约） |
| `cargo test -p cad-host --lib --locked` | PASS（**495 passed**，含 `tests::protractor` 1 个：交互路径只读、记录顶点在引擎第二位） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**966 passed**，含 `tests_protractor` 3 个：选择子/路由、GUIDE 具名拒绝、其它参数拒绝） |
| `(cd apps/desktop-ui && npm test)` | PASS（tsc --noEmit + node --test：**2128 passed**，Protractor 不再是 pending） |
| `cargo clippy -p cad-app -p cad-host -p cad-ui-core --all-targets --locked -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` | PASS（2979 files） |
| `python3 scripts/check-i18n.py` | PASS（3696 keys） |
| `python3 scripts/check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI/GPU 像素证据；捕获/提交只有 app 与 host 的合成契约，真实窗口交互与其它
载体运行时 **NOT VERIFIED**。

## 3. NOT VERIFIED / 不在范围

- guide/参考线实体与 guide 表、按角度旋转的参考线、`FOLLOWME` 以 guide 为路径；
- 任意 3D 工作平面朝向的角度、角度吸附/增量输入/拖动修改；
- 真实 GUI/GPU 交互与其它载体运行时验收。
