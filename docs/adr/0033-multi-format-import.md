# ADR 0033：多格式导入（OBJ/STL/PLY/OFF/STEP/IFC/RVT）与网格语义边界

- 状态：**Accepted（2026-11-10）**。验证边界：库能力结论来自 Linux 主机的**实际编译与样本运行**
  （见各决策中的数字）；本仓库内已实现的适配只以合成 fixture 与公开样本做过契约测试，
  无真机/真实 GPU/像素/性能结论。Windows/macOS/Android/Web 一律 **NOT VERIFIED**。
- 日期：2026-11-10。
- 相关：[ADR-0024](0024-dom-canvas-wasm-cad-path.md)（wasm 载体）、[ADR-0010](0010-react-linux-host.md)
  （宿主分层）、`docs/kernel-acis.md`（现有 B-rep/离散 seam）、`docs/compatibility.md`（能力表）、
  `scripts/check-architecture.py`（依赖门禁）、`fixtures/manifest`（样本台账）。
- 影响：新增 `cad-import-core`/`cad-import-obj`/`cad-import-stl`/`cad-import-mesh`/
  `cad-import-step`/`cad-import-ifc`/`cad-import-rvt`（后者只做 **preview**），
  `cad-app::import::FormatImporter` 派发器，三处打开入口。**不**改变：`acadrust` 只能被 `cad-import-acadrust` 依赖；`cad-db` 是唯一文档
  权威；未支持能力必须显式建模；`native-below`/`dom-canvas` 的文档所有权不变。

---

## 1. 问题

1. 仓库此前只有 DWG/DXF 一个导入路径，格式判定与派发**硬编码在解析器内部**
   （`cad-import-acadrust` 的字节嗅探 + `Importer` trait 同处一 crate）。用户要求加入 OBJ、
   STEP、STL，并追问 IFC、RVT 与“FreeCAD 式”的多格式能力。
2. 直接后果是依赖方向错误：任何新解析器若只为一个 trait 依赖 `cad-import-acadrust`，就会把
   DWG 解析器（及其 `acadrust` 依赖）拖进 OBJ/STL 的构建与 wasm 产物；反过来把新解析器塞进
   该 crate，则 crate 名与实际职责不符，且 `acadrust` 单一依赖边界失去意义。
3. 这些格式与 DWG 的语义完全不同：OBJ/STL/PLY/OFF 是**显示网格**，STEP/IFC 是**精确 B-rep**
   需要离散，RVT 是**厂商私有**且只能近似还原。它们都不进入 `cad-db` 的二维实体模型，
   只能落在 `SemanticGeometry::Mesh`，而现有能力表把 `Mesh` 的 measure 明确记为
   `Unsupported`（`crates/cad-import-acadrust/src/entity/push.rs`）。把“能看”说成“能测”就是
   铁律 2 禁止的假成功。
4. 候选库的实际可用性与流行度严重不符，且多数 README 与真实能力有差距：
   - `ruststep` 0.4.0：README 首行 "DO NOT USE FOR PRODUCT"；只有 AP201/AP203 生成 schema，
     无 AP214/AP242；无几何求值、无离散、无写出；对合法的空参数表 `()` 直接失败，
     因而在 NIST AP242 与 Plasticity/HOOPS AP203 文件上失败。
   - `truck-stepio` 0.3.0：crate 文档自述输入侧 "will come further down the road"；
     `Table::from_step` 在 NIST AP242 文件上返回 `None`。
   - `solidcore` 1.0.1（SMLib 的 Rust 移植，Apache-2.0）：**没有** ISO 10303 代码、没有任何
     文件 I/O，且 README 明示 tessellation 等入口 "not ported yet"；`query::poly::DisplayList`
     只建模 display list id。既不能读 STEP，也不能产出三角形。
5. 载体约束是硬条件：`dom-canvas`（X11/XWayland、Web）与 Android 的 wasm/wasm 旁路要求解析器
   能编译到 `wasm32-unknown-unknown`；不能编译的解析器必须按载体**显式拒绝**，不得只在
   native 静默可用。

## 2. 决策

### D1 契约上移，解析器按格式分 crate

- 新建 `cad-import-core`：`ImportRequest`/`ImportLimits`/`ImportReport`/`ImportedDrawing`/
  `Importer` trait/进度类型/`compute_identity`/`XrefOptions`，以及格式嗅探 `DrawingFormat`
  与共享的 `MeshImport`（把网格落成 `DbEntity`、生成能力行与诊断、并在插入前调用
  `cad_db::validate_geometry`）。
- `cad-import-acadrust` 只做 `pub use` 再导出，保持既有调用点与 `acadrust` 单一依赖不变。
- 每个格式一个适配 crate；`cad-app::import::FormatImporter` 是唯一派发点，按**字节内容**
  嗅探，不按扩展名。扩展名只用于各载体文件选择器的默认过滤，不构成支持承诺。

### D2 非 DWG 格式的语义边界：只产网格，只承诺显示/拾取

- 一律映射到 `SemanticGeometry::Mesh`；能力表记 display `Verified`、pick `Verified`、
  measure `Unsupported`。
- 单位未知时用 `UnitContext::drawing_units()`，**不得**假设毫米。STEP/IFC 若解析库提供单位则
  显式映射；RVT 沿用其来源语义。
- 不保留精确 B-rep：`cad-kernel-adapter` 的中性 `BrepData` 目前只有
  `Plane|Sphere|Torus|Cylinder` 与 `Line|Circle`，扩展它（NURBS 曲面/曲线 + 离散器）是独立
  工程，不在本 ADR 范围。因此“导入 STEP 后能精确测量”当前**不成立**。

### D3 OBJ：`tobj` 4.0.5，`default-features = false`

- 默认特性经 `ahash → getrandom 0.3` 在 `wasm32-unknown-unknown` 上编译失败（实测），关掉默认
  特性后可用（改用 `std` HashMap）。
- `triangulate: true` 是必需项；随之 `face_arities` 为空，因此 n-gon 只能靠扫描原始 `f` 记录
  判定，并记 `import.obj.ngon_fan_triangulated` + `Partial`。**普通四边形也走 fan**，凹多边形
  可能三角化错误，必须在能力表写明。
- 材质：导入入口只有字节流，没有目录句柄，因此外部 `.mtl` **不加载**；记
  `import.obj.mtl_not_loaded`，颜色回退 `ByLayer`。要真正支持材质需要给导入契约增加资源来源
  通道（与字体/xref 同级的契约变更），不在本 ADR 内。

### D4 STL：`stl_io` 0.11

ASCII 与二进制同一路径；缺失/零/非有限法向由三角形几何重算（`import.stl.normal_recomputed`），
退化面丢弃并记诊断；无颜色、无分组、无单位。

### D4b PLY / OFF：仓库内手写解析器，不新增依赖

- PLY（ASCII、`binary_little_endian`、`binary_big_endian` 1.0）与 OFF/COFF（仅 ASCII）由
  `cad-import-mesh` 的 `PlyImporter`/`OffImporter` 解析。**不新增第三方依赖**：候选 crate
  （`ply-rs` 0.1.3，2020 年）过旧，或绑定本仓库不需要的格式族；两种规范已冻结且体量小，
  严格性/截断诊断比便利更重要，因此解析器留在仓库内（纯 `std`，用
  `f32::from_le_bytes`/`from_be_bytes` 一类转换，无需 `byteorder`）。
- 两者都只产**一个**中性网格实体；单位未知（`UnitContext::drawing_units()`）；不保留面/边
  属性、材质或纹理；measure 仍为 `Unsupported`。
- PLY 属性按**名称**从任意声明顺序读取，未知属性/元素按声明宽度跳过；OFF 只接受
  `OFF`/`COFF`，`NOFF`/`CNOFF`/`STOFF`/`4OFF` 等按名拒绝 `import.off.variant_refused`。
- n-gon 一律扇三角化，记 `import.ply.ngon_fan_triangulated` /
  `import.off.ngon_fan_triangulated` + `Partial`（凹多边形可能不正确）；n<3、退化三角形、
  被丢弃的颜色各自有稳定诊断码。
- 截断载荷永远是 `CorruptData`（`import.ply.truncated_payload` /
  `import.off.truncated_payload`），不是静默丢弃尾部的部分成功。

### D5 STEP AP203/214/242：`cadmpeg-codec-step` + `cadmpeg-ir` + `cadmpeg-tessellate` 0.6.11

- 实测（同一台机器）：AP214 立方体 6 面 → 12 三角形；NIST CTC-05 AP242（878 KB，13,959 实体）
  156 面 → **155 面成功、1 面显式拒绝**，334,879 三角形；ABC 数据集 AP214 文件 25 面 → 22 面。
  拒绝原因均为同一类几何限制（`"a wrapping loop bounds a strip with no far side"`）。
- `wasm32-unknown-unknown` 的 `check`/`build` 均通过（含 codegen）。
- 拒绝理由见 §1.4：`ruststep`/`truck-stepio`/`solidcore` 均不能作为 STEP 查看路径。
- 多体装配：逐面网格处于**局部坐标系**。必须应用 occurrence/placement，否则多体会叠在原点却
  看起来“成功”。实现必须二选一：应用变换并有测试断言包围盒互不重叠，或在无法诚实定位时
  显式拒绝多体文件。**禁止**输出叠在原点的结果。
- 每面拒绝与解码 loss 均转成稳定码诊断 + `Completeness::Partial`；全部面都失败必须是错误，
  不是空成功。

### D6 IFC 2X3/4/4X3：`ifc-lite-core` + `ifc-lite-geometry` 28.2.0（MPL-2.0）

- 实测：buildingSMART Certification 语料 9 个真实文件（IFC2X3/IFC4/IFC4X3，127–6,393 实体）
  全部解析成功，几何在 1–33 ms 内产出，无硬失败；覆盖 tessellated face set、faceted B-rep、
  拉伸体、mapped item、swept disk、布尔挖洞（墙体 24 → 44 三角形）。
- 重要反例：`cadmpeg` **不能**当 IFC 解析器——它能 tokenize IFC，但产出 0 bodies / 0 faces；
  `ruststep`/`step-p21` 只是 Part-21 词法器，没有 IFC schema 与几何。
- CSG/布尔保真度、NURBS/`IfcAdvancedBrep` 未在真实样本上验证；按 best-effort 处理，
  拒绝必须显式。能力表不得宣称与 IfcOpenShell 等价。

### D7 RVT/RFA：`rvt` 0.17.0，经内部 glTF 落地，定位为 **preview**

**已实现**（2026-11-10）为 `crates/cad-import-rvt`。本节记录的是代码的真实 API、单位结论与
状态映射；若与代码不一致，以代码与契约测试为准。

- **库与许可**：`rvt` **0.17.0**（Apache-2.0，MSRV 1.87，纯 Rust，上游声明 wasm 已验证）在
  workspace 中**精确固定**（`=0.17.0`），只由 `cad-import-rvt` 依赖；未修改上游源码、无
  Cargo patch。读 GLB 用 `gltf` **1.4.1**（MIT OR Apache-2.0，
  `default-features = false, features = ["utils", "names"]`，不引入 `image`）。
- **真实 API**（读自 `rvt` 0.17.0 源码，不是猜测）：
  - 解码入口 `RevitFile::open_bytes_with_limits`（`src/reader.rs:468`，要求 OLE/CFB 魔数）；
  - 导出 `ifc::run_export(&mut RevitFile, &ExportOptions::default())`
    （`src/ifc/export_pipeline.rs:535`）→ `ExportRun { model, diagnostics, unread, notes }`；
  - GLB `ifc::gltf::model_to_glb(&IfcModel) -> Vec<u8>`（`src/ifc/gltf.rs:814`）。
- **几何上限**：Revit **2008–2023** 的构件体是**包围盒**（在可读的连接处切分），2024/2025 为
  `partial`，2026/2027 为 `experimental`；**`.rfa`/`.rft` 只有元数据、预览图与类型，没有家族
  几何**。`rvt` README 明确警告不要把输出用于测量/协调。
- **状态上限（不是“已支持”）**：`ExportDiagnostics.input.release_support` 来自
  `capability::release_support`（内嵌 `docs/support-matrix.json`）。映射为
  `verified → SupportStatus::Verified`、`partial → Partial`、`experimental`/`unknown →
  Unverified`、其它（含 `unsupported`）→ `Unsupported`。该值**同时限制** read/semantic/
  render/pick 四行；measure **恒为 `Unsupported`**。release 状态为 `unsupported` 时直接拒绝
  `import.rvt.release_unsupported`；`confidence.has_geometry = false` 或 GLB 没有可绘制节点时
  拒绝 `import.rvt.no_geometry`（不是空成功）。每个成功导入仍恒为
  `Completeness::Partial` + `import.rvt.preview_geometry`，因为包围盒预览不可能无损。
  `confidence`（level/score/has_geometry）、`source_coverage`（status/fractions，无可信分母时保持 unset/null）、`rvt` 的 `skipped`/`unread`/`unsupported_features`/
  `warnings`/`notes` 都落成稳定 `import.rvt.*` 诊断。
- **家族拒绝**：导入契约只有字节，没有扩展名。按字节判定：`PartAtom` 的 `<A:family>` 类型表
  （可解析出类型，或原始 XML 出现 `<A:family>`）⇒ 家族/家族模板，拒绝
  `import.rvt.family_no_geometry`；否则 `ProjectInformation` 流（该流仅项目文件有，`rvt`
  `src/project_information.rs:1-3`）⇒ 项目；两者都读不到时按项目解码，没有几何再由
  `import.rvt.no_geometry` 拒绝，不猜。
- **单位（已确定，不是 `units_unknown`）**：`rvt` 的构件节点用**英尺**，GLB 有一个根节点矩阵
  `Z_UP_FEET_TO_Y_UP_METRES`（`src/ifc/gltf.rs:242`）做 `0.3048` 缩放与 `(x,y,z)→(x,z,-y)`
  轴交换。因此 GLB 本身是 glTF 规范的**米、+Y up**；适配器应用世界变换后再把 +Y up 映射回
  CAD 的 +Z up，入库坐标是**米、+Z up**，`UnitContext::from_meter()`，记
  `import.rvt.units_metres`。不假设 Revit 显示单位或毫米。
- **节点变换**：`gltf::scene::Transform::matrix()` 返回列主序矩阵；逐节点
  `world = parent_world * local`，场景根用单位矩阵。合成双节点 GLB 夹具（`group` 平移
  +0.5 再带局部 +0.5 的子节点；另一节点用矩阵平移 -1）断言世界包围盒 x∈[-1.5,-0.5] 与
  [0.5,1.5] 不重叠——忽略节点变换（全堆原点）或忽略父子复合（-1 与 +0.5）都会失败。法向用
  逆转置矩阵并归一化。
- **glTF 是实现细节**：不加入检测（`detect_format` 只认 `.rvt`/`.rte` 的 OLE 魔数）、不在
  文件选择器出现、不是用户格式、没有能力行；`D9` 的“拒绝 glTF”仍成立。
- **颜色与透明度（2026-11-10，只做颜色，不做纹理）**：只读
  `pbrMetallicRoughness.baseColorFactor`（glTF **线性** RGBA）。RGB 按 IEC 61966-2-1 传函
  （`c ≤ 0.0031308` 线性段，否则 `1.055·c^(1/2.4) − 0.055`）转回 sRGB 字节写入
  `EntityColor::Explicit`——这正是 `rvt` 自身 `colour_material`（`src/ifc/gltf.rs:665-675`）
  线性化的逆运算，合成夹具对 `(200,100,50)` 与 `(30,144,255)` 断言**字节级往返**；alpha 仅在
  `alphaMode = BLEND` 且 `a < 1.0` 时写成 `EntityTransparency::Explicit(opacity)`（本仓约定该
  值是**不透明度**，`1.0` 为不透明），`a = 1.0` 不写多余的显式值；primitive 无 `material`
  保持 `ByLayer`（不发明颜色）；非有限或超出 `[0,1]` 的因子、以及同一 node 合并的多个
  primitive 材质不一致 → 保持 `ByLayer` 并记 `import.rvt.material_ignored`（并**报告**合并会
  丢失每个 primitive 的材质区分，而不是静默挑一个）；`material` 索引越界由 `gltf` 文档校验
  直接拒绝整份 GLB（`import.rvt.invalid_glb`），不会 panic，也不静默降级。**纹理不在范围内且
  上游不可得**：`rvt` 0.17.0 不导出任何纹理（无 `images`/`textures`/`samplers`，无
  `TEXCOORD_0`，`uri` 恒为 `None`），`cad_domain::Mesh` 也只有逐顶点 `colors`、没有 UV 通道。
- **夹具与证据**：unit 1 用 `rvt` 上游 Apache-2.0 的 `gen-fixture` 脚手架（`synthetic: false`
  台账，0 几何，断言 `import.rvt.no_geometry`）；unit 2 用本仓库合成的两个 GLB
  （`synthetic: true`：`synthetic-two-node.glb` 证明世界变换与不重叠包围盒，
  `synthetic-materials.glb` 证明材质颜色/alpha 与 `ByLayer` 回退）。**没有真实 Revit 模型**，
  真实文件的几何/包围盒/定位/单位/颜色行为 **NOT VERIFIED**，不能由这些夹具推论。

### D8 SKP：`skpwright` 0.6.0，**GPL-3.0-only**，定位为 **preview**

**已实现**（2026-11-13）为 `crates/cad-import-skp`。本节记录代码的真实 API、单位结论与状态
映射；若与代码不一致，以代码与契约测试为准。

- **库与许可**：`skpwright` **0.6.0**，**GPL-3.0-only**，纯 Rust，**只有一个依赖**
  `miniz_oxide`（纯 Rust，无平台依赖），上游 <https://github.com/hew3d/skpwright>。workspace 中
  **精确固定**（`=0.6.0`），未启用额外 feature，只由 `cad-import-skp` 依赖；**未修改上游源码、
  无 Cargo patch**。主仓许可是 **AGPL-3.0**，依 AGPLv3 §13 可与 GPL-3.0 代码组合；**GPL 部分
  对下游仍按 GPLv3 分发**，不因组合而改许可（见 `THIRD_PARTY_NOTICES.md`）。固定版本的理由与
  `rvt` 相同：pre-1.0、单一维护者，且 2021–2025 的结论依赖容器等价，caret range 不能悄悄浮动
  到改变格式声明的新版本。
- **容器与 release**：`skpwright` 读两种容器——release **3–8 与 2013–2020** 的 MFC `CArchive`
  stream，以及 **2026/2027** 的 ZIP/tagged-record 容器。**2021–2025** 与 2026 同容器，但
  **没有观测样本**，只按容器等价读。检测只认固定 32 字节头（`FF FE FF 0E` + UTF-16LE
  `SketchUp Model`），且是 dispatcher 里**最先**探测的一项；头只说明“这是 SketchUp 模型”，
  release 与可读性由适配器回答，检测层不看扩展名。
- **单位（已确定，不是 `units_unknown`）**：`.skp` 存储 f64 **英寸**；`skpwright` 把几何与
  节点平移都换算成**米**。入库 `UnitContext::from_meter()`，坐标是**米、+Z up**，记
  `import.skp.units_metres`。不假设 SketchUp 的显示单位或毫米。
- **状态上限（不是“已支持”）**：`ceiling_for(version, container)`：stream 容器的 3–8 与
  13–20（2013–2020）⇒ `Verified`，ZIP 容器的 26/27（2026/2027）⇒ `Verified`；9–12 与 21–25
  ⇒ `Unverified`；其余（含无法解析的版本串、`Container::Unknown`）⇒ `Unsupported`，导入直接
  拒绝 `import.skp.version_unsupported`。该上限**同时限制** read/semantic/render/pick 四行；
  **measure 恒为 `Unsupported`**。每个成功导入恒为 `Completeness::Partial` +
  `import.skp.preview_geometry`；`Unverified` 的 release 另记 `import.skp.version_unverified`。
- **读什么**：每个 geometry run 实例一个 `DbEntity`，三角形顶点已是世界坐标（实例/组件变换
  按行主序组合后应用），tag（图层）**扁平成 layer 0** 但**遵守可见性**（隐藏实例、隐藏面与
  不可见图层被裁剪并计数，`import.skp.hidden_pruned`），材质读**颜色 + 不透明度**；**纹理只取
  平均色**（`import.skp.textures_not_sampled`），不采样图像、不放 UV。
- **三角化**：先**耳切**（ear clipping）并做面积校验；失败回退 **even-odd**（带洞的面直接走
  even-odd）；**两者都失败**才丢弃该面并记 `import.skp.faces_dropped`（不近似、不静默）。
- **拒绝码**：头缺失/截断 → `CorruptData(import.skp.not_a_sketchup_file)`；头匹配但解析失败 →
  `CorruptData(import.skp.parse_failed)`；release 无 reader → `Unsupported(import.skp.version_unsupported)`；
  没有可视面 → `Unsupported(import.skp.no_geometry)`；**几何只有边/曲线** →
  `Unsupported(import.skp.edge_only_geometry)`（SketchUp 的线/曲线工作无法用三角形表达，
  显式拒绝，**不是空成功**）；超限 → 命名真实计数的 `InvalidInput(import.skp.limits_exceeded)`。
- **wasm**：导入 lane 是纯 Rust、无平台依赖（`miniz_oxide` 亦为纯 Rust），因此 `dom-canvas`
  的 wasm 会话可以链接它；本轮实跑 `cargo check -p cad-import-skp --target
  wasm32-unknown-unknown --locked` **通过**（编译证据，不是 Web 运行/像素证据）。
- **明确非目标**：边/曲线、纹理图像与 UV 放置、场景（scenes/pages）、剖切面、标注、
  动态组件属性，以及任何 SketchUp 语义/参数、DWG 回写与测量。它们不在后续“补齐”清单里，
  属于本 ADR 的有意边界。
- **证据**：19 条 crate 契约测试（`cargo test -p cad-import-skp`：release 上限、版本串解析、
  材质颜色/不透明度/平均色回退/`ByLayer`、四边形→两三角、带洞面、镜像实例翻转绕向、平移实例、
  隐藏 tag/面裁剪、继承材质只涂未着色面、退化面/越界环索引/零长法向的显式拒绝、真实模型的
  env-gated 契约、两种容器同模型一致、非 SketchUp 文件按名拒绝）。**外部 corpus**（`skpwright`
  的语料，未提交、不可再分发）：全语料 **763** 个 `.skp` ⇒ **728 成功、25
  `import.skp.edge_only_geometry`、10 `import.skp.no_geometry`、0 失败 / 0 panic**；box 1 实体/
  12 三角、house 64/1440、face-with-hole 1/8、instance-scaled 3/36；theater-2017 1246 实体/
  96,333 三角与 theater-2026 1246/96,344 的**面积 1020.981869 m² 一致**（相对差 5e-14，
  容器不同而几何不变）。`YACR_TEST_SKP`/`YACR_TEST_SKP_2026` 未设置即跳过，**跳过不是已运行
  证据**；corpus 数字是有界扫描，不是 golden 回归、性能或产品验收。

### D9 明确拒绝的能力

| 格式/方向 | 理由 |
|---|---|
| IGES | 无成熟纯 Rust 解析器；只能走 OCCT（LGPL + C++/wasi-sdk），与“无 OCCT”决策冲突 |
| BREP（OCCT 序列化） | 文本可解析，但 NURBS 求值 + 离散仍需内核 |
| FCStd | 几何是 FreeCAD 自有 B-rep 序列化，同样需要内核 |
| 3MF / AMF | 超出“便宜层”范围（zip+XML / XML 场景语义），无需求驱动 |
| IFC 导出 | 唯一可用的纯 Rust 写出栈是 AGPL/商业（`openbim`）；宽松许可的写出器不存在。自写“只写一个 IFC4 模型”约 2–4 周，通用授权/校验库以月计 → 独立项目决策 |
| STEP 导出 | 同上，`cadmpeg` 的写侧未纳入本 ADR |

拒绝必须以稳定标记呈现（`unsupported-runtime:` / `import.*` 诊断），不得空白画布或静默回退。

### D10 失败与限额建模

- 解析器的 refusal/loss 一律转成诊断（稳定码）+ `Partial`/`Unsupported`，禁止静默丢弃。
- 超限（`max_file_bytes`、`max_mesh_faces`）返回 `InvalidInput` 并给出实际计数，禁止截断后
  宣称成功。
- 未知输入只有一条拒绝信息，且必须列出当前真实支持的内容。

## 3. 兼容性与许可证

- `tobj`（MIT）、`stl_io`（MIT）、`cadmpeg-*`（Apache-2.0）、`rvt`（Apache-2.0）与
  `gltf`/`gltf-json`/`gltf-derive`（MIT OR Apache-2.0）与 AGPL-3.0 主仓兼容。
- `ifc-lite-*` 是 **MPL-2.0**：文件级 copyleft，与 AGPL-3.0 组合可行；若修改其文件必须披露
  修改。升级策略同其它第三方解析器：只改适配层。
- `skpwright` 是 **GPL-3.0-only**：主仓 **AGPL-3.0** 可与 GPL-3.0 代码组合（AGPLv3 §13），
  但 **GPL 部分对下游仍按 GPLv3 分发**，组合不改变其许可。这是本 ADR 唯一引入的强 copyleft
  依赖；若要移除，只能整条删除 `cad-import-skp`，不得把其代码复制进 MIT/Apache 风格的 crate。
- 新增依赖必须记入 `THIRD_PARTY_NOTICES.md`。`[patch.crates-io]` 仍只允许 `acadrust`
  （ADR-0031），本 ADR 不引入任何 patch。

## 4. 测试与证据

- 合成 fixture（OBJ/STL/PLY/OFF）与带 provenance 的公开样本（STEP/IFC）进 `fixtures/manifest`；
  `synthetic: true` 不得声明 `complete/verified/supported`。RVT 按**两个可独立测试的单元**
  分层：① 上游 `rvt` 0.17.0 的 Apache-2.0 脚手架（`synthetic: false` 台账，0 几何，断言显式
  `import.rvt.no_geometry` 拒绝）；② 本仓库合成的两个 GLB（`synthetic: true`：双节点定位，
  材质颜色/alpha/`ByLayer` 回退）。`.rfa` 拒绝另由测试在内存中构造的家族形状 CFB 覆盖。
- 契约测试覆盖：正常解析、n-gon、缺法向、退化面、超限、损坏输入、取消、显式拒绝路径、
  以及 `validate_geometry` 前的网格不变量（法向/颜色为空或与顶点数相等）。
- SKP 的 19 条契约测试在 crate 内（无提交夹具）；**真实文件证据是外部 corpus**（`skpwright`
  的 763 个 `.skp`，未提交、不可再分发，见 `fixtures/skp/SOURCE.md`）：728 成功、25
  `edge_only_geometry`、10 `no_geometry`、0 失败/0 panic。`YACR_TEST_SKP`/`YACR_TEST_SKP_2026`
  未设置即跳过，**跳过不是已运行证据**。
- 真实模型兼容性、性能、GPU/像素证据均 **NOT RUN**，不得由合成 fixture 推论。RVT 没有任何
  真实 Revit 模型，真实文件的几何/包围盒/定位/单位行为 **NOT VERIFIED**；SKP 的 corpus 结论
  是有界扫描，不构成 golden 回归、性能或产品验收。

## 5. 后果

- 正面：新增格式不再触碰 DWG 解析器；契约、派发与能力记账集中一处；每个解析器可独立替换。
- 负面/成本：多 crate 与多依赖；STEP/IFC 只能显示不能测量；RVT 只有包围盒级 preview；
  SKP 只有三角形 preview，且引入 **GPL-3.0-only** 依赖（`skpwright`），对下游分发有许可约束；
  网格格式的材质/单位/精确几何均为已知缺口。
- 待办（不在本 ADR 完成）：
  - **跟进项 A —— `Mesh.face_sources` 由 STEP/IFC 源面填充（仅数据层）**：两个适配器现在只写
    三角形，`Mesh.face_sources` 为空（`SubElementId` 身份缺失），因此面级拾取（`cad-spatial`
    已按该字段读数）、面到面距离与特征边都还没有可依赖的输入。补齐时只改数据层（`cad-import-step`
    / `cad-import-ifc` 构造 `Mesh` 时写入源面 id），不引入内核、不改渲染与 wire 契约。
    **状态：未实现。**
  - **跟进项 B —— `cad-measure` 的网格测量路径**：该 crate 今天**没有** `Mesh` 测量入口
    （上述格式的 `measure` 能力因此是 `Unsupported`；RVT 上游文档也明确禁止测量）。跟进项 A 落地后
    才可能做只读网格量测（点/边/面到面）；未建模的请求仍须显式拒绝，不得给出近似数字冒充精确值。
    **状态：未实现。**
  - 可测量路径若要精确几何，需要中性 B-rep 扩展 + 离散器（或未来引入内核）；
  - IFC 导出需单独 ADR；`ImportRequest` 若要支持外部资源（MTL/纹理/字体）需增加资源来源通道。
