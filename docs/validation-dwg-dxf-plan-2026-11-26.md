# 验证记录：DWG/DXF 默认 2D 视图与块属性坐标系修复（2026-11-26）

## 范围

- **默认观察**：经 `dwg` / `dxf` 连接器打开文档后，native 载体的打开漏斗始终停在 plan 视图，
  即使 model space 含 `Body3d` 或显著 z 范围；若复用视口此前停在 3D，先重置到 canonical top
  再取景。其它来源仍按 `document_is_3d` 选择等轴测 3D。从 plan 下按 `FIT` 保持 2D（既有规则，
  未改）。需要 3D 时用户仍可显式选标准视图。
- **`complex-test.dwg` 显示异常**：块引用携带的 `ATTRIB` 在文件中是绘制坐标系放置，导入器此前
  原样存进块局部字段，表示层再次乘引用变换 ⇒ 字高/位置被二次缩放，model bounds 被撑到约
  2.7 亿单位，FIT 后整图不可见。现在导入按引用变换逆映射回块局部，导出正向映射回 `ATTRIB`。

## 实现落点

- `crates/cad-app/src/host/documents.rs` — `fit_opened_document` 的 `opened_from_2d_connector`
  例外（`dwg`/`dxf`）。
- `crates/cad-connector-acadrust/src/entity/attribute.rs`、`entity/insert.rs` — ATTRIB 世界→块局部。
- `crates/cad-connector-acadrust/src/export/attribute.rs`、`export/insert.rs` — 块局部→世界。

## 已运行

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-connector-acadrust --locked` | 通过：546 passed，1 ignored，0 failed |
| `cargo test -p cad-app --lib --locked` | 通过：3410 passed，3 ignored，0 failed（含 `host::tests::document` 18 passed） |
| `cargo test -p cad-host --locked` | 通过：480 passed，0 failed |
| `cargo clippy -p cad-connector-acadrust -p cad-app --all-targets --locked -- -D warnings` | 通过 |
| `cargo fmt -p cad-connector-acadrust -p cad-app` | 已格式化（仅本轮文件改动） |
| 临时集成测试：导入 `fixtures/complex-test.dwg` 并打印 `DrawingDatabase::bounds()` | 修复后 `x [-9863.93, 419018.05]`、`y [-268126.18, 266618.66]`、`z ±13470.49`；model-space `Body3d` 计数 0。临时测试已删除，不入库 |
| `target/debug/cad-cli-tools render fixtures/complex-test.dwg --png <tmp>/plan-fixed.png --width 1600 --height 1000` | exit 0；2D plan camera；`pixels.non_background=20889`、`coverage=0.0131`（修复前同一命令是 17 px / `1.06e-05`，整图空白）。修复后的帧能看到多张楼层平面与「2#楼…层」文字，没有异常巨字。Vulkan / Quadro P620 的无头 GPU 帧，不是 GUI 或像素验收 |

回归契约：

- `attributes_store_block_local_placements_under_a_rotated_scaled_reference` 钉住旋转+缩放引用下
  的 ATTRIB 位置/旋转/字高恢复，并断言包围盒不再被二次缩放。
- `fit_opened_document_keeps_a_dwg_in_plan_even_with_3d_geometry` 先把视口留在 3D，再断言
  DWG 打开会重置到 plan 并保持正交；既有
  `fit_opened_document_enters_the_isometric_3d_view_for_a_3d_model` 仍覆盖无连接器来源的
  3D 默认。

## 未运行 / 未验证

- 全 workspace 测试、`cargo check` 全 workspace、其它载体 crate：**NOT RUN**。
- 真实 GUI 目视（Linux `native-below` / X11 / Windows / macOS / Android）、真实 GPU 窗口内像素、
  `complex-test.dwg` 修复后的 GUI 截图：**NOT VERIFIED**。已渲染的无头 CLI 帧只证明数据/表示
  链路在同一真实 GPU 上能产出可见平面图，不是窗口、输入或像素验收。
- Web/`dom-canvas` 运行时：**NOT VERIFIED**（该载体本来就是 2D-only，不套用 native 默认观察）。
- 真实第三方 CAD 读取器打开导出 DXF/DWG 的 ATTRIB 放置：**NOT RUN**（导出侧只有本仓库
  writer→reader 往返契约）。

## 证据边界

以上是导入契约、宿主契约、编译/lint 与一张无头 GPU 帧的证据，不是真机、GUI、真实窗口像素、
视觉或 vendor 兼容验收。无头帧使用 Vulkan / Quadro P620，但仍是 headless render，不得推广为
其它 GPU/驱动或产品验收。
