# 参考线点与单条删除（GUIDE POINT / DELETE，`.ymodel` minor 27）（2026-11-28）

guide 表新增 point 行与单条删除：`GUIDE POINT <x> <y> <z>` 追加点标记（minor 27，三轴十字），
`GUIDE DELETE <n>` 删除 1-based 行，两者各一次整表写入/一步撤销。设计见
[ADR-0085](adr/0085-guide-points-and-delete.md)。

## 1. 已接线

- **模型**：`Guide::Line | Guide::Point`；point 行 `{point}`，line 行 `{point,direction}`；存在
  point 行写 minor 27，line-only 保持 26，空表保持原档位；共享 64 行上限/严格校验。
- **命令**：`GUIDE POINT`/`GUIDE DELETE <n>`；越界 `guide.unknown_index`，非有限点
  `guide.invalid`；Viewer `PermissionDenied`；整表写入一步撤销。`GUIDE LIST` 带 1-based 行号与
  line/point 类型。
- **渲染**：point 画三轴十字（臂长 = 包围盒对角线 ×0.005，钳制 1..25），合成 source，不进实体/
  拾取/导出。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked` | PASS（**3553 passed**，guides 6：point/delete、单条 undo、point 渲染十字） |
| `cargo test -p cad-db --lib --locked` | PASS（**526 passed**） |
| `cargo test -p cad-history --locked` | PASS（**43 passed**） |
| `cargo test -p cad-store-zip --test guides_minor --locked` | PASS（**3 passed**：line 26、point 27、无 guide） |
| `cargo test -p cad-store-zip --locked` | PASS（archive manifest 档位更新到 27） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**994 passed**，GUIDE DELETE/POINT 语法） |
| `cargo test -p cad-host --lib --locked` | PASS（**505 passed**） |
| `(cd apps/desktop-ui && npm test)` | PASS（**2140 passed**） |
| `cargo clippy -p cad-app -p cad-db -p cad-domain -p cad-history -p cad-store-zip -p cad-ui-core -p cad-host --all-targets --locked -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` / `check-i18n.py` / `check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI/GPU 像素证据；point 只有模型/命令/场景批次契约，真实窗口与其它载体运行
时 **NOT VERIFIED**；自动 point、虚线、角度圆弧 guide 与图纸空间 guide 未建模。

## 3. NOT VERIFIED / 不在范围

- 捕获中自动创建 point、虚线、角度圆弧、单条拾取删除、图纸空间 guide；
- 真实 GUI/GPU 交互与其它载体运行时验收。
