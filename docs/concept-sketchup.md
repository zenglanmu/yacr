# SketchUp（`concept` 域）官方工具覆盖表

> 目的：给 `/loop` 一个**可数**的验收口径，而不是“看起来像 SketchUp”。
> 来源：本仓库 `apps/desktop-ui/src/domains/concept/{toolbar,menus,tray}.ts` 对官方
> Toolbars/Menus/Default Tray 的 clean-room 转写；工具名与分组来自官方帮助页，图标/文案是我
> 们自己的。**不复制** SketchUp 图标、截图或帮助正文。
>
> 证据边界：本表统计的是**命令面覆盖**——某个工具是否有真实命令、是否操作文档数据，不是
> vendor 等价、UX 保真或像素验收。每个 `full` 仍受该命令自己的文档边界约束。

## 1. 口径

| 状态 | 含义 |
|---|---|
| `full` | 该 token 有真实命令，按本仓文档的保真度执行其命名操作，失败有具名拒绝 |
| `partial` | 命令存在，但只覆盖官方语义的一个子集（例如只对精确 B-Rep 生效、没有 guide、只读面板） |
| `pending` | 没有实现；UI 显示为禁用/pending，**不显示可点假按钮** |
| `refusal` | 有稳定具名拒绝（比 `pending` 更明确，但仍不是功能） |

**50% 验收线**：官方工具单元（当前 69 个 toolbar/menu 单元）中 `full` 占比 ≥ 50%。
`partial` 不计入分子；它只用于说明“有命令但语义未满”。分母固定为本文列出的 69 个单元；
新增工具必须同时更新分母与表。

## 2. 汇总（2026-11-27）

| 状态 | 数量 | 占比 |
|---|---:|---:|
| `full` | 54 | **78.3%** |
| `partial` | 15 | 21.7% |
| `pending` | 0 | 0.0% |
| 合计 | 69 | 100% |

- **50% 线已满足**（54/69 = 78.3%）；`full` + `partial` = 69/69 = 100% 有真实命令（其中 `full` 仍只有 54/69 = 78.3%；`partial` 不是验收，见各行的边界）；工具层不再有 `pending`。
- `full` 的主体仍是二维制图/编辑/导航/文件命令；SketchUp 特有的网格核心已完成布尔/修复：
  `UNION`/`SUBTRACT`/`INTERSECT` 现在同时接受精确 `Body3d{Solid,Exact}` 与闭合网格
  （混合选择按 `boolean.operand_class_mismatch` 拒绝），`MESHREPAIR`/`MESHREBUILD` 修复闭合网格；
  结果仍是**非精确**网格。`PRESSPULL` 现在同时支持精确面与带 `face_sources` 的网格面；
  `MOVE` 面选择、`MESHREFINE` 细分、`MESHSMOOTH` 平滑、`MESHCAP` 边界环补洞（含共面嵌套环带）
  已接线。
  `FOLLOWME`（2026-11-27）把「闭合轮廓 + 开放路径」扫掠成**闭合网格**（非精确 B-Rep）：
  轮廓/路径都保持原位、一次事务/一步撤销，拐角在角平分面上剪切，结果是单壳、外向、每面有
  `face_sources` 的 `AcDbSubDMesh`。轮廓可以是闭合直线 `LWPOLYLINE`、`LINE` 闭环或 `CIRCLE`
  （圆按 48 边形采样，SketchUp 的圆也是多边形）；弧线/bulge 轮廓、闭合路径与反向路径按名拒绝。
  **柔化边线**（2026-11-27，ADR-0064）：`SOFTENEDGES [<angle>]` 把所选三维体的
  `render_attributes.soften` 写成角度（默认 20°），渲染隐藏相邻面夹角 ≤ 阈值的边；无法与显示
  三角形匹配的边保持可见；一次写入一步撤销，元素行只在设置时写键（minor 21）。
  **阴影与雾**（2026-11-27，ADR-0065）：`tables/display.json`（minor 22）存文档太阳
  （方位角/高度角 + 开关）与雾区间/颜色；`SHADOWS`/`FOG` 各一次 revision/一步撤销，渲染器把太阳
  当方向光替换相机 key light、可见 shader 按视距向雾色混合；**投射阴影仍未实现**。
  托盘 Shadows/Fog 面板通过 `cad_display` host read 显示当前值并 dispatch 上述命令。
  **大纲**（2026-11-27，ADR-0066）：`cad_outliner` 投影分组 / INSERT / 模型空间实体 / 块成员
  （前序扁平 + `depth`，≤1024 行，`total`/`truncated` 诚实），托盘 Outliner 面板本地展开折叠、
  点名称用既有 `cad_select_entities` 选中（分组一次选中全部成员，块成员行不可选）；纯只读。
  **场景阴影/雾快照**（2026-11-27，ADR-0080）：`SCENE SAVE` 把文档 `tables/display.json`
  记录存进场景行的 `display`（minor 24）；`SCENE RESTORE` 在记录有差异时执行一次与
  `SHADOWS`/`FOG` 相同的显示写入（一步撤销），Viewer 下有差异则在改动前拒绝。
  **场景柔化清除**（2026-11-28，ADR-0094）：`SCENE SAVE` 记录无显式 soften 的 body
  （`soften_cleared`，≤1024，minor 29），恢复时在同一 display 属性事务里清空；超过上限的保存按
  `scene.soften_cleared_invalid` 拒绝。
  **场景柔化快照**（2026-11-28，ADR-0090）：`SCENE SAVE` 捕获有显式 `render_attributes.soften`
  的实体（≤1024，行的 `soften`，minor 28），恢复按一次 display 属性事务写回；无显式值的实体不被
  场景改动；Viewer 有差异时在改动前拒绝。
  **Outliner 逐行显示**（2026-11-28，ADR-0091）：新增会话命令 `SHOWOBJECTS`，把当前选择从
  `hidden_entities` 移除；隐藏行 Show 先选中该行 ids 再 dispatch，Viewer 合法、不写文档。
  **Outliner 组内行重排**（2026-11-28，ADR-0098）：分组成员按 `(draw_order,id)` 排序，组内
  entity 行可 ↑/↓ 走 `DRAWORDER BELOW/ABOVE`；块成员（不可选）与嵌套展开仍未建模。
  **Outliner 行重排**（2026-11-28，ADR-0097）：顶层 entity/INSERT 行 ↑/↓ 先选相邻行再
  dispatch `DRAWORDER BELOW/ABOVE`；Outliner 顶层按 `draw_order` 排序；分组行/块成员不重排。
  **Outliner 拖拽撤销分组**（2026-11-28，ADR-0096）：行拖到 Ungroup 区域，按旧 `parentGroup`
  分组 dispatch `GROUPEDIT "<旧组>" REMOVE`；无新命令/格式。
  **Outliner 拖拽归组**（2026-11-28，ADR-0095）：行拖到目标分组先 `GROUPEDIT REMOVE` 再从
  `GROUPEDIT ADD`（两步撤销）；分组成员行可 Remove from group；行顺序重排仍未实现。
  **Outliner 多选建组**（2026-11-28，ADR-0093）：Ctrl/Cmd/Shift 点击累计行选择，
  `cad_select_entities` 发送并集，名称输入 dispatch `GROUP CREATE "<name>"`；重排/拖拽归组仍未实现。
  **Outliner 隐藏/显示**（2026-11-28，ADR-0089）：行可 `HIDEOBJECTS` 隐藏（先选中该行 ids），
  隐藏态由 `cad_outliner.hidden` 标注，隐藏行可 `SHOWOBJECTS` 逐行显示，Show all 走
  `UNISOLATEOBJECTS`；无持久化。
  **Outliner 分组改名**（2026-11-28，ADR-0088）：分组行内输入 dispatch 既有
  `GROUPEDIT "<name>" RENAME "<new>"`，一步撤销；重排/拖拽归组/建组/隐藏仍未实现。
  **自动参考线点**（2026-11-28，ADR-0087）：`DIST GUIDE` 的两个拾取点重合时写 point 标记
  （不是退化为错误），测量仍返回 0 距离。
  **文档剖面**（2026-11-28，ADR-0103）：`SECTIONPLANE ON|OFF|TOGGLE|CLEAR|LIST|SET ...` 写
  `tables/sections.json`（minor 32）；模型空间 scene 构建用 `cad-scene::clip_delta` 真实裁剪
  网格/线/弧（conic 激活时转线段），简单闭合切边环耳切成 cap，嵌套孔/非流形边界只画切边；图片四边形/既有 clip 多边形按同一平面裁剪并插值 UV，wipeout mask 与纸空间不裁剪；场景保存/恢复
  剖面，View → Section Planes 菜单与命令线均为真实入口。
  **参考线样式**（2026-11-28，ADR-0101）：`GUIDE STYLE COLOR/DASH` 持久化共享颜色与虚线开关
  （`tables/guides.json` 的 style 行，minor 30），renderer 使用该颜色/开关；concept 域 Styles
  托盘提供颜色输入与虚线开关（dispatch 同一 `GUIDE STYLE`），其它域仍是 typed 命令。
  **参考线点与单条删除**（2026-11-28，ADR-0085）：`GUIDE POINT` 写 point-only 行（minor 27，
  三轴十字渲染），`GUIDE DELETE <n>` 删除一行；两者都是一次整表写入/一步撤销。
  **构造参考线**（2026-11-28，ADR-0084）：`DIST GUIDE` / `PROTRACTOR GUIDE` 在测量完成时写
  非实体 `tables/guides.json`（minor 26）；`GUIDE LIST|CLEAR` 管理，渲染器按模型包围盒把每条
  参考线由**相机 overlay** 画成有界虚线段（6/4 逻辑像素 × `view_scale`，缩放时 dash 周期随之更新），point 为 6px 三轴十字；`GUIDE STYLE COLOR/DASH` 持久化共享颜色/虚线开关（concept 域 Styles 托盘有控件）；不进实体/导出/拾取。
  **Flip Along**（2026-11-28，ADR-0083）：`FLIP X|Y|Z` 以选择包围盒中心为平面原地反射
  （源删除 + 镜像副本、一次事务/一步撤销、字形真反射）；新副本是新 entity id，块定义成员/子图元
  仍按名拒绝。
  **材质编辑表单**（2026-11-28，ADR-0100）：文档材质行 Edit 打开名称/颜色/透明度，只对变化字段
  发 `MATERIAL ... RENAME/COLOR/TRANSPARENCY`（每条一步撤销）；贴图/材质库未建模。
  **材质编辑命令**（2026-11-28，ADR-0099）：`MATERIAL <name> RENAME|COLOR|TRANSPARENCY|DELETE`
  改文档材质行（一步撤销，delete 清实体/面赋值）；面板现有重涂 + Edit（名称/颜色/透明度）+ Delete；多字段保存按字段分命令。
  **逐面材质**（2026-11-27，ADR-0081）：`PAINT <name>` 接受带 `SubElementId` 的网格面选择，
  材质表行新增 `faces` 快照（minor 25）；渲染器按面材质把网格拆成多个 fragment（fragment 自带
  颜色/alpha），未涂面保留实体颜色；陈旧 face key 只报告不重映射。
  **材质面板文档行**（2026-11-27，ADR-0079）：`cad_materials` 把 `tables/materials.json` 的
  名称/颜色/透明度投影给托盘，文档材质不再只存在于 `MATERIALS` 文本报告；每个色块走既有
  `PAINT <name>`（整实体或选中的网格面；逐面材质编辑器/贴图/材质库仍未建模）。
  **场景可见标签**（2026-11-27，ADR-0078）：`SCENE SAVE` 把每个图层的有效可见性存进
  `tables/scenes.json` 行的 `tags` 快照（minor 23），`SCENE RESTORE` 把它作为会话图层覆盖重新应用
  （不写图层表）；阴影/雾/软边/隐藏实体/剖面已随场景切换，场景缩略图仍未建模。
  **样式面板**（2026-11-27，ADR-0077）：托盘 Styles 从“只有主题切换”变成两行真实 token——
  `WIREFRAME`/`SHADED` 走既有 `VSCURRENT <token>`，当前 token 从 `cad_status.visualStyle` 高亮；
  其它样式名仍由命令层按 `vscurrent.style_not_implemented` 拒绝，界面主题与 SketchUp Style 分开。
  **实体边线**（2026-11-28，ADR-0104）：`EDGES` 是 payload-free 会话开关，关闭后 shaded body
  保留填充但发 `MeshWithoutEdgeOverlay`，wire-only body 画空；独立线实体/尺寸不受影响；Styles
  托盘有真实开关，`cad_status.bodyEdges` 投影当前值。
  **组件面板**（2026-11-27，ADR-0076）：托盘 Components 从只读列表变成可操作面板——每行
  `INSERT <name>`、名称输入 + `BLOCK <name>`（块名经 pending text 在捕获点后自动应用）；命令层
  仍负责未知块/空选择/重名拒绝。
  **照片匹配/叠加层面板**（2026-11-27，ADR-0075）：`MATCHPHOTO`/`OVERLAYS` 从 pending 变成真实
  host-read 面板——Match Photo 报告 `modeled:false` + 稳定原因，Overlays 只读列出渲染器拥有的通道；
  两个动作都是具名拒绝（`matchphoto.not_implemented` / `overlays.not_implemented`），不写文档。
  **漫游/环视**（2026-11-27，ADR-0074）：`WALK` 与 `LOOKAROUND` 从 pending 变成真实视图导航
  （一点捕获或 typed 数值），只改透视相机、记 `ZOOM PREVIOUS`，平面/退化输入按名拒绝。
  **放置相机**（2026-11-27，ADR-0073）：`CAMERA` 从 pending 变成真实的两点工具——先点眼睛、
  再点目标，设一个透视相机（eye/target/世界 +Z up、沿用当前 FOV），只改视图并记 `ZOOM PREVIOUS`；
  `CAMERA TWO-POINT` 把当前透视相机放平（眼高、world +Z up、保留镜头），竖线保持竖直；`CAMERA WALK` 仍按名拒绝（WALK/LOOKAROUND 是独立工具）。
  **分割**（2026-11-27，ADR-0072）：`SPLITSOLID` 以第一个选中体为目标、其余为切割体，先并集
  切割体，再把目标分成外部（`−`）与内部（`∩`）两块同时插入，切割体保留；无交集/一侧为空按名拒绝，
  其余校验同 Trim。
  **修剪**（2026-11-27，ADR-0071）：`TRIMSOLID` 以第一个选中体为目标、其余为刀具，经既有
  manifold 布尔做 subtract 折叠并只替换目标（刀具行保留），一次事务/一步撤销；开放壳/非流形/
  精确实体/跨空间/超预算/无 modeler 都在启动前按名拒绝。
  **外壳**（2026-11-27，ADR-0070）：`OUTERSHELL` 复用 `UNION` 的 manifold 网格布尔，把所选闭合
  网格体合并成一个外壳（一次事务/一步撤销）；开放壳/非流形/精确实体/超预算/无 modeler 都在启动前
  按名拒绝。
  **三维文字**（2026-11-27，ADR-0069）：`TEXT3D` 用宿主 outline 引擎的**已填充字形网格**挤出
  一个 `AcDbSubDMesh`（盖沿用显示三角形、侧墙来自边界边），一次事务/一步撤销；SHX 描边与超预算
  按名拒绝，无字体对话框/旋转/对齐。
  **自由手绘**（2026-11-27，ADR-0067）：`SKETCH` 从裸选择器变成真实拖拽工具——host 按 2 px
  步长采样（≤1024 点）、松开经 `screen_to_world` 投影后喂给捕获并以既有 `SKETCH` 命令写入一条
  开放 `LWPOLYLINE`（一次事务/一步撤销）；点击未拖动是 `sketch.too_few_points`，3D 视图是
  `sketch.requires_plan_view`，两者都不写文档。
  **场景**（2026-11-27，ADR-0063）：`tables/scenes.json`（minor 20）存命名相机/空间/样式快照，
  `SCENE SAVE/RESTORE/LIST/DELETE` 各按一条 revision/一步撤销（RESTORE 是会话应用，记
  `ZOOM PREVIOUS`），托盘 Scenes 面板通过 `cad_scenes` host read 列表并 dispatch 恢复/保存/
  删除；可见 Tags、阴影/雾、柔化、顶层隐藏实体与剖面随场景切换（ADR-0065/0090/0094/0102/0103），场景缩略图仍未建模。
  **材质**（2026-11-27，ADR-0062）：`tables/materials.json` 存命名扁平材质（颜色 + 透明度）与
  每个实体一个赋值，`PAINT <名称>` 涂当前选择（内置 12 色首次涂刷落成文档行），渲染在展开时
  用材质色覆盖实体颜色/透明度；一次表写入 = 一次 revision = 一步撤销，删除实体清掉赋值。
  边身份使用几何派生 `mesh:edge:<bits>` key，`MOVE` 可移动边；wire-only 边/跨多个三角形边的
  stored polyline 仍不可编辑，因此本表的“75.4%”不是“SketchUp 已复刻七成半”的营销结论。
- **矩形→推拉的核心回路已闭环**（2026-11-27）：Push/Pull 在所选是闭合平面轮廓时（闭合直线
  LWPOLYLINE、LINE 闭环、单个 CIRCLE）先把轮廓拉成新的精确实体，再对实体继续推拉；轮廓本身
  仍是文档里的持久输入。轮廓必须平行 XY、沿 ±Z 拉伸（这是 `EXTRUDE` 的既有边界），其它平面
  按 `extrude.not_planar` 具名拒绝，不静默换轴。
- P-MESH-1a/1b 已实现（2026-11-26；2026-11-27 起四个 `native-below` 载体均注入
  `ManifoldMeshModeler`）：Linux 有运行/契约证据，Windows/macOS/Android 只有跨目标编译/契约
  证据（运行时 NOT VERIFIED）；`dom-canvas`（Web、X11/XWayland）注入同一内核但走**同步例外**
  （inline、阻塞 UI、无中途取消，总三角面 ≤ 200,000，超出按 `mesh.sync_budget` 拒绝）。

## 3. Toolbar 单元

### Getting Started（15；full 12 / partial 2 / pending 1）

| 工具 | token | 状态 | 边界 |
|---|---|---|---|
| Select | `SELECT` | full | 选择/框选；子元素选择受各命令模型限制 |
| Eraser | `ERASE` | full | 删除实体；网格面级擦除未实现 |
| Line | `LINE` | full | CAD 线；SketchUp 的“边自动切分面”未建模 |
| Arc | `ARC` | full | 三点/起点-圆心-端点等既有 ARC 形式 |
| Rectangle | `RECTANG` | full | |
| Offset | `OFFSET` | full | 二维偏移；网格面偏移未实现 |
| Move | `MOVE` | full | 实体移动；面/边级跟随推拉未实现 |
| Rotate | `ROTATE` | full | |
| Scale | `SCALE` | full | |
| Mirror | `MIRROR` | partial | 是 CAD 镜像，不是 SketchUp 的 Flip 语义 |
| Tape Measure | `DIST` | partial | 距离 + `DIST GUIDE`（完成两点后写一条持久构造参考线，`tables/guides.json`）；`GUIDE POINT <x> <y> <z>` 创建点标记，`GUIDE DELETE <n>` 单条删除，`GUIDE LIST`/`CLEAR` 管理；无偏移/延伸/多次复制模式 |
| Paint Bucket | `MATERIALS` | partial | `MATERIALS` 是只读材质报告（文档材质 + 内置调色板），托盘的 Materials 面板是真实色块调色板，点击 dispatch `PAINT <名称>` 涂**当前选择**；没有“点一个面涂一个面”的拾取、没有纹理材质、没有 INSERT 级联（ADR-0062） |
| Orbit | `ORBIT` | full | |
| Pan | `PAN` | full | |
| Zoom Extents | `FIT` | full | |

### Drawing（8；full 6 / partial 1 / pending 1）

| 工具 | token | 状态 | 边界 |
|---|---|---|---|
| Line | `LINE` | full | 同上 |
| Freehand | `SKETCH` | partial | 真实拖拽工具（概念域工具栏/菜单不再 pending）+ `SKETCH` 命令：按下开始采样、移动追加（2 px 步长、1024 上限）、松开提交**一条开放 LWPOLYLINE**（一次事务/一步撤销，不闭合/不拟合）；点击未拖动 `sketch.too_few_points`、3D 视图 `sketch.requires_plan_view`；无压感/曲线拟合/自动切面 |
| Rectangle | `RECTANG` | full | |
| Rotated Rectangle | `RECTANGROTATED` | full | 三次点击：角点、第一边端点、宽度点；在三点确定的平面上生成一个闭合四顶点 `LWPOLYLINE`（可直接推拉/跟随） |
| Circle | `CIRCLE` | full | |
| Polygon | `POLYGON` | full | |
| Arc | `ARC` | full | |
| Pie | `PIE` | partial | 三次点击：圆心、圆周点（半径）、终点角；弧按 24 段/圈采样成**闭合直线** `LWPOLYLINE`。三点捕获只能确定 ≤180° 的转向，更大的扇形需 `ARC`+`LINE` |

### Edit（7；full 6 / partial 1）

| 工具 | token | 状态 | 边界 |
|---|---|---|---|
| Move | `MOVE` | full | |
| Push/Pull | `PRESSPULL` | full | 精确面或带 `face_sources` 的网格面；**所选为闭合平面轮廓时把轮廓拉成新精确实体**（矩形/圆的第一次推拉，走 `EXTRUDE` 的同一命令与事务，轮廓保持原位）；无面身份且非轮廓时拒绝；边移动由 `MOVE` 处理 |
| Rotate | `ROTATE` | full | |
| Follow Me | `FOLLOWME` | full | 把所选闭合直线轮廓沿所选开放直线路径扫掠成闭合网格体（拐角按角平分面剪切、两端封盖，一面一个 `face_sources` 身份，可继续 `PRESSPULL`）；不支持圆/弧轮廓（投影会变椭圆）、闭合路径、通过点选路径面驱动 |
| Scale | `SCALE` | full | |
| Mirror/Flip | `MIRROR` / `FLIP` | partial | `MIRROR` 是 CAD 镜像（保留原件、新增副本）；`FLIP X\|Y\|Z`（`RED/GREEN/BLUE`）是 SketchUp Flip Along：绕**选择包围盒中心**、垂直于该轴的平面**原地**反射（源删除 + 镜像副本，一次事务/一步撤销，文字字形真反射）。**边界**：副本获得新 entity id（不是同一 group/component 对象身份）；块定义成员/子图元按名拒绝；`MIRROR` 与 `FLIP` 的选区/显示属性继承差异沿用既有规则 |
| Offset | `OFFSET` | full | 二维 |

### Construction（6；full 3 / partial 1 / pending 2）

| 工具 | token | 状态 | 边界 |
|---|---|---|---|
| Tape Measure | `DIST` | partial | 真实两点距离；`DIST GUIDE` 在两点完成后写一条**持久构造参考线**（`tables/guides.json`；两点重合时自动写 point 标记；非实体、无图层/导出/拾取，渲染为有界**虚线段**（transient overlay，dash/gap = 6/4 逻辑像素 × `view_scale`；无 view_scale 时回退模型 period；线长仍是模型包围盒 10 倍、≥1000 单位，不是真无限线））；`GUIDE POINT <x> <y> <z>` 写点标记（minor 27，渲染为 6px 三轴十字）；`GUIDE STYLE COLOR r g b`/`STYLE DASH ON|OFF` 写共享样式行（minor 30；concept 域 Styles 托盘有颜色/虚线控件）；`DIST GUIDE` 两点重合时也自动写该点标记，`GUIDE DELETE <n>` 单条删除、`GUIDE LIST`/`CLEAR` 管理。无偏移/延伸/多次复制模式 |
| Dimension | `DIMLINEAR` | full | 是 CAD 标注，不是 SketchUp LayOut 尺寸 |
| Protractor | `PROTRACTOR` | partial | 真实三点角度量具（`BeginMeasurement(Protractor)`：先点顶点、再各点一条射线，结果按 `Angle3Points` 记录并格式化角度；普通模式只读、不写文档）+ 托盘/工具栏/菜单真实工具；`PROTRACTOR GUIDE` 在三点点完后从顶点沿**第二条射线**写一条持久参考线（同一 `tables/guides.json`）；guide 点/单条删除/清空与 Tape 共用 `GUIDE POINT`/`DELETE`/`LIST`/`CLEAR`。**未建模**：其它 guide 类型/角度圆弧、无限线渲染与虚线 |
| Text | `TEXT` | full | 二维文字 |
| Axes | `UCS` | full | 会话坐标轴 |
| 3D Text | `TEXT3D` | partial | `TEXT3D <x> <y> <z> <height> <extrusion> <font\|-> <text>`：用宿主已安装的 outline 字体引擎取**已填充字形面**（与显示/TXTEXP 同一份），挤成 `AcDbSubDMesh`（上下盖 + 每个边界边侧墙），一次事务/一步撤销；SHX 描边字体按 `text3d.not_solid` 拒绝、超出预算按 `text3d.budget` 拒绝；无字体选择对话框、旋转/对齐/斜体/描边、截面或圆角 |

### Camera（9；full 6 / pending 3）

| 工具 | token | 状态 | 边界 |
|---|---|---|---|
| Orbit | `ORBIT` | full | |
| Pan | `PAN` | full | |
| Zoom In | `ZOOMIN` | full | |
| Zoom Out | `ZOOMOUT` | full | |
| Zoom Extents | `FIT` | full | |
| Previous | `VIEWBACK` | full | |
| Position Camera | `CAMERA` | partial | 真实两点放置：先点眼睛、再点目标 → `CAMERA` 设一个透视相机（eye/target/世界 +Z up、沿用当前 FOV），保存 2D 相机并进入 3D（只改视图，无文档写入/撤销，记 `ZOOM PREVIOUS`）。`CAMERA TWO-POINT` 把当前透视相机**放平**（target z = eye z、world +Z up、保留镜头），竖直边保持竖直；已水平时报 `camera.two_point_unchanged`。**未建模**：lens-shift 倾斜两点透视（相机俯仰时仍保持竖线）、相机高度锁定/拖动；`CAMERA WALK` 仍按名拒绝（WALK/LOOKAROUND 是独立工具） |
| Walk | `WALK` | partial | 真实视图导航：裸 `WALK` 一点捕获（走到该地面点，眼睛 z 不变）或 `WALK <distance>` 沿水平视向前进（负值后退），target 随 eye 平移（视向/距离不变），记 `ZOOM PREVIOUS`，只改视图、Viewer 合法；平面/正交视图 `walk.requires_perspective`、零步 `walk.no_distance`、垂直视向 `walk.degenerate_view`；不含碰撞/上下楼/连续按键漫游 |
| Look Around | `LOOKAROUND` | partial | 真实视图导航：裸 `LOOKAROUND` 一点捕获（固定 eye 看向该点）或 `LOOKAROUND <yaw> <pitch>`（度，绕世界 +Z 偏航、绕相机右轴俯仰，保持视距），记 `ZOOM PREVIOUS`，只改视图、Viewer 合法；平面视图 `lookaround.requires_perspective`、看向眼点 `lookaround.no_direction`、垂直极点 `lookaround.pole`；不含自由拖动/连续环视 |

### Views（7；full 7）

七个标准视图（top/bottom/front/back/left/right/isometric）由 `STANDARD_VIEWS` 与
`cad_set_view` 承担。

### Solid Tools（6；full 3 / pending 3）

| 工具 | token | 状态 | 边界 |
|---|---|---|---|
| Union | `UNION` | full | 精确 `Body3d{Solid,Exact}` 或闭合网格；混合选择拒绝；网格结果非精确 |
| Subtract | `SUBTRACT` | full | 同上 |
| Intersect | `INTERSECT` | full | 同上 |
| Outer Shell | `OUTERSHELL` | partial | `OUTERSHELL` 把所选**闭合网格体**经既有 manifold 布尔路径合并成一个外壳（`AcDbSubDMesh`，一次事务/一步撤销）；启动前按名拒绝 `<2` 个操作数、开放壳（`outershell.open_shell`）、非流形（`non_manifold`）、精确实体（`not_mesh`）、超预算（`budget`）与无 modeler 载体；不含“保留内部空腔/逐体 shell 偏移”，也不接受精确实体或混合类 |
| Trim | `TRIMSOLID` | partial | `TRIMSOLID`：**第一个选中体是目标**、其余选中体是刀具；经既有 manifold 布尔做 `Subtract` 折叠（`target − cutter…`），只删除目标并插入结果，**刀具行原样保留**；一行 `AcDbSubDMesh`、一次事务/一步撤销。启动前按名拒绝 `<2`、子元素/实例路径、跨空间、非网格（精确实体提示用 `SUBTRACT`）、开放壳/非流形、输入 >1M 三角形与无 modeler 载体；不含“刀具也参与后续分组/材质继承”或精确实体裁剪 |
| Split | `SPLITSOLID` | partial | `SPLITSOLID`：**第一个选中体是目标**、其余是切割体；worker 先并集切割体，再算 `target − ∪cutters`（外部）与 `target ∩ ∪cutters`（内部），**两块同时插入**、目标删除、切割体保留，一次事务/一步撤销；`splitsolid.no_intersection` / `empty_piece` 拒绝无解切割；开放壳/非流形/精确实体/跨空间/超预算/无 modeler 都在启动前按名拒绝；不含“切割体本身被分割”或精确实体分割 |

### Standard（11；full 11）

New/Open/Save/Cut/Copy/Paste/Erase/Undo/Redo/Print/Toolbars 由既有文件与编辑命令承担；
它们不是 SketchUp 特有，但官方工具栏包含这些单元。

## 4. Default Tray 面板（不计入 69 单元）

| 面板 | 状态 | 边界 |
|---|---|---|
| Entity Info | full | 属性行来自 `PropertyRow`；网格行有限 |
| Components | partial | 真实面板：列出块定义（名称 + 成员数）；每行 **Insert** 走 `INSERT <name>`（块名预先交给捕获，点一下就放置），名称输入 + **Create from selection** 走 `BLOCK <name>`（基点点一下即定义）；命令层仍负责未知块/空选择/重名等具名拒绝。**未建模**：组件缩略图、右键替换/重载、嵌套组件的就地编辑、拖放与组件浏览器搜索；面板只列出前 12 个（`cad_status.blocks` 的上限） |
| Styles | partial | 真实面板：两行 `WIREFRAME`/`SHADED` 走既有 `VSCURRENT <token>`（当前 token 高亮，来自 `cad_status.visualStyle`），另保留界面主题切换与 **实体边线** 开关（`EDGES`，`cad_status.bodyEdges`；保留填充、隐藏 body 边线覆盖层，独立线实体不受影响）。**未建模**：基于材质/贴图的 SketchUp 样式、profiles/深度提示/端点/背景等其它边/面开关、样式库保存/缩略图/水印；界面主题不是 SketchUp Style |
| Tags | full | 走图层面板 |
| Instructor | full | 工具提示/步骤 |
| Materials | partial | 托盘真实列出**文档自己的**材质（`cad_materials` / `kind:"materials"`：名称、sRGB 色、透明度；来自 `tables/materials.json`）并可直接 `PAINT <name>` 重涂；下方保留内置 12 色调色板（首次涂刷落成文档行），`MATERIALS` 报告仍可用。**逐面涂刷**：选中网格面（pick 的 `SubElementId` / `Mesh.face_sources` key）时只涂该面，材质表行的 `faces` 快照（minor 25）；渲染器按材质把网格拆成多个 fragment（未涂面保留实体颜色，透明材质乘实体 alpha），陈旧 face key 不重映射并报 `representation.material_face_inert`。**编辑器**：`MATERIAL <name> RENAME|COLOR|TRANSPARENCY|DELETE` 直接改文档材质行（rename 保 seq/赋值；delete 清实体与面赋值；一步撤销）；面板文档材质行提供 **Edit**（名称/颜色/透明度，按变更字段各发一条命令；多字段保存是多步撤销）与 **Delete**。**未建模**：贴图/纹理、材质库、材质缩略图/拖放；整实体涂刷仍在 |
| Scenes | partial | 真实面板 + `SCENE SAVE/RESTORE/LIST/DELETE`：场景保存相机、2D/3D、工作平面、空间与 `WIREFRAME`/`SHADED` 样式（`tables/scenes.json`，minor 20），保存每个图层**保存时的有效可见性**（行的 `tags` 快照，minor 23）、文档 **Shadows/Fog 记录**（行的 `display` 快照，minor 24）、**显式逐实体柔化角度**（行的 `soften` 快照，minor 28）、**无显式柔化的 body 列表**（行的 `soften_cleared`，minor 29）、**会话隐藏的顶层实体**（行的 `hidden`，minor 31；空数组是真实快照，旧档缺字段则不触碰隐藏集）与**文档剖面**（行的 `section`，minor 32；`null` 是真实“无平面”捕获）；恢复把标签快照作为会话图层覆盖应用（不写图层表），阴影/雾各写一次，柔化按一次 display 属性事务写回显式值并清空 cleared body，隐藏集只替换当前文档的顶层 refs（嵌套/子元素隐藏保留），剖面有差异时写一次 section slot（一步撤销），Viewer 下有差异的恢复在改动前按 `PermissionDenied` 拒绝。**未建模**：场景缩略图、隐藏几何/边级快照、多剖面/section box；剖面带孔/非流形切面不封盖、不裁 wipeout mask/点云 |
| Shadows | partial | 托盘面板（开关 + 方位角/高度角 + 应用）+ `SHADOWS [ON|OFF|SUN <az> <elev>]`：把文档太阳当作方向光替换相机相对 key light，`tables/display.json`（minor 22），一次写入一步撤销；**投射阴影（shadow map）、软化、地面淡出 NOT IMPLEMENTED**，面板与命令报告都写明 |
| Fog | partial | 托盘面板（开关 + 区间 + 颜色）+ `FOG [ON|OFF|RANGE <start> <end>|COLOR <r> <g> <b>]`：可见颜色 shader 按片元到视点距离向雾色混合（模型单位区间，`tables/display.json` minor 22）；没有天空/背景混合、没有距离百分比表达 |
| Match Photo | partial | 真实面板 + host read（`cad_match_photo` / `kind:"matchPhoto"`）：明确报告 `modeled:false` 与稳定原因（照片匹配模型未建模：无照片导入/相机求解/照片背景），动作按钮 dispatch `MATCHPHOTO`，命令以 `matchphoto.not_implemented` 具名拒绝；不伪造照片列表 |
| Soften Edges | partial | 托盘面板（角度 0..=180 + 应用）+ `SOFTENEDGES [<angle>]`：按相邻显示三角形夹角隐藏柔化边（默认 20°，0 只隐藏共面边），per-entity 存 `render_attributes.soften`（minor 21），一次写入一步撤销；没有 Smooth normals/逐边手工柔化；场景可捕获/恢复显式 `render_attributes.soften`，并清除保存时无显式值的 body（ADR-0090/0094） |
| Overlays | partial | 真实面板 + host read（`cad_overlays` / `kind:"overlays"`）：只读列出**渲染器实际拥有**的叠加通道（坐标轴/网格/选中高亮/捕捉提示/选择框/悬停高亮/测量读数/绘制预览）及其当前开关；没有用户管理的叠加定义模型，动作按钮 dispatch `OVERLAYS`，以 `overlays.not_implemented` 具名拒绝 |
| Outliner | partial | 托盘面板（`cad_outliner` 只读投影：分组 / INSERT / 模型空间实体 / 块成员，前序 + `depth`，本地展开折叠）+ 点名称走既有 `cad_select_entities` 选择（分组发全部成员；块成员行不可选并说明原因）；**分组行可改名**：行内输入 dispatch 既有 `GROUPEDIT "<name>" RENAME "<new>"`（一步撤销；名称含 `"`/换行时按钮不提供）；行可 **Hide**（先 `cad_select_entities` 再 `HIDEOBJECTS`，隐藏态由 `cad_outliner.hidden` 标注）、隐藏行可 **Show**（同一选择后 `SHOWOBJECTS`，只移除选中 refs）并有 **Show all hidden**（`UNISOLATEOBJECTS`）。**多选 + Group selected**：Ctrl/Cmd/Shift 点击行累计选择，名称输入 dispatch 既有 `GROUP CREATE "<name>"`（一步撤销；名称含 `"`/换行时按钮禁用）。**拖拽归组**：把行拖到分组行上先 `GROUPEDIT "<旧组>" REMOVE` 再从 `GROUPEDIT "<目标>" ADD`（两步撤销）；分组成员行有 **Remove from group**；拖到 **Ungroup** 区域按旧组 `GROUPEDIT REMOVE`（一步/组）。**行重排**：顶层 entity/INSERT 行与**分组成员 entity 行** ↑/↓ 先选同 scope 的 [相邻行, 当前行] 再 dispatch `DRAWORDER BELOW/ABOVE`（一步撤销；顶层与组内成员均按 `(draw_order, id)` 排序）。分组行本身、块成员顺序与嵌套展开仍未建模，只投影模型空间 |

## 5. 本轮验收与后续

- **P-MESH-0（已实现 2026-11-26）**：网格内核接缝 + 真实布尔/修复**原语**
  （`MeshModeler` 在 `cad-kernel-manifold`，真实内核测试）。
- **P-MESH-1a/1b（已实现 2026-11-26）**：复用 `UNION`/`SUBTRACT`/`INTERSECT` 接网格体
  （结果写新的 `AcDbSubDMesh`，不冒充来源/精确）；`MESHREPAIR`（orientation）与
  `MESHREBUILD`（winding rebuild）修复闭合网格，返回 `Partial` 报告；worker + 取消 +
  陈旧检查 + 一条事务/一步撤销。open shell 仍按 `mesh.not_closed` 拒绝。
- **P-MESH-2（已实现 2026-11-26）**：SKP 面身份（`Mesh.face_sources`）+ 面选择
  `MOVE`/`PRESSPULL` + `MESHREFINE` 细分 + `MESHSMOOTH` 平滑；边选中使用几何派生
  `mesh:edge:<bits>` key，`MOVE` 平移两个端点，全部在无内核共享层 `cad-mesh-edit`。
  wire-only 边/跨多个三角形边的 stored polyline 仍不可编辑。
- **P-MESH-3（已实现 2026-11-26）**：`MESHCAP` 对简单边界环做补片并逐洞报告；平面环直接补，
  非平面环投影到最佳拟合平面后补并标 `non_planar`；多个独立环各自补；共面嵌套环按奇偶深度
  成组，外环以内环为孔做环带（annulus），深度 2 的岛独立补；自交/退化环、同向绕行或找不到
  可见桥的嵌套组仍按名拒绝且保持开口。
- **真实 SKP 样本（2026-11-26）**：本地 `house.skp`（2,237,796 bytes，SHA-256
  `9cd2689df7c1bf0d1ce0d496e4e83c9367abab6d8ff3e21dc9bb8162e982dd30`，不入库）导入
  64 meshes / 1440 triangles；`face_sources` 1440 条、28 个 distinct keys；网格内核拓扑
  21 closed / 41 open / 2 non-manifold（后者也带 boundary edge，按 `mesh.not_closed` 拒绝），
  21 个闭合网格修复成功、41 个开放网格按 `mesh.not_closed` 拒绝。该 SKP 测试仍是内核级；
  app 命令路径由 `mesh_command`/`mesh_real_samples` 覆盖。详见 `docs/validation-mesh-2026-11-26.md`。
- 工具层与托盘面板都没有 `pending`；Match Photo/Overlays 是只读/具名拒绝的真实面板（ADR-0075），
  其余面板也都已有真实 body；Shadows/Fog/Outliner/Scenes/Soften Edges/
  Protractor 都已是有真实命令与面板的 `partial`（`cad_display`/`cad_outliner`/`cad_scenes`）。
  任何一项实现都要同时改本表、改 UI（从 `pending` 改为命令）并补契约测试。
