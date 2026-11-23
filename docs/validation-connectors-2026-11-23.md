# 连接器 crate 合并验证（2026-11-23）

目标：ADR-0048 的 crate 合并（每格式一个 `cad-connector-*`，`cad-app` 只有一张
`FormatConnector` 表）。代码改动在 `a0852cf1..cde67ffe`（10 个 `refactor(connectors): …` 提交），
本记录是**该工作树**上的编译/契约证据，以及文档轮（crate 名/路径/命令更新）实际跑过的门禁。

> 本文件只记录**编译/契约测试与构建产物**。没有任何运行时、真实 GPU、像素、性能、真机或 vendor
> 格式验收证据；`cargo test` 通过或 bundle 构建成功都不等于格式兼容性验收。
>
> 验证时工作树里还存在**未提交**的源码注释/路径改名与 `Cargo.toml` 注释、`use` 排序（不是本次
> 文档轮产生的改动）；这些改动不改变语义，也未包含在本记录的任何主张里。

## 跑过的命令与结果

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app connector --locked` | **28 passed**，0 failed（其余 target 按名称过滤，0 匹配） |
| `cargo test -p cad-app connector --no-default-features --locked` | **14 passed**，0 failed（slim 注册表） |
| `cargo test -p app-web --locked` | **148 passed**，0 failed |
| `cargo test -p cad-connector-ifc --locked` | **76 passed**，0 failed |
| `cargo test -p cad-connector-gltf --locked` | **74 passed**，0 failed |
| `cargo test -p cad-connector-skp --locked` | **44 passed**，0 failed |
| `cargo test -p cad-connector-mesh --locked` | **84 passed**，0 failed |
| `cargo check --workspace --all-targets --locked` | 通过（`Finished dev profile`） |
| `cargo clippy -p cad-connector-core -p cad-connector-acadrust -p cad-connector-dgn -p cad-connector-step -p cad-connector-rvt -p cad-connector-ifc -p cad-connector-gltf -p cad-connector-skp -p cad-connector-mesh --all-targets --locked -- -D warnings` | 通过（40.00 s，无 warning 升级为 error） |
| `cargo check -p app-web --target wasm32-unknown-unknown --locked` | 通过；`cad-render-wgpu` 在 wasm 目标有 1 条**既有** `dead_code` warning（`pick_arc_pipeline`/`PICK_REASON_*`），与本轮 crate 合并无关，也不在 `-D warnings` 范围内 |
| `python3 scripts/check-architecture.py` | `Architecture OK: 38 packages, acyclic and core boundaries intact` |
| `python3 scripts/check-file-size.py` | `file-size OK: 2633 code file(s), limit 1000 lines` |
| `python3 scripts/check-i18n.py` | `i18n OK: 3129 keys, 2 catalogs`（文档改动不影响键集） |
| `python3 scripts/check-command-docs.py` | `OK: 0 mismatches` |
| `cargo tree -p cad-connector-mesh --edges normal --locked` | 抽查：第三方写出/读入依赖仍是 `tobj 4.0.5`、`stl_io 0.11.0`，其余为经 `cad-db` 的共享传递依赖 |
| `cargo fmt --all -- --check` | 通过（集成轮运行；crate 改名导致的 `use` 排序已由 `cargo fmt --all` 归一） |
| `WITH_FONTS=0 bash scripts/build-web-ui.sh` | 通过；`apps/app-web/dist-ui/canvas/app_web_bg.wasm` = 11,681,407 B（≈11.1 MiB，低于 25 MiB 发布上限），slim 的 `cargo tree` 泄漏守卫通过。**只是构建产物证据**，不是浏览器运行/GPU/像素验收 |

`cad-app connector` 两套测试固定的是行为不变量：扁平 probe 顺序（DGN V8 在 RVT 之前）、14 个导出键
（`dxf/dwg/stl/stl-ascii/obj/ply/off/ifc/skp/glb/gltf/step/dgn/rvt`）、`export.<id>.not_implemented`
→ `unsupported-runtime: export.<key>` → `export.unknown_format` 的解析顺序，以及
`import.is_some()` ⇔ descriptor 声明导入能力、`exports` 非空 ⇔ 声明导出能力。

## `cargo tree` 第三方并集

重构提交在合并 `cad-connector-mesh` 时用 `cargo tree` 对比过第三方并集（`cad-import-mesh` +
`cad-import-obj` + `cad-import-stl` + `cad-export-mesh` 的并集与合并后的
`cad-connector-mesh` 一致，`tobj`/`stl_io` 是唯一格式专属依赖）。本轮**没有重跑"旧四棵树"对比**：
旧 crate 已在本提交范围内删除，工作树里无法再生成那四棵树；上表只做了合并后树的抽查。
`formats-*` feature 树与 slim 泄漏守卫（`scripts/lib/wasm-toolchain.sh`）本轮未重跑。

## 文档轮验证

- `grep -rn "cad-import-\|cad-export-\|cad_import_\|cad_export_" AGENTS.md docs CAD_IMPLEMENTATION_SPEC.md
  | grep -vE 'docs/(handoff-archive|validation)'`：living docs 无残留；剩余命中只有
  ①历史 ADR（0004/0005/0006/0010/0015/0016/0017/0018/0019/0021/0025/0031/0033/0035/0036/0037/0038/0039/0041/0042）、
  ②`docs/batch-annot-2026-11-07.md`（带日期的历史批次记录）、
  ③`docs/evidence/dgn-linux-gpu/README.md`（GPU 证据记录）、
  ④新增 ADR-0048 中**故意保留**的旧 crate 名（描述"删除了什么"）。
  这些都按"不改写历史证据"保留。
- `python3 scripts/check-i18n.py` 与 `python3 scripts/check-command-docs.py` 见上表。

## NOT RUN

- 全量 `cargo test --workspace --locked -- --test-threads=1`（只跑了连接器相关的 crate + `app-web`，
  以及 workspace 的 `--all-targets` **check**）。
- 真实 GPU / 像素 / 性能测试；`linux-host-smoke`；任何宿主启动后的打开/保存/导出行为复看。
- `bash scripts/build-web-ui.sh` / `bash scripts/build-web.sh` 的 slim bundle **已在集成轮运行**
  （`WITH_FONTS=0`，见上表）；`build-web.sh`（standalone harness）与浏览器 smoke **未运行**。
  上次体积证据仍以 `docs/validation-web-slim-2026-11-22.md` 为准。
- 浏览器 smoke、Android/Windows/macOS 载体运行与 Tauri 打包。
- vendor 接受性（AutoCAD/SketchUp/Revit/IFC 工具链打开导出文件）与任何真实样本复跑。
- `cargo fmt --all -- --check`：集成轮已运行并通过（见上表）；结果只覆盖格式与排序，不是行为证据。

## 边界

- crate 改名/合并是仓库内部重构，未发布到 crates.io；`cad_app::DESCRIPTOR_*` 与
  `ConnectorRegistry` 的公开路径保持可用。
- 行为不变量由契约测试固定：probe 顺序、导出键集合、拒绝码、按载体编译的格式集（原生与
  X11 `dom-canvas` 全格式，部署的浏览器 bundle 只有 DWG/DXF）。这些是**代码契约**结论，
  不是格式兼容性、运行时或跨平台验收。
