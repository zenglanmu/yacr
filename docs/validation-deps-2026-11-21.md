# 验证记录：IFC / DGN 依赖升级（2026-11-21）

范围：`ifc-lite-core` / `ifc-lite-geometry` / `ifc-lite-export` / `ifc-lite-processing`
由 `=28.2.0` 升到 **`=28.3.0`**；`cadkit-dgn` / `cadkit-core` 由 `=0.5.2` 升到
**`=0.5.4`**（`default-features = false` 保留）。`rvt` 仍是 `=0.25.0`，`vendor/rvt` 补丁不变
（只改 `src/ifc/gltf.rs` 的透明度选择），没有新增 `[patch.crates-io]` 键。

**这不是能力验收**：本轮只证明编译与既有契约测试通过。IFC 三角化输出可能随上游几何修正
变化，DGN 线型单位是真实破坏点；两者都没有用真实厂商样本或 golden 对照复跑。

环境：Linux x86_64，Rust 1.99.1（`rust-toolchain.toml`），全部命令 `--locked`。

**记录方式**：下表的计数由依赖升级 lane 在其工作树上执行后记录；本文档 lane 只做整理，
**没有重跑任何 cargo 命令**（构建门禁由另一进程持有）。因此计数是实现 lane 的报告，不是
文档 lane 的复现。

## 1. 改了什么

- workspace `Cargo.toml`：四个 `ifc-lite-*` 精确 pin 改为 `=28.3.0`；`cadkit-dgn` /
  `cadkit-core` 改为 `=0.5.4`。`ifc-lite-processing` 仍与 `ifc-lite-export` 一起 pin，
  防止 28.3.0 家族被 caret 依赖漂移。
- `crates/cad-export-ifc/Cargo.toml`：只更新“28.3.0 家族”注释，依赖不变。
- DGN 适配器（`crates/cad-import-dgn`）的**最小适配**：`map/tables.rs` 不再用
  `dgn.uor_per_master` / `dgn.rescaled_to_document_units` 二次缩放 `Linetype::pattern`，
  改为原样写入（0.5.4 已把 pattern 换算成 drawing units）；`map/mod.rs` 删除
  `document_units_per_uor`；`linetype_pattern_unscaled` 收窄为“内置纸面线型（style `1..=7`）
  + 文件无 master-unit 尺寸”；`lib.rs` / `map/report.rs` 的诊断消息同步为 drawing-units
  语义。
- IFC 导入适配器：**没有 API/逻辑适配改动**，只同步版本与上游源码行号注记；导出适配器依赖
  版本随 workspace 更新。
- `[patch.crates-io]` 未新增键；`vendor/rvt` 未改。

## 2. 上游语义变化（源码检视，不是运行证据）

### 2.1 `ifc-lite` 28.3.0

- 我们使用的 API 无破坏性变化：`build_entity_index`、`EntityDecoder`、
  `EntityScanner::next_entity`、`GeometryRouter::with_units`、
  `process_element_with_voids_parts`、`process_element_with_submeshes_textured`、
  `mesh::{Mesh,SubMesh}`、`ifc_lite_export::plan::skeleton` 签名不变。
- 行为变化：void 宿主改为在元素 root frame 内切割后再应用 RTC；CSG consolidate /
  CDT incircle / mesh orient / weld 有修正；新增 `output_policy`。
- **后果**：同一模型的三角化输出**可能**变化。现有契约测试（计数/包围盒/单位/自洽往返）
  通过**不能**写成几何 golden 验收。
- 默认 feature：`ifc-lite-export` 默认启用 `gltf-textures` + `cityjson`（新增传递依赖
  `i_overlay`，MIT OR Apache-2.0）；`ifc-lite-processing` 默认启用 `native-result-cache`
  （build.rs 在 unix 编 `libc`，运行时默认不启用缓存）。`ifc-lite-core` /
  `ifc-lite-geometry` 默认 feature 为空。

### 2.2 `cadkit` 0.5.4

1. **唯一真实破坏点**：`Linetype::pattern` 语义从“名义值/UOR”改为 **drawing units**；内置
   装饰线型 `1..=7` 由纸面 mm 经 master unit 换算。适配器不再二次除 UOR。有 master unit 的
   文件因此得到真实纸面尺寸：契约夹具 `DGN_DOT_DASH` 由 0.00045 m 变为正确的 0.0045 m
   （旧值是把纸面毫米当 UOR 再除 UOR 因子的结果）。
2. `import.dgn.linetype_pattern_unscaled` 触发条件收窄为“内置纸面线型 + 文件无 master-unit
   尺寸”；code 稳定，仍显式建模（退 continuous，不猜比例）。
3. V8 level：上游能从 240 字节 level-entry 布局解析颜色索引（写 `Layer.color`）与 raw
   weight/style index；**仍不解码 freeze/lock**。适配器**仍不写** `LayerAttributes`，
   `import.dgn.level_symbology_undecoded` 仍恒发、每个成功导入仍 `Partial`。
4. V8 property bit `0x0080` 现在使图元 `visible = false`（进 hidden 集），此前上游总报
   visible。
5. 名称/文本解码增强：`ff fe` BOM + UTF-16LE 的 linkage 名称（level/model/cell/tag set/
   单位 label）、V8 无 stored-length 的文本由 range 推导、2D tag 旋转改按 angle 读（兼容
   识别旧 quaternion）。
6. 复合链/形 header 提前关闭超声明计数的父链（上游 issue #27），新增上游聚合 warning
   `dgn.complex_count`；扩展色 >255 仍回退 `ByLayer`，新增上游聚合 warning
   `dgn.color_extended_unresolved`。两个 warning 都在 `cadkit_core::Document::warnings`；
   本适配器当前**不转发** `Document::warnings`，也不把它们变成 `import.dgn.*` 稳定码。

## 3. 已运行

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-import-ifc --locked` | **20 passed** |
| `cargo test -p cad-export-ifc --locked` | **13 passed** |
| `cargo test -p cad-import-dgn --locked` | **23 passed** |
| `cargo test -p cad-app --locked` | **3069 + 4 + 11 + 18 + 15 + 5 + 2 passed**（多个测试二进制） |
| `cargo check -p cad-import-ifc --all-targets --locked` | 通过 |
| `cargo check -p cad-export-ifc --all-targets --locked` | 通过 |
| `cargo check -p cad-import-dgn --all-targets --locked` | 通过 |
| `cargo clippy -p cad-import-ifc --all-targets --locked -- -D warnings` | 通过 |
| `cargo clippy -p cad-export-ifc --all-targets --locked -- -D warnings` | 通过 |
| `cargo clippy -p cad-import-dgn --all-targets --locked -- -D warnings` | 通过 |
| `cargo fmt --all -- --check` | 通过 |

`cad-app` 覆盖连接器派发/导入导出注册与报告路径；本轮没有新增 `cad-app` 专用契约，只确认
升级后既有用例仍通过。

## 4. NOT RUN / NOT VERIFIED

- **wasm32 编译**：`cargo check -p app-web --target wasm32-unknown-unknown --locked` 与
  workspace wasm lib 检查均 **NOT RUN**。DGN 的 `cadkit` 与 IFC 的 `ifc-lite-*` 仍声称
  纯 Rust，但本轮没有复核。
- **门禁脚本**：`scripts/check-architecture.py`、`check-tauri-capabilities.py`、
  `check-fixture-manifest.py`、`check-file-size.py`、`check-command-docs.py`、
  `check-i18n.py`、`check-workflows.py` 全部 **NOT RUN**。
- **全工作区门禁/测试**：`cargo check --workspace --all-targets`、全量 clippy、全量
  `cargo test --workspace --test-threads=1` **NOT RUN**。
- **真实平台构建**：Windows / macOS / Android 的真实构建、`--target` 类型检查与运行一律
  **NOT RUN / NOT VERIFIED**。
- **真实样本**：`YACR_TEST_DGN` / `YACR_TEST_IFC` 未设置，真实 DGN/IFC 文件的导入、
  三角化差异、单位/线型外观、vendor 接受性 **NOT VERIFIED**；没有 golden、像素、GPU 或
  性能结论。
- **DGN 视觉证据**：`docs/evidence/dgn-linux-gpu/` 的 2026-11-14 运行发生在 `cadkit-dgn`
  0.5.2 上；0.5.4 未重跑视觉证据。

## 5. 边界结论

- 升级通过只代表**编译 + 既有契约测试**通过，不代表 IFC 几何输出与旧版本逐三角形一致，
  也不代表 DGN 在真实 MicroStation 文件上的线型/level/隐藏元素行为已验收。
- ADR-0033 D6/D11、ADR-0038 §1 的历史决策正文不改；受影响小节末尾的
  `2026-11-21` 注记只记录版本事实与新的证据边界。
- 能力表（`docs/compatibility.md`）、`docs/dgn.md` 与 `docs/architecture.md` 已同步版本与
  行为边界；没有把上游 warning 写成适配器的稳定诊断。
