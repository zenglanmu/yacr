# 逐面材质（Per-face materials，`.ymodel` minor 25）（2026-11-27）

选中网格面时 `PAINT <name>` 只涂该面：材质表行的 `faces` 快照保存面赋值，渲染器按材质把网格
拆成多个 fragment。设计见 [ADR-0081](adr/0081-per-face-materials.md)。

## 1. 已接线

- **表**：`tables/materials.json` 行可带 `"faces": [{"entity","face"}]`；无 `faces` 保持
  minor 19，非空升 **minor 25**（`FORMAT_MINOR = 25`）。校验 face key/材质存在/重复/上限
  （65 536），载入要求实体存在；face key 不在当前网格中不阻止载入（见 D3 的 inert 报告）。
- **命令**：`PAINT <name>` 接受 pick 产生的网格面 `sub_element`；只写该面，整实体+同面混合选择
  按 `paint.mixed_targets` 拒绝，未知面/非网格面按 `paint.face_unknown` /
  `paint.face_not_paintable` 拒绝；一次写入/一步撤销。`MATERIALS` 报告分别统计对象与面。
- **渲染**：`cad-representation` 按面材质拆分网格 fragment；未涂面保留实体颜色，透明材质乘
  alpha；`MeshWithStoredEdges` 的边只挂第一个 fragment；陈旧 face key 报
  `representation.material_face_inert` 且不重映射。
- **UI**：Materials 托盘提示改为“选中对象或网格面都可涂刷”；文档材质列表/重涂沿用 ADR-0079。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked` | PASS（**3540 passed**，含 materials 13：面涂刷、未知面、混合选择、非网格面、报告计数） |
| `cargo test -p cad-db --lib --locked` | PASS（**523 passed**，含面赋值编解码、擦除清理、缺失材质拒绝） |
| `cargo test -p cad-representation --lib --locked` | PASS（**381 passed**，含面拆分/透明 alpha/陈旧 key 三个新用例） |
| `cargo test -p cad-domain --locked` | PASS（**290 unit + 2/9/14/3/3 integration passed**） |
| `cargo test -p cad-store-zip --test material_faces_minor --locked` | PASS（**2 passed**：minor 25 往返、整实体表保持 19） |
| `cargo test -p cad-store-zip --locked` | PASS（含 archive manifest 档位更新到 25） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**989 passed**） |
| `cargo test -p cad-host --lib --locked` | PASS（**505 passed**） |
| `(cd apps/desktop-ui && npm test)` | PASS（**2140 passed**） |
| `cargo clippy -p cad-app -p cad-db -p cad-domain -p cad-representation -p cad-store-zip -p cad-ui-core -p cad-host --all-targets --locked -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` / `check-i18n.py` / `check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI/GPU 像素证据；面材质只有命令/编解码/展开契约，真实窗口点击、面高亮与
其它载体运行时 **NOT VERIFIED**；材质编辑器、贴图/材质库仍未建模。

## 3. NOT VERIFIED / 不在范围

- 材质编辑器（改名/改色/删除）、贴图/纹理、材质库、逐面选择高亮；
- 真实 GUI/GPU 交互与其它载体运行时验收。
