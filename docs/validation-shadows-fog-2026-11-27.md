# 阴影与雾（Shadows and Fog，`.ymodel` minor 22）（2026-11-27）

SketchUp 的 Shadows 与 Fog 首次成为**文档级显示设置**：`tables/display.json` 保存太阳
（方位角/高度角 + 开关）与雾（区间 + 颜色），渲染器把太阳当方向光、可见 shader 做距离雾，
`.ymodel` 以 **minor 22** 保存并在重开后保持。设计见
[ADR-0065](adr/0065-shadows-and-fog.md)。

## 1. 已接线

- **模型**：`cad-domain::display::{DisplaySettings, ShadowSettings, FogSettings}`；方位角
  `[0, 360)` 自 +Y 顺时针、高度角 `[0, 90]`、雾 `end > start ≥ 0`、颜色 sRGB 字节。
  拒绝标记 `display.invalid_value` / `display.invalid_row`。
- **存储**：`cad-db` 的 `TableKind::Display` + `tables/display.json` 单行记录；默认记录不写表；
  写入 = 一条 revision + 一个 table dirty mark；`apply_display_patches` 供撤销/重做整记录回放；
  解码严格（未知键/错误类型/两行/越界值拒绝）。
- **命令**：`SHADOWS [ON|OFF|SUN <azimuth> <elevation>]`、`FOG [ON|OFF|RANGE <start> <end>|COLOR
  <r> <g> <b>]` typed argument；裸选择器只报告；Work-only；一次写入一步撤销；无变化
  `display.unchanged`；Viewer 拒绝。
- **历史**：`cad-history` 新增 display lane（单整记录 patch，独占校验），`cad-app` 的
  `app_drawing/display_replay.rs` 走 `apply_display_patches`。
- **渲染**：共享 `Camera` 160 → 192 B（words 40..47 = `fog_color`/`fog_range`），拉取
  `PullCamera` 80 → 128 B（+`fog_color`/`fog_range`/`eye`）；`mesh`/`mesh_merged`/`mesh_textured`/
  `mesh_pbr`/`line`/`line_instanced`/`arc`/`mesh_pulled`/`line_pulled` 在最终颜色上按视距混合；
  `ViewSnapshot.sun/fog` 由宿主从文档记录解析；**投射阴影 NOT IMPLEMENTED**。
- **host read**：第 8 个 kind `display`（wire 行 `cad_display`），无通道载体答
  `unsupported-runtime: display`。
- **UI**：概念域托盘 Shadows/Fog 从 pending 变为真实面板（勾选框 + 数值 + 颜色，dispatch 上面的
  命令）；catalog 增加 `SHADOWS`/`FOG`；i18n 中英各 20 个键。
- **格式**：`FORMAT_MINOR` 21 → 22；无显示记录文档保持原档位；21 及以下照常打开；超过 22 拒绝。

## 2. 命令与拒绝

| 命令 | 语义 | 拒绝 |
|---|---|---|
| `SHADOWS` | 报告当前太阳与开关（不写） | Viewer |
| `SHADOWS ON\|OFF` | 开关太阳（方向不变） | Viewer |
| `SHADOWS SUN <az> <elev>` | 写方位角/高度角 | `display.invalid_value`、Viewer |
| `FOG` | 报告当前雾（不写） | Viewer |
| `FOG ON\|OFF` | 开关雾 | Viewer |
| `FOG RANGE <start> <end>` | 写区间 | `display.invalid_value`（`end ≤ start` 等）、Viewer |
| `FOG COLOR <r> <g> <b>` | 写 sRGB 字节 | 语法（非 `u8`） |

## 3. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-domain --lib --locked` | PASS（**290 passed**，含 4 个 display 单测） |
| `cargo test -p cad-db --lib --locked` | PASS（**521 passed**，含 `tests::display` 5 个） |
| `cargo test -p cad-store-zip --test display_minor --locked` | PASS（**4 passed**：minor 22、无记录普通档、minor 21 仍开、超档拒绝） |
| `cargo test -p cad-history --locked` | PASS（**43 passed**） |
| `cargo test -p cad-app --lib --locked` | PASS（**3489 passed**，含 `tests::display` 6 个：写入/撤销、太阳、雾、无操作、越界、Viewer） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**958 passed**，含 `tests_display` 5 个） |
| `cargo test -p cad-host --lib --locked` | PASS（**488 passed**，含新增的 snapshot 太阳/雾契约） |
| `cargo test -p cad-engine --lib --locked` | PASS（**134 passed**） |
| `cargo test -p cad-render-wgpu --test wgsl_validation --locked` | PASS（**14 passed**，含 192 B `Camera` 断言与三个新增 shader 的 WGSL/GLSL 校验） |
| `cargo test -p cad-render-wgpu --lib --locked -- --test-threads=1` | PASS（**367 passed**，串行；并行跑整套 GPU 用例在本机驱动上 SIGSEGV，见下） |
| `cargo test -p cad-render-wgpu --test accelerated_tier --locked tier_basics::` | PASS（**16 passed**，覆盖 128 B `PullCamera` 的真实拉取管线） |
| `cargo test -p cad-render-wgpu --test render_effects --locked instanced_lines_match_legacy_pixels` | PASS（1 passed，通用 tier 像素等价） |
| `(cd apps/desktop-ui && npm test)` | PASS（tsc --noEmit + node --test：**2122 passed**） |
| `cargo clippy -p cad-domain -p cad-db -p cad-store-zip -p cad-history -p cad-app -p cad-ui-core -p cad-host -p cad-engine -p cad-render-wgpu --all-targets --locked -- -D warnings` | PASS（9 crates） |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-architecture.py` | PASS |
| `python3 scripts/check-file-size.py` | PASS（2954 files） |
| `python3 scripts/check-i18n.py` | PASS（3682 keys） |
| `python3 scripts/check-command-docs.py` | PASS（0 mismatches） |
| `python3 scripts/check-tauri-capabilities.py` | PASS |

**CPU 之外的边界**：本轮没有真实 GUI/GPU 像素证据；`cad-render-wgpu --lib` 的整套 GPU 用例在默认
并行度下段错误（本机驱动/多 device 问题），串行通过；`accelerated_tier` 覆盖了放大后的拉取 uniform
真实渲染。以上都不是性能或视觉验收。

## 4. NOT VERIFIED / 不在范围

- 真实 GUI/GPU/像素、Windows/macOS/Android/Web 运行时（仅编译/契约）。
- **投射阴影（shadow map）、软阴影、地面淡出、逐物体阴影**：未实现，面板与报告都写明。
- 太阳的地理位置/日期/时间模型、雾与天空/背景混合、距离百分比表达。
- 场景（`SCENE`）仍不保存显示设置；插件 API 不暴露太阳/雾。
