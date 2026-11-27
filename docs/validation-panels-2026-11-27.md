# 照片匹配与叠加层面板（Match Photo / Overlays）（2026-11-27）

托盘最后两个 pending 面板变成**真实的 host-read 面板 + 具名拒绝**：Match Photo 报告未建模的
能力，Overlays 只读列出渲染器拥有的通道。设计见
[ADR-0075](adr/0075-match-photo-and-overlays-panels.md)。无格式档位变化。

## 1. 已接线

- **host read**：`cad_match_photo`（`kind:"matchPhoto"`）返回 `modeled:false` + 稳定原因；
  `cad_overlays`（`kind:"overlays"`）返回 8 个真实通道（axes/grid/selectionHighlight/snapHints/
  selectionBox/hoverTint/measurement/drawPreview）及当前状态。无通道载体分别答
  `unsupported-runtime: matchPhoto` / `unsupported-runtime: overlays`。
- **命令**：`MATCHPHOTO`（`matchphoto.not_implemented`）与 `OVERLAYS`
  （`overlays.not_implemented`）都是无 payload 的具名拒绝，不写文档、不入历史，Viewer 合法。
- **UI**：概念域托盘 Match Photo/Overlays 不再是 pending（真实 body：原因/通道列表 + 动作按钮），
  catalog 增加两行与提示键；面板 body 移到 `domains/ui/tray-panels.tsx`。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked` | PASS（**3525 passed**，含 panels 3 个：两个拒绝、Viewer 合法） |
| `cargo test -p cad-host --lib --locked` | PASS（**503 passed**，含 `tests::panels` 2 个：能力读、通道列表） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**982 passed**，含 `tests_panels` 2 个：两个 kind 往返 + 投影） |
| `(cd apps/desktop-ui && npm test)` | PASS（tsc --noEmit + node --test：**2132 passed**，含 panels 4 个） |
| `cargo clippy -p cad-app -p cad-host -p cad-ui-core --all-targets --locked -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` | PASS（3006 files） |
| `python3 scripts/check-i18n.py` | PASS（3736 keys） |
| `python3 scripts/check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI/GPU 像素证据；面板只有宿主读取与前端解析/接线契约，真实窗口交互与其它
载体运行时 **NOT VERIFIED**；照片匹配与用户管理叠加都仍未建模（面板明说）。

## 3. NOT VERIFIED / 不在范围

- 照片导入/灭点求解/相机对齐/照片背景；
- 用户自定义叠加、排序、持久化；
- 真实 GUI/GPU 交互与其它载体运行时验收。
