# ADR 0035：`rvt` 受控补丁（透明度）与 `vendor/rvt`

- 状态：**Accepted（2026-11-13）**。验证边界：Linux 上的契约测试/真实 GPU 离屏渲染/本地
  真实 Revit 样本；补丁的 Revit 透明度保真度 **NOT VERIFIED**；Windows/macOS/Android/Web
  只有编译/契约状态。
- 日期：2026-11-13。
- 相关：[ADR-0031](0031-acadrust-vendored-patch-policy.md)（acadrust 补丁策略与台账）、
  [ADR-0033](0033-multi-format-import.md)（多格式导入，RVT 行）、
  `docs/rvt-patches.md`（本补丁台账）、`scripts/check-architecture.py`（门禁）、
  `docs/compatibility.md`（能力表）。
- 影响：根 `Cargo.toml` 的 `[patch.crates-io]`、`vendor/rvt`、架构门禁、`cad-import-rvt`
  的诊断与测试、文档/台账。**不**改变：`rvt` 只被 `cad-import-rvt` 依赖；`cad-db` 是唯一
  权威；RVT 仍是**预览**（测量恒 `Unsupported`，completeness 恒 `Partial`）。

---

## 1. 问题

1. `cad-import-rvt` 只读 `rvt` 导出的内部 GLB。上游 `rvt` 0.17.0 的
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

已核对上游 `rvt` 0.20.0（2026-11 最新）：新增 `MaterialExtras`
（`extras.colourSource`/`extras.alphaSource`）来源标记，但**没有**改变多材质构件的
材质选择逻辑，因此升级不修复该问题。工作区侧也无法规避：GLB 只携带逐 node 的单一
`material`，多材质关联（`IfcMaterialConstituentSet`）只存在于 `rvt` 的解码模型里，
`cad-import-rvt` 读不到。故允许第二个补丁。

### D2 只有 `acadrust` 与 `rvt` 可以打补丁，且必须固定来源

- 根 `Cargo.toml` 允许的唯一 patch 形状是 `[patch.crates-io]`，其键只能是 `acadrust`、
  `rvt`；不允许其它 patch 表、不允许浮动分支/浮动版本。
- `rvt` 的补丁源是仓库内**固定路径** `vendor/rvt`（上游 0.17.0 源码的完整复制，版本号保持
  `0.17.0`，以便匹配 workspace 的 `rvt = "=0.17.0"`）。workspace 依赖的版本字符串仍是唯一
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
4. 每个材质写 `extras.colourSource`/`alphaSource`（`"file"`/`"rvt-rs"`）。类别材质恒
   `rvt-rs`，层带材质恒 `file`。`cad-import-rvt` 必须把 `alphaSource = "rvt-rs"` 的显式
   不透明度计进 `import.rvt.alpha_chosen` 诊断，不得写成本构件自身的透明度。
5. 几何、节点/世界变换、颜色选取、类别色板、IFC/STEP 导出、release 状态与 completeness
   上限**不变**；透明度不会让 RVT 变成可测量。

### D5 依赖边界不变

- `rvt` 仍只允许 `cad-import-rvt` 依赖；平台类型（`web-sys`/`android-activity`/`jni`/`ndk`）
  不得进入 `cad-*`；补丁不得引入新依赖。
- `vendor/rvt` 被 `[workspace] exclude`，不是 workspace 成员：`cargo fmt/clippy --workspace`
  与 `scripts/check-file-size.py`（跳过 `vendor` 路径段）都不改写上游 129k 行源码；
  `scripts/check-architecture.py` 的包图也不含它。

## 3. 数据契约

- `Cargo.toml`：`workspace.dependencies.rvt = "=0.17.0"`；`[patch.crates-io] rvt = { path =
  "vendor/rvt" }`；`[workspace] exclude = ["vendor/rvt"]`。
- `docs/rvt-patches.md` 表格列固定（id / upstream / local path / files / why / delta /
  tests / NOT RUN / upstream / remove-when），不得删列。
- `cad-import-rvt` 诊断码 `import.rvt.alpha_chosen` 是稳定码；GLB 无 `extras` 时按文件自身
  alpha 读取（补丁前的 GLB 与合成夹具的行为不变）。

## 4. 失败闭合

- 非 `acadrust`/`rvt` 的 patch 键 ⇒ 架构门禁失败。
- `[patch.crates-io] rvt` 但台账缺失或不含 `0.17.0` ⇒ 架构门禁失败。
- 台账行缺测试/NOT RUN 信息 ⇒ 评审拒绝（人工门禁；脚本无法验证诚实性）。
- `vendor/rvt` 的改动超出 `src/ifc/gltf.rs`（或改了版本号）⇒ 评审拒绝，除非本 ADR 与台账
  同时更新。

## 5. 测试与证据（已运行）

- `bash scripts/test-rvt-patch.sh`：**4 passed**（`--locked` + 上游随包 `Cargo.lock`；脚本断言
  “4 passed”，删掉补丁的 `mod tests` 会失败，而不是“0 filtered out”静默通过。覆盖最透明构件
  胜出；全不透明不产生 alpha；未读到 transparency 不发明 alpha；2024 material frame 的零值
  反常数仍为不透明）。
- `cargo test -p cad-import-rvt --locked`：**33 passed**，含新增
  `an_exporter_chosen_opacity_is_stored_and_counted`（`rvt-rs` 标记 → `alpha_chosen = 1`；
  无 extras/`file` → 0）。
- 本地真实样本（`YACR_TEST_RVT`，仓库外）：`explicit_opacity` 580 → **746**（504 来自文件：
  438 玻璃幕墙嵌板 + 66 单材质灯具；242 是导出器选择：76 房间类别默认 + 166 多材质预览体），
  `completeness` 仍 `partial`，诊断新增 `import.rvt.alpha_chosen`。
- `cad-cli-tools render`（Quadro P620，真实 GPU，离屏）：`transparent_batches` 580 →
  **746**，`invisible_batches = 0`。
- `python3 scripts/check-architecture.py`：通过（`rvt` patch + 台账；36 packages）。四种漂移
  实测都 fail-closed：workspace 钉住版本改 `=0.17.1`、`vendor/rvt/Cargo.toml` 版本改
  `0.17.9`、`path` 改 `vendor/rvt2`、`Cargo.lock` 给 `rvt` 补回 registry `source`。门禁自测：
  `python3 scripts/test-ui-migration-guards.py` **23 passed**，其中新增 5 条 patch-策略契约
  （固定路径 + 台账 + 钉住版本被接受；vendored 版本漂移、第二个被改文件、lock 仍解析
  registry、允许名单外的 patch 键各自被拒绝）。
- `cargo fmt --all -- --check`、`rustfmt --check --edition 2024
  vendor/rvt/src/ifc/gltf.rs`、`cargo clippy -p cad-import-rvt --all-targets --locked -- -D
  warnings`、`cargo check -p cad-import-rvt --target wasm32-unknown-unknown --locked`：
  通过。
- `python3 scripts/check-workflows.py` / `check-file-size.py` /
  `check-fixture-manifest.py`：通过；CI `core-quality` 新增 `bash scripts/test-rvt-patch.sh`
  步骤。
- **NOT VERIFIED**：Revit 透明度保真度（无第二份样本交叉验证）；逐像素差异（该固定视角下
  新增透明构件被不透明构件遮挡，前后 PNG 字节相同，**不能**当作像素证据）；Windows/macOS/
  Android/Web 的补丁构建只有编译/契约状态；未向上游提交 issue/PR。
