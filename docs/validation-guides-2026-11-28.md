# 构造参考线（DIST GUIDE / PROTRACTOR GUIDE，`.ymodel` minor 26）（2026-11-28）

Tape Measure 与 Protractor 在 guide 模式下写持久构造参考线：`tables/guides.json` 保存归一化
point/direction，渲染器画成有界实线段，`GUIDE LIST|CLEAR` 管理。设计见
[ADR-0084](adr/0084-construction-guides.md)。

## 1. 已接线

- **表**：`tables/guides.json`（≤64 行）整表编码；无表保持原档位，非空写 **minor 26**；校验
  有限坐标/归一化非零方向/未知键；一次写入一步撤销，历史独占回放。
- **测量模式**：`DIST GUIDE` 两点完成后写两点连线；`PROTRACTOR GUIDE` 三点点完后从顶点沿第二
  条射线写 guide；Viewer 启动前按 `guide.requires_edit` 拒绝；普通测量仍只读。
- **管理**：`GUIDE`/`GUIDE LIST` 只读报告（前 8 行），`GUIDE CLEAR` 整表清空（一步撤销/
  Viewer 拒绝）；`GUIDE` 命令数 260 → 262（含 FLIP）。
- **渲染**：模型包围盒对角线 ×10（至少 1000 单位）的有界实线段；合成 source，不进实体索引/
  选择/导出；仅模型空间。
- **边界**：无 guide 点/角度圆弧/虚线/单条删除/图纸空间 guide。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked` | PASS（**3551 passed**，guides 5：DIST/PROTRACTOR guide、LIST/CLEAR、Viewer、渲染批次） |
| `cargo test -p cad-db --lib --locked` | PASS（**526 passed**，guides 3：写入/快照、cap/clear、退化方向） |
| `cargo test -p cad-history --locked` | PASS（**43 passed**，guide 步进校验） |
| `cargo test -p cad-store-zip --test guides_minor --locked` | PASS（**2 passed**：minor 26 往返、无 guide 保持 4） |
| `cargo test -p cad-store-zip --locked` | PASS（含 archive manifest 档位更新到 26） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**994 passed**，tests_guides 3 + protractor guide 模式） |
| `cargo test -p cad-host --lib --locked` | PASS（**505 passed**） |
| `(cd apps/desktop-ui && npm test)` | PASS（**2140 passed**） |
| `cargo clippy -p cad-app -p cad-db -p cad-domain -p cad-history -p cad-store-zip -p cad-ui-core -p cad-host --all-targets --locked -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` / `check-i18n.py` / `check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI/GPU 像素证据；guide 只有表/命令/场景批次契约，真实窗口与其它载体运行
时 **NOT VERIFIED**；无限线、虚线、guide 点与单条选择/擦除未建模。

## 3. NOT VERIFIED / 不在范围

- 无限线渲染、虚线、guide 点/角度圆弧、单条删除、图纸空间 guide；
- 真实 GUI/GPU 交互与其它载体运行时验收。
