# 样式面板（Styles：WIREFRAME/SHADED）（2026-11-27）

Styles 托盘从“只有主题切换”变成两行真实样式的面板：`WIREFRAME`/`SHADED` 走既有
`VSCURRENT <token>`，当前 token 由 `cad_status.visualStyle` 高亮。设计见
[ADR-0077](adr/0077-styles-panel-tokens.md)。无新命令、无格式档位变化。

## 1. 已接线

- **状态投影**：`CadStatus.visual_style`（wire `visualStyle`，可选）由会话真实
  `view_extras.visual_style` 提供；`unsupported_status` 与无会话快照省略它（不伪造默认）。
- **面板**：Styles 列出 `WIREFRAME`/`SHADED`，点击 dispatch `VSCURRENT WIREFRAME` /
  `VSCURRENT SHADED`；当前值 `aria-pressed` + “当前”标注；无 `VSCURRENT` 能力时禁用。
- **边界文案**：明确这两个 token 就是全部样式模型，界面主题不是 SketchUp Style；未建模
  （材质/贴图样式、边/面开关、样式库/缩略图/水印、背景/天空）写进提示与覆盖表。
- **助手**：`domains/concept/styles.ts` 冻结 token、派发线与标签键；未知 token 不被接受。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-ui-core --lib --locked` | PASS（**987 passed**，含 `tests_styles` 2 个：默认/切换/恢复 SHADED、未知样式按名拒绝且不改 token） |
| `cargo test -p cad-app --lib --locked` | PASS（**3529 passed**） |
| `cargo test -p cad-host --lib --locked` | PASS（**503 passed**） |
| `(cd apps/desktop-ui && npm test)` | PASS（tsc --noEmit + node --test：**2137 passed**，含 styles 3 个） |
| `cargo clippy -p cad-app -p cad-host -p cad-ui-core --all-targets --locked -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` | PASS |
| `python3 scripts/check-i18n.py` | PASS |
| `python3 scripts/check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI/GPU 像素证据；面板只有状态投影与前端按钮契约，真实窗口点击与其它载体
运行时 **NOT VERIFIED**；SketchUp 风格的材质/边/面/样式库语义未建模。

## 3. NOT VERIFIED / 不在范围

- 材质/贴图样式、边/面显示开关、背景/天空/水印、样式库与缩略图；
- 真实 GUI/GPU 交互与其它载体运行时验收。
