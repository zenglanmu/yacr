# 参考线样式（GUIDE STYLE COLOR/DASH，`.ymodel` minor 30）（2026-11-28）

guide 表新增文档级 style 行：颜色 + 虚线开关；`GUIDE STYLE COLOR/DASH` 写入，renderer overlay
使用该颜色/开关。设计见 [ADR-0101](adr/0101-guide-style.md)。

## 1. 已接线

- **模型**：`GuideStyle` 默认 `#3373d9`/dashed；style 行 `{"style":{"color":[...],"dashed":...}}`，
  最多一条；非默认写 minor 30，旧数组解码默认。
- **命令**：`GUIDE STYLE COLOR r g b` / `STYLE DASH ON|OFF`（整表写入/一步撤销，Work-only）；
  LIST 报告颜色/dash。
- **渲染**：overlay 用 style 颜色；dashed=false 不切段并使用 continuous pattern；point 十字实线。
- **状态/UI**：有文档时 `cad_status.guideStyle {color, dashed}` 投影表样式；concept 域 Styles
  托盘有颜色输入与虚线开关，dispatch 同一 `GUIDE STYLE` 命令（无新命令）。
- **边界**：托盘只在 concept 域；每 guide 独立样式未建模。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked` | PASS（**3568 passed**，guide style color/dash/Viewer + overlay style） |
| `cargo test -p cad-store-zip --test guides_minor --locked` | PASS（**4 passed**：minor-30 style 往返） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**997 passed**，GUIDE STYLE grammar + status 投影） |
| `cargo test -p cad-host --lib --locked` | PASS（**508 passed**） |
| `(cd apps/desktop-ui && npm test)` | PASS（**2156 passed**，颜色/虚线命令与 hex 往返） |
| `cargo fmt --all -- --check` / file-size / i18n / architecture / command-docs | PASS |

**边界**：没有真实 GUI **交互**证据；样式只有表/命令/状态投影/CPU overlay 与前端契约，
真实窗口点击与其它载体运行时 **NOT VERIFIED**。

## 3. NOT VERIFIED / 不在范围

- 每 guide 独立样式、非 concept 域托盘控件、真实 GUI 交互/GPU 运行时。
