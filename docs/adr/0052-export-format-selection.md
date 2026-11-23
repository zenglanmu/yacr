# ADR 0052：导出的显式格式选择与源格式默认名

- 状态：**Accepted（2026-11-23）**。增补 [ADR-0041](0041-save-export-split-and-format-provenance.md)
  的 D2/D6：D2 的“目标扩展名决定写出器”不变，新增一个**带参数的选择子**；D6 的“不做导出预设”
  仍然成立（这里做的是**单一格式的显式导出入口**，不是批量/预设）。
- 日期：2026-11-23。
- 相关：[ADR-0037](0037-ymodel-schema-and-persistence.md)（`.ymodel` 唯一原生格式）、
  [ADR-0033](0033-multi-format-import.md)（连接器）、[ADR-0048](0048-format-connector-crates.md)
  （连接器 crate 拆分）、`docs/compatibility.md` 连接器矩阵。
- 影响：`cad-host`（`policy`/`save`/`core`）、`cad-ui-core`（参数选择子与语法）、
  `apps/desktop-ui`（File ribbon 的导出格式子菜单、`ARGUMENT_COMMANDS`）、i18n、docs。
  **不改变**：`SAVE` 恒写 `.ymodel`；扩展名仍是写出的最终权威；连接器写出仍是 L1 + 逐条损失报告；
  `.ymodel` 的载体接线范围；`patch` 禁令。

---

## 0. 背景

ADR-0041 D2 规定 `EXPORT` 打开一个包含所有可写扩展名的选择器，**目标扩展名决定写出器**。实现完全
遵守了这条规则，但用户侧只有一个无参命令：

- 选择器只有一个“所有导出格式”过滤器，建议名固定为 `<stem>.dxf`（`cad-host::save::request_export`）；
- File ribbon 只有单个 `EXPORT` 按钮，没有任何格式入口；
- `dom-canvas` 的 Web 载体没有对话框，目标名就是建议名，因此永远落 `.dxf`。

结果是：**打开 `.glb` 后按 `EXPORT`，建议名仍是 `.dxf`**；用户必须先手打扩展名才能拿到 glTF。
这看起来像“导出只支持 DXF”，但连接器侧 DXF/DWG/glTF/STL/OBJ/PLY/OFF/IFC/SKP 的写出器其实都已
接线（STEP/DGN/RVT 才是稳定拒绝）。本 ADR 只修**表达与默认值**，不新增任何写出器。

## 1. 决策

### D1 — `EXPORT` 保持规范命令，新增 typed 参数形式

- 裸 `EXPORT`：行为不变，打开全格式选择器。
- `EXPORT <TOKEN>`：`EXPORT` 的**参数形式**，不是新命令。它复用 `ARGUMENT_SELECTORS` +
  `CadSession::argument_command` 的既有通道（同 `TABLEEXPORT <path>`、`HATCH PATTERN <name>`），
  不新增 `CommandId`、不新增第二 IPC、不新增 `CommandPayload` 变体。

Token（大小写不敏感）→ 写出器：

| token | `format_id` | 默认名扩展名 |
|---|---|---|
| `DXF` | `dxf` | `dxf` |
| `DWG` | `dwg` | `dwg` |
| `STL` | `stl` | `stl` |
| `OBJ` | `obj` | `obj` |
| `PLY` | `ply` | `ply` |
| `OFF` | `off` | `off` |
| `COFF` | `off` | `coff` |
| `IFC` | `ifc` | `ifc` |
| `SKP` | `skp` | `skp` |
| `GLB` | `glb` | `glb` |
| `GLTF` | `gltf` | `gltf` |

`COFF` 只存在于语法，不出现在 ribbon 菜单行（`.coff` 由裸 `EXPORT` 的选择器覆盖）。
`STLASCII` 故意没有 token：ASCII STL 没有自己的扩展名，目标扩展名仍是唯一入口。

### D2 — 裸 `EXPORT` 的建议名由源格式派生

建议名的扩展名取**打开来源**的扩展名，但只限于**非实验性**的写回格式：

- `.dxf`/`.ifc`/`.gltf`/`.glb`/`.stl`/`.obj`/`.ply`/`.off`/`.coff` 源 → 同名扩展名；
- `.dwg`、`.skp` 源 → **`.dxf`**：这两种写出器是实验性的（ADR-0041 D5 的
  `ExportCapability.experimental`），默认把用户引向“覆盖源文件并写出实验性格式”不合适；
- `.ymodel`、无源（空白图）、`.step`/`.dgn`/`.rvt`（只读）→ `.dxf`。

名字仍是 `<stem>.<ext>`（`policy::default_save_name`）。**不恢复** ADR-0041 D3 删除的
`<stem>-export.<ext>` 强制改名：覆盖是用户在选择器里的显式动作，安全性由既有原子写 `.bak` 承担。

### D3 — 扩展名仍是最终权威；强制格式不符则显式拒绝

所有载体都保持一个“所有导出格式”的选择器，不引入按格式的 filter / preselected type API
（`PlatformFiles::begin_save` 继续只接收 `purpose` + 建议名）。

`EXPORT <TOKEN>` 的目标扩展名通常就是 token 的默认名扩展名。用户若在选择器里改名成别的扩展名：

- 分类结果与强制 `format_id` **一致** → 正常写出；
- **不一致** → 写出前拒绝
  `export.format_mismatch: requested <id>, target .<ext> is <chosen>`；不写任何字节、不落
  `tables/compat.json` 导出记录。
- 裸 `EXPORT` 没有强制格式，行为和今天一致：扩展名决定。

`.coff` 与 `.off` 都分类到 `off`，所以 `EXPORT OFF`/`EXPORT COFF` 对这两个扩展名都匹配。

### D4 — 载体边界

| 载体 | 行为 |
|---|---|
| `native-below`（Linux Wayland / Windows / macOS / Android） | 全部可写格式；选择器由各载体既有实现提供，建议名携带目标扩展名。真实对话框交互只在 Linux 有运行证据 |
| `dom-canvas` Web（slim bundle） | 只编译 DWG/DXF 写出器；`EXPORT GLB` 等按 `unsupported-runtime: export.<id>` 显式拒绝（经 `save.export_failed` 呈现），**不得**静默换格式。Web 无对话框，目标名即建议名 |
| `dom-canvas` Linux X11/XWayland（full bundle） | 与 native 同一格式集；仍无对话框，目标名即建议名 |

**菜单不按载体隐藏。** 共享层目前没有“本载体已编译哪些写出器”的 wire 枚举（`cad_commands` 只发布
名字），而显式拒绝是既有合同（AGENTS 铁律 2/8）。如果以后加 `cad_status` 的导出格式枚举，再让
ribbon 变体按可用性禁用；本 ADR 不新增这个字段。

### D5 — 不做

- 不做按格式的选择器过滤器 / `SavePurpose` 携带格式（避免五个载体 API 同时扩张）；
- 不做批量导出、导出预设、`EXPORTOFFICE`/`WBLOCK` 真实现；
- 不做导出报告弹窗（报告仍只落 `tables/compat.json` 与状态行）；
- 不改 `SAVE`（恒 `.ymodel`），不改 `.ymodel` 的载体接线范围。

## 2. 验证计划

| 层 | 证据 |
|---|---|
| 语法 | `EXPORT GLB` 被 `ARGUMENT_SELECTORS` 与前端 `ARGUMENT_COMMANDS.EXPORT` 同时接纳；`EXPORT ZIP` 被拒绝；不新增 `CommandId` |
| 默认名 | 源 `.glb`/`.ifc`/`.obj` → 建议同名扩展名；源 `.dwg`/`.skp`、`.ymodel`、空白 → `.dxf` |
| 强制格式 | `EXPORT GLB` + `.glb` 目标走 glTF 写出器；`EXPORT GLB` + `.dxf` 目标在写前以 `export.format_mismatch` 拒绝 |
| UI | File ribbon 的 `EXPORT` 是 11 行 split button（裸 + 10 个格式），行文本即命令文本；i18n 键集一致 |
| 文档门禁 | `check-command-docs.py` / `check-i18n.py` / `check-file-size.py` 通过 |
| 未运行 | Windows/macOS/Android 的真实对话框与文件系统行为 **NOT VERIFIED**；vendor 打开我们导出的文件 **NOT RUN**；Web 只用软件渲染/无头浏览器证据 |

## 3. 证据边界

本轮只做代码路径、语法、契约测试与文案。“某个格式的导出能被原软件打开”仍然只有各连接器自己的
自洽往返证据；DWG/SKP 写出仍是实验性。Web 的 slim 构建只能证明 `unsupported-runtime: export.<id>`
是显式拒绝，不是该格式在浏览器里可用。
