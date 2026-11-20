# ymodel 连接器与导出验证记录（2026-11-20）

范围：ADR-0037 D1/D2/D3/D11/D14 的**连接器注册表**、统一导出契约、STL/OBJ/PLY/OFF/IFC4/SKP
写出、STEP/DGN/RVT 显式拒绝，以及对象级报告落盘（`tables/compat.json`/`tables/sources.json`，
`FORMAT_MINOR = 2`）。环境：Linux x86_64，Rust 1.99.1（`rust-toolchain.toml`），`--locked`；
网络不稳定时对本地已缓存 crate 使用 `--offline`。**本文只记录实际执行过的命令**；未执行项在
§4 逐条标注，不做产品验收结论。

## 1. 实现摘要

- `cad-import-core`：`ConnectorDescriptor`/`Connector`、`ExportJob`/`ExportOutput`/
  `CompatReport`/`Exporter`；`ImportReport::compat` 与 `completeness_key`。
- `cad-import-acadrust`：`AcadrustExporter` 适配既有 DXF/DWG 写出器；`ExportDiagnostic` 增加
  `ExportOutcome { Converted, Dropped, Note }`，报告状态不再从 code 字符串猜测。
- `cad-app::ConnectorRegistry`：12 个格式的有序 probe 与导出 `format_id` 派发；`FormatImporter`
  是薄封装。DXF/DWG/STL/OBJ/PLY/OFF/IFC/SKP/glTF 已接线，STEP/DGN/RVT 为
  `export.<id>.not_implemented`。
- `cad-export-mesh`：STL（binary/ascii）、OBJ、PLY（binary LE）、OFF/COFF；确定性输出、模型空间
  过滤、逐对象 drop、实体/三角预算。
- `cad-export-ifc`：`ifc-lite-export` 28.2.0 写 IFC4 `IfcBuildingElementProxy` +
  `IfcTriangulatedFaceSet`；源单位换算为米。
- `cad-export-skp`：`skpwright` 0.6.0 写 2026 容器；源单位换算为米。
- `cad-import-gltf` / `cad-export-gltf`：glTF 2.0 读写（`.gltf` JSON + GLB 容器），见 §7。
- 宿主：`ExportSettings`/`export_connector`；Save As 按扩展名派发并写入连接器字节；
  `record_export_report` 把报告追加到 `Document.compat`（导出记录最多 32 条，导入记录不淘汰）。
- 持久化：`TableKind::{Compat,Sources}`、`cad-store-zip` 接受两张表、`FORMAT_MINOR = 2`。

## 2. 已运行命令与结果

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-import-core --locked` | **exit 0**（27 passed） |
| `cargo test -p cad-import-acadrust --locked` | **exit 0**（531 lib + 集成，0 failed） |
| `cargo test -p cad-app --locked` | **exit 0**（3063 lib + 集成，0 failed） |
| `cargo test -p cad-host --locked` | **exit 0**（404 passed） |
| `cargo test -p cad-store-zip --locked` | **exit 0**（30 + 18 + 7 + 6） |
| `cargo test -p cad-db --locked` | **exit 0**（326 + 13 + 5） |
| `cargo test -p cad-export-mesh --locked` | **exit 0**（21 passed） |
| `cargo test -p cad-export-ifc --locked` | **exit 0**（11 passed） |
| `cargo test -p cad-export-skp --locked` | **exit 0**（19 passed） |
| `cargo test -p cad-import-gltf --locked` | **exit 0**（15 passed） |
| `cargo test -p cad-export-gltf --locked` | **exit 0**（22 passed） |
| `cargo test -p app-linux --locked` | **exit 0**（207 + 1 集成） |
| `cargo check --workspace --lib --target wasm32-unknown-unknown --locked` | **exit 0**（仅既有 `cad-engine`/`app-web` 警告） |
| `cargo check -p app-linux --all-targets --locked` | **exit 0** |
| `cargo fmt --all -- --check` | **exit 0** |
| `cargo clippy -p cad-import-core -p cad-import-acadrust -p cad-app -p cad-host -p cad-export-mesh -p cad-export-ifc -p cad-export-skp -p cad-store-zip -p cad-db -p app-linux --all-targets --locked -- -D warnings` | **exit 0** |
| `python3 scripts/check-architecture.py` | **exit 0**（42 packages） |
| `python3 scripts/check-i18n.py` | **exit 0**（2992 keys） |
| `python3 scripts/check-file-size.py` | **exit 0**（2388 files，≤1000） |
| `python3 scripts/check-tauri-capabilities.py` | **exit 0** |
| `(cd apps/desktop-ui && npm test)` | **1825 pass / 0 fail** |
| `cargo check --workspace --all-targets --offline --locked` | **exit 0** |
| `cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --offline --locked -- -D warnings` | **exit 0** |
| `cargo check -p app-web --target wasm32-unknown-unknown --offline --locked` | **exit 0** |
| `cargo check -p app-windows -p app-macos --all-targets --offline --locked` | **exit 0**（仅编译；运行 NOT VERIFIED） |
| `CARGO_INCREMENTAL=0 cargo test --workspace --offline --locked -- --test-threads=1` | **exit 0**；8095 passed / 0 failed（含 glTF，见 §2.1） |

### 2.1 全量回归

见文末「全量回归结果」一节；未运行前不得据此宣称全绿。

## 3. 已由测试证明（库层 + 宿主契约）

- 注册表：11 个描述符唯一、probe 顺序与旧 `match` 等价、未知字节为 `UNSUPPORTED_INPUT`、
  每个导出 lane 的 `descriptor.export.is_some()`、每个 refusal 的 `export.is_none()`、
  `stl-ascii` 可解析。
- 网格导出：STL binary/ASCII、OBJ、PLY、OFF 与对应 importer 的**自洽往返**；两网格 PLY 索引
  基址正确；块/图纸空间实体逐条 `space_not_exported`；空文档 `no_geometry`；三角/实体预算；
  确定性字节。
- IFC/SKP：写出→各自 importer 读回同一三角数/顶点数；IFC 米坐标、SKP 1 mm→0.001 m；单位未知
  按名拒绝；颜色/法向/图层损失落报告并降为 `Partial`；固定时间/种子下字节相同。
- 宿主：DXF/DWG 经注册表写出且 ACIS 状态键保持；`.stl`/`.ifc` 等扩展名写出并追加 compat 记录；
  源文件覆盖保护；`record_export_report` 保留导入记录、淘汰最旧导出记录。
- 持久化：`tables/compat.json`/`tables/sources.json` 在保存/重开后保留；导入记录
  `counts: null`、能力行结构化、`Temporary` 身份不伪造 checksum；`FORMAT_MINOR=2`，
  minor >2 拒绝，minor 1 仍可打开且两张表缺省为空。

## 4. NOT RUN / 未实现（不得据此宣称完成）

- **厂商接受性**：DWG（AutoCAD/ODA）、IFC（IfcOpenShell/buildingSMART）、SKP（SketchUp/Trimble）、
  glTF（three.js/Blender）都没有打开过本仓库写出的文件，glTF-Validator 也未运行；只有自洽往返。
- **DXF 导入/导出 L1 不动点**、**L0 源字节直通**（`raw/source-*.bin`）未实现/未运行。
- **报告弹窗 UI** 未实现；`tables/compat.json` 只能直接读取，尚无 UI 点击入口。
- **载体**：Windows/macOS/Android/Web 的 Save As 选择器未扩展；Web 下载流程对连接器格式为
  NOT IMPLEMENTED；这些载体的运行/真机/像素一律 **NOT VERIFIED**。
- **性能门槛**：ADR-0037 D17 的 100k 元素保存、200 MB raw-copy、峰值磁盘/内存未测。
- **STEP 写出**：cadmpeg 有 AP242 写出器，但本轮未接线；仍为显式拒绝。
- **DGN 写出**：上游只有 seed-based V8 写出器，未接线。
- **真实 GPU/像素/跨平台**：无。
- 全量 `cargo test --workspace` 的逐项结果以 §2.1 为准。

## 5. 依赖与许可

- `ifc-lite-export` / `ifc-lite-processing` `=28.2.0`（MPL-2.0，与既有 `ifc-lite-core/geometry`
  同一版本列车；精确固定，未修改源码）。
- `skpwright` `=0.6.0`（GPL-3.0-only，workspace 既有依赖；AGPLv3 §13 组合，见
  `THIRD_PARTY_NOTICES.md`）。
- `cadmpeg-*` 0.6.12（Apache-2.0，既有依赖；本轮未新增）。
- `Cargo.lock` 中 `dlib` 因新增依赖重新解析为 `libloading 0.8.9`（`dlib` 声明
  `>=0.7,<0.9`）；构建通过，运行时未验证。

## 6. 跟进项

1. 用真实厂商文件验证 DWG/IFC/SKP 写出（当前只有自洽往返）。
2. DXF L1 不动点与 L0 源字节直通。
3. 报告弹窗/可点击条目与 wire/JSON 导出。
4. Windows/macOS/Android/Web 的 Save As 扩展名选择器。
5. STEP（AP242）与 DGN（seed-based V8）写出评估。
6. ADR-0037 D17 的性能门槛测量。

## 7. glTF/GLB（2026-11-20 追加）

- **新 crate**：`cad-import-gltf`（glTF 2.0 / GLB 导入，`gltf` 1.4.1 文档校验 + 手写场景遍历；
  外部 buffer 只认 `data:` base64 或 GLB 自带 BIN chunk，外部 `.bin` 按名拒绝
  `import.gltf.external_buffer_unavailable`，纹理不采样并记 `import.gltf.texture_not_imported`）与
  `cad-export-gltf`（`GltfContainer::{Glb, Gltf}`，同一 descriptor 两个 registry key `glb`/`gltf`；
  JSON `.gltf` 的 buffer 内嵌为 base64 data URI）。
- **坐标系**：glTF 是米 / +Y up；导入先烘焙节点世界变换再 `(x,y,z) → (x,−z,y)` 落到 CAD 的米 /
  +Z up（`import.gltf.units_metres`、`import.gltf.frame_converted`）；导出反向执行同一次换算，并要求
  已知源单位，`DrawingUnits`/缺单位按名拒绝 `export.gltf.units_unknown`。
- **颜色与材质**：glTF `COLOR_0` 是**线性**值，`baseColorFactor` 也是线性并按规范与 `COLOR_0`
  相乘。导入把 `COLOR_0 × factor` 一次乘入线性颜色再转 sRGB；无材质索引时保持 `ByLayer`（不发明
  白色）；`alphaMode` 为 `OPAQUE` 时忽略 factor alpha，`BLEND` 且 alpha<1 存显式不透明度，
  `MASK` 记 `import.gltf.alpha_mask_not_imported` + `Partial`。导出把显式实体颜色/不透明度写成
  去重的线性 `materials[]`（alpha<1 才写 `BLEND`），`ByLayer` 颜色/透明度记
  `export.gltf.layer_appearance_not_written`，顶点色覆盖显式实体色记
  `export.gltf.entity_color_overridden`，顶点色 + 显式不透明度<1 记
  `export.gltf.vertex_color_opacity_not_written` 并降为 `Partial`。
- **证据**：仅 crate 内**合成夹具的自洽往返**——`cargo test -p cad-import-gltf`（15 条）与
  `cargo test -p cad-export-gltf`（22 条），写出→读回同一三角数/顶点数、米坐标、线性颜色
  （`[10,128,255]` sRGB 定点 ±1）、显式颜色/不透明度材质；字节确定性由固定实体顺序保证。
- **NOT RUN**：本轮**未重跑**上述两个 crate 的测试（见 §2 列表，只有文件关联门禁与文档相关门禁
  在本轮执行）；**three.js / Blender / glTF-Validator / 任何厂商查看器都未打开本仓库写出的文件**；
  外部 `.bin` 与纹理路径无真实样本；多场景/蒙皮/动画/相机只到"按名报告"级契约，未做真实资产回归；
  宿主 Save As 的 `.gltf`/`.glb` 运行证据与任何载体的运行/像素结论均 **NOT VERIFIED**。

## 全量回归结果

- 第一次 `cargo test --workspace --offline --locked -- --test-threads=1` 在**链接阶段**因
  `/home` 磁盘耗尽（`No space left on device`，`target/debug` 152 GB、incremental 46 GB）失败；
  **不是测试失败**。清理 `target/debug/incremental`（释放 46 GB）后以
  `CARGO_INCREMENTAL=0` 重跑。
- 重跑（连接器阶段）：`CARGO_INCREMENTAL=0 cargo test --workspace --offline --locked --
  --test-threads=1` → **exit 0**，**8051 passed / 0 failed**。
- glTF 补齐后最终重跑同一命令 → **exit 0**，**8095 passed / 0 failed**（完整日志
  `/tmp/opencode/full-test-gltf.log`；`grep -c "test result: FAILED"` = 0）。
- `cargo check --workspace --all-targets --offline --locked` → 见 §2 网关行。
- 全量 clippy（排除 `app-android`/`app-web`）→ 见 §2 网关行。
- `cargo check -p app-web --target wasm32-unknown-unknown`、`-p app-windows -p app-macos
  --all-targets` → 见 §2 网关行；Windows/macOS 仅编译，运行 **NOT VERIFIED**。
- `app-android` 的 `aarch64-linux-android` 目标检查未运行（缺 NDK/Android SDK），**NOT RUN**。
