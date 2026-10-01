# 诊断与资源计量（cad-diagnostics / cad-resources）

本文记录 F11（诊断/报告完整性）与 F10（资源计量）在两个 crate 内的实现契约、
稳定机器码、验证边界与仍开放项。规范权威：`CAD_IMPLEMENTATION_SPEC.md` v2.0
§8.4、§11.2、§11.4、§19；审计条目见 `docs/code-audit-and-agent-handoff.md`
的 F10/F11、B20、B27、B30、U08。

## 1. 对象级诊断模型（F11）

审计 F11/U08：消费方只取首条诊断，表示缺失不汇总，于是含 HATCH/TEXT 的图纸
仍可能被报成完整。`cad-diagnostics::model` 提供结构化、对象级、聚合全部原因的模型：

- `DiagnosticReason { code, severity, parameters, completeness }`：单条原因。
  `code` 是稳定机器码；`parameters` 是类型化、locale 无关的参数（对象 ID、
  逻辑 key、标识符、计数、字节、限额、深度）。核心不本地化文本（N01 要求
  表现层本地化），原因里只有码与参数。
- `ObjectDiagnostics`：一个对象的**全部**原因，`completeness()` 取最弱判定，
  `severity()` 取最高严重度。
- `DiagnosticsModel`：按对象分组 + 文档级原因。`completeness()` 与 `summary()`
  跨全部对象聚合，`DiagnosticsSummary.complete` 只有在每个对象都 `Complete`
  且无原因时才为真；否则区分 `partial`/`missing`/`unverified` 计数。
- `DiagnosticPackage::encode_model_redacted` 输出含全部原因的脱敏 JSON，
  `summary` 明确区分“完整”与“部分/缺失”。

### 稳定码（schema 键，禁止在无迁移时重命名）

| 码 | 含义 |
|---|---|
| `representation.unavailable` | 对象无法构建显示表示（Missing） |
| `representation.approximate` | 表示已构建但不精确（Partial） |
| `representation.not_implemented` | 语义已读，输出未实现（Unverified） |
| `resource.missing` | 依赖资源不可用 |
| `resource.over_budget` | 资源超出预算 |
| `resource.recursion_limit` | 引用递归超限 |
| `resource.font_unresolved` | 字体名在目录中解析不到 |
| `resource.font_unsupported` | 字体已解析但技术不受支持 |
| `import.unknown_entity` | 导入器遇到无法建模的实体类型 |
| `proxy.undecoded` | 代理记录无法解码 |

## 2. 资源计量（F10）

审计 F10/B27：`ResourceLimits` 只有单项上限，没有运行总量；`max_xref_depth`、
`max_image_pixels`、`font_cache_bytes` 声明了却没接线，超限可能静默丢弃。
规范 §11.2 要求“限制值可配置并写入诊断，不能静默截断后报告完整”。

### 2.1 cad-resources

- `ResourceLimits` 增加 `total_bytes`（运行总量）。`MapResolver::grant` 现在
  同时校验单项 `max_bytes` 与总量 `total_bytes`，超限返回结构化
  `ResourceIssue`（`resource.over_budget` + `ResourceBudget::{PerResource,TotalBytes}`），
  而不是静默丢弃；被拒的授权不消耗预算。`used_bytes()` 暴露当前占用。
- `ResourceLimits::check_image_pixels` / `check_xref_depth`：在分配前显式判定
  `image_pixels` 与 `xref_depth`，返回 `ResourceIssue`。
- `ResourceIssue` 是 locale 无关的结构化原因（码、类别、预算名、实测值、限额、
  逻辑 key），便于 cad-diagnostics 聚合。
- `ResourceKind` 补齐 `Debug/Clone/Copy/PartialOrd/Ord/Hash` 与 `as_str()`。

### 2.2 cad-diagnostics

`cad-diagnostics::budget` 提供跨模块可复用的总量 + 分类 + 递归预算
（`ResourceBudget`、`BudgetCategory`）：`charge()` 全有或全无，超限返回
`resource.over_budget` 原因；`check_depth()` 返回 `resource.recursion_limit`。
整数运算、无平台类型，可在原生 / Wasm / CLI 复用。

## 3. 能力表修正（F10）

审计指出“能力表错误/类别被误报为支持”。旧模型只有 `ResourceKind` 枚举，隐含
所有类别都受支持。`cad-resources::resource_capabilities()` 显式声明每个类别
`resolve`（本 crate 能否给出逻辑取数 URL）与 `decode`（核心能否把字节变成
字形/几何）：

| 类别 | resolve | decode |
|---|---|---|
| `font_ttf` | verified | unverified（有 TTF/OTF/WOFF1 轮廓，但无授权字体实证） |
| `font_shx` | verified | partial（SHX 可解析，bigfont/编码覆盖未验证） |
| `big_font` | unverified | not_implemented |
| `image` | not_implemented | not_implemented |
| `external_reference` | not_implemented | not_implemented |

`plan_fonts_report()` 取代静默丢弃：未解析名字进入 `unresolved`、不受支持技术
（如 `woff2`，解析为 `FontKind::Other`）进入 `unsupported`，并各自产出
`ResourceIssue`；`is_complete()` 反映是否所有请求都得到处理。旧 `plan_fonts()`
签名与“先取后由整形引擎拒绝”的行为保持不变，改动是纯增量。

“`resource_keys` 永远空”一条已由合并的 `cad-platform::fonts::requested_fonts`
（只读收集 TEXT `font` 与样式 `resource_keys`，含 Compound 递归）解决，本轮
不重复实现，仅确认其存在并有 `crates/cad-platform/src/fonts.rs` 测试
`collects_font_and_style_resource_keys`。

## 4. 验证

命令见仓库 AGENTS.md 与 `docs/validation.md`。本轮新增测试覆盖：

- `cad-diagnostics::model`：聚合全部原因、最弱完整性、完整/部分/缺失摘要、
  编码保留码与参数。
- `cad-diagnostics::budget`：总量拒收而非静默丢弃、分类上限、递归深度。
- `cad-resources`：运行总量、单项/像素预算、xref 递归、能力表不误报支持、
  字体计划报告区分 unresolved/unsupported。

## 5. 仍开放（OPEN，不在本轮 crate 范围）

以下项审计已列出，但实现位于本任务范围之外（importer/proxy/UI/representation），
**未完成、未验收**，不得据本文声称闭环：

1. **能力表的 importer 侧错误**（F11/B20）：`note_capability` 的 `read` 恒为
   `Verified`、render/pick 复制 semantic、`model_render` 未汇总实体完整性。
   位于 `crates/cad-import-acadrust`，本轮不可改 → OPEN。
2. **UI 只取首条诊断**（U08）：状态栏/诊断抽屉接线在 `cad-ui-slint`/`apps/**`，
   → OPEN。本 crate 已提供可消费的全量聚合模型。
3. **表示缺失不汇总**（B20/F11）：应读取/语义/表示/场景/GPU/拾取分别判定，
   位于 importer/representation/CLI → OPEN。
4. **代理线面/状态栈与对象级缺图报告**（F11/B21）：位于 `cad-proxy`/
   `cad-import-acadrust` → OPEN。
5. **资源包 UI、外参/图片显示、SHX/BigFont 真正解码、字体图集**（F10）：
   跨 representation/宿主 → OPEN；本 crate 只提供真实能力表与预算契约。
6. **CLI/诊断包共享结构化输出与原子导出**（B30）：位于 `cad-cli-tools`，
   → OPEN；核心结构化模型已就绪，待 CLI 接线。
7. 真实授权字体/图纸/厂商样本与浏览器/真机验收：**未运行**，无证据。
