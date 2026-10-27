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

- ~~Slint 外壳 `crates/cad-ui-slint/ui/app.slint` 的「测量」按钮~~：该外壳与 `app.slint`
  **已随 2026-10-16 的 Slint 退役删除，无同名替代**。当前桌面测量入口是 `cad-ui-core` 的选择子
  （`DIST`/`MEASUREPOLYLINE`/`MEASUREANGLE`/`AREA`/`ID`，经 ribbon 与命令行），见
  `apps/desktop-ui/src/ribbon/ribbon-model.ts`；本条不再作为当前 UI 缺口追踪。
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
| 空闲悬停高亮 | **默认开启**：`SELECTIONPREVIEW` 初值 **3**（no command 时 + "Select objects" 提示时都开）；文档只按"显示/视觉效果"描述，是**叠加高亮**而非重生成 | [SELECTIONPREVIEW](https://help.autodesk.com/cloudhelp/2024/ENU/AutoCAD-Core/files/GUID-77B3B59F-95E5-4F8A-85E2-D9411D712AF0.htm) | **已对齐**：默认 **开启**（与初值 3 一致），可用 `HOVERPREVIEW` 命令/状态栏"悬停预览"关闭。开启时走 flags-only 快路径（见下行），不触发 overlay 重准备；关闭时空闲移动**零拾取**、零高亮 |
| 悬停延时 | **无**：AutoCAD 高亮是"pickbox 滚过即亮"的即时命中，整个 sysvar 集合**没有任何毫秒/秒级悬停门控**（`TOOLTIPDELAY`/`HOVER*` 不存在；Options 里的 "seconds" 只属于 ribbon/toolbar 工具提示） | [SELECTIONPREVIEW](https://help.autodesk.com/cloudhelp/2024/ENU/AutoCAD-Core/files/GUID-77B3B59F-95E5-4F8A-85E2-D9411D712AF0.htm)、[Display Tab](https://help.autodesk.com/cloudhelp/2026/ENU/AutoCAD-LT/files/GUID-F464608C-15BC-48E8-9484-D311053D9B73.htm) | **有意偏离**：`SessionState.hover_dwell_ms` 默认 **20ms**（同一光圈内稳定 20ms 才上色；`0` = 即时=AutoCAD 语义）。20ms 在 60Hz 下约 **1 帧**，只过滤帧内抖动与极小扫掠，**不再**抑制刻意的快速扫掠（默认值由 300ms 下调，产品负责人决定）。**不得**把它说成 AutoCAD 行为 |
| 悬停命中口径 | 命中范围是 **pickbox 大小**（`PICKBOX` 初值 **3**），高亮"若此刻点击会选中的整个对象"；`PICKBOX=0` 关闭 preview | [About Cursors](https://help.autodesk.com/cloudhelp/2024/ENU/AutoCAD-Core/files/GUID-566B8C0C-57BC-4986-942D-BC683C4F90A4.htm)、[PICKBOX](https://help.autodesk.com/cloudhelp/2024/ENU/AutoCAD-Core/files/GUID-363698CF-C3DC-4770-81EF-CB09D86B3D3A.htm) | 已对齐：`TolerancePolicy::pickbox_logical_pixels = 3.0` 同时用于**单击与悬停**（高亮 = 点击会得到什么）。用户可见后果：点击命中范围由 12 收紧到 3，细线更难命中（AutoCAD 原样行为，单一可调数值） |
| 捕捉查找范围 | 与选择分开：`APERTURE` 初值 **10**（设备无关像素），只影响对象捕捉候选搜索/磁吸 | [APERTURE](https://help.autodesk.com/cloudhelp/2021/ENU/AutoCAD-LT/files/GUID-C8603032-7E55-4EEF-B2DF-CD2FD9EDEF91.htm) | `TolerancePolicy::interaction_logical_pixels = 12.0` 只服务 snap 候选（未随 pickbox 变窄）；12 vs AutoCAD 10 的差异如实记录 |
| 悬停排除项 | `PREVIEWFILTER` 初值 **3**：锁定图层对象与 xref 内对象**不高亮**（但仍可被选中） | [PREVIEWFILTER](https://help.autodesk.com/cloudhelp/2024/ENU/AutoCAD-Core/files/GUID-9B75C028-6429-4FC0-9EC6-72C21E29A5D8.htm) | **部分对齐**：`pick_at_screen_with_visibility`/`pick_ray_with_visibility`/`ref_is_visible` 已按图层可见性过滤（隐藏图层对象既不高亮、也不可点选）；**锁定图层与 xref 的排除仍未实现**（AutoCAD 中二者可选中但不预览）。**开放项** |
| 悬停性能上限 | `SELECTIONPREVIEWLIMIT` 初值 **20000**：预览对象数超限时**整体停止**预览高亮 | [SELECTIONPREVIEWLIMIT](https://help.autodesk.com/cloudhelp/2024/ENU/AutoCAD-Core/files/GUID-9CDDA4F9-5B88-428F-B0C8-345E9A9DC4F1.htm) | **未实现**（无上限、无开关）。**开放项** |
| 命令预览优先 | `COMMANDPREVIEW=1` 时 `SELECTIONPREVIEW` **被忽略**（TRIM/EXTEND/ERASE 等） | [COMMANDPREVIEW](https://help.autodesk.com/cloudhelp/2024/ENU/AutoCAD-Core/files/GUID-2B245436-029A-4E14-92C2-0ED4F4F1362E.htm) | 工具占用指针时宿主直接抑制空闲 hover；**但**在需要拾取实体的捕获步骤（TRIM/EXTEND）里，光标下的候选实体仍以**同一 `FLAG_HOVERED` 通道**着色（`OverlayInputs::hovered`），即"命令内对象高亮"；同一时刻只有一个生产者。候选仅走 flags-only 发布（`input_revision` 不变） |
| TRIM/EXTEND 选取语义 | 先选**剪切边/边界**（可多选，`Enter` 表示全部），再逐个点选要裁剪/延伸的对象 | Autodesk『TRIM』（本轮**未逐字复核**） | **诚实子集**：本项目是"**目标 + 单一边界**"两次**实体拾取**（点空处当场以 `draw pick did not hit supported geometry` 拒绝且不推进），TRIM 目标仅 LINE、边界须为直线段；多边界/曲线边界/多目标/`Enter`=全部 均**未实现**。目标/边界的解析在**取点时**完成（不在确认时二次拾取），提交用的是用户看到的那两个实体 |
| 空闲悬停的 OSNAP 标记 | **不显示**：文档明确 "Object snaps work only when you are prompted for a point"；空闲时既不显示标记也不显示追踪 | [About Using Object Snaps](https://help.autodesk.com/cloudhelp/2024/ENU/AutoCAD-Core/files/GUID-8F5E5431-9EFB-414E-BC6D-2C65EFB2DAC3.htm) | 一致：无工具光标时 `snap_hints_near_cursor` 返回空 |
| 捕捉总开关（F3） | `F3` 切换对象捕捉开关（AutoCAD 帮助「Function Keys」；本轮**未逐字复核**，仅作对照）。注意 AutoCAD `SNAP`（F9）是**栅格捕捉**，与对象捕捉是两件事 | Autodesk『Function Keys』（未逐字复核） | **已实现**：`OSNAP` 选择子 ↔ `CommandId::ToggleSnap`，`F3` 经正常命令通路派发；只切会话级 `snap_enabled`（**不改**持久化 viewer config），无 ChangeSet/revision/历史；状态栏 OSNAP 是真实开关，投影 `CadStatus.snapEnabled`。**`SNAP` 不是 `OSNAP` 的别名**：`SNAP` 是独立的栅格捕捉选择子（`CommandId::ToggleSnapGrid`，F9），间距由 `SNAPUNIT <value>` 设置，二者互不替代 |
| 空闲左键拖拽 | = **隐含窗口/交叉选择**（`PICKAUTO` 初值 **5**、`PICKDRAG` 初值 **2**）；左→右=window（完全包含），右→左=crossing（相交） | [PICKAUTO](https://help.autodesk.com/cloudhelp/2024/ENU/AutoCAD-Core/files/GUID-7BAAA374-8409-4296-B520-C5355941C836.htm)、[To Select Objects](https://help.autodesk.com/cloudhelp/2024/ENU/AutoCAD-Core/files/GUID-243E4DD0-8947-4905-AFE2-BE9B903A8C3F.htm) | **已对齐（P1b 实施，行为变更）**：2D 左键拖拽（≥ `DRAG_THRESHOLD_LOGICAL_PX=4`）= 框选，左→右 WINDOW、右→左 CROSSING；`Shift`=Add，否则 Replace；按住时画橡皮筋（WINDOW 蓝实线 / CROSSING 绿虚线，screen-space 5px dash，≤512 段）；<4px 仍是单点 pick。**3D 左键拖拽仍是轨道旋转**，3D 框选显式 NOT IMPLEMENTED |
| 平移 / 缩放 | **中键（滚轮）按住拖动 = 平移**（`MBUTTONPAN=1`）；滚轮 = 缩放；`PAN` 命令内才有左键平移 | [PAN](https://help.autodesk.com/cloudhelp/2024/ENU/AutoCAD-Core/files/GUID-2F05DC89-7065-4655-BA49-AC149B0F5E1A.htm) | **已对齐**：中键拖拽=平移（2D/3D 均然），滚轮=缩放；左键**不再**平移（见上行）。Secondary 拖拽现状仍平移；
  Back/Forward 在**协议层就被拒**（`crates/cad-ui-core/src/session/input.rs`），不会进入宿主手势——AutoCAD 该
  位置的右键快捷菜单本项目未实现，记为开放项 |
| 高亮的两套 | 悬停=preselection（`PRESELECTIONEFFECT`），选中后=`HIGHLIGHT`/`SELECTIONEFFECT`；`HIGHLIGHT` **不随文件保存**（每会话回到 1） | [HIGHLIGHT](https://help.autodesk.com/cloudhelp/2024/ENU/AutoCAD-Core/files/GUID-A0543122-803C-4692-9EEB-090483C4025C.htm) | 本项目两种状态分别为 `FLAG_HOVERED`/`FLAG_SELECTED`，语义一致；无持久化设置项 |
| 只读上下文 | **AutoCAD 未定义**只读/查看器的悬停策略；preview 由系统变量门控，与文件可编辑性无关（DWF Design Review 未查证） | 见 `SELECTIONPREVIEW` 条目 | 本项目按"显示特性 + 开关"处理，不绑定 Viewer/Work 模式 |

**推断（非逐字文档）**：① "悬停不重生成图形"——Autodesk 未给逐字句，但从"视觉/显示效果"与
`SELECTIONPREVIEWLIMIT` 的性能说明可推；② "点提示与对象选择提示互斥显示 OSNAP 标记/对象预选"由
两处文档条目合成。

**结论（本项目要补的）**：悬停高亮本身与 AutoCAD 一致，需要修的是**实现代价**（悬停不得触发
overlay/几何重准备）与**命中过滤/上限**（`PREVIEWFILTER` 式隐藏图层排除、`SELECTIONPREVIEWLIMIT`
式上限、用户可见开关）；左键拖拽语义按 ADR-0014 决策 B 改为框选。

**当前已落地（2026-10-18）**：悬停=flags-only（不推进 `input_revision`/`content_revision`，ADR-0012 §10 S2
修订）；`pickbox_logical_pixels = 3.0` 与 snap 光圈 12.0 分离；空闲悬停预览默认**开启**（AutoCAD
`SELECTIONPREVIEW=3` 对齐，`HOVERPREVIEW` 可关）；悬停延时 20ms（`hover_dwell_ms`，显式偏离；60Hz 下约 1 帧）；
hidden 图层对象既不高亮也不可点选；2D 左键拖拽=框选（橡皮筋走 renderer overlay，非 DOM）且中键=平移；
`F3`/`OSNAP` 切捕捉总开关。
**仍待做**：`SELECTIONPREVIEWLIMIT` 式上限（当前 hover 恒为单实体）、`PREVIEWFILTER` 的锁定图层/xref
排除（目前只有隐藏图层过滤）、`COMMANDPREVIEW` 互斥、Secondary 拖拽的右键菜单语义、3D/纸空间框选、
`PICKADD` 式选择加选策略、框选对 Opaque/超深 INSERT 的显式拒绝面复核。

## 4. 视口光标与对象捕捉标记（2026-10-19，人工复测后修复）

**背景**：人工复测报告两处缺陷——视口仍是**系统默认箭头**（无十字/拾取框），且**捕捉标记过小、形状不可辨**。勘察确认：仓库**从未设置过光标**（无 GTK cursor 代码、视口无 cursor CSS 规则）；捕捉标记的尺寸参数 `PreviewOptions::marker_size` 文档为**世界单位**且**宿主从未覆盖**（三处构造均用 `PreviewOptions::default()`）⇒ 屏幕尺寸随缩放漂移。另外字形**与 AutoCAD 对调**（中点/象限），且**标记与拾取框同级重叠、拾取框绘制在其上**。

**十字光标 + 拾取框**：由 renderer overlay 绘制（`crates/cad-app/src/render_scene/overlay/cursor.rs`，`CURSOR_DRAW_ORDER = 2_500_000`）。贯穿视口的两条线取 `viewport_bounds`（与网格同一矩形），拾取框半边长 = `cad_domain::TolerancePolicy::default().pickbox_logical_pixels`（**3**，与 `cad_app::picking::pick_tolerance` 同一常量）× 视图比例 ⇒ **拾取框的内接圆恰是一次点击能命中的范围**（该常量在本仓库是**半径**语义，故框边 = 2×3 逻辑像素）。**颜色偏离 AutoCAD 的白**：`CURSOR_COLOR = [0.72, 0.75, 0.80]`（冷中灰）——白色在默认纯白几何（`cad_scene` 默认 `[1,1,1]`）上不可见；中灰在黑背景与白几何上都可读，且与既有 overlay 色（琥珀捕捉、品红预览、青测量、淡蓝 POINT、蓝/绿框选）都不冲突。

**系统光标置空的条件**：**只在宿主确实在画十字时**。新增诚实的 `cad_status.crosshair: bool`（取自渲染真正使用的那个 overlay 输入 `overlays().cursor.is_some()`），React 侧据此加 `.is-crosshair` 类；缺失该字段时**保留箭头**（不猜）。**3D 视图永远保留系统箭头**——overlay 是纯世界空间的，透视相机下无有效 2D 视口矩形（`screen_to_world` 映射到工作平面），故**不**画十字，也**不**置空。**已记录局限**：该标志随 ~250 ms 的 `cad_status` 轮询，故**进入** 2D 区域时箭头最多滞留一个轮询周期；反方向（3D 保留箭头）是即时且始终正确的。

**对象捕捉标记**：**按像素**定尺寸（`SNAP_MARKER_HALF_EXTENT_PIXELS = 9.0` ⇒ 字形 18 逻辑像素；`PreviewOptions::for_view_scale` 在构造点换算，非有限/非正视图比例回退到文档化的世界默认，**永不**产出 0/NaN）。拾取框 6 px ⇒ **3:1**（与 AutoCAD 同量级：捕捉字形是"大形状"、拾取框是小方块）。**字形按 AutoCAD 约定**：端点=方块、**中点=三角**、圆心=圆、**象限=菱形**、**垂足=⊥**、交点=X（每种类型的精确线段集合由 `snap_marker_glyphs_match_autocad_per_kind` 钉住，将来再对调会失败）。**绘制层级高于十字**（`SNAP_HINT_DRAW_ORDER = 3_000_000` > 光标 2_500 000），因为**标记才是用户在捕捉点要读的东西**；十字仍在工具预览（2 000 000）之上，故橡皮筋不会挡住它锚定的指针。

**真机证据（X11 / Quadro P620 / Vulkan；合成 DOM 指针事件、非真实鼠标）**：指针在捕捉点上时 `snapKind:"endpoint"` ×26，**琥珀方块 19×24 px** 与 **6×7 px 拾取框同心**且**可分辨**；指针偏离 10 px 时两者**并排**（相距 10.5 px）仍可分辨；**中点**捕捉渲染为**三角形**（尖顶 + 两斜边 + 底边），与端点方块明显不同。**诚实边界**：夹具只有直线，故只拍到端点与中点两种字形，⊥/菱形/圆/X **未拍照**（由测试钉住尺寸与线段集合）；`cad_status` 无几何通道 ⇒ 真机只证"计数 + 内容存在"。
