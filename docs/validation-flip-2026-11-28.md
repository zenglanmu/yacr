# Flip Along（FLIP X|Y|Z）（2026-11-28）

`FLIP X|Y|Z`（`RED|GREEN|BLUE`）绕选择包围盒中心、垂直于该轴的平面**原地**反射：源删除 +
镜像副本，一次事务/一步撤销，文字字形真反射。设计见 [ADR-0083](adr/0083-flip-along.md)。
无格式档位变化。

## 1. 已接线

- **命令**：新选择子 `FLIP`（命令数 260 → 261）；参数只接受一个轴，裸/非法文本
  `flip.axis_required`；空选择 `flip.selection_required`；Viewer `PermissionDenied`。
- **语义**：反射平面由 `selected_object_bounds` 的中心决定（与 `ZOOM OBJECT` 同一目标解析），
  复用既有镜像路径（`erase_source=true`、`mirrortext=true`），成功后选中新副本；undo 清空旧 id
  选择。
- **边界**：副本为新 entity id（不保留 group/component 身份）；子图元/块定义成员沿用既有镜像
  拒绝；无格式变化、无前端改动。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked` | PASS（**3546 passed**，flip 3：X 反射/选区/undo、Y/Z 轴中心、空选择与 Viewer 拒绝） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**991 passed**，tests_flip 2：语法/裸行拒绝、typed 派发；命令表 261） |
| `cargo test -p cad-host --lib --locked` | PASS（**505 passed**，未改 host read） |
| `(cd apps/desktop-ui && npm test)` | PASS（**2140 passed**，未改前端） |
| `cargo clippy -p cad-app -p cad-ui-core -p cad-host --all-targets --locked -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` / `check-i18n.py` / `check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI/GPU 像素证据；Flip 只有命令/事务契约，真实窗口与其它载体运行时
**NOT VERIFIED**；对象身份保留、块成员原地 Flip 仍未建模。

## 3. NOT VERIFIED / 不在范围

- group/component 同一对象身份、块定义成员/子图元原地 Flip；
- 真实 GUI/GPU 交互与其它载体运行时验收。
