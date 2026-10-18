# 交互工具状态机与撤销/重做可用状态

本文件记录 `cad-app` 内与 U04（工具状态机/预览/取消）和 U11（撤销/重做）
相关的接线契约。规范权威仍是 `CAD_IMPLEMENTATION_SPEC.md` v2.0；
需求追踪见 `docs/requirements.md`（F06/F08）与
`docs/code-audit-and-agent-handoff.md`（F06/U04/U11）。

## 1. 测量工具（F06 / U04）

`crates/cad-app/src/measure_tool.rs` 提供真正的测量工具状态机：

- `MeasurementToolKind`：`Distance` / `PolylineLength` / `Angle` / `Area`。
  - `algorithm()` 显式给出对应 `MeasurementAlgorithm`，**不再按点数猜测**。
  - `exact_points()`：距离=2、角度=3；折线长度与面积是开放端点（≥2 / ≥3）。
  - `auto_completes()`：距离/角度在点数足够时自动完成；折线/面积需显式确认。
- `MeasurementTool`：跨多条命令累积点、维护 `cursor` 预览，`preview()`
  返回 `MeasurementPreview { kind, points, cursor, remaining, ready }`。
- `ToolState::Measuring(MeasurementTool)`：会话持有活动工具。

命令入口（`Application::execute`）：

| 命令 | 载荷 | 行为 |
|---|---|---|
| `CommandId::Measure` | `MeasureTool(kind)` | 启动/重启指定算法工具，返回预览诊断 `measure.preview` |
| `CommandId::Measure` | `Points(points)` | 向活动工具追加点；自动完成的工具在此提交并回到 `Idle` |
| `CommandId::Measure` | `None` | 启动默认「距离」工具（兼容旧 UI 只发 None），不再直接报错 |
| `CommandId::ConfirmMeasurement` | `None` | 确认开放端点工具（折线/面积）；点数不足返回 `InvalidInput`，状态保留 |
| `CommandId::CancelMeasurement` | `None` | 取消工具；**不产生任何事务** |
| `CommandId::Measure` | `Points(points)`（无活动工具） | 保留旧的统计推断路径，供 CLI/直接调用方使用 |

结构化结果：`CommandOutcome` 新增 `measurement: Option<MeasurementRecord>`，
提交成功时携带引擎返回的 `MeasurementRecord`（algorithm/inputs/plane/value/
units/source/precision），而不是只给格式化字符串。

面积测量使用**视口工作平面**（`MeasurementSpace::Plane`），因此非共面点会被
引擎拒绝（审计 B24），不会被静默压平；距离/折线/角度仍走 `World3d`。

取消语义：测量是只读操作，`CancelMeasurement` 只调用 `SessionState::cancel_tool`，
不写库、不记历史，因此取消恒为「零事务」（规格 §4.10）。
预览更新 `set_measurement_cursor` 是纯状态更新，不发命令。

### 仍未接线（诚实开放项）

- Slint 外壳 `crates/cad-ui-slint/ui/app.slint` 的「测量」按钮仍发送
  `CommandPayload::None`；按钮已能启动距离工具，但**点选/确认/取消与算法选择
  尚未接入 UI**（U04 的 UI 侧仍待完成）。
- 宿主（Web/Android）尚未把画布指针映射为测量点或预览几何；`PreviewState`
  还没有消费 `SessionState::measurement_preview()` 来绘制预览。
- `Switch2d3d` / `Orbit` 仍为 `pending(...)`，本任务不涉及 3D。

## 2. 撤销/重做可用状态（U11）

- `Application::can_undo` / `can_redo` 分别读取 `History` 的 undo/redo 栈，
  二者独立；撤销到空后 `can_undo=false` 而 `can_redo=true`。
- `Application::history_availability(document) -> HistoryAvailability`
  与 `HostController::history_availability()` 是**纯 getter**，用于刷新按钮
  状态，绝不派发命令，避免程序化刷新触发重复命令。
- UI 约定：撤销按钮绑定 `can_undo`，重做按钮绑定 `can_redo`，不得共用
  `can-undo`（`app.slint` 的 `can-redo` 属性与 `UiHandle::set_can_redo`
  由 UI crate 侧完成，见交接说明）。

## 3. 鼠标语义基线（AutoCAD 对照，2026-10-18 外部查证）

本节是**外部权威对照**（Autodesk Help 2024 / AutoCAD 官方文档），用于判定本项目鼠标行为是否
"与 AutoCAD 一致"。**它不是本项目的已验证行为**；本项目当前状态见每条末尾。

| 主题 | AutoCAD 权威事实 | 出处 | 本项目当前状态 |
|---|---|---|---|
| 空闲悬停高亮 | **默认开启**：`SELECTIONPREVIEW` 初值 **3**（no command 时 + "Select objects" 提示时都开）；文档只按"显示/视觉效果"描述，是**叠加高亮**而非重生成 | [SELECTIONPREVIEW](https://help.autodesk.com/cloudhelp/2024/ENU/AutoCAD-Core/files/GUID-77B3B59F-95E5-4F8A-85E2-D9411D712AF0.htm) | **有意偏离**：本项目默认 **关闭**（`HOVERPREVIEW` 命令 / 状态栏"悬停预览"开关可开），因为产品负责人不要空闲悬停高亮。开启后走 flags-only 快路径（见下行），不触发 overlay 重准备 |
| 悬停延时 | **无**：AutoCAD 高亮是"pickbox 滚过即亮"的即时命中，整个 sysvar 集合**没有任何毫秒/秒级悬停门控**（`TOOLTIPDELAY`/`HOVER*` 不存在；Options 里的 "seconds" 只属于 ribbon/toolbar 工具提示） | [SELECTIONPREVIEW](https://help.autodesk.com/cloudhelp/2024/ENU/AutoCAD-Core/files/GUID-77B3B59F-95E5-4F8A-85E2-D9411D712AF0.htm)、[Display Tab](https://help.autodesk.com/cloudhelp/2026/ENU/AutoCAD-LT/files/GUID-F464608C-15BC-48E8-9484-D311053D9B73.htm) | **有意偏离**：`SessionState.hover_dwell_ms` 默认 **300ms**（同一光圈内稳定 300ms 才上色，快速扫过不闪；`0` = 即时，AutoCAD 语义）。**不得**把它说成 AutoCAD 行为 |
| 悬停命中口径 | 命中范围是 **pickbox 大小**（`PICKBOX` 初值 **3**），高亮"若此刻点击会选中的整个对象"；`PICKBOX=0` 关闭 preview | [About Cursors](https://help.autodesk.com/cloudhelp/2024/ENU/AutoCAD-Core/files/GUID-566B8C0C-57BC-4986-942D-BC683C4F90A4.htm)、[PICKBOX](https://help.autodesk.com/cloudhelp/2024/ENU/AutoCAD-Core/files/GUID-363698CF-C3DC-4770-81EF-CB09D86B3D3A.htm) | 已对齐：`TolerancePolicy::pickbox_logical_pixels = 3.0` 同时用于**单击与悬停**（高亮 = 点击会得到什么）。用户可见后果：点击命中范围由 12 收紧到 3，细线更难命中（AutoCAD 原样行为，单一可调数值） |
| 捕捉查找范围 | 与选择分开：`APERTURE` 初值 **10**（设备无关像素），只影响对象捕捉候选搜索/磁吸 | [APERTURE](https://help.autodesk.com/cloudhelp/2021/ENU/AutoCAD-LT/files/GUID-C8603032-7E55-4EEF-B2DF-CD2FD9EDEF91.htm) | `TolerancePolicy::interaction_logical_pixels = 12.0` 只服务 snap 候选（未随 pickbox 变窄）；12 vs AutoCAD 10 的差异如实记录 |
| 悬停排除项 | `PREVIEWFILTER` 初值 **3**：锁定图层对象与 xref 内对象**不高亮**（但仍可被选中） | [PREVIEWFILTER](https://help.autodesk.com/cloudhelp/2024/ENU/AutoCAD-Core/files/GUID-9B75C028-6429-4FC0-9EC6-72C21E29A5D8.htm) | **未实现**：`pick_at_screen` 不做图层可见性过滤（`select_all`/`select_in_box` 才做）⇒ 临时隐藏图层上的对象仍可能被高亮/点中。**开放项** |
| 悬停性能上限 | `SELECTIONPREVIEWLIMIT` 初值 **20000**：预览对象数超限时**整体停止**预览高亮 | [SELECTIONPREVIEWLIMIT](https://help.autodesk.com/cloudhelp/2024/ENU/AutoCAD-Core/files/GUID-9CDDA4F9-5B88-428F-B0C8-345E9A9DC4F1.htm) | **未实现**（无上限、无开关）。**开放项** |
| 命令预览优先 | `COMMANDPREVIEW=1` 时 `SELECTIONPREVIEW` **被忽略**（TRIM/EXTEND/ERASE 等） | [COMMANDPREVIEW](https://help.autodesk.com/cloudhelp/2024/ENU/AutoCAD-Core/files/GUID-2B245436-029A-4E14-92C2-0ED4F4F1362E.htm) | 工具占用指针时宿主直接抑制 hover（`hover_suppressed`），效果一致 |
| 空闲悬停的 OSNAP 标记 | **不显示**：文档明确 "Object snaps work only when you are prompted for a point"；空闲时既不显示标记也不显示追踪 | [About Using Object Snaps](https://help.autodesk.com/cloudhelp/2024/ENU/AutoCAD-Core/files/GUID-8F5E5431-9EFB-414E-BC6D-2C65EFB2DAC3.htm) | 一致：无工具光标时 `snap_hints_near_cursor` 返回空 |
| 空闲左键拖拽 | = **隐含窗口/交叉选择**（`PICKAUTO` 初值 **5**、`PICKDRAG` 初值 **2**）；左→右=window（完全包含），右→左=crossing（相交） | [PICKAUTO](https://help.autodesk.com/cloudhelp/2024/ENU/AutoCAD-Core/files/GUID-7BAAA374-8409-4296-B520-C5355941C836.htm)、[To Select Objects](https://help.autodesk.com/cloudhelp/2024/ENU/AutoCAD-Core/files/GUID-243E4DD0-8947-4905-AFE2-BE9B903A8C3F.htm) | **不一致**：当前 2D 左键拖拽是**平移**。ADR-0014 决策 B 已规定改为框选（P1b 实施） |
| 平移 / 缩放 | **中键（滚轮）按住拖动 = 平移**（`MBUTTONPAN=1`）；滚轮 = 缩放；`PAN` 命令内才有左键平移 | [PAN](https://help.autodesk.com/cloudhelp/2024/ENU/AutoCAD-Core/files/GUID-2F05DC89-7065-4655-BA49-AC149B0F5E1A.htm) | 需在 P1b 确认中键拖拽仍平移（现状 `pointer_move` 的非主键分支即平移） |
| 高亮的两套 | 悬停=preselection（`PRESELECTIONEFFECT`），选中后=`HIGHLIGHT`/`SELECTIONEFFECT`；`HIGHLIGHT` **不随文件保存**（每会话回到 1） | [HIGHLIGHT](https://help.autodesk.com/cloudhelp/2024/ENU/AutoCAD-Core/files/GUID-A0543122-803C-4692-9EEB-090483C4025C.htm) | 本项目两种状态分别为 `FLAG_HOVERED`/`FLAG_SELECTED`，语义一致；无持久化设置项 |
| 只读上下文 | **AutoCAD 未定义**只读/查看器的悬停策略；preview 由系统变量门控，与文件可编辑性无关（DWF Design Review 未查证） | 见 `SELECTIONPREVIEW` 条目 | 本项目按"显示特性 + 开关"处理，不绑定 Viewer/Work 模式 |

**推断（非逐字文档）**：① "悬停不重生成图形"——Autodesk 未给逐字句，但从"视觉/显示效果"与
`SELECTIONPREVIEWLIMIT` 的性能说明可推；② "点提示与对象选择提示互斥显示 OSNAP 标记/对象预选"由
两处文档条目合成。

**结论（本项目要补的）**：悬停高亮本身与 AutoCAD 一致，需要修的是**实现代价**（悬停不得触发
overlay/几何重准备）与**命中过滤/上限**（`PREVIEWFILTER` 式隐藏图层排除、`SELECTIONPREVIEWLIMIT`
式上限、用户可见开关）；左键拖拽语义按 ADR-0014 决策 B 改为框选。

**当前已落地（2026-10-18）**：悬停=flags-only（不推进 `input_revision`/`content_revision`，ADR-0012 §10 S2
修订）；`pickbox_logical_pixels = 3.0` 与 snap 光圈 12.0 分离；空闲悬停预览默认关闭（`HOVERPREVIEW`）；
悬停延时 300ms（`hover_dwell_ms`，显式偏离）；hidden 图层对象既不高亮也不可点选。
**仍待做**：`SELECTIONPREVIEWLIMIT` 式上限（当前 hover 恒为单实体）、`PREVIEWFILTER` 的锁定图层/xref
排除（目前只有隐藏图层过滤）、`COMMANDPREVIEW` 互斥、左键拖拽=框选（P1b）。

