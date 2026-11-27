# 屏幕空间参考线虚线（Camera-scaled guide dashes）（2026-11-28）

line guide 从 base scene 移到相机相关 overlay：dash/gap = 6/4 逻辑像素 × `view_scale`，point
十字臂长 6px；缩放时 overlay 重建而 dash 屏幕长度保持稳定。设计见
[ADR-0092](adr/0092-screen-space-guide-dashes.md)。无命令/格式变化。

## 1. 已接线

- **overlay**：`guide_overlay` 生成 line/point 批次；有界线长不变（包围盒 ×10、≥1000），
  `subdivide_dashes` 用屏幕 scale；无 scale 回退模型 period。颜色/source/draw order 明确。
- **失效**：`overlay_fingerprint` 计入 `view_scale`，zoom 重建 overlay；base scene 不再含 guide。
- **point**：三轴十字臂长 6px × scale（回退 extent×0.005 钳制 1..25），保持连续线型。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked` | PASS（**3561 passed**，guide_tests 4：scale 0.5/1.0 dash 长度、无 scale 回退、point 6px 十字、base scene 无 guide；fingerprint 已更新） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**994 passed**） |
| `cargo test -p cad-host --lib --locked` | PASS（**506 passed**） |
| `(cd apps/desktop-ui && npm test)` | PASS（**2143 passed**） |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` | PASS |
| `python3 scripts/check-i18n.py` / `check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GPU/像素证据；dash 只有 CPU overlay 批次契约，真实窗口与其它载体运行时
**NOT VERIFIED**。

## 3. NOT VERIFIED / 不在范围

- 自定义 dash/颜色、屏幕空间抗锯齿/像素验证；
- 真实 GUI/GPU 交互与其它载体运行时验收。
