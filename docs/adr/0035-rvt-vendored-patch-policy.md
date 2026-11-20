# ADR 0035：`rvt` 受控补丁（透明度）与 `vendor/rvt`

- 状态：**Accepted（2026-11-13）；2026-11-14 重挂到上游 0.22.0；2026-11-15 重挂到上游
  0.24.0；2026-11-20 重挂到上游 0.25.0（当前）**。验证边界：Linux 上的契约测试/真实 Revit
  样本（真实样本最近一次实跑在 0.24.0，0.25.0 上 **NOT RUN**）；补丁的 Revit 透明度保真度
  **NOT VERIFIED**；本轮未重跑逐像素/GPU 证据；Windows/macOS/Android/Web 只有编译/契约状态。
- 日期：2026-11-13（决策）；2026-11-14（0.22.0 重挂）；2026-11-15（0.24.0 重挂）；2026-11-20
  （0.25.0 重挂）。
- 相关：[ADR-0031](0031-acadrust-vendored-patch-policy.md)（acadrust 补丁策略与台账）、
  [ADR-0033](0033-multi-format-import.md)（多格式导入，RVT 行）、
  `docs/rvt-patches.md`（本补丁台账）、`scripts/check-architecture.py`（门禁）、
  `docs/compatibility.md`（能力表）。
- 影响：根 `Cargo.toml` 的 `[patch.crates-io]`、`vendor/rvt`、架构门禁、`cad-import-rvt`
  的诊断与测试、文档/台账。**不**改变：`rvt` 只被 `cad-import-rvt` 依赖；`cad-db` 是唯一
  权威；RVT 仍是**预览**（测量恒 `Unsupported`，completeness 恒 `Partial`）。

---

## 1. 问题

1. `cad-import-rvt` 只读 `rvt` 导出的内部 GLB。上游 `rvt` 0.22.0（0.24.0、0.25.0 同样）的
   `ifc::gltf` 在“构件类型由**多个**材质组成”（`IfcMaterialConstituentSet` ≥ 2）时，整块
   预览体退回**类别颜色**且 `alpha = 1`：材质自身的透明度被丢弃。单材质但**颜色未读到**的
   构件也被整个忽略（过滤条件是 `color_packed.is_some()`），同样丢 alpha。
2. 真实样本证据（本地 `rmeadvancedsampleproject.rvt`，Revit 2024，未提交）：6,804 个 mesh
   中原本只有 580 个显式不透明度；**17 个玻璃门**（`Tür - Glas`）与 **149 个玻璃灯具**
   （`Glass - Clear, Grey` / `Glass - White, High Luminance`）在 Revit 里是透明/半透明，
   导出后是全不透明。这是观感缺陷，也是“不透明度来源不可区分”的诚实性缺陷。
3. 先例是 ADR-0031：补丁是例外，但必须固定来源 + 台账 + 门禁 fail-closed。当时只放开了
   `acadrust`；现在需要第二个例外，且不能变成通用 vendoring。

## 2. 决策

### D1 先升级、再规避、最后补丁

已核对上游 `rvt` 0.24.0（2026-11-14 的 crates.io 最新版；0.22.0 已于 2026-11-13 核对过）与
`main`：#1818 的
`MaterialExtras`（`extras.colourSource`/`extras.alphaSource`）已在包内，`view-graphics-
overrides` 也已被声明为 `unsupported`（与 Revit 自身 IFC 导出一致）；#1826 只改
`src/ifc/step_writer.rs`（IFC 未读到透明度时写 `$`），不碰 GLB。**多材质构件的材质选择
逻辑没有改变**：`single_material` 仍只接受“单材质且读到颜色”的集合，其余回退到
`category_material`（alpha 1.0 / 色板值）；0.23.0/0.24.0 的 release notes 与源码都没有实现
alpha 选择。0.24.0 的 `src/ifc/gltf.rs` 相对 0.22.0 只有一处上游改动：material 选择链上
加了越界过滤 `material_index.filter(|&index| index < model.materials.len())` 与一段注释
（越界索引在 STEP 写入器里等于“没有材质”，在 glTF 里会指向 `materials[]` 之外）。本补丁
**保留**该过滤，并把它放进重写后的 `match` 里。

2026-11-20 对 **0.25.0**（2026-11-15 的 crates.io 最新版）重做同一核对：本补丁触及的材质
选择链与 0.24.0 **一致**，多材质退回类别颜色且 alpha=1 的问题未变。0.25.0 的 upstream 变化
在别处：`metallic/roughness` 的名称启发式映射抽到 `src/ifc/pbr.rs`（0.24.0 的写出器已在用
同样映射）；构件体 placement/extrusion 数学改动（预览几何变化，glTF 交接结构不变）。
**0.25.0 仍然没有** `images`/`textures`/`TEXCOORD_0`，`appearance_asset_id` 依旧只解码不
解析位图——升级不带来 RVT 纹理。因此升级到 0.24.0/0.25.0 都不修复该问题。工作区侧也无法规避：
GLB 只携带逐 node 的单一 `material`，多材质关联（`IfcMaterialConstituentSet`）只存在于
`rvt` 的解码模型里，`cad-import-rvt` 读不到。故允许第二个补丁，并在重挂时删掉上游已有
的 extras 实现。

### D2 只有 `acadrust` 与 `rvt` 可以打补丁，且必须固定来源

- 根 `Cargo.toml` 允许的唯一 patch 形状是 `[patch.crates-io]`，其键只能是 `acadrust`、
  `rvt`；不允许其它 patch 表、不允许浮动分支/浮动版本。
- `rvt` 的补丁源是仓库内**固定路径** `vendor/rvt`（上游 0.25.0 源码的完整复制，版本号保持
  `0.25.0`，以便匹配 workspace 的 `rvt = "=0.25.0"`）。workspace 依赖的版本字符串仍是唯一
  “被补丁的上游版本”来源。

### D3 台账是补丁存在的前提

- `docs/rvt-patches.md` 是 `rvt` 补丁的唯一权威台账（`acadrust` 仍是
  `docs/acadrust-patches.md`）。
- `scripts/check-architecture.py` 在看到 `[patch.crates-io] rvt` 时，要求该台账存在且正文
  含 workspace 解析出的 `rvt` 版本字符串；`rvt` 还必须是固定仓库内路径
  `{ path = "vendor/rvt" }`，`vendor/rvt/Cargo.toml` 必须是 `name = "rvt"` 且版本等于 workspace
  的钉住版本，`Cargo.lock` 的 `rvt` 条目**不得**带 `source`（否则说明 Cargo 忽略了 patch）。
  任一不满足即门禁失败（fail-closed）。非 `acadrust`/`rvt` 的 patch 键仍直接失败。
- `vendor/rvt/SHA256SUMS` 记录除 `src/ifc/gltf.rs` 外**每一个** vendored 文件的摘要；门禁逐个
  校验，并拒绝未登记的新文件。改第二个上游文件（或塞入新文件）不能静默通过。

### D4 补丁的最小行为契约

只改 `vendor/rvt/src/ifc/gltf.rs`：

1. 类型由多个材质组成的预览体：保留类别颜色，alpha 取**最透明构件**材质的不透明度
   （`1 - transparency` 的最小值），且只在该集合里至少一个材质**读到了** transparency 时
   生效。预览体是一个整体包围盒，无法按部位分配材质；取最透明构件是**导出器的选择**，不是
   文件对该构件本身的断言。
2. 未按层绘制的层集构件（`material_layer_set_index`）同样按层材质取该 alpha；已按层绘制的
   构件保持逐层材质的既有行为。
3. 单材质但**颜色未读到**（或单 profile 颜色未读到）的构件：颜色仍取类别色板，只补上读到的
   alpha；`single_material`/单 profile 的过滤条件仍是“必须读到颜色”，颜色选取不变。
4. 每个材质的 `extras.colourSource`/`alphaSource`（`"file"`/`"rvt-rs"`）由上游 0.24.0/0.25.0
   写（#1818），本补丁不再自带。类别材质恒 `rvt-rs`，层带材质恒 `file`；被本补丁补上
   alpha 的类别材质同样是 `rvt-rs`。`cad-import-rvt` 必须把 `alphaSource = "rvt-rs"` 的
   显式不透明度计进 `import.rvt.alpha_chosen` 诊断，不得写成本构件自身的透明度。
5. 几何、节点/世界变换、颜色选取、类别色板、IFC/STEP 导出、release 状态与 completeness
   上限**不变**；透明度不会让 RVT 变成可测量。
6. 上游 0.24.0 新增并在 0.25.0 保留的 `material` 索引越界过滤（`index < model.materials.len()`）
   必须保留：
   越界索引按“该构件没有自身材质”处理，随后仍走上面的类别/alpha 回退，既不让 GLB 指向
   `materials[]` 之外，也不 panic。

### D5 依赖边界不变

- `rvt` 仍只允许 `cad-import-rvt` 依赖；平台类型（`web-sys`/`android-activity`/`jni`/`ndk`）
  不得进入 `cad-*`；补丁不得引入新依赖。
- `vendor/rvt` 被 `[workspace] exclude`，不是 workspace 成员：`cargo fmt/clippy --workspace`
  与 `scripts/check-file-size.py`（跳过 `vendor` 路径段）都不改写上游 129k 行源码；
  `scripts/check-architecture.py` 的包图也不含它。

## 3. 数据契约

- `Cargo.toml`：`workspace.dependencies.rvt = "=0.25.0"`；`[patch.crates-io] rvt = { path =
  "vendor/rvt" }`；`[workspace] exclude = ["vendor/rvt"]`。
- `docs/rvt-patches.md` 表格列固定（id / upstream / local path / files / why / delta /
  tests / NOT RUN / upstream / remove-when），不得删列。
- `cad-import-rvt` 诊断码 `import.rvt.alpha_chosen` 是稳定码；GLB 无 `extras` 时按文件自身
  alpha 读取（补丁前的 GLB 与合成夹具的行为不变）。

## 4. 失败闭合

- 非 `acadrust`/`rvt` 的 patch 键 ⇒ 架构门禁失败。
- `[patch.crates-io] rvt` 但台账缺失或不含 `0.25.0` ⇒ 架构门禁失败。
- 台账行缺测试/NOT RUN 信息 ⇒ 评审拒绝（人工门禁；脚本无法验证诚实性）。
- `vendor/rvt` 的改动超出 `src/ifc/gltf.rs`（或改了版本号）⇒ 评审拒绝，除非本 ADR 与台账
  同时更新。

## 5. 测试与证据（已运行）

- `bash scripts/test-rvt-patch.sh`：**7 passed**（`--locked` + 上游随包 `Cargo.lock`；脚本断言
  “7 passed”＝上游 0.25.0 自带 3 个 writer 测试 + 补丁 4 个，删掉补丁的
  `element_alpha_tests` 会失败，而不是“0 filtered out”静默通过。覆盖最透明构件胜出；
  全不透明不产生 alpha；未读到 transparency 不发明 alpha；2024 material frame 的零值
  反常数仍为不透明）。
- `vendor/rvt` 自带测试套件（0.25.0）：**331 passed**（`scripts/test-rvt-patch.sh` 之外的
  独立运行，本轮记录）。
- `cargo test -p cad-import-rvt --locked`：**41 passed**（0.25.0 工作树，PBR 实现 lane 报告，
  见 `docs/validation-pbr-2026-11-20.md`；0.24.0 轮次为 **33 passed**）。含
  `an_exporter_chosen_opacity_is_stored_and_counted`（`rvt-rs` 标记 → `alpha_chosen = 1`；
  无 extras/`file` → 0）。本文件未保存 0.25.0 测试日志。
- 本地真实样本（`YACR_TEST_RVT`，仓库外，`rmeadvancedsampleproject.rvt`，2026-11-15 在
  **0.24.0 + 本补丁**上实跑）：`meshes = 6783`、`triangles = 3,317,956`、
  `explicit_opacity = 746`、`explicit_colors = 6783`、`by_layer_colors = 0`，`completeness`
  仍 `partial`，诊断含 `import.rvt.alpha_chosen`。与 0.22.0 轮次记录（`meshes = 6785`、
  `triangles = 3,317,804`、`explicit_opacity = 746`）相比：**alpha 结果不变，几何略有差异**
  （-2 个 mesh / +152 个三角形，属上游 0.23/0.24 的构件体改动，不是本补丁造成的）。0.22.0
  的 `explicit_opacity` 580（未打补丁）→ 746（本补丁）这一对比来自 2026-11-14 轮次的记录，
  本轮未重跑未打补丁的 0.24.0。**0.25.0 上的真实样本重基线 NOT RUN。**
- `cad-cli-tools render`（Quadro P620，真实 GPU，离屏）：`transparent_batches` 580 →
  **746**，`invisible_batches = 0`。**该条来自 0.17.0 补丁轮次；0.22.0、0.24.0 与 0.25.0
  重挂后都 NOT RUN。**
- `python3 scripts/check-architecture.py`：通过（`rvt` patch 0.25.0 + 台账；`vendor/rvt`
  210 个 vendored 文件；`Cargo.lock` 的 `rvt` 无 `source`；本轮实测 44 packages）。漂移探针
  （workspace 钉住版本、`vendor/rvt/Cargo.toml` 版本、`path` 指向、`Cargo.lock` 补回
  registry `source`）在 0.22.0 轮次用 `=0.22.1`/`0.22.9` 实测 fail-closed，本轮只跑常规通过
  路径，未重做漂移探针。门禁自测：`python3 scripts/test-ui-migration-guards.py` **23 passed**
  （0.22.0 轮次；本轮 NOT RUN），含 5 条 patch-策略契约。
- `cargo fmt --all -- --check`、`rustfmt --check --edition 2024
  vendor/rvt/src/ifc/gltf.rs`、`cargo clippy -p cad-import-rvt --all-targets --locked -- -D
  warnings`、`cargo check -p cad-import-rvt --target wasm32-unknown-unknown --locked`：
  0.24.0 重挂后重跑通过；**0.25.0 重挂后本轮 NOT RUN**（构建门禁被并发的 PBR 变更占用）。
- **NOT VERIFIED**：Revit 透明度保真度（无第二份样本交叉验证）；0.22.0/0.24.0/0.25.0 重挂后的
  逐像素/GPU 证据（0.17.0 轮次该固定视角下新增透明构件被不透明构件遮挡，前后 PNG 字节
  相同，**不能**当作像素证据）；Windows/macOS/Android/Web 的补丁构建只有编译/契约状态。
