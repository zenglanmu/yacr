# ADR-0043 P0–P3 验证记录（2026-11-21）

- 范围：
  - **P0**：参数表 + 元素 `params` 块 + `num/ref/四则/pi` + 三个不改变拓扑的槽位 +
    `PARAM` 结构命令 + `SETENTITYPROPERTY param.<name>` + 合并写入/撤销 + 面板动态行。
  - **P1**：`neg/pow/fn/if` 表达式、`tables/configurations.json`（稀疏覆盖 + 抑制 + 父链）、
    `PARAM CONFIG` 运行时选择、`paramList` host-read kind。`Insert.scale` 与文本模板**未实现**。
  - **P2**：`tables/features.json`（元素 id + `order` + 操作枚举 + 输入 + 参数绑定）与
    `voids`/`fills` 具名 kind。**求值器未接线**：`rel.not_implemented`；传播 `rel.no_rule`。
  - **P3**：`tables/relations.json`（typed face/edge/vertex + 状态机）。**没有几何传播**。
- 环境：Linux 开发宿主，Rust `1.99.1`（`rust-toolchain.toml`），`--locked`；Node 前端测试。
- **性质：库层/命令层契约与单元测试、编译与静态门禁、无 GPU 的 headless 求值。**
  不是 GPU、像素、性能、真机或真实宿主运行验收。

## 1. 通过的验证（逐条实际运行）

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-domain --locked` | 318 passed |
| `cargo test -p cad-db --locked` | 493 passed |
| `cargo test -p cad-history --locked` | 43 passed |
| `cargo test -p cad-store-zip --locked` | 全部 suite passed（参数 11、raw-copy 2） |
| `cargo test -p cad-ui-core --locked` | 852 passed |
| `cargo test -p cad-host --locked` | 450 passed |
| `cargo test -p cad-app --locked -- --test-threads=1` | 3171 passed |
| `cargo test -p cad-cli-tools --locked` | 76 passed（含 headless 求值） |
| `cargo check --workspace --all-targets --locked` | exit 0 |
| `cargo check --workspace --lib --target wasm32-unknown-unknown --locked` | exit 0 |
| `cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings` | exit 0 |
| `cargo fmt --all -- --check` | exit 0 |
| `python3 scripts/check-architecture.py` | exit 0（44 packages） |
| `python3 scripts/check-file-size.py` | exit 0（所有文件 < 1000 行） |
| `python3 scripts/check-i18n.py` | exit 0 |
| `python3 scripts/check-command-docs.py` | exit 0（0 mismatches） |
| `python3 scripts/check-fixture-manifest.py` | exit 0（48 fixtures） |
| `(cd apps/desktop-ui && npm test)` | 1923 passed |

另：app crate 的 `param` 过滤测试、store 的 `parameters` / `parameters_raw_copy`、db 的
`configuration` / `feature` / `relation`、host 的 `param_list` 与各 carrier `host_read` 契约测试
均单独跑过并通过。

## 2. 覆盖到的契约

### P0
- 格式：元素 `params` 往返、空/缺省省略、`"params": null` 拒绝、未知槽位拒绝、参数表严格解码、
  重复名/`seq` 拒绝、悬空 `ref`/环拒绝、4096 行与 8 MiB 预算拒绝。
- 图：`ref` 成环 = `param.cycle` 且事务拒绝；改一个参数只重算闭包；三个圆中只有两个绑定 →
  第三个几何字节不变；同一参数多元素同时更新。
- 写入：一次命令 = 一个 revision = 一条 undo；`ChangeScope` 标 `TableKind::Parameters` + 每个被重写
  shard；带 previous archive **保存两次**后重开仍是新几何，未编辑 shard 不重写，`document.json` 不变；
  空批不 bump。
- 失败：几何构造失败参数已提交、几何保持旧值、`param.blocked` 可重现；未知引用/未知槽位/单位不符
  整单拒绝，不落库；锁定层 `layer.locked`。
- minor：有参数写 8、无参数保持 4、有 BIM 无参数仍是 7、minor-7 档案可打开。
- 面板：`param.<name>` 动态行可编辑；部分绑定的多选不出现该行，陈旧写入按 `param.mixed` 整单拒绝；
  非绑定元素无参数行。
- 撤销：值编辑由纯函数重算；**bind 的 undo 恢复精确的绑定前几何**，redo 恢复派生几何。
- 打开：存储几何与派生不一致只发 `param.recomputed`，构造失败发 `param.blocked`，不修改文档。

### P1
- 表达式：`neg/pow/fn/if` 求值与量纲（`sqrt`/`pow` 只接受无量纲；三角接受 number/angle；
  `if` 只求值选中分支；非有限/负 sqrt/除零拒绝）；未知 `fn`、缺 `exp`/`arg`、多余 tag 解码拒绝。
- 配置：覆盖/父链/子覆盖/抑制；未知父、环、覆盖或抑制不存在的 `seq` 拒绝；抑制破坏其它表达式拒绝；
  活动配置不落盘；`PARAM CONFIG` 一 revision 且无 undo；`param.<name>` 在覆盖/抑制时
  `param.config_override`；绑定到被抑制参数 → `param.suppressed` 保留 last-good。
- `paramList`：wire kind、session/host 回答、无文档空表、未知 kind 拒绝、无 seam
  `unsupported-runtime: paramList`。
- `Insert.scale`：`param.no_evaluator`（明确未实现）。

### P2/P3（schema only）
- 特征：严格编解码、`order` 唯一、操作枚举、输入/元素存在、无自输入、槽位可解析、预算；
  `evaluate_feature` 对所有操作 `rel.not_implemented`，不 bump、不伪造几何。
- `voids`/`fills`：token/parse、D1 Reference、`relation_propagation == NoRule`、IFC 按 kind 计未写出。
- in-context：严格编解码、重复 `(source,target,reference)` 拒绝、自环拒绝、状态机合法/非法转移、
  `Dangling` 终态、`update_relation_state` 一 revision + `TableKind::Relations` scope。
- minor：feature 表 = 10、relations 表 = 11；参数-only 仍是 8，配置/P1 表达式 = 9。

## 3. 评审驱动的实现修正（同日）

1. **量纲代数**：`number` 参数按无量纲处理；`Div` 只允许同量纲相除为无量纲；`pow/sqrt` 只接受
   无量纲操作数。
2. **锁定层边界**：几何与绑定写入都走 `layer.locked`。
3. **bind/unbind undo**：`ParamBindingPatch` 携带 `before_geometry`/`after_geometry`，undo 精确恢复。
4. **历史步校验**：参数步要求 `record.patches` 为空（几何由绑定 patch 携带），避免 replay 静默丢弃。
5. **静态 vs blocked**：未知引用/槽位/单位整单拒绝；只有 `param.geometry_failed` 保留 last-good。
6. **打开重算**：只比较与诊断，不修改（存储行是上一次成功求值结果）。
7. **预算/标记**：参数预算用 `param.budget_exceeded`；载入绑定数超限也拒绝；空 `params` 不写、
   不抬 minor。
8. **配置语义**（ADR 未定，采用并记录）：活动配置是运行时状态；`PARAM CONFIG` 不记 undo；
   值编辑在覆盖/抑制时拒绝；父抑制 + 子覆盖 = 子覆盖重新启用（status 不再同时标 suppressed）。
9. **引用完整性（第二轮评审）**：参数写入与保存都会重验全部配置；删除实体前拒绝 feature id/input 与
   relation endpoint 引用，并剪掉该实体的参数绑定/载入诊断；删除被 feature 绑定的参数被拒绝；
   feature/relation 编码前重跑文档门禁，避免“保存成功、重开失败”。

## 4. NOT RUN / NOT VERIFIED

- 真实 GPU、像素、帧率或性能结论：**NOT RUN**。
- Linux 真实宿主（Wayland/X11）打开含参数/配置/特征/关系文档并目视验证：**NOT RUN**。
- Windows / macOS / Android / Web 运行时：**NOT VERIFIED**（仅 workspace/wasm 编译与契约测试；
  app-web 的 `HostReadAnswer` 两变体编译修复未在浏览器运行）。
- 跨机自包含、字体度量、恢复快照与参数文档的交互：**NOT RUN**。
- P2 特征求值（solidcore）、`(host,hosted)` 传播、P3 几何 in-context 传播、
  `cad-dependencies` 统一调度：**未实现**（`rel.not_implemented` / `rel.no_rule`）。
- `PARAM` 结构命令只有命令层：没有 payload producer、IPC 或 UI 路径；生产可达的写入是
  `SETENTITYPROPERTY param.<name>`。配置选择（`active_configuration`）因此也只经测试可达。
- 打开诊断（`param.recomputed`/`param.blocked`）保存在 `cad-db` 的 `param_diagnostics()`，**没有宿主/UI
  消费者**；不得写成“用户会看到诊断”。
- `if` 的**数值求值**只执行被选中的分支，但量纲/引用检查会遍历两个分支（未选中的悬空引用仍拒绝）；
  “只求值选中分支”仅指数值求值。
- `Insert.scale`、文本模板、文档级参数面板的 React 消费者：**未实现**。
- 约束求解器、配合/装配求解、特征历史 rollback、跨文档外部参考：**非目标**（ADR-0043 D14）。
