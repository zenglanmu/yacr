# rvt 受控补丁台账（`[patch]` policy）

> **当前状态（2026-11-14）：存在一个补丁。** 上游 `rvt` **0.22.0**（Apache-2.0，
> <https://github.com/DrunkOnJava/rvt-rs>）的源码以**固定路径** `vendor/rvt` 复制进仓库，
> 只修改 `src/ifc/gltf.rs`（多材质/未读到颜色构件的**透明度选择**），根 `Cargo.toml` 的
> `[patch.crates-io]` 只有 `rvt` 一个键，workspace 依赖仍精确钉住 `=0.22.0`。决策见
> [ADR-0035](adr/0035-rvt-vendored-patch-policy.md)；`acadrust` 的补丁台账仍是
> [`docs/acadrust-patches.md`](acadrust-patches.md)（ADR-0031），当前没有 `acadrust` 补丁。

## 1. 目的

`rvt` 是 `cad-import-rvt` 唯一的 Revit 解码/glTF 导出依赖（AGENTS.md 铁律 1）。它的
glTF 导出把“类型由多个材质组成的构件”整块退回类别颜色**并且 alpha = 1**，材质自身的
透明度被丢弃；单材质但颜色未读到的构件同样丢 alpha。真实样本
（`rmeadvancedsampleproject.rvt`，Revit 2024）上这是 17 个玻璃门 + 149 个玻璃灯具
（`import.rvt.alpha_chosen` 之前的静默丢失）。

本台账让这个补丁**可追溯、可复现、可移除**，并保证“改了上游”不会被静默忘记。

补丁不是默认路径：先找上游版本升级、可行的工作区规避、或显式 `Unsupported` 建模。
2026-11-14 已按 issue #1812 的处置逐条核对上游 **0.22.0**（2026-11-13 发布的 crates.io
最新版）与 `main`：

- #1818（已并入 0.20.0+）给每个 glTF 材质加了 `extras.colourSource`/`alphaSource` 来源
  标记，并新增 `view-graphics-overrides` 支持矩阵行（V/G 覆盖**不在**导出契约内，与
  Revit 自身 IFC 导出一致）。这部分本补丁不再自带，直接使用上游实现。
- #1826（并入 0.20.0+）只改 `src/ifc/step_writer.rs`：IFC 的
  `IfcSurfaceStyleRendering.Transparency` 在未读到透明度时写 `$` 而非 0。它**不碰 GLB**，
  对 `cad-import-rvt` 读取的 glTF 路径没有影响。
- 多材质构件“整块退回类别颜色且 alpha = 1”的选择逻辑在 0.22.0 与 `main` 的
  `src/ifc/gltf.rs` 中**没有变化**：`single_material` 仍只接受“单材质且读到颜色”的集合，
  其余走 `category_material`。无自身材质的构件取类别材质仍是上游 #1603（open）。

因此升级到 0.22.0 不能修复该问题；工作区侧也无法规避（GLB 里没有材质与构件的多材质
关联，`cad-import-rvt` 只读 GLB）。补丁随之重挂到 0.22.0，并**删掉上游已有的 extras
部分**。

## 2. 策略（`scripts/check-architecture.py` 强制）

1. **只允许 `acadrust` 与 `rvt`**：根 `Cargo.toml` 的 patch 表只能是
   `[patch.crates-io]`，键只能是这两个之一。任何其它 crate 的 patch 或任何其它 patch
   表一律拒绝。
2. **固定版本/路径**：workspace 依赖仍钉住明确的 `rvt = "=0.22.0"`；补丁源是仓库内固定
   路径 `vendor/rvt`（不是浮动分支、不是 `path` 指向未提交的本地修改）。上游随包的
   `vendor/rvt/Cargo.lock` 一并跟踪，供 `--locked` 跑补丁自己的测试；补丁源码保持上游
   `Cargo.toml` 不变（`--locked` 顺带证明 vendored manifest 未被改）。
3. **必须先登记**：存在 `[patch.crates-io] rvt` 时，本文件必须存在且正文必须包含
   workspace 解析出的版本字符串（`0.22.0`）；缺失或不含版本 ⇒ 架构门禁失败。
   门禁还强制：patch 值必须是 `{ path = "vendor/rvt" }`（固定仓库内路径，不接受 git 分支）、
   `vendor/rvt/Cargo.toml` 必须是 `name = "rvt"` 且版本等于钉住版本、`Cargo.lock` 的 `rvt`
   条目**不带** `source`（否则说明 Cargo 忽略了 patch，构建的仍是 registry 版本）；并且
   `vendor/rvt/SHA256SUMS` 必须列出除 `src/ifc/gltf.rs` 外每个 vendored 文件的摘要、逐文件
   校验通过、且目录里没有未登记文件。
4. **一行一个补丁**：所有列都要填写；“NOT RUN”如实写 NOT RUN。
5. **不做第二套 API**：业务层仍只通过 `cad-import-rvt` 使用 `rvt`；`rvt` 只允许
   `cad-import-rvt` 依赖的规则不变。
6. **随时可移除**：上游发布等价修复后优先删补丁、退版本、删除 `vendor/rvt`。

## 3. 台账

| id | upstream version / rev | local commit or path | files touched | why | behaviour delta | tests run | NOT RUN | upstream issue / PR | remove when |
|---|---|---|---|---|---|---|---|---|---|
| `rvt-alpha-1` | `rvt` 0.22.0（crates.io，registry 复制，未改 `Cargo.toml` 版本号；2026-11-14 由 0.17.0 重挂） | `vendor/rvt`（仓库内固定路径；由 `Cargo.toml` 的 `[patch.crates-io] rvt = { path = "vendor/rvt" }` 选中） | `src/ifc/gltf.rs`（`MaterialInfo` 导入 / `most_transparent_alpha` / `element_alpha`（含单材质但颜色未读到的构件与单 profile、层集）/ `category_alpha_materials` 与类别回退的 alpha；文件末尾新增 `#[cfg(test)] mod element_alpha_tests`。**不含** extras：0.22.0 上游已自带 #1818 的 `MaterialExtras` / `SOURCE_FILE` / `SOURCE_RVT_RS`） | 0.22.0 的 glTF 导出仍丢弃多材质构件与“颜色未读到”构件的材质透明度（issue #1812 已按子项关闭，但选择逻辑未改；#1818 只加来源标记、#1826 只改 IFC 侧 `$`），Revit 预览里玻璃门/玻璃灯具被画成不透明 | ① 多材质类型（`IfcMaterialConstituentSet` ≥ 2 个材质）的整块预览体沿用类别颜色，但 alpha 取**最透明构件**材质的不透明度（`1 - transparency` 的最小值），且只在该集合里至少一个材质**读到了** transparency 时生效；② 未按层绘制的层集构件（`material_layer_set_index`）同样按层材质取该 alpha；③ 单材质但**颜色未读到**、或单 profile 颜色未读到的构件：颜色仍取类别色板（**不**换成 `PbrMaterial` 的灰色，`single_material`/单 profile 的过滤条件保持“必须读到颜色”），只补上读到的 alpha；④ extras 的来源标记由上游 0.22.0 写（本补丁不再重复实现）；⑤ 其余（几何、节点、颜色选取、类别色板、IFC/STEP 导出、release 状态）不变 | `bash scripts/test-rvt-patch.sh` **7 passed**（`--locked` 用上游随包 `Cargo.lock`；脚本断言“7 passed”＝上游 3 个 writer 测试 + 补丁 4 个，删掉 `element_alpha_tests` 会失败）；`cargo test -p cad-import-rvt --locked` **33 passed**；`YACR_TEST_RVT=<local sample>` 实跑（0.22.0 上游 vs 本补丁，同一台机、同一 target）：`explicit_opacity` 580 → **746**，诊断 `import.rvt.alpha_chosen` 存在；`python3 scripts/check-architecture.py` 通过（0.22.0 / 207 个 vendored 文件 / `Cargo.lock` 无 `source`）；`python3 scripts/test-ui-migration-guards.py`、`check-file-size.py`、`check-fixture-manifest.py`、`cargo fmt --all -- --check`、`rustfmt --check --edition 2024 vendor/rvt/src/ifc/gltf.rs`、`cargo clippy -p cad-import-rvt --all-targets --locked -- -D warnings` 通过 | 真实 Revit 透明度保真度（单一样本，无第二份交叉验证）；逐像素/GPU 渲染证据本轮 **NOT RUN**（沿用 0.17.0 补丁时 Quadro P620 的 580→746 记录，未在 0.22.0 重跑）；Windows/macOS/Android/Web 只到编译/契约；上游 issue #1812 已关闭但多材质 alpha 与 #1603 仍未修 | 上游 issue #1812（closed，多材质 alpha 未修）与 #1603（open）；PR #1818 / #1826（已并入 0.20.0+） | 上游 `rvt` 在 glTF 里为多材质构件保留材质 alpha（或提供等价选择规则）且发布后，删 `vendor/rvt`、删 `[patch.crates-io]`、升版本 |
`python3 scripts/check-architecture.py` **通过**（36 packages；新增固定路径/版本/lock 生效检查，四种漂移实测均 fail-closed：workspace 版本改 `=0.17.1`、`vendor/rvt` 版本改 `0.17.9`、`path` 改 `vendor/rvt2`、`Cargo.lock` 给 `rvt` 补回 registry `source`）；`python3 scripts/check-workflows.py`、`check-file-size.py`、`check-fixture-manifest.py` 通过；`cargo fmt --all -- --check`、`rustfmt --check --edition 2024 vendor/rvt/src/ifc/gltf.rs`、`cargo clippy -p cad-import-rvt --all-targets --locked -- -D warnings`、`cargo check -p cad-import-rvt --target wasm32-unknown-unknown --locked` 通过；CI `core-quality` 新增 `bash scripts/test-rvt-patch.sh` 步骤（`check-workflows.py` 的必需片段） | 真实 Revit 透明度保真度（没有第二份样本交叉验证）；逐像素证据（该固定视角下新增的透明构件被不透明构件遮挡，前后 PNG 字节相同）；Windows/macOS/Android/Web 只到编译/契约；未向上游提交 issue/PR | 未提交（本地无 fork 凭据）；候选：上游 `ifc::gltf` 多材质 alpha 与 `MaterialExtras`（0.20.0 已有后者） | 上游 `rvt` 在 glTF 里为多材质构件保留材质 alpha（或提供等价选择规则）且发布后，删 `vendor/rvt`、删 `[patch.crates-io]`、升版本 |

## 4. 如何更新这个补丁

1. **先证明必要性**：写清上游版本、复现步骤（真实样本路径不入库）、失败用例，并确认
   升级上游或工作区规避不可行。
2. **只改目标文件**：`vendor/rvt/src/ifc/gltf.rs`；不夹带无关重构，不改 `Cargo.toml`
   版本号（`[patch]` 必须能匹配 `=0.22.0`）。
3. **同步契约测试**：`vendor/rvt` 内 `#[cfg(test)] mod element_alpha_tests` 覆盖新规则
   （上游 0.22.0 自带的 3 个 writer 测试保留，`scripts/test-rvt-patch.sh` 断言合计 7）；
   `cad-import-rvt` 覆盖 GLB 侧可观测结果（`GlbScene::alpha_chosen` 与
   `import.rvt.alpha_chosen`）。
4. **跑证据**（`vendor/rvt/Cargo.lock` 是上游随包 lock，随补丁源码一起跟踪，供
   `--locked` 使用；改 `src/ifc/gltf.rs` 以外任何 vendored 文件都要重生成
   `vendor/rvt/SHA256SUMS` 才会过门禁）：
   ```bash
   bash scripts/test-rvt-patch.sh
   cargo test -p cad-import-rvt --locked
   YACR_TEST_RVT=<absolute .rvt> cargo test -p cad-import-rvt real_revit_project --locked -- --nocapture
   python3 scripts/check-architecture.py
   ```
5. **更新本文件**（行、行为差异、已运行/未运行）。

## 5. 边界

- 台账不是兼容性声明：补丁测试通过只证明补丁自己的合成/本地样本用例；真实
  Revit 透明度保真度仍是 **NOT VERIFIED**。
- `vendor/rvt` **不是**通用 vendoring 区域：`scripts/check-file-size.py` 跳过 `vendor`
  路径段只是为了让上游 129k 行源码不被本仓的行数规则改写；架构门禁的
  `load_packages` 不把 `vendor/rvt` 当 workspace 成员，`cargo fmt/clippy --workspace`
  也不会覆盖它。
- 补丁产生的 alpha 是**导出器选择**（`extras.alphaSource = "rvt-rs"`），
  `cad-import-rvt` 必须把这类构件计进 `import.rvt.alpha_chosen` 诊断，不允许当成
  “文件自身的构件透明度”。
