# ADR 0031：acadrust 受控补丁策略与补丁台账

- 状态：**Accepted（2026-11-07）**。验证边界：仅 Linux 上的契约/架构门禁；补丁本身**不存在**，
  因此没有任何补丁运行证据。Windows/macOS/Android/Web 与真实 DWG/AutoCAD 兼容性均
  **NOT VERIFIED**。
- 日期：2026-11-07。
- 相关：[ADR-0010](0010-react-linux-host-replaces-slint.md)（宿主分层）、`docs/acadrust-patches.md`
  （补丁台账）、`scripts/check-architecture.py`（门禁）、`docs/compatibility.md`（能力表）。
- 影响：根 `Cargo.toml` 的 `[patch]` 规则、架构门禁、文档。**不**改变：`acadrust` 只能被
  `cad-import-acadrust` 依赖；`cad-db` 是唯一权威；未支持能力必须显式建模。

---

## 1. 问题

1. 原规则是“`acadrust` 源码不改、Cargo patch 一律禁止”（AGENTS.md 铁律 1）。它简单、可审计，
   但把唯一的解析/写出依赖当成不可修复的黑盒：上游 DWG/DXF 缺陷只能等工作区规避或长期
   `Unsupported`。
2. 用户已放开“允许为 `acadrust` 打补丁”，但要求**每个补丁都被记录**，且不能变成隐形的上游
   分叉。只靠 Cargo.toml 无法表达“为什么改、改了哪些文件、何时能删”。
3. 同时必须防止放宽后的规则被滥用成通用 vendoring：不能给任何其它 crate 打 patch，也不能让
   补丁绕过 `cad-import-acadrust` 的单一依赖边界。

## 2. 决策

### D1 补丁是例外，不是默认

默认仍然使用上游固定版本（当前 `acadrust 0.6.8`，registry `rsproxy`，features `serde` +
`import`）。优先级：升级上游 → 工作区规避/显式 `Unsupported` → 本地补丁。补丁只在无法规避
且必须修复时使用。

### D2 只有 `acadrust` 可以打补丁，且必须固定来源

- 根 `Cargo.toml` 允许的唯一 patch 形状是：
  ```toml
  [patch.crates-io]
  acadrust = { git = "…", rev = "<40-hex>" } # 或仓库内固定路径
  ```
- 不允许其它 patch 表、不允许 `[patch.crates-io]` 中出现其它键、不允许浮动分支。
- workspace 依赖仍钉住一个明确版本字符串（当前 `0.6.8`），补丁不得悄悄改变 API 语义。

### D3 台账是补丁存在的前提

- `docs/acadrust-patches.md` 是唯一权威台账。每行记录：id / upstream version-or-rev / 本地
  commit 或路径 / 被改文件 / 原因 / 行为差异 / 已运行测试 / NOT RUN / 上游 issue 或 PR /
  remove-when。
- **没有台账行就没有补丁**：`scripts/check-architecture.py` 在看到
  `[patch.crates-io] acadrust` 时，要求台账文件存在且正文包含 workspace 解析出的 `acadrust`
  版本字符串；否则门禁失败（fail-closed）。

### D4 依赖边界不变

- `acadrust` 仍然只允许 `cad-import-acadrust` 依赖；补丁不得把平台类型、UI 或 GPU 依赖带进
  `cad-*`。
- 补丁不得把“未支持能力”改成静默成功或假数据；能力缺失仍然显式拒绝/诊断。

### D5 当前不添加补丁

本 ADR 落地时**没有**任何实际补丁，`Cargo.toml` 没有 `[patch]`。实验性 DWG 导出直接使用上游
`acadrust::DwgWriter`；其证据只有合成的 acadrust self-roundtrip，**不是** vendor/AutoCAD 验证。

## 3. 数据契约

- `Cargo.toml`：`workspace.dependencies.acadrust.version` 是“被补丁的上游版本”的唯一来源；
  台账必须包含该字符串。
- `docs/acadrust-patches.md` 表格列固定，不得删除列。
- 门禁错误信息必须点名违规项（非 `acadrust` 的 patch 键、缺失台账、缺失版本字符串）。

## 4. 失败闭合

- 非 `acadrust` patch ⇒ 架构门禁失败。
- `[patch.crates-io] acadrust` 但台账缺失或不含当前版本 ⇒ 架构门禁失败。
- 台账行缺测试/NOT RUN 信息 ⇒ 评审拒绝（人工门禁；脚本无法验证诚实性）。
- 任何“补丁已存在但台账为空”的状态都视为未记录补丁，禁止合入。

## 5. 测试与证据

- `python3 scripts/check-architecture.py`：无 patch 时通过；acadrust patch + 台账通过；
  acadrust patch 无台账失败；非 acadrust patch 失败。**本轮只运行了“无 patch 通过”这一
  实际仓库状态**；后三种 fixture 因仓库内不存在 `scripts/test-check-architecture.py`，为
  **NOT RUN**（不在本轮虚构测试脚本）。
- `cargo test -p cad-import-acadrust --locked`：DWG 导出的合成 self-roundtrip 是现有 DWG
  写出的唯一证据；与补丁无关。
- **NOT VERIFIED**：真实 DWG/AutoCAD/vendor 兼容性；Windows/macOS/Android/Web 的补丁构建；
  性能。
