# 参考线虚线段（Dashed guides）（2026-11-28）

line guide 从连续实线段改为按现有 `subdivide_dashes` 分段的虚线；point 十字保持实线。设计见
[ADR-0086](adr/0086-dashed-guides.md)。无新命令、无格式档位变化。

## 1. 已接线

- **渲染**：line guide 的有界端点（模型包围盒 ×10、≥1000 单位）按
  `period=max(extent/100,5)` 的 pattern 切 runs，每个 run 转独立 `LineSegments` 对；
  fragment `linetype` 记录真实 pattern。
- **point**：三轴十字仍是连续 `LineSegments`，不受 dash 影响。
- **边界**：dash 长度与模型尺度绑定，不是屏幕空间恒定；无自定义颜色/dash/虚线验证。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked` | PASS（**3553 passed**，guide_tests 2：line 批次 >2 顶点/段长/linetype，point 6 顶点） |
| `cargo test -p cad-db --lib --locked` | PASS（**526 passed**） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**994 passed**） |
| `cargo test -p cad-host --lib --locked` | PASS（**505 passed**） |
| `(cd apps/desktop-ui && npm test)` | PASS（**2140 passed**） |
| `cargo clippy -p cad-app -p cad-db -p cad-domain -p cad-history -p cad-store-zip -p cad-ui-core -p cad-host --all-targets --locked -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` / `check-i18n.py` / `check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI/GPU 像素证据；dash 分段只有 CPU 场景批次契约，真实窗口与其它载体运行时
**NOT VERIFIED**。

## 3. NOT VERIFIED / 不在范围

- 屏幕空间恒定 dash、用户自定义颜色/dash、抗锯齿/像素证据；
- 其它 guide 类型与图纸空间 guide。
