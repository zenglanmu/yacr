# acadrust 补丁台账（`[patch]` policy）

> **当前状态（2026-11-09 复核）：没有补丁。** 上游 `acadrust` **0.6.8**（`rsproxy` registry，
> features `serde` + `import`）未被修改，`Cargo.toml` 中没有 `[patch]`。实验性 DWG 导出使用的是
> 上游已有的 `acadrust::DwgWriter`，不是本地补丁。

> 2026-11-09：本轮确认补丁**允许路径**已放宽为“只允许 `acadrust` + 固定版本/rev + 台账登记”，但**不是**强制打补丁；本轮未修改上游，台账仍为空。任何未来补丁必须先在本文件登记再进入 `Cargo.toml`。

## 1. 目的

`acadrust` 是 `cad-import-acadrust` 唯一的解析/写出依赖（AGENTS.md 铁律 1）。上游 bug 或缺失
能力（例如 DWG 写出缺陷）有时只能通过本地补丁解决。本台账让每一个补丁都**可追溯、可复现、
可移除**，并保证“改了上游”这件事不会被静默忘记。

补丁不是默认路径：先找上游版本升级、可行的工作区规避、或显式 `Unsupported` 建模。只有三者
都不成立时才允许打补丁。

## 2. 策略（`scripts/check-architecture.py` 强制）

1. **只允许 `acadrust`**：根 `Cargo.toml` 的 patch 表只能是 `[patch.crates-io]`，且其唯一键
   必须是 `acadrust`。任何其它 crate 的 patch 或任何其它 patch 表一律拒绝。
2. **固定版本/revision**：workspace 依赖仍然钉住一个明确的 `acadrust` 版本（当前 `0.6.8`）；
   补丁源必须是固定的 git revision 或仓库内固定路径，不跟随浮动分支。
3. **必须先登记**：存在 `[patch.crates-io] acadrust` 时，`docs/acadrust-patches.md` 必须存在，
   且正文必须包含 workspace 解析出的 `acadrust` 版本字符串。缺失台账或不含该版本 ⇒ 架构门禁
   失败。
4. **一行一个补丁**：每个补丁在下方台账中占一行，所有列都要填写；“NOT RUN”如实写 NOT RUN。
5. **不做第二套 API**：业务层仍然只通过 `cad-import-acadrust` 的适配层使用 acadrust；“acadrust
   只允许 `cad-import-acadrust` 依赖”的规则不变。
6. **随时可移除**：每行必须写明“remove when”条件；上游修复发布后优先删除补丁并升级版本。

## 3. 台账

| id | upstream version / rev | local commit or path | files touched | why | behaviour delta | tests run | NOT RUN | upstream issue / PR | remove when |
|---|---|---|---|---|---|---|---|---|---|
| — | — | — | — | *（当前没有补丁）* | — | — | — | — | — |

## 4. 如何新增一个补丁

1. **先证明必要性**：记录上游版本、复现步骤、失败用例（最小夹具/测试名），并确认升级上游或
   工作区规避不可行。
2. **固定来源**：优先 `acadrust = { git = "…", rev = "<40-hex>" }` 或仓库内固定目录；把上游
   版本/revision 写进台账。禁止浮动分支、禁止 `path` 指向未提交的本地修改。
3. **最小改动**：补丁只修目标问题，不夹带无关重构；记录每个被改文件与行为差异。
4. **在 `Cargo.toml` 根加 patch**（示例，当前**不要**添加）：
   ```toml
   [patch.crates-io]
   acadrust = { git = "https://github.com/<owner>/acadrust", rev = "<40-hex>" }
   ```
   同时在 `docs/acadrust-patches.md` 台账新增一行（含 `0.6.8` / 新版本字符串，门禁会检查）。
5. **补测试与证据**：补丁必须有契约测试（优先在 `cad-import-acadrust` 内，用合成夹具）；跑
   `cargo test -p cad-import-acadrust --locked`，并如实登记未运行项。
6. **跑门禁**：`python3 scripts/check-architecture.py`、`cargo check --workspace --all-targets
   --locked`、`cargo fmt --all -- --check`，以及改动 crate 的 clippy。
7. **写上游 issue/PR**（若可提交），填入台账；上游合并后按“remove when”删除补丁并升级。

## 5. 边界

- 台账不是兼容性声明：补丁测试通过只证明该补丁的合成用例；真实 DWG/AutoCAD 兼容性仍然是
  **NOT VERIFIED**，除非有明确的 vendor/AutoCAD 证据。
- 不得用补丁把“未支持能力”改成“空成功”或假数据（AGENTS.md 铁律 2）。
- 不得通过 patch 机制把平台/UI 依赖塞进 `cad-*`，也不得绕过 `cad-import-acadrust` 的单一
  依赖边界。

> **2026-11-12（Wave 26）**：本轮未使用 `[patch.crates-io]`，台账仍为空。DWG 保存仍走上游
> `acadrust::DwgWriter`，行为是“另存新文件、不覆盖源图、保存前确认”；若以后为 DWG 往返质量
> 打补丁，按 §4 固定 revision 并先在本台账登记，再用 `scripts/check-architecture.py` 门禁。
