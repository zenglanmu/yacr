# ADR 0041：保存与导出分离；非公开格式的来源提示

- 状态：**Accepted（2026-11-20）**。**2026-11-20 已实现**：`SavePurpose` 收敛为
  `SaveYmodel`/`SaveExport`/`Plot`；`SAVE` 恒写 `.ymodel` 且无归档载体在开选择器前失败；`EXPORT`
  成为规范命令（从 `PLOT` 别名表移除）；导出侧的源文件覆盖拒绝全部删除，Linux 原子写保留 `.bak`；
  `SpecVisibility` + `ExportCapability.experimental` 与 `cad_status.openedSource` 已接线，前端用
  非模态横幅提示；Android 同步接上 `.ymodel` 保存/打开（SAF 非原子写、`save.no_backup` 明示）。
  证据见 `docs/validation-resources-2026-11-20.md` 与 `docs/validation-save-export-2026-11-21.md`；
  厂商打开我们导出的文件、真实 SAF/provider 行为一律 **NOT RUN**。取代"`.dwg` 源保存时强制另存为新文件 + 保存前确认框"的旧设计
  （`docs/handoff-archive/2026-11.md` Wave 26 条目、`docs/autocad-2026-command-coverage.md` 的
  `SAVE` 行、`save.dwg_confirm.*`）。
- 日期：2026-11-20。
- 相关：[ADR-0037](0037-ymodel-schema-and-persistence.md)（`.ymodel` 是唯一原生格式）、
  [ADR-0033](0033-multi-format-import.md)（连接器）、[ADR-0040](0040-ymodel-embedded-resources.md)
  （保存前询问打包资源）、`docs/compatibility.md` 连接器矩阵、`docs/dgn.md`、
  `docs/3d-solidcore-and-mesh-import.md`、`docs/adr/0031`、`docs/adr/0035`。
- 影响：`cad-ui-core`（命令表/别名）、`cad-platform`（`SavePurpose`）、`cad-host`（保存与导出入口、
  政策）、`apps/app-linux`（选择器标题/过滤器）、`cad-import-core` + 各 import crate
  （`SpecVisibility` / `ExportCapability.experimental`）、`cad-app` + `cad-ui-core`（`cad_status`
  的 `openedSource`）、`apps/desktop-ui`（删除 DWG 确认框、新增导出入口与来源横幅）、i18n、docs。
  **不改变**：`.ymodel` 仍是唯一可直接再打开的格式；载体范围（`.ymodel` 仅 Linux 接线）；
  patch 禁令；连接器写出仍是 L1 + 逐条损失报告。

---

## 0. 背景

旧设计解决的问题是"实验性 DWG 写出器不要覆盖用户的源图纸"，办法是：`.dwg` 源按 SAVE 时
预填 `<stem>-export.dwg`、弹一个确认框、并在 host 侧以 `save.refused_source_path` 拒绝把目标
指回源文件。它有两个后果：

1. **保存语义被格式绑死**：`.ymodel` 才是唯一无损可再打开的格式，但 SAVE 的默认行为由"源扩展名"
   决定，用户要额外理解三个 purpose。
2. **提示出现得太晚**：真正需要提醒的是"这个源格式我们只能靠逆向读"，而这句话应该在**打开时**
   说，而不是在保存时用改名动作表达。

## 1. 决策

### D1 — SAVE 只写 `.ymodel`

`request_save` 不再按源扩展名选目的：**一律 `SaveYmodel`**，默认名 `<stem>.ymodel`
（`<stem>` 取源文件显示名的 stem，无源时取文档标签）。选择器只给 `*.ymodel`。

理由：`.ymodel` 是唯一无损、可再次打开、带完整分片与报告的格式；把 SAVE 绑到源格式，等于让
"保存"承担"导出"的风险。

### D2 — 导出是显式命令

新增 `CommandId::Export`（规范命令名 `EXPORT`）。`EXPORT` 打开**导出**选择器：一个过滤器里包含
所有可写出的扩展名（dxf、dwg、stl、obj、ply、off、coff、ifc、skp、glb、gltf），**目标扩展名
决定写出器**（沿用 `save.rs::export_save` 现有的按扩展名分派）。

同时把 `EXPORT` 从 `PLOT` 的别名表里移除：矢量出图仍由 `PLOT`/`PRINT` 负责，`EXPORT` 回到
AutoCAD 的语义（导出为其它格式）。`docs/shortcuts.md` §2b 与覆盖表同步。

> **2026-11-23 增补**：`EXPORT <TOKEN>` 显式格式选择、裸 `EXPORT` 的源格式默认名与
> `export.format_mismatch` 见 [ADR-0052](0052-export-format-selection.md)。D2 的“目标扩展名决定
> 写出器”不变；D6 的“不做导出预设”不包含单一格式的显式导出入口。

`SavePurpose` 收敛为三个：`SaveYmodel`、`SaveExport`、`Plot`。旧 `SaveDwg` 变体删除——
DWG 只是 `SaveExport` 里的一种扩展名。

### D3 — 取消"强制改名"

删除：`default_dwg_export_name`（`<stem>-export.dwg`）、`save.dwg_confirm.*` 确认框及其 UI 门、
`save.picker_waiting_dwg`、`.dwg` 专用的 purpose 分支。`useDwgSaveConfirm` 只保留"保存前询问是否
打包外部资源"（ADR-0040）这一道门。

### D4 — 允许覆盖源文件，靠 `.bak` 兜底

`export_save` 删除两处"目标等于源就拒绝"的判断（平台身份守卫与非 DXF 的归一化路径兜底）。
覆盖是用户的显式选择，安全性由既有的原子写保证：

- Linux 的 `atomic_replace` 已在 `keep_backup = true` 下先做 `.bak`（硬链接，失败回退复制），
  对**所有**经 `write_atomic` 的写出生效，不只是 `.ymodel`；
- 这一条必须有契约测试钉住（导出覆盖已存在文件 → 出现 `<name>.<ext>.bak`，且原内容在 `.bak` 里）。

非 Linux 载体：`.ymodel` 仍未接线；导出写出的备份语义按各载体既有实现，未实现者必须显式拒绝
而不是静默写。

### D5 — 非公开格式在**打开时**提示

格式元数据（唯一来源，`cad-import-core`）：

```rust
pub enum SpecVisibility { Public, ReverseEngineered }
// ConnectorDescriptor 增:
pub spec: SpecVisibility,
// ExportCapability 增:
pub experimental: bool,
```

分类（依仓库自身证据）：

| 连接器 | spec | 写出 |
|---|---|---|
| DWG | `ReverseEngineered` | 有，`experimental: true`（acadrust，未对 AutoCAD/ODA/BricsCAD 验证） |
| DGN | `ReverseEngineered` | 无（`export.dgn.not_implemented`） |
| SKP | `ReverseEngineered` | 有，`experimental: true`（skpwright 容器，未对 SketchUp 验证） |
| RVT | `ReverseEngineered` | 无（`export.rvt.not_implemented`） |
| DXF / STEP / IFC / glTF / STL / OBJ / PLY / OFF | `Public` | 按各自连接器（STEP 无写出器） |

`cad_status` 增 `openedSource: Option<{ connector, spec, canWrite, experimental }>`，
仅在**打开成功且连接器是 `ReverseEngineered`** 时出现。前端用**非模态、可关闭**横幅
（`UpdateNotice` 范式，`role="status"`，不抢焦点）展示：

1. 这句格式不是公开规范，靠逆向读取，未使用官方 SDK；
2. 按该格式导出**可能无法被原软件正确打开**，会丢失本软件尚未建模的信息；
3. **能不能回存**：`experimental` → "写回是实验性的"；无写出器 → "不支持写回，SAVE 会存为
   `.ymodel`"；
4. 建议保存为 `.ymodel`。

每格式一句具体说明（`open.notice.detail.{dwg,dgn,skp,rvt}`），不共用一句泛泛的免责声明。
提示**不阻止**打开，也不在保存时重复弹出。

### D6 — 不做

- 不做导出预设/批量导出/`EXPORTOFFICE`/`WBLOCK` 真实现。（单一格式的显式 `EXPORT <TOKEN>`
  入口由 ADR-0052 增补，不属于“预设”。）
- 不做 L0 源字节直通；不做"导出后回写源格式并保持原文件其余部分不变"。
- 不改 `.ymodel` 的载体接线范围（仍只有 Linux）。
- 不做格式授权/版权校验。

## 2. 验证计划

| 层 | 证据 |
|---|---|
| SAVE | 任意源（dwg/dxf/skp/无源）SAVE → purpose 恒为 `SaveYmodel`、默认名 `<stem>.ymodel`、只给 `.ymodel` 过滤器 |
| EXPORT | `EXPORT` 是规范命令、不再映射 `PLOT`；`PLOT`/`PRINT` 不变；导出选择器含 11 个扩展名且按扩展名分派写出器 |
| 覆盖 | 导出目标 = 已存在文件（含被打开的源文件）→ 写入成功且 `.bak` 含旧字节 |
| 提示 | 四个逆向连接器 → `openedSource` 出现且 `spec=reverse_engineered`；DXF/STEP/IFC/glTF/网格 → 字段为 `None`；横幅文案逐格式解析、无硬编码 CJK |
| 命令文档 | `check-command-docs.py` 与 `test-check-command-docs.py` 通过（§2b 别名表同步） |
| 未运行 | 真实 AutoCAD/MicroStation/SketchUp/Revit 打开我们导出的文件 **NOT RUN**；真实覆盖后人工核对 `.bak` **NOT RUN** |

## 3. 证据边界

"可能不兼容原文件、会丢信息"是**基于仓库已有证据的保守表述**，不是逐格式实测结论：
`docs/compatibility.md` 的 vendor 行均为 `NOT VERIFIED`。本轮只做代码路径、文案与契约测试，
不新增任何厂商接受性结论。
