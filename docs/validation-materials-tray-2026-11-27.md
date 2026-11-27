# 材质面板的文档材质行（cad_materials）（2026-11-27）

Materials 托盘现在读取并列出文档自己的 `tables/materials.json` 行（名称/颜色/透明度），点击
色块走既有 `PAINT <name>`；内置调色板保留。设计见
[ADR-0079](adr/0079-materials-tray-document-rows.md)。无新命令、无格式档位变化。

## 1. 已接线

- **host read**：`cad_materials`（`kind: "materials"`）返回文档材质行 + `total`/`truncated`
  （上限 256）；空文档/无材质是空列表；无通道载体答 `unsupported-runtime: materials`。
- **session/registry**：`CadSession::materials`、`cad_materials` 行与 `HostReadAnswer::Materials`
  走同一请求携带型 seam；桌面/网页载体无需逐载体文件。
- **UI**：托盘上方列出文档材质（真实颜色/透明度），点击 dispatch `PAINT <name>`；下方是内置
  12 色；文档变化后刷新，失败显示原因。提示明确涂刷仍是整实体、逐面/编辑器/贴图未建模。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-host --lib --locked` | PASS（**505 passed**，含 materials 2 个：PAINT 后列出真实行、不同内置材质保留两行） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**989 passed**，含 tests_materials 2 个：kind 往返/无通道标记、wire 投影） |
| `cargo test -p cad-app --lib --locked` | PASS（**3532 passed**，未改命令层） |
| `(cd apps/desktop-ui && npm test)` | PASS（tsc --noEmit + node --test：**2140 passed**，含 materials 3 个） |
| `cargo clippy -p cad-app -p cad-host -p cad-ui-core --all-targets --locked -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` / `check-i18n.py` / `check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI/GPU 像素证据；面板只有 host read 与前端解析/接线契约，真实窗口点击与
其它载体运行时 **NOT VERIFIED**；逐面材质、材质编辑器、贴图/材质库仍未建模。

## 3. NOT VERIFIED / 不在范围

- 逐面/子元素材质、改名/改色/删除、贴图与材质库、缩略图/拖放；
- 真实 GUI/GPU 交互与其它载体运行时验收。
