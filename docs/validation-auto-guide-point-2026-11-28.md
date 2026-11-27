# DIST GUIDE 自动参考线点（2026-11-28）

`DIST GUIDE` 的两个拾取点重合（距离 ≤ 1e-9）时写 point 标记而不是 `guide.degenerate`，测量仍
返回 0 距离。设计见 [ADR-0087](adr/0087-auto-guide-point.md)。无格式档位变化。

## 1. 已接线

- `add_measurement_guide` 的 DistanceLine 分支：重合点 → `GuidePoint`，否则 line。
- point 行复用既有 minor-27 表与三轴十字渲染；无需新表/命令。
- Protractor 退化方向仍按 `guide.degenerate` 拒绝。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked` | PASS（**3554 passed**，guides 7：重合点写 point 且测量为 0） |
| `cargo test -p cad-db --lib --locked` | PASS（**526 passed**） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**994 passed**） |
| `cargo test -p cad-host --lib --locked` | PASS（**505 passed**） |
| `(cd apps/desktop-ui && npm test)` | PASS（**2140 passed**） |
| `cargo clippy -p cad-app -p cad-db -p cad-domain -p cad-history -p cad-store-zip -p cad-ui-core -p cad-host --all-targets --locked -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` / `check-i18n.py` / `check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：无真实 GUI/GPU 像素证据；仅命令/表契约，真实窗口与其它载体运行时 **NOT VERIFIED**。

## 3. NOT VERIFIED / 不在范围

- 其它自动 point 规则、屏幕空间样式、真实 GUI/GPU 运行时。
