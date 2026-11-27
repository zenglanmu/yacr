# 三维文字（3D Text）（2026-11-27）

3D Text 从 pending 变成真实命令：`TEXT3D <x> <y> <z> <height> <extrusion> <font|-> <text>` 用宿主
outline 字体引擎**已填充的字形面**挤出一个 `AcDbSubDMesh`（上下盖沿用显示三角形、侧墙来自边界边），
一次事务/一步撤销。设计见 [ADR-0069](adr/0069-3d-text.md)。无格式档位变化（保存的是普通网格行）。

## 1. 已接线

- **命令**：`CommandId::Text3d`（canonical `TEXT3D`）+ typed grammar（5 个有限数 + 字体 token
  `-`/名称 + 非空字符串）；Work-only，Viewer 拒绝。
- **几何**：`FontEngine::shape`/`shape_unstyled` → `ShapedText.meshes`；每个字形上下盖复制已填充
  三角形，边界边生成朝外四边形侧墙；结果经 `commit_new_entity_typed` 以 `AcDbSubDMesh` 写入。
- **拒绝**：`text3d.payload` / `invalid_point` / `invalid_size` / `invalid_text` /
  `font_unavailable` / `no_outlines` / `not_solid`（SHX 或填充失败）/ `budget`
  （>131 072 三角形或 >100 000 顶点）；拒绝不写文档。
- **UI**：概念域工具栏/Draw 菜单 3D Text 不再是 pending（`command: 'TEXT3D'` + 提示键），
  catalog 增加 `TEXT3D` 行（`draw.kind.text3d`）。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked` | PASS（**3498 passed**，含 `tests::text3d` 3 个 + `app_drawing::text3d` 4 个：网格/撤销、坏尺寸与文本、Viewer、not_solid/budget/厚度） |
| `cargo test -p cad-host --lib --locked` | PASS（**496 passed**，含 `tests::text3d`：一行 `AcDbSubDMesh`、一条 revision、z 跨度 0..2） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**969 passed**，含 `tests_text3d` 3 个：grammar、非法行、命令层拒绝） |
| `(cd apps/desktop-ui && npm test)` | PASS（tsc --noEmit + node --test：**2128 passed**，3D Text 不再是 pending） |
| `cargo clippy -p cad-app -p cad-host -p cad-ui-core --all-targets --locked -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` | PASS（2983 files） |
| `python3 scripts/check-i18n.py` | PASS（3698 keys） |
| `python3 scripts/check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI/GPU 像素证据；挤出几何只有 CPU/app/host 契约与合成指针/命令，真实窗口
交互、性能与其它载体运行时 **NOT VERIFIED**。

## 3. NOT VERIFIED / 不在范围

- 字体选择对话框、旋转/对齐/斜体/描边、字距/行距；
- 截面/圆角/倒角、参数化可编辑文字、材质/纹理；
- 真实 GUI/GPU 交互与其它载体运行时验收。
