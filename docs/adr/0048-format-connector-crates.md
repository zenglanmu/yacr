# ADR 0048：每个格式一个连接器 crate（导入 + 导出同源）

- 状态：**Accepted（2026-11-23）**，同日实现。`cad-import-{acadrust,dgn,step,rvt,ifc,gltf,skp,mesh,obj,stl}`、
  `cad-import-core`、`cad-export-{ifc,gltf,skp,mesh}` 已删除；替代者是
  `cad-connector-core` + `cad-connector-{acadrust,dgn,step,rvt,ifc,gltf,skp,mesh}`。验证记录见
  `docs/validation-connectors-2026-11-23.md`（只有编译/契约证据，无运行时、GPU、像素或 vendor 验收）。
- 日期：2026-11-23。
- 相关：[ADR-0033](0033-multi-format-import.md)（多格式导入与网格语义边界，仍是**能力**决策的归属）、
  [ADR-0041](0041-save-export-split-and-format-provenance.md)（保存/导出分离与来源提示）、
  [ADR-0031](0031-acadrust-vendored-patch-policy.md)/[ADR-0035](0035-rvt-vendored-patch-policy.md)
  （补丁台账）、`docs/compatibility.md`（连接器与导出矩阵）、`docs/architecture.md`（crate 分层）。
- 影响：workspace 成员、`cad-app` 的连接器注册表、`docs/*` 中的 crate 路径与测试命令。
  **不改变**：任何格式的能力/状态/证据结论、probe 顺序、导出键、拒绝码、按载体编译的格式集、
  `.ymodel`/`DocumentArchive`、补丁与单一依赖边界。
- 影响文件：`Cargo.toml`（workspace 成员与 `[workspace.dependencies]`）、各 `crates/cad-connector-*`、
  `crates/cad-app/src/connector.rs`、`scripts/check-architecture.py` 的依赖归属断言、`docs/*` 的
  crate 名与 `cargo test -p …` 命令。

---

## 0. 背景

ADR-0033 把导入按格式拆成独立 crate（`cad-import-core` 提供契约，`cad-import-*` 提供解析器），
理由是格式判定、`Importer` trait 与具体解析器不能同处一个 crate：否则任何新格式都会把
`acadrust` 拖进它的构建，`acadrust` 单一依赖边界失去意义。该决策仍然成立。

之后导出按同样思路拆成 `cad-export-{ifc,gltf,skp,mesh}`（ADR-0041/0038/0039 系列落地），
于是 2026-11 的连接器集合是 10 个导入 crate + 4 个导出 crate + 1 个 core。问题不在数量本身，而在
**同一个格式的知识被切成两半**：

1. **同一格式的知识被拆到两处**：`ConnectorDescriptor` 由导入 crate 声明，导出 crate 反而要反向
   依赖它；导出侧又各自维护一套 PBR/纹理/报告约定。任何格式能力变化都要跨两个 crate 手工对齐，
   读者还得在两个 crate 里确认"写出侧和读入侧对同一字节语义的假设是否一致"。
2. **拆分没有产生构建收益**：`cad-app` 的 `formats-*` feature 从来是"格式"粒度
   （`formats-ifc = [dep:cad-import-ifc, dep:cad-export-ifc]`），没有任何载体选择"只编导出"或
   "只编导入"。载体要么整格式编入，要么整格式拒绝。
3. **同一格式的导入/导出共享事实**：IFC 的"导入用 `ifc-lite-geometry`、导出用 `ifc-lite-export`"、
   SKP 的 2026 容器、glTF 的材质/PBR 语义、mesh 的单位与三角化约定，都是**同一个格式契约**的两面；
   分开后读者要在两个 crate 里确认"写出侧和读入侧对同一个字节语义的假设是否一致"。
4. **测试命令与台账分裂**：`cargo test -p cad-import-ifc` 与 `-p cad-export-ifc` 分别跑，能力表却
   按格式列两列（导入/导出）；文档里同一格式的路径散落在两处。

同时，`cad-app` 仍保留**两套**派发结构（导入 connector 列表、导出 connector 列表）和第三份
"已知导出键"清单，三者必须手工保持一致；slim 构建的 `unsupported-runtime` 拒绝也要在两处维护。

## 1. 决策

### D1 — 一个格式一个 crate，同时拥有导入与导出

`cad-connector-<format>` 是某格式的唯一实现 crate：

| 格式 | crate | 方向 |
|---|---|---|
| DWG/DXF | `cad-connector-acadrust` | 导入 + 导出（DXF/DWG，实验性 DWG） |
| DGN V7/V8 | `cad-connector-dgn` | 只导入（无写出器） |
| STEP AP203/214/242 | `cad-connector-step` | 只导入 |
| RVT/RTE | `cad-connector-rvt` | 只导入（预览几何） |
| IFC2X3/4/4X3 | `cad-connector-ifc` | 导入 + IFC4 导出 |
| glTF 2.0/GLB | `cad-connector-gltf` | 导入 + 导出（GLB + JSON `.gltf`） |
| SKP | `cad-connector-skp` | 导入 + SKP 导出（2026 容器） |
| STL/OBJ/PLY/OFF | `cad-connector-mesh` | 导入 + 导出（STL/OBJ/PLY/OFF） |
| —（共享契约） | `cad-connector-core` | `Importer`/`Exporter`/`ConnectorDescriptor`/报告/`MeshImport`/嗅探 |

只导入的格式保留"无写出器"的诚实状态：`FormatConnector.exports` 为空，导出请求是稳定的
`export.<id>.not_implemented`，不是空成功。合并不是为了让每个格式都长出一个写出器。

`cad-connector-core` 只是 `cad-import-core` 的更名 + 范围扩展（它此前已同时承载导出契约
`ExportJob`/`CompatReport`）。重建 `cad-import-core` 以保留旧名没有价值：crate 内已无"仅导入"语义。

### D2 — `cad-app` 只有一张 `FormatConnector` 表

```
FormatConnector {
    descriptor: &'static ConnectorDescriptor,   // 由实现 crate 声明，cad-app 只接线
    import:    Option<ImportLane>,              // importer + 有序 probes
    exports:   &'static [ExportLane],           // format_id + writer
}
```

- 导入：按 connector 顺序、每个 lane 的 probe 顺序求值，**第一个 `true` 获胜**；扁平顺序与旧实现
  逐项相同（见 D3）。
- 导出：按 `format_id` 线性查找 writer（`"stl"`/`"stl-ascii"` 是同一 descriptor 的两个键）。
- 不变量（注册表测试固定，不是约定）：`import.is_some()` ⇔ descriptor 声明导入能力；
  `exports` 非空 ⇔ descriptor 声明导出能力。lane 不能悄悄丢失或凭空出现。
- 未接线写出、被 feature 裁掉的格式、未知键的拒绝顺序固定为：注册 lane → `export.<id>.not_implemented`
  → `unsupported-runtime: export.<key>` → `export.unknown_format`。

`cad_app::import::FormatImporter` 继续存在，但只是 `ConnectorRegistry` 的 `Importer` 薄包装；
新增格式只改注册表与格式 crate，不再新增派发结构。

### D3 — probe 顺序仍是 `cad-app` 的策略，逐项不变

`SKP → DGN V7 → DGN V8（OLE `Dgn~H`）→ OLE（其余交给 RVT）→ DWG → DXF → IFC（`FILE_SCHEMA`）
→ STEP → glTF → STL → PLY → OFF → OBJ`。

- DGN 的两个 probe 挂在同一个 `FormatConnector` 上，但仍在 RVT 的 OLE probe **之前**求值；
  合并 crate 不会把顺序变成"按 crate"。
- 该顺序是**派发策略**，属于 `cad-app`，不属于格式 crate：通用 probe（`detect_format` 的薄包装）
  与顺序都留在 `cad-app`；格式 crate 只额外提供自身解析器必须的嗅探函数（DGN 的
  `DgnImporter::sniff`、glTF 的 `sniff`），不决定自己在全局里的优先级。
- 顺序与导出键集合由特征化测试（characterization tests）钉住，重构前后同一份断言。

### D4 — 导出键与拒绝码不变

14 个导出键保持原样：`dxf`、`dwg`、`stl`、`stl-ascii`、`obj`、`ply`、`off`、`ifc`、`skp`、`glb`、
`gltf`、`step`、`dgn`、`rvt`（后三个在完整构建里是"已知但无写出器"的具名拒绝，在 slim 构建里是
`unsupported-runtime`）。所有 `import.*`/`export.*` 稳定码、诊断、`CompatReport` 语义都不变；
合并只改 crate 归属，不改字节或消息。

### D5 — 按载体编译的格式集不变

- `formats-*` 仍是 `cad-app` 的格式粒度 feature，`default = ["formats-all"]`。
- 原生载体与 Linux X11/XWayland `dom-canvas`（`full-formats`）编译全部格式。
- 部署的浏览器 bundle（`app-web` slim）仍只有 DWG/DXF 导入 + 导出；其余格式按
  `unsupported-runtime: import.<id>` / `export.<key>` 显式拒绝。
- `cad-connector-mesh` 承接四个 mesh 格式，但注册表仍是**四个** `FormatConnector`（STL/PLY/OFF/OBJ），
  slim 形态的 profile 与 `cargo tree` 第三方并集不因合并而变化（`formats-mesh` 一个 feature 原本就
  同时编入这四个 lane）。

### D6 — `.ymodel`/`DocumentArchive` 与补丁边界不变

- `.ymodel` 仍由 `cad-store-zip`/`cad-platform::DocumentArchive` 承载，连接器只负责外部格式的
  导入与导出；归档 minor 版本、表项与恢复快照不受影响。
- 单一依赖边界改为按 crate 名表述：`acadrust` 只允许 `cad-connector-acadrust`，`rvt` 只允许
  `cad-connector-rvt`，`cadkit-dgn`/`cadkit-core` 只允许 `cad-connector-dgn`；根
  `[patch.crates-io]` 仍只允许 `acadrust`/`rvt` 两个键，台账（ADR-0031/0035）不变。

## 2. 未改变（避免误解）

- **能力与证据结论**：ADR-0033 仍是格式能力（网格 vs 矢量、measure `Unsupported`、RVT/SKP 预览
  上限、IFC `Partial`）的唯一决策来源；本 ADR 只移动 crate 边界。`docs/compatibility.md` 的每一格
  状态与证据都没有升级。
- **恢复快照、保存/导出分离、来源提示**（ADR-0041）、**BIM 语义**（ADR-0042）、**参数化**
  （ADR-0043）都不变；相关路径只换 crate 名。
- **导出仍是 L1 + 逐条损失报告**，不引入 L0 直通。

## 3. 后果

- 正面：格式知识集中在一处；新增格式只加一个 crate + 一个注册表条目；描述符/拒绝码/报告不再跨
  crate 对齐；测试按格式一库跑完；slim 构建的拒绝逻辑只有一份。
- **已接受的取舍**：载体不能再按格式组合"只编导出、不编导入"。旧结构理论上允许，实际没有任何
  载体使用；若未来出现真正需要导出-only 的载体，应在 `formats-*` 之外**新增一个显式的方向 feature**，
  而不是重新拆 crate。
- 成本：`cad-connector-mesh` 是四个格式的宿主，单 crate 体积与测试面更大；它仍以"每个格式一个
  descriptor/lane"记账，不合并格式语义。`cad-connector-ifc` 对 `cad-connector-step` 只有
  dev-dependency（测试证明 IFC 字节不会进入 STEP 适配器），这个边在合并前就存在。
- 仍待独立决策：网格边线（edges）等格式能力的扩展、`.ymodel` 之外的导出-only 载体、以及任何
  新的第三方解析库引入，都不由本 ADR 预判。
- 版本/兼容：crate 名变化是**仓库内部**重构，未发布到 crates.io，不构成对外 API 破坏；`cad-app`
  的 `pub use` 保持 `cad_app::DESCRIPTOR_*` 路径可用。

## 4. 验证

2026-11-23 的编译/契约记录见 `docs/validation-connectors-2026-11-23.md`：连接器注册表测试
（full 与 `--no-default-features` 两套）、各合并 crate 的契约测试、`app-web` 测试、workspace
`cargo check`、合并 crate 的 clippy、`app-web` wasm check、`check-architecture.py`、
`check-file-size.py`、`cargo tree` 第三方并集对比，以及 slim bundle 构建
（`scripts/build-web-ui.sh`，构建产物证据；**不是**浏览器/GPU/像素验收）。

明确 **NOT RUN**：全量 `cargo test --workspace`、真实 GPU/像素/性能测试、浏览器 smoke、任何
vendor/真机运行。crate 名变化没有改变任何格式字节，但该结论只是代码契约与构建产物证据，不是
格式兼容性验收。
