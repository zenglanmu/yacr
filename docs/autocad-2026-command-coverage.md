# AutoCAD 2026 官方命令覆盖清单（按字母序）

> **这是清单审计，不是验收。** 「已实现」只表示**命令路径存在**并有契约/单测覆盖；不表示与
> AutoCAD 行为等价，也不表示在真实窗口/真实 GPU/真实图纸上验证过。逐条能力边界以
> `docs/compatibility.md` 为权威；更早的、手工维护的缺口审计见 `docs/autocad-command-coverage.md`。
>
> **证据边界**：本清单的状态来自**源码表 + 既有契约测试 + 本文件的静态映射**；
> 没有 GUI/像素/运行时证据。Windows/macOS/Android/Web 结论均为 **NOT VERIFIED**。

## 数据来源

- 官方命令索引：<https://help.autodesk.com/view/ACD/2026/ENU/?page=commands>
- 27 个字母页（`3D*` + `A*`–`Z*`；K、Y 官方为空）以及 83 个 `-VARIANT` 独立参考页，
  检索日期 **2026-11-06（UTC）**，全部返回 HTTP 200。
- 官方索引 **981** 条 = **815** 条标准命令 + **83** 条 Express Tools + **83** 条
  `-` 前缀命令行变体（官方不列在 A–Z 索引，但在各自参考页交叉链接）。
- 只含**命令**，不含系统变量；不含 `acad.pgp` 别名（Autodesk 单独发布别名表）。

## 总计

| 指标 | 数量 |
| --- | ---: |
| 官方命令总数 | **981** |
| 完整实现 | **50** |
| 部分实现（有写明的界） | **64** |
| **已实现合计（完整 + 部分）** | **114** |
| 未实现 | **867** |

> **禁止的读法**：上述百分比/计数只是本清单的逐条计数，**不是**「AutoCAD 兼容率」。
> 同名命令的选项、提示顺序、系统变量副作用基本没有逐条对照；`部分实现` 的界之外一律
> 显式 `Unsupported`/`InvalidInput`。

## 状态口径

- **已实现**：可键入的命令路径存在，行为落库/落会话状态；未支持选项显式拒绝。
- **部分实现**：命令可用，但有下表写明的界；界之外显式拒绝。
- **未实现**：键入该名得到 `command.unsupported`（或 chrome 不承认该名字）。
- **同名不同义**：`MEASURE` 在 AutoCAD 是沿对象放置点/块，本项目是测量入口，
  因此**不计**为已覆盖（表中状态仍是「未实现」并注明）。

## 已实现 / 部分实现命令索引（按字母序）

| Command | 状态 | 说明 |
| --- | --- | --- |
| `ABOUT` | 已实现 | chrome 本地选择子：打开 About 对话框（与 File > About 同一路径） |
| `ALIGN` | 部分实现 | 仅两对点（平移 + 旋转 + 可选均匀缩放）；无 3 对点 3D、无非均匀缩放 |
| `ARC` | 已实现 | — |
| `AREA` | 已实现 | — |
| `ARRAYPOLAR` | 部分实现 | 独立副本、非关联；角度步长为我们的规则 |
| `ARRAYRECT` | 部分实现 | 独立副本、非关联；无 Path/Levels/关联编辑 |
| `AUDIT` | 部分实现 | v1 只检测：逐实体跑 cad-db 校验并报告诊断；不修复（AutoCAD 的 corrects errors 未实现） |
| `BLOCK` | 已实现 | — |
| `BREAK` | 部分实现 | 两点打断；无 F 第一点覆盖、无多段打断；LWPOLYLINE 按名拒绝 |
| `BREAKATPOINT` | 部分实现 | 交互工具：先拾取对象再指定断点（断点可投影到对象上）；仅 LINE/ARC/开放 LWPOLYLINE；圆/闭合多段线/SPLINE 按名拒绝（闭合对象不拆分） |
| `CHAMFER` | 部分实现 | 两线形式 + 独立 CHAMFER POLYLINE 选择子；无 Angle/Multiple/Trim |
| `CIRCLE` | 已实现 | — |
| `CLEANSCREENOFF` | 部分实现 | chrome 本地选择子：显式退出 clean-screen；只恢复 chrome 可见性，不重开各面板 |
| `CLEANSCREENON` | 部分实现 | chrome 本地选择子：显式进入 clean-screen；只隐藏 chrome，不记录各面板的独立开关 |
| `COPY` | 已实现 | — |
| `COPYBASE` | 部分实现 | 应用内剪贴板 + 基点；不是 OS 剪贴板，不跨应用 |
| `COPYCLIP` | 部分实现 | 应用内剪贴板；不是 OS 剪贴板，不跨应用 |
| `CUTBASE` | 部分实现 | 应用内剪贴板 + 基点；不是 OS 剪贴板，不跨应用 |
| `CUTCLIP` | 部分实现 | 应用内剪贴板；不是 OS 剪贴板，不跨应用 |
| `DIMALIGNED` | 已实现 | — |
| `DIMANGULAR` | 部分实现 | 仅两线形式可交互；三点形式只到载荷层 |
| `DIMBASELINE` | 已实现 | — |
| `DIMCENTER` | 已实现 | — |
| `DIMCONTINUE` | 已实现 | — |
| `DIMDIAMETER` | 已实现 | — |
| `DIMLINEAR` | 已实现 | — |
| `DIMORDINATE` | 已实现 | — |
| `DIMRADIUS` | 已实现 | — |
| `DIST` | 已实现 | — |
| `DIVIDE` | 部分实现 | 仅 LINE/ARC/CIRCLE/直线段 LWPOLYLINE；按等弧长放 POINT |
| `ELLIPSE` | 已实现 | — |
| `ERASE` | 已实现 | — |
| `EXPLODE` | 部分实现 | 一次一层；拒绝 Dimension/MULTILEADER/非均匀 INSERT |
| `EXPORT` | 部分实现 | PLOT 别名：仅矢量 SVG/PDF |
| `EXTEND` | 部分实现 | 目标仅 LINE/ARC；SPLINE/CIRCLE 目标拒绝 |
| `FILLET` | 部分实现 | 两线形式 + 独立 FILLET POLYLINE 选择子；无 Multiple/Trim |
| `FS` | 部分实现 | Express 工具、仅命令行（无功能区按钮）；取当前唯一选择为种子，把**直接**接触（相交或相切都算）的可见模型空间对象并入选择。默认 `FSMODE=OFF`：链式接触（`FSMODE=ON`）未实现；不展开块实例成员 |
| `GRID` | 已实现 | — |
| `HATCH` | 部分实现 | 整实体闭合边界 + LINE/ARC 边链；无 .pat/关联性/实时预览 |
| `HATCHGENERATEBOUNDARY` | 部分实现 | 预选择 hatch；每个 loop 一条非关联闭合多段线（同层同空间、保留 bulge、含内岛）；无关联/无选项提示 |
| `HATCHTOBACK` | 部分实现 | 全部模型空间 hatch 置底（仅改写存储 draw order、保持相对次序、可撤销）；图纸空间/块内不重排 |
| `ID` | 已实现 | — |
| `INSERT` | 已实现 | — |
| `JOIN` | 部分实现 | 仅共线 LINE 或同心相续 ARC |
| `LAYCUR` | 部分实现 | 预选择输入（不弹 Select objects）；把选中对象移到当前层；当前层缺失/目标层锁定显式拒绝；实体写入可撤销 |
| `LAYER` | 已实现 | chrome 本地选择子：打开图层面板 |
| `LAYERCLOSE` | 已实现 | chrome 本地选择子：关闭图层面板（面板关闭按钮走同一命令） |
| `LAYFRZ` | 部分实现 | 预选择输入（不弹 Select objects）；仅 Entity 选择，无 Settings/Viewports/Block 选项；v1 在全部视口冻结；当前层整命令拒绝；图层表写入不可撤销 |
| `LAYERP` | 部分实现 | 会话内有界 16 步；不持久化 |
| `LAYISO` | 已实现 | — |
| `LAYLCK` | 部分实现 | 预选择输入（不弹 Select objects）；锁定选中对象所在的不同图层（含当前层）；图层表写入不可撤销 |
| `LAYMCUR` | 部分实现 | 预选择输入（不弹 Select objects）；取第一个选中对象的图层并用既有的锁定/冻结/关闭当前层拒绝；仅会话状态 |
| `LAYOFF` | 已实现 | — |
| `LAYON` | 已实现 | — |
| `LAYTHW` | 部分实现 | 无选择输入（载荷即拒绝）；解冻全部冻结图层；不处理视口级冻结（本构建无视口模型）；图层表写入不可撤销 |
| `LAYULK` | 部分实现 | 预选择输入（不弹 Select objects）；解锁选中对象所在的不同图层；图层表写入不可撤销 |
| `LAYUNISO` | 已实现 | — |
| `LENGTHEN` | 部分实现 | 仅 LINE/ARC，只改末端 |
| `LINE` | 已实现 | — |
| `LIST` | 部分实现 | 读取**当前选择**而非 AutoCAD 的交互式 “Select objects”；逐对象报告 id/handle/type/layer/space/draw order、标准颜色/线型/线宽/透明度与几何摘要。不建模厚度/拉伸方向；不可寻址引用逐条报告后继续 |
| `MATCHPROP` | 部分实现 | 只转移 layer/color/linetype/lineweight/transparency |
| `MIRROR` | 部分实现 | 保留原件；拒绝 Text/MultiLeader |
| `MLEADER` | 部分实现 | 单簇/2 顶点/仅直线；无 Options |
| `MODEL` | 已实现 | payload-free 选择子，复用 SPACE MODEL 的校验 + 自动取景；Model 标签页走同一命令 |
| `MOVE` | 已实现 | — |
| `MTEXT` | 部分实现 | 两角宽度模型；无行距/分栏/背景填充 |
| `NEW` | 已实现 | — |
| `OFFSET` | 部分实现 | LINE/CIRCLE/ARC/LWPOLYLINE；ELLIPSE 拒绝；无全局自交修剪 |
| `OOPS` | 部分实现 | v1 只恢复最近一次 ERASE（不覆盖 AutoCAD 的 BLOCK/WBLOCK 删除）；恢复实体用新 id；撤销恢复副本但不恢复快照 |
| `OPEN` | 已实现 | — |
| `OPTIONS` | 已实现 | chrome 本地选择子：打开选项对话框 |
| `ORTHO` | 已实现 | — |
| `OSNAP` | 已实现 | — |
| `OVERKILL` | 部分实现 | v1 仅删除完全相同的重复对象（同空间/图层/几何，1e-9）；无部分重叠、共线合并、圆弧覆盖圆、折线段优化、容差与图层/颜色忽略 |
| `PAN` | 已实现 | — |
| `PASTEBLOCK` | 部分实现 | 应用内剪贴板 → 自动命名块；不是 OS 剪贴板 |
| `PASTECLIP` | 部分实现 | 应用内剪贴板；不是 OS 剪贴板，不跨应用 |
| `PASTEORIG` | 部分实现 | 应用内剪贴板；不是 OS 剪贴板，不跨应用 |
| `PLAN` | 部分实现 | payload-free 选择子：Top/plan 2D 视图；无 UCS/World/Current 选项（本构建无 UCS 模型） |
| `PLINE` | 已实现 | — |
| `PLOT` | 部分实现 | 仅矢量 SVG/PDF；无 CTB/STB、无批量多布局 |
| `POINT` | 已实现 | — |
| `POLYGON` | 已实现 | 默认内接（顶点在拾取点上）；外切变体是独立的多词选择子（见下一行） |
| `POLYGON CIRCUMSCRIBED` | 已实现 | **本项目多词选择子**（不属于官方 981 条，不计入上方总计）：`POLYGON` 的 `Circumscribed about circle`（`C`）构造；第二点落在**内切（相切）圆**上，拾取方向为第一条边中点，`R = r / cos(π/n)`；共享会话 `POLYSIDES`；无 Edge/数字半径定向 |
| `PROPERTIES` | 已实现 | chrome 本地选择子：打开特性面板 |
| `PROPERTIESCLOSE` | 已实现 | chrome 本地选择子：关闭特性面板（面板关闭按钮走同一命令） |
| `QSAVE` | 部分实现 | SAVE 别名：有损 DXF 另存；源为 .dwg 时可实验性导出 DWG（新文件、默认 <stem>-export.dwg、绝不覆盖源；仅合成 self-roundtrip 证据，vendor/AutoCAD NOT VERIFIED） |
| `RAY` | 部分实现 | 两点创建，但落库为裁剪到模型包围盒的有限线段 |
| `RECTANG` | 已实现 | — |
| `REDO` | 已实现 | — |
| `REVCLOUD` | 部分实现 | v1 对象模式：把一条选中的闭合直线段 LWPOLYLINE 的每条边换成一段向外弧；无矩形/多边形绘制、无 Arc length |
| `REVERSE` | 部分实现 | 仅 LINE/LWPOLYLINE/SPLINE，当前选择为全部输入；无交互拾取提示，无螺旋/3D 变体 |
| `ROTATE` | 已实现 | — |
| `SAVE` | 部分实现 | 有损 DXF 另存；源为 `.dwg` 时可**实验性导出 DWG**（上游 `acadrust::DwgWriter`，新文件、默认 `<stem>-export.dwg`、绝不覆盖源）；DWG 导出有损/不完整，仅合成 self-roundtrip 证据，vendor/AutoCAD **NOT VERIFIED** |
| `SAVEAS` | 部分实现 | SAVE 别名：有损 DXF 另存；源为 .dwg 时可实验性导出 DWG（新文件、默认 <stem>-export.dwg、绝不覆盖源；仅合成 self-roundtrip 证据，vendor/AutoCAD NOT VERIFIED） |
| `SCALE` | 已实现 | — |
| `SCALETEXT` | 已实现 | — |
| `SELECT` | 部分实现 | 以选择载荷设置会话选择；不是 AutoCAD 的交互式 SELECT |
| `SELECTSIMILAR` | 部分实现 | v1 仅按几何类型匹配并并入当前选择；无 layer/颜色/线型等属性匹配；仅模型空间 |
| `SETBYLAYER` | 部分实现 | 预选择输入（不弹 Select objects）；只重置颜色/线型/线宽为随层；不重置透明度与材质，文字/标注/填充/多段线厚度未建模；Include blocks 未实现；锁定图层整命令拒绝；显示写入可撤销 |
| `SNAP` | 已实现 | — |
| `SPLINE` | 已实现 | — |
| `STATUS` | 部分实现 | 报告图形/数据库 id、修订、实体数（模型/图纸/块）、图层（off/frozen/locked）、块/线型/文字样式/标注样式/多重引线样式/布局数、模型空间范围、当前图层、会话模式与网格/捕捉/正交/对象捕捉状态、缩放历史深度。创建/编辑时间、内存、图形界限与基点**不跟踪**（无图形头模型），以 `status.not_tracked` 显式说明 |
| `STRETCH` | 部分实现 | 恒交叉窗口；TEXT/MultiLeader 整命令拒绝 |
| `TCASE` | 部分实现 | Express 工具、选择式：只改 Text/MText 的可见字符大小写；裸命令默认 Title，键入 `TCASE UPPER/LOWER/TITLE/SENTENCE/TOGGLE`（或 `U/L/T/S/G`）；MTEXT 格式转义逐字保留；属性/标注文字未建模（`tcase.not_reset`）；MULTILEADER 走 `with_text` |
| `TEXT` | 已实现 | — |
| `TEXTEDIT` | 部分实现 | 整串替换；无光标级编辑、不解析 MTEXT 标记 |
| `TORIENT` | 部分实现 | Express 工具、选择式：裸命令按 180° 步进归一到 `(-π/2, π/2]`（Most Readable），`TORIENT <角度>` 设为绝对旋转；只改 rotation，位置/字高/内容不变；非 Text（含 MULTILEADER/INSERT 属性）报告 `torient.not_reset`，不整体失败 |
| `TRIM` | 部分实现 | 目标 LINE/ARC/CIRCLE/SPLINE；边界含折线/样条采样 |
| `TXT2MTXT` | 部分实现 | 选择式：至少一个模型空间 Text/MText，按上→下、左→右稳定排序后用 `\P` 原样拼接成一个 `AcDbMText`，沿用首个源的样式/位置/字高/旋转/对齐/层/绘制顺序/`wrap_width`，同一事务删除全部源；设置对话框与强制统一行距未实现 |
| `U` | 已实现 | — |
| `UNDO` | 已实现 | — |
| `WIPEOUT` | 部分实现 | v1 多边形遮罩：拾取边界点（≥3，回车结束）→ 一个 `Mask`（type-key `AcDbWipeout`，`inverted: false`，环不重复首点）。无边框模型（Frames ON/OFF/不打印）、无 `Polyline`/`Erase Polyline` 选项、不检查自交、不能拾取已有遮罩 |
| `XLINE` | 部分实现 | 两点创建，但落库为裁剪到模型包围盒的有限线段 |
| `ZOOM` | 部分实现 | 无 OBJECT/DYNAMIC/图纸空间 nXP；WINDOW/PREVIOUS 已实现 |

## 全量清单（官方字母序）

| # | Command | 状态 | 官方说明 | 说明 / 本项目选择子 |
| ---: | --- | --- | --- | --- |
| 1 | `3DALIGN` | 未实现 | Aligns objects with other objects in 2D and 3D. | — |
| 2 | `3DARRAY` | 未实现 | Creates nonassociative, 3D rectangular or polar arrays. | — |
| 3 | `3DCLIP` | 未实现 | Opens the Adjust Clipping Planes window, where you can specify what portions of a 3D model to display. | — |
| 4 | `3DCORBIT` | 未实现 | Rotates the view in 3D space with continuous motion. | — |
| 5 | `3DDISTANCE` | 未实现 | Starts the interactive 3D view and makes objects appear closer or farther away. | — |
| 6 | `3DDWF` | 未实现 | Creates a 3D DWF or 3D DWFx file of your 3D model and displays it in the DWF Viewer. | — |
| 7 | `3DEDITBAR` | 未实现 | Reshapes splines and NURBS surfaces, including their tangency properties. | — |
| 8 | `3DFACE` | 未实现 | Creates a three-sided or four-sided surface in 3D space. | — |
| 9 | `3DFLY` | 未实现 | Changes the 3D view in a drawing interactively to create the appearance of flying through the model. | — |
| 10 | `3DFORBIT` | 未实现 | Rotates the view in 3D space without constraining roll. | — |
| 11 | `3DMESH` | 未实现 | Creates a free-form polygon mesh. | — |
| 12 | `3DMOVE` | 未实现 | In a 3D view, displays the 3D Move gizmo to aid in moving 3D objects a specified distance in a specified direction. | — |
| 13 | `3DORBIT` | 未实现 | Provides interactive viewing in 3D with the mouse. Right-click to display viewing options. | — |
| 14 | `3DORBITCTR` | 未实现 | Sets a specific center of rotation in 3D Orbit view. | — |
| 15 | `-3DOSNAP` | 未实现 | Uses command prompts to set running object snap modes for 3D objects. | — |
| 16 | `3DOSNAP` | 未实现 | Sets the object snap modes for 3D objects. | — |
| 17 | `3DPAN` | 未实现 | Shifts the view without changing the viewing direction or magnification. | — |
| 18 | `3DPOLY` | 未实现 | Creates a 3D polyline. | — |
| 19 | `3DPRINT` | 未实现 | Specifies 3D Plot settings, and prepares your drawing for 3D printing. | — |
| 20 | `3DPRINTSERVICE` | 未实现 | Creates an STL file that can be sent to a 3D printing service. | — |
| 21 | `3DROTATE` | 未实现 | In a 3D view, displays the 3D Rotate gizmo to aid in revolving 3D objects around a base point. | — |
| 22 | `3DSCALE` | 未实现 | In a 3D view, displays the 3D Scale gizmo to aid in resizing 3D objects. | — |
| 23 | `3DSIN` | 未实现 | Imports a 3ds Max (3DS) file. | — |
| 24 | `3DSWIVEL` | 未实现 | Changes the target of the view in the direction that you drag. | — |
| 25 | `3DWALK` | 未实现 | Changes the 3D view in a drawing interactively to create the appearance of walking through the model. | — |
| 26 | `3DZOOM` | 未实现 | Zooms in and out in a perspective view. | — |
| 27 | `ABOUT` | 已实现 | Displays information about your product. | chrome 本地选择子：打开 About 对话框（与 File > About 同一路径） |
| 28 | `ACADINFO` | 未实现 | Creates a file that stores information about your AutoCAD installation and current setup. | — |
| 29 | `ACISIN` | 未实现 | Imports an ACIS (SAT) file and creates 3D solid, body, or region objects. | — |
| 30 | `ACISOUT` | 未实现 | Exports 3D solid, region, or body objects to an ACIS file. | — |
| 31 | `ACTBASEPOINT` | 未实现 | Inserts a base point or base point prompt in an action macro. | — |
| 32 | `ACTIVITYINSIGHTSCLOSE` | 未实现 | Closes the Activity Insights palette which displays activities for the current drawing. | — |
| 33 | `ACTIVITYINSIGHTSOPEN` | 未实现 | Opens the Activity Insights palette to view activities for the current drawing. | — |
| 34 | `ACTMANAGER` | 未实现 | Manages action macro files. | — |
| 35 | `ACTRECORD` | 未实现 | Starts the recording an action macro. | — |
| 36 | `-ACTSTOP` | 未实现 | Stops the Action Recorder and, from the command line, provides the option of saving the recorded actions to an action macro file | — |
| 37 | `ACTSTOP` | 未实现 | Stops the Action Recorder and provides the option of saving the recorded actions to an action macro file. | — |
| 38 | `ACTUSERINPUT` | 未实现 | Pauses for user input in an action macro. | — |
| 39 | `-ACTUSERMESSAGE` | 未实现 | From the command line, inserts a user message into an action macro. | — |
| 40 | `ACTUSERMESSAGE` | 未实现 | Inserts a user message into an action macro. | — |
| 41 | `ADCCLOSE` | 未实现 | Closes DesignCenter. | — |
| 42 | `ADCENTER` | 未实现 | Manages and inserts content such as blocks, xrefs, and hatch patterns. | — |
| 43 | `ADCNAVIGATE` | 未实现 | Loads a specified drawing file, folder, or network path in the DesignCenter Folders tab. | — |
| 44 | `ADDSELECTED` | 未实现 | Creates a new object of the same type and general properties as a selected object, but with different geometric values. | — |
| 45 | `ADJUST` | 未实现 | Adjusts the fade, contrast and monochrome settings of the selected underlay (DWF, DWFx, PDF, or DGN), or image. | — |
| 46 | `ALIASEDIT` | 未实现 | Creates, modifies, and deletes AutoCAD command aliases. | — |
| 47 | `ALIGN` | 部分实现 | Aligns objects with other objects in 2D and 3D. | 仅两对点（平移 + 旋转 + 可选均匀缩放）；无 3 对点 3D、无非均匀缩放 |
| 48 | `ALIGNSPACE` | 未实现 | Adjusts the pan and zoom factor of a view in a layout viewport based on the alignment points specified in model space and paper space. | — |
| 49 | `AMECONVERT` | 未实现 | Converts AME solid models to AutoCAD solid objects. | — |
| 50 | `ANALYSISCURVATURE` | 未实现 | Displays a color gradient onto a surface to evaluate different aspects of its curvature. | — |
| 51 | `ANALYSISDRAFT` | 未实现 | Displays a color gradient onto a 3D model to evaluate whether there is adequate space between a part and its mold. | — |
| 52 | `ANALYSISOPTIONS` | 未实现 | Sets the display options for zebra, curvature, and draft analysis. | — |
| 53 | `ANALYSISZEBRA` | 未实现 | Projects stripes onto a 3D model to analyze surface continuity. | — |
| 54 | `ANIPATH` | 未实现 | Saves an animation of a camera moving or panning in a 3D model. | — |
| 55 | `ANNORESET` | 未实现 | Resets the locations of all alternate scale representations of the selected annotative objects. | — |
| 56 | `ANNOUPDATE` | 未实现 | Updates existing annotative objects to match the current properties of their styles. | — |
| 57 | `APERTURE` | 未实现 | Controls the size of the object snap target box. | — |
| 58 | `APPAUTOLOADER` | 未实现 | Lists or reloads all plug-ins in the application plug-in folder. | — |
| 59 | `APPLOAD` | 未实现 | Loads and unloads applications and defines which applications to load at startup. | — |
| 60 | `APPSTORE` | 未实现 | Opens the Autodesk App Store website. | — |
| 61 | `ARC` | 已实现 | Creates an arc. | — |
| 62 | `-ARCHIVE` | 未实现 | From the command prompt, packages the current sheet set files for storage. | — |
| 63 | `ARCHIVE` | 未实现 | Packages the current sheet set files for storage. | — |
| 64 | `ARCTEXT` | 未实现 | Places text along an arc. | — |
| 65 | `AREA` | 已实现 | Calculates the area and perimeter of objects or of defined areas. | — |
| 66 | `-ARRAY` | 未实现 | From the command line, creates copies of objects arranged in a pattern. | — |
| 67 | `ARRAY` | 未实现 | Creates copies of objects arranged in a pattern. | — |
| 68 | `ARRAYCLASSIC` | 未实现 | Creates arrays using the legacy dialog box. | — |
| 69 | `ARRAYCLOSE` | 未实现 | Saves or discards changes made to an array's source objects and exits the array editing state. | — |
| 70 | `ARRAYEDIT` | 未实现 | Edits associative array objects and their source objects. | — |
| 71 | `ARRAYPATH` | 未实现 | Evenly distributes object copies along a path or a portion of a path. | — |
| 72 | `ARRAYPOLAR` | 部分实现 | Evenly distributes object copies in a circular pattern around a center point or axis of rotation. | 独立副本、非关联；角度步长为我们的规则 |
| 73 | `ARRAYRECT` | 部分实现 | Distributes object copies into any combination of rows, columns, and levels. | 独立副本、非关联；无 Path/Levels/关联编辑 |
| 74 | `ARX` | 未实现 | Loads, unloads, and provides information about ObjectARX applications. | — |
| 75 | `ASSISTANTCLOSE` | 未实现 | Closes Autodesk Assistant. | — |
| 76 | `ASSISTANTOPEN` | 未实现 | Displays Autodesk Assistant. | — |
| 77 | `-ATTACH` | 未实现 | From the Command prompt, inserts references to external files such as other drawings, raster images, point clouds, coordination models, and underlays. | — |
| 78 | `ATTACH` | 未实现 | Inserts references to external files such as other drawings, raster images, point clouds, coordination models, and underlays. | — |
| 79 | `ATTACHURL` | 未实现 | Attaches hyperlinks to objects or areas in a drawing. | — |
| 80 | `-ATTDEF` | 未实现 | At the Command prompt, creates an attribute definition for storing data in a block. | — |
| 81 | `ATTDEF` | 未实现 | Creates an attribute definition for storing data in a block. | — |
| 82 | `ATTDISP` | 未实现 | Controls the visibility overrides for all block attributes in a drawing. | — |
| 83 | `-ATTEDIT` | 未实现 | At the Command prompt, changes attribute information in a block. | — |
| 84 | `ATTEDIT` | 未实现 | Changes attribute information in a block. | — |
| 85 | `-ATTEXT` | 未实现 | From the Command prompt, extracts attribute data, informational text associated with a block, into a file. | — |
| 86 | `ATTEXT` | 未实现 | Extracts attribute data, informational text associated with a block, into a file. | — |
| 87 | `ATTIN` | 未实现 | Imports block attribute values from an external, tab-delimited ASCII file. | — |
| 88 | `ATTIPEDIT` | 未实现 | Changes the textual content of an attribute within a block. | — |
| 89 | `ATTOUT` | 未实现 | Exports block attribute values to an external file in tab-delimited ASCII format. | — |
| 90 | `ATTREDEF` | 未实现 | Redefines a block and updates associated attributes. | — |
| 91 | `ATTSYNC` | 未实现 | Applies attribute changes in a block definition to all block references. | — |
| 92 | `AUDIT` | 部分实现 | Evaluates the integrity of a drawing and corrects some errors. | v1 只检测：逐实体跑 cad-db 校验并报告诊断；不修复（AutoCAD 的 corrects errors 未实现） |
| 93 | `AUTOCONSTRAIN` | 未实现 | Applies geometric constraints to a selection set of objects based on orientation of the objects relative to one another. | — |
| 94 | `AUTOPUBLISH` | 未实现 | Publishes drawings to DWF, DWFx, or PDF files automatically to a specified location. | — |
| 95 | `BACTION` | 未实现 | Adds an action to a dynamic block definition. | — |
| 96 | `BACTIONBAR` | 未实现 | Displays or hides action bars for a selection set of parameter objects. | — |
| 97 | `BACTIONSET` | 未实现 | Specifies the selection set of objects associated with an action in a dynamic block definition. | — |
| 98 | `BACTIONTOOL` | 未实现 | Adds an action to a dynamic block definition. | — |
| 99 | `BASE` | 未实现 | Sets the insertion base point for the current drawing. | — |
| 100 | `BASSOCIATE` | 未实现 | Associates an action with a parameter in a dynamic block definition. | — |
| 101 | `BATTMAN` | 未实现 | Manages the attributes for a selected block definition. | — |
| 102 | `BATTORDER` | 未实现 | Specifies the order of attributes for a block. | — |
| 103 | `BAUTHORPALETTE` | 未实现 | Opens the Block Authoring Palettes window in the Block Editor. | — |
| 104 | `BAUTHORPALETTECLOSE` | 未实现 | Closes the Block Authoring Palettes window in the Block Editor. | — |
| 105 | `BCLOSE` | 未实现 | Closes the Block Editor. | — |
| 106 | `BCONSTRUCTION` | 未实现 | Converts block geometry into construction geometry, which may be hidden or displayed. | — |
| 107 | `BCOUNT` | 未实现 | Creates a report of the number of instances of each block in a selection set or in the entire drawing. | — |
| 108 | `-BCPARAMETER` | 未实现 | Applies constraint parameters to selected objects, or converts dimensional constraints to parameter constraints. | — |
| 109 | `BCPARAMETER` | 未实现 | Applies constraint parameters to selected objects, or converts dimensional constraints to parameter constraints. | — |
| 110 | `BCYCLEORDER` | 未实现 | Changes the cycling order of grips for a dynamic block reference. | — |
| 111 | `BDETECT` | 未实现 | Displays the Detect palette where you can start object detection and manage the objects that are eligible for conversion. | — |
| 112 | `-BEDIT` | 未实现 | Opens the block definition in the Block Editor from the Command prompt. | — |
| 113 | `BEDIT` | 未实现 | Opens the block definition in the Block Editor. | — |
| 114 | `BESETTINGS` | 未实现 | Displays the Block Editor Settings dialog box. | — |
| 115 | `BEXTEND` | 未实现 | Extends objects to blocks. | — |
| 116 | `BGRIPSET` | 未实现 | Creates, deletes, or resets grips associated with a parameter. | — |
| 117 | `BLEND` | 未实现 | Creates a spline in the gap between two selected lines or curves. | — |
| 118 | `-BLOCK` | 未实现 | At the Command prompt, creates a block definition from selected objects. | — |
| 119 | `BLOCK` | 已实现 | Creates a block definition from selected objects. | — |
| 120 | `BLOCK?` | 未实现 | Lists the objects in a block definition. | — |
| 121 | `BLOCKICON` | 未实现 | Generates preview images for blocks displayed in DesignCenter. | — |
| 122 | `BLOCKREPLACE` | 未实现 | Replaces all instances of a specified block with a different block. | — |
| 123 | `BLOCKSDATAOPTION` | 未实现 | Displays the data collection consent dialog box. | — |
| 124 | `BLOCKSPALETTE` | 未实现 | Displays the Blocks palette, which you can use to insert blocks and drawings into the current drawing. | — |
| 125 | `BLOCKSPALETTECLOSE` | 未实现 | Closes the Blocks palette. | — |
| 126 | `BLOCKTOXREF` | 未实现 | Replaces all instances of a specified block with an xref. | — |
| 127 | `BLOOKUPTABLE` | 未实现 | Displays or creates a lookup table for a dynamic block definition. | — |
| 128 | `BMPOUT` | 未实现 | Saves selected objects to a file in device-independent bitmap format. | — |
| 129 | `-BOUNDARY` | 未实现 | At the Command prompt, creates regions or polylines from enclosed areas. | — |
| 130 | `BOUNDARY` | 未实现 | Creates regions or a polylines from enclosed areas. | — |
| 131 | `BOX` | 未实现 | Creates a 3D solid box. | — |
| 132 | `BPARAMETER` | 未实现 | Adds a parameter with grips to a dynamic block definition. | — |
| 133 | `BREAK` | 部分实现 | Breaks the selected object between two points. | 两点打断；无 F 第一点覆盖、无多段打断；LWPOLYLINE 按名拒绝 |
| 134 | `BREAKATPOINT` | 部分实现 | Breaks the selected object into two objects at a specified point. | 交互工具：先拾取对象再指定断点（断点可投影）；仅 LINE/ARC/开放 LWPOLYLINE；圆/闭合多段线/SPLINE 按名拒绝（闭合对象不拆分） |
| 135 | `BREAKLINE` | 未实现 | Creates a breakline, a polyline that includes the breakline symbol. | — |
| 136 | `BREP` | 未实现 | Removes the history from 3D solids and composite solids, and associativity from surfaces. | — |
| 137 | `-BREPLACE` | 未实现 | At the Command prompt, replaces specified block references with a different block. | — |
| 138 | `BREPLACE` | 未实现 | Replaces specified block references with a different block. | — |
| 139 | `BROWSER` | 未实现 | Launches the default web browser defined in your system's registry. | — |
| 140 | `BSAVE` | 未实现 | Saves the current block definition. | — |
| 141 | `BSAVEAS` | 未实现 | Saves a copy of the current block definition under a new name. | — |
| 142 | `BSCALE` | 未实现 | Scales a block reference relative to its insertion point. | — |
| 143 | `BSEARCH` | 未实现 | Displays the Convert dialog box, which provides options to convert selected entities and identical instances into blocks. | — |
| 144 | `BTABLE` | 未实现 | Stores variations of a block in the Block Properties Table. | — |
| 145 | `BTESTBLOCK` | 未实现 | Displays a window within the Block Editor to test a dynamic block. | — |
| 146 | `BTRIM` | 未实现 | Trim objects to blocks. | — |
| 147 | `BURST` | 未实现 | Explodes selected blocks, preserving the block layer and converting attribute values to text objects. | — |
| 148 | `BVHIDE` | 未实现 | Makes objects invisible in the current visibility state or all visibility states in a dynamic block definition. | — |
| 149 | `BVSHOW` | 未实现 | Makes objects visible in the current visibility state or all visibility states in a dynamic block definition. | — |
| 150 | `-BVSTATE` | 未实现 | At the command prompt, creates, sets, or deletes a visibility state in a dynamic block. | — |
| 151 | `BVSTATE` | 未实现 | Creates, sets, or deletes a visibility state in a dynamic block. | — |
| 152 | `CAL` | 未实现 | Evaluates mathematical and geometric expressions at the Command prompt or within a command. | — |
| 153 | `CAMERA` | 未实现 | Sets a camera and target location to create and save a 3D perspective view of objects. | — |
| 154 | `CDORDER` | 未实现 | Arranges the drawing order of selected objects by their color number. | — |
| 155 | `CENTERDISASSOCIATE` | 未实现 | Removes associativity of center marks or centerlines from objects they define. | — |
| 156 | `CENTERLINE` | 未实现 | Creates centerline geometry associated with selected lines and linear polyline segments. | — |
| 157 | `CENTERMARK` | 未实现 | Creates an associative, cross-shaped mark at the center of a selected circle or arc. | — |
| 158 | `CENTERREASSOCIATE` | 未实现 | Associates or reassociates a center mark or centerline object to selected objects. | — |
| 159 | `CENTERRESET` | 未实现 | Resets the centerlines to the current value specified in the CENTEREXE system variable. | — |
| 160 | `CHAMFER` | 部分实现 | Bevels or chamfers the edges of two 2D objects or the adjacent faces of a 3D solid. | 两线形式 + 独立 CHAMFER POLYLINE 选择子；无 Angle/Multiple/Trim |
| 161 | `CHAMFEREDGE` | 未实现 | Bevels the edges of 3D solids and surfaces. | — |
| 162 | `CHANGE` | 未实现 | Changes the properties of existing objects. | — |
| 163 | `CHECKSTANDARDS` | 未实现 | Checks the current drawing for standards violations. | — |
| 164 | `CHPROP` | 未实现 | Changes the properties of an object. | — |
| 165 | `CHSPACE` | 未实现 | On a layout, transfers selected objects between model space and paper space. | — |
| 166 | `CHURLS` | 未实现 | Provides a method to edit a previously attached URL for selected objects. | — |
| 167 | `CIRCLE` | 已实现 | Creates a circle. | — |
| 168 | `CLASSICGROUP` | 未实现 | Opens the legacy Object Grouping dialog box. | — |
| 169 | `CLASSICIMAGE` | 未实现 | Manages referenced image files in the current drawing. | — |
| 170 | `CLASSICINSERT` | 未实现 | Inserts a block or drawing into the current drawing using the classic version of the INSERT command. | — |
| 171 | `CLASSICLAYER` | 未实现 | Opens the legacy Layer Properties Manager. | — |
| 172 | `CLASSICXREF` | 未实现 | Manages referenced drawing files in the current drawing. | — |
| 173 | `CLEANSCREENOFF` | 部分实现 | Restores the state of the display before CLEANSCREENON was used. | chrome 本地选择子：显式退出 clean-screen；只恢复 chrome 可见性，不重开各面板 |
| 174 | `CLEANSCREENON` | 部分实现 | Clears the screen of toolbars and dockable windows, excluding the command window. | chrome 本地选择子：显式进入 clean-screen；只隐藏 chrome，不记录各面板的独立开关 |
| 175 | `CLIP` | 未实现 | Crops selected objects such as blocks, external references, images, viewports, and underlays to a specified boundary. | — |
| 176 | `CLIPIT` | 未实现 | Clips xrefs or images with lines and curves. | — |
| 177 | `CLOSE` | 未实现 | Closes the current drawing. | — |
| 178 | `CLOSEALL` | 未实现 | Closes all currently open drawings. | — |
| 179 | `CLOSEALLOTHER` | 未实现 | Closes all other open drawings, except the current drawing. | — |
| 180 | `-COLOR` | 未实现 | Sets the color for new objects from the Command prompt. | — |
| 181 | `COLOR` | 未实现 | Sets the color for new objects. | — |
| 182 | `COMMANDLINE` | 未实现 | Displays the Command window. | Ctrl+9 / 状态栏可切换命令行；命令行尚未准入该名字 |
| 183 | `COMMANDLINEHIDE` | 未实现 | Hides the Command window. | — |
| 184 | `COMMANDMACROS` | 未实现 | Opens the Command Macros palette from which you can manage and use command macro recommendations. | — |
| 185 | `COMMANDMACROSCLOSE` | 未实现 | Closes the Command Macros palette. | — |
| 186 | `-COMPARE` | 未实现 | At the Command prompt, compares a specified drawing to the current drawing. | — |
| 187 | `COMPARE` | 未实现 | Compares a specified drawing file with the current drawing file, highlighting the differences with color within revision clouds. | — |
| 188 | `COMPARECLOSE` | 未实现 | Closes the DWG Compare toolbar and ends the comparison. | — |
| 189 | `COMPAREEXPORT` | 未实现 | Exports the results of the comparison into a new drawing file, called a snapshot drawing. | — |
| 190 | `COMPAREIMPORT` | 未实现 | Imports objects from the compared drawing into the current drawing. | — |
| 191 | `COMPAREINFO` | 未实现 | Provides a way to insert or copy the property information about the two compared drawing files. | — |
| 192 | `COMPILE` | 未实现 | Compiles shape files and PostScript font files into SHX files. | — |
| 193 | `CONE` | 未实现 | Creates a 3D solid cone. | — |
| 194 | `CONSTRAINTBAR` | 未实现 | Displays or hides the geometric constraints on an object. | — |
| 195 | `CONSTRAINTSETTINGS` | 未实现 | Controls the display of geometric constraints on constraint bars. | — |
| 196 | `CONVERT` | 未实现 | Converts "legacy" 2D polylines and hatches for use with later product releases. | — |
| 197 | `CONVERTCTB` | 未实现 | Converts a color-dependent plot style table (CTB) to a named plot style table (STB). | — |
| 198 | `CONVERTOLDLIGHTS` | 未实现 | Converts lights created in previous drawing file formats to the current format. | — |
| 199 | `CONVERTOLDMATERIALS` | 未实现 | Converts older materials to use the current materials format. | — |
| 200 | `CONVERTPOLY` | 未实现 | Converts 2D polylines between the "legacy" heavy and optimized lightweight formats. | — |
| 201 | `CONVERTPSTYLES` | 未实现 | Converts the current drawing to either named or color-dependent plot styles. | — |
| 202 | `CONVTOMESH` | 未实现 | Converts 3D objects such as polygon meshes, surfaces, and solids to mesh objects. | — |
| 203 | `CONVTONURBS` | 未实现 | Converts 3D solids and surfaces into NURBS surfaces. | — |
| 204 | `CONVTOSOLID` | 未实现 | Converts eligible 3D objects to 3D solids. | — |
| 205 | `CONVTOSURFACE` | 未实现 | Converts objects to 3D surfaces. | — |
| 206 | `COORDINATIONMODELATTACH` | 未实现 | Inserts references to coordination models such as NWD and NWC Navisworks files. | — |
| 207 | `COPY` | 已实现 | Copies objects a specified distance in a specified direction. | — |
| 208 | `COPYBASE` | 部分实现 | Copies selected objects to the Clipboard along with a specified base point. | 应用内剪贴板 + 基点；不是 OS 剪贴板，不跨应用 |
| 209 | `COPYCLIP` | 部分实现 | Copies selected objects to the Clipboard. | 应用内剪贴板；不是 OS 剪贴板，不跨应用 |
| 210 | `COPYFROMTRACE` | 未实现 | Copies objects from a trace into the drawing. | — |
| 211 | `COPYHIST` | 未实现 | Copies the text in the command line history to the Clipboard. | — |
| 212 | `COPYLINK` | 未实现 | Copies the current view to the Clipboard for linking to other OLE applications. | — |
| 213 | `COPYM` | 未实现 | Copies multiple objects with Repeat, Array, Divide and Measure options. | — |
| 214 | `-COPYTOLAYER` | 未实现 | Copies one or more objects to another layer from the Command prompt. | — |
| 215 | `COPYTOLAYER` | 未实现 | Copies one or more objects to another layer. | — |
| 216 | `COUNT` | 未实现 | Counts and highlights the instances of the selected object in the drawing. | — |
| 217 | `COUNTAREA` | 未实现 | Defines the area to count the instances of an object or block. | — |
| 218 | `COUNTAREACLOSE` | 未实现 | Cancels the count selection area. | — |
| 219 | `COUNTCLOSE` | 未实现 | Closes the Count toolbar and exits the count. | — |
| 220 | `COUNTFIELD` | 未实现 | Creates a field that's set to the value of the current count. | — |
| 221 | `COUNTLIST` | 未实现 | Opens the Count palette to display and manage the counted blocks. | — |
| 222 | `COUNTLISTCLOSE` | 未实现 | Closes the Count palette. | — |
| 223 | `COUNTNAVNEXT` | 未实现 | Zooms to the next object in the count result. | — |
| 224 | `COUNTNAVPREV` | 未实现 | Zooms to the previous object in the count result. | — |
| 225 | `COUNTTABLE` | 未实现 | Inserts a table containing the block names and the corresponding count of each block in the drawing. | — |
| 226 | `CUI` | 未实现 | Manages the customized user interface elements in the product. | — |
| 227 | `CUIEXPORT` | 未实现 | Exports customized settings from the main CUIx file to an enterprise or partial CUIx file. | — |
| 228 | `CUIIMPORT` | 未实现 | Imports customized settings from an enterprise or partial CUIx file to the main CUIx file. | — |
| 229 | `CUILOAD` | 未实现 | Loads a customization file (CUIx). | — |
| 230 | `CUIUNLOAD` | 未实现 | Unloads a CUIx file. | — |
| 231 | `CUSTOMIZE` | 未实现 | Customizes tool palettes and tool palette groups. | — |
| 232 | `CUTBASE` | 部分实现 | Copies selected objects to the Clipboard, along with a specified base point, and removes them from the drawing. | 应用内剪贴板 + 基点；不是 OS 剪贴板，不跨应用 |
| 233 | `CUTCLIP` | 部分实现 | Copies selected objects to the Clipboard and removes them from the drawing. | 应用内剪贴板；不是 OS 剪贴板，不跨应用 |
| 234 | `CVADD` | 未实现 | Adds control vertices to NURBS surfaces and splines. | — |
| 235 | `CVHIDE` | 未实现 | Turns off the display of control vertices for all NURBS surfaces and curves. | — |
| 236 | `CVREBUILD` | 未实现 | Rebuilds the shape of NURBS surfaces and curves. | — |
| 237 | `CVREMOVE` | 未实现 | Removes control vertices from NURBS surfaces and curves. | — |
| 238 | `CVSHOW` | 未实现 | Displays the control vertices for specified NURBS surfaces or curves. | — |
| 239 | `CYLINDER` | 未实现 | Creates a 3D solid cylinder. | — |
| 240 | `-DATAEXTRACTION` | 未实现 | Extracts data as specified in an existing attribute extraction template (BLK) file or data extraction (DXE or DXEX) file. | — |
| 241 | `DATAEXTRACTION` | 未实现 | Extracts drawing data and merges data from an external source to a data extraction table or external file. | — |
| 242 | `DATALINK` | 未实现 | Displays the Data Link dialog box. | — |
| 243 | `DATALINKUPDATE` | 未实现 | Updates data to or from an established external data link. | — |
| 244 | `DBCCLOSE` | 未实现 | Closes the Select Data Object dialog box (dbConnect Manager). | — |
| 245 | `DBCCONFIGURE` | 未实现 | Opens the Configure a Data Source dialog box (dbConnect Manager) | — |
| 246 | `DBCDEFINELLT` | 未实现 | Opens the Select a Database Object dialog box. | — |
| 247 | `DBCDEFINELT` | 未实现 | Opens the Select Data Object Dialog Box (dbConnect Manager) | — |
| 248 | `DBCONNECT` | 未实现 | Provides an interface to external database tables. | — |
| 249 | `DBLIST` | 未实现 | Lists database information for each object in the drawing. | — |
| 250 | `DCALIGNED` | 未实现 | Constrains the distance between two points on different objects. | — |
| 251 | `DCANGULAR` | 未实现 | Constrains the angle between line or polyline segments, the angle swept out by an arc or a polyline arc segment, or the angle between three points on objects. | — |
| 252 | `DCCONVERT` | 未实现 | Converts associative dimensions to dimensional constraints. | — |
| 253 | `DCDIAMETER` | 未实现 | Constrains the diameter of a circle or an arc. | — |
| 254 | `DCDISPLAY` | 未实现 | Displays or hides the dynamic constraints associated with a selection set of objects. | — |
| 255 | `DCFORM` | 未实现 | Specifies whether the dimensional constraint being created is dynamic or annotational. | — |
| 256 | `DCHORIZONTAL` | 未实现 | Constrains the X distance between points on an object, or between two points on different objects. | — |
| 257 | `DCLINEAR` | 未实现 | Creates a horizontal, vertical, or rotated constraint based on the locations of the extension line origins and the dimension line. | — |
| 258 | `DCRADIUS` | 未实现 | Constrains the radius of a circle or an arc. | — |
| 259 | `DCVERTICAL` | 未实现 | Constrains the Y distance between points on an object, or between two points on different objects. | — |
| 260 | `DELAY` | 未实现 | Provides a timed pause within a script. | — |
| 261 | `DELCONSTRAINT` | 未实现 | Removes all geometric and dimensional constraints from a selection set of objects. | — |
| 262 | `DETACHURL` | 未实现 | Removes hyperlinks in a drawing. | — |
| 263 | `DETECTCLOSE` | 未实现 | Ends the detection review. | — |
| 264 | `DETECTCONVERT` | 未实现 | Displays the Convert dialog box, which provides options to convert the detected objects and instances into blocks. | — |
| 265 | `DETECTPRIMARY` | 未实现 | Specifies the detected instance from which a new block definition is created. | — |
| 266 | `DETECTREMOVE` | 未实现 | Removes selected instances from the set. | — |
| 267 | `DETECTREVIEWNEXT` | 未实现 | Displays the next set of detected objects. | — |
| 268 | `DETECTREVIEWPREV` | 未实现 | Displays the previous set of detected objects. | — |
| 269 | `DGNADJUST` | 未实现 | Adjust the fade, contrast, and monochrome settings of a DGN underlay. | — |
| 270 | `-DGNATTACH` | 未实现 | Attach a DGN underlay from the command line. | — |
| 271 | `DGNATTACH` | 未实现 | Inserts a DGN file as an underlay into the current drawing. | — |
| 272 | `DGNCLIP` | 未实现 | Crops the display of a selected DGN underlay to a specified boundary. | — |
| 273 | `-DGNEXPORT` | 未实现 | At the Command prompt, creates one or more DGN files from the current drawing. | — |
| 274 | `DGNEXPORT` | 未实现 | Creates one or more DGN files from the current drawing. | — |
| 275 | `-DGNIMPORT` | 未实现 | At the Command prompt, imports data from DGN files. | — |
| 276 | `DGNIMPORT` | 未实现 | Imports the data from a DGN file into a new DWG file or the current DWG file, depending on the DGNIMPORTMODE system variable. | — |
| 277 | `DGNLAYERS` | 未实现 | Controls the display of layers in a DGN underlay. | — |
| 278 | `DGNMAPPING` | 未实现 | Allows users to create and edit user-defined DGN mapping setups. | — |
| 279 | `DIGITALSIGN` | 未实现 | Attaches a digital signature to a drawing, which is removed if an unauthorized change is made. | — |
| 280 | `DIM` | 未实现 | Creates multiple dimensions and types of dimensions with a single command. | — |
| 281 | `DIMALIGNED` | 已实现 | Creates an aligned linear dimension. | — |
| 282 | `DIMANGULAR` | 部分实现 | Creates an angular dimension. | 仅两线形式可交互；三点形式只到载荷层 |
| 283 | `DIMARC` | 未实现 | Creates an arc length dimension. | — |
| 284 | `DIMBASELINE` | 已实现 | Creates a linear, angular, or ordinate dimension from the baseline of the previous or selected dimension. | — |
| 285 | `DIMBREAK` | 未实现 | Breaks or restores dimension and extension lines where they cross other objects. | — |
| 286 | `DIMCENTER` | 已实现 | Creates the non-associative center mark or the centerlines of circles and arcs. | — |
| 287 | `DIMCONSTRAINT` | 未实现 | Applies dimensional constraints to selected objects or points on objects, or converts associative dimensions to dimensional constraints. | — |
| 288 | `DIMCONTINUE` | 已实现 | Creates a dimension that starts from an extension line of the previous or selected dimension. | — |
| 289 | `DIMDIAMETER` | 已实现 | Creates a diameter dimension for a circle or an arc. | — |
| 290 | `DIMDISASSOCIATE` | 未实现 | Removes associativity from selected dimensions. | — |
| 291 | `DIMEDIT` | 未实现 | Edits dimension text and extension lines. | — |
| 292 | `DIMEX` | 未实现 | Exports named dimension styles and their settings to an external file. | — |
| 293 | `DIMIM` | 未实现 | Imports named dimension styles and their settings from an external file. | — |
| 294 | `-DIMINSPECT` | 未实现 | From the Command prompt, adds or removes inspection information from a selected dimension. | — |
| 295 | `DIMINSPECT` | 未实现 | Adds or removes inspection information for a selected dimension. | — |
| 296 | `DIMJOGGED` | 未实现 | Creates jogged dimensions for circles and arcs. | — |
| 297 | `DIMJOGLINE` | 未实现 | Adds or removes a jog line on a linear or aligned dimension. | — |
| 298 | `DIMLINEAR` | 已实现 | Creates a linear dimension. | — |
| 299 | `DIMORDINATE` | 已实现 | Creates ordinate dimensions. | — |
| 300 | `DIMOVERRIDE` | 未实现 | Controls overrides of system variables used in selected dimensions. | — |
| 301 | `DIMRADIUS` | 已实现 | Creates a radius dimension for a circle or an arc. | — |
| 302 | `DIMREASSOC` | 未实现 | Restores the measured value to overridden or modified dimension text. | — |
| 303 | `DIMREASSOCIATE` | 未实现 | Associates or reassociates selected dimensions to objects or points on objects. | — |
| 304 | `DIMREGEN` | 未实现 | Updates the locations of all associative dimensions. | — |
| 305 | `DIMROTATED` | 未实现 | Creates a rotated linear dimension. | — |
| 306 | `DIMSPACE` | 未实现 | Adjusts the spacing between linear dimensions or angular dimensions. | — |
| 307 | `-DIMSTYLE` | 未实现 | At the Command prompt, creates and modifies dimension styles. | — |
| 308 | `DIMSTYLE` | 未实现 | Creates and modifies dimension styles. | — |
| 309 | `DIMTEDIT` | 未实现 | Moves and rotates dimension text and relocates the dimension line. | — |
| 310 | `DIST` | 已实现 | Measures the distance and angle between two points. | — |
| 311 | `DISTANTLIGHT` | 未实现 | Creates a distant light. | — |
| 312 | `DIVIDE` | 部分实现 | Creates evenly spaced point objects or blocks along the length or perimeter of an object. | 仅 LINE/ARC/CIRCLE/直线段 LWPOLYLINE；按等弧长放 POINT |
| 313 | `DONUT` | 未实现 | Creates a filled circle or a wide ring. | — |
| 314 | `DOWNLOADMANAGER` | 未实现 | Reports the status of the of the current download. | — |
| 315 | `DRAGMODE` | 未实现 | Controls the way dragged objects are displayed. | — |
| 316 | `DRAWINGRECOVERY` | 未实现 | Displays a list of drawing files that can be recovered after a program or system failure. | — |
| 317 | `DRAWINGRECOVERYHIDE` | 未实现 | Closes the Drawing Recovery Manager. | — |
| 318 | `DRAWORDER` | 未实现 | Changes the draw order of images and other objects. | — |
| 319 | `DSETTINGS` | 未实现 | Sets grid and snap, polar and object snap tracking, object snap modes, Dynamic Input, and Quick Properties. | — |
| 320 | `DVIEW` | 未实现 | Defines parallel projection or perspective views by using a camera and target. | — |
| 321 | `DWFADJUST` | 未实现 | Adjust the fade, contrast, and monochrome settings of a DWF or DWFx underlay. | — |
| 322 | `-DWFATTACH` | 未实现 | From the Command prompt, attach a DWF underlay. | — |
| 323 | `DWFATTACH` | 未实现 | Inserts a DWF or DWFx file as an underlay into the current drawing. | — |
| 324 | `DWFCLIP` | 未实现 | Crops the display of a selected DWF or DWFx underlay to a specified boundary. | — |
| 325 | `DWFFORMAT` | 未实现 | Sets the default format to DWF or DWFx for output in specific commands. | — |
| 326 | `DWFLAYERS` | 未实现 | Controls the display of layers in a DWF or DWFx underlay. | — |
| 327 | `DWGCONVERT` | 未实现 | Converts drawing format version for selected drawing files. | — |
| 328 | `DWGLOG` | 未实现 | Creates and maintains an individual log file for each drawing file as it is accessed. | — |
| 329 | `DWGPROPS` | 未实现 | Sets and displays the file properties of the current drawing. | — |
| 330 | `DXBIN` | 未实现 | Imports an AutoCAD DXB (drawing interchange binary) file. | — |
| 331 | `EATTEDIT` | 未实现 | Edits attributes in a block reference. | — |
| 332 | `-EATTEXT` | 未实现 | From the Command line, extracts data as specified in an existing attribute extraction template (BLK) file or data extraction (DXE or DXEX) file. | — |
| 333 | `EATTEXT` | 未实现 | Exports block attribute information to a table or to an external file. | — |
| 334 | `EDGE` | 未实现 | Changes the visibility of 3D face edges. | — |
| 335 | `EDGESURF` | 未实现 | Creates a mesh between four contiguous edges or curves. | — |
| 336 | `EDITSHOT` | 未实现 | Edits a saved named view with or without motion. | — |
| 337 | `EDITTIME` | 未实现 | Tracks the amount of active editing time for a drawing. | — |
| 338 | `ELEV` | 未实现 | Sets elevation and extrusion thickness of new objects. | — |
| 339 | `ELLIPSE` | 已实现 | Creates an ellipse or an elliptical arc. | — |
| 340 | `ERASE` | 已实现 | Removes objects from a drawing. | — |
| 341 | `-ETRANSMIT` | 未实现 | From the Command line, packages a set of files for internet transmission. | — |
| 342 | `ETRANSMIT` | 未实现 | Packages a set of files for Internet transmission. | — |
| 343 | `EXOFFSET` | 未实现 | Offsets selected objects. | — |
| 344 | `EXPLAN` | 未实现 | Displays an orthographic view of the XY plane of a specified UCS without changing the view magnification. | — |
| 345 | `EXPLODE` | 部分实现 | Breaks a compound object into its component objects. | 一次一层；拒绝 Dimension/MULTILEADER/非均匀 INSERT |
| 346 | `-EXPORT` | 未实现 | Creates a DWF, DWFx, or PDF file from the command line. | — |
| 347 | `EXPORT` | 部分实现 | Saves the objects in a drawing to a different file format. | PLOT 别名：仅矢量 SVG/PDF |
| 348 | `EXPORTDWF` | 未实现 | Creates a DWF file and allows you to set individual page setup overrides on a sheet-by-sheet basis. | — |
| 349 | `EXPORTDWFX` | 未实现 | Creates a DWFx file where you can set individual page setup overrides on a sheet-by-sheet basis. | — |
| 350 | `EXPORTLAYOUT` | 未实现 | Creates a visual representation of the current layout in the model space of a new drawing. | — |
| 351 | `EXPORTPDF` | 未实现 | Generates a PDF file from a single layout, all layouts, or a specified area in model space. | — |
| 352 | `EXPORTSETTINGS` | 未实现 | Adjusts the page setup and drawing selection when exporting to a DWF, DWFx, or PDF file. | — |
| 353 | `EXPORTTOAUTOCAD` | 未实现 | Creates a version of a drawing file that can be opened in products such as AutoCAD and previous releases of a toolset. | — |
| 354 | `EXPRESSMENU` | 未实现 | Loads the AutoCAD Express Tools menu and displays the Express menu on the menu bar. | — |
| 355 | `EXPRESSTOOLS` | 未实现 | Loads the AutoCAD Express Tools libraries, places the Express folder in the search path, and loads and places the Express menu on the menu bar. | — |
| 356 | `EXTEND` | 部分实现 | Extends objects to meet the edges of other objects. | 目标仅 LINE/ARC；SPLINE/CIRCLE 目标拒绝 |
| 357 | `EXTERNALREFERENCES` | 未实现 | Opens the External References palette. | — |
| 358 | `EXTERNALREFERENCESCLOSE` | 未实现 | Closes the External References palette. | — |
| 359 | `EXTRIM` | 未实现 | Trims all objects at the cutting edge specified by a selected polyline, line, circle, arc, ellipse, text, mtext, or attribute definition. | — |
| 360 | `EXTRUDE` | 未实现 | Creates a 3D solid from an object that encloses an area, or a 3D surface from an object with open ends. | — |
| 361 | `FADEMARKUP` | 未实现 | Fade individual markups so they are less visible on a trace. | — |
| 362 | `FIELD` | 未实现 | Creates a multiline text object with a field that can be updated automatically as the field value changes. | — |
| 363 | `FILETAB` | 未实现 | Displays the file tabs at the top of the drawing area. | — |
| 364 | `FILETABCLOSE` | 未实现 | Hides the file tabs at the top of the drawing area. | — |
| 365 | `FILL` | 未实现 | Controls the display of filled objects such as hatches, 2D solids, and wide polylines. | — |
| 366 | `FILLET` | 部分实现 | Rounds or fillets the edges of two 2D objects or the adjacent faces of a 3D solid. | 两线形式 + 独立 FILLET POLYLINE 选择子；无 Multiple/Trim |
| 367 | `FILLETEDGE` | 未实现 | Rounds and fillets the edges of solid objects. | — |
| 368 | `FILTER` | 未实现 | Creates a list of requirements that an object must meet to be included in a selection set. | — |
| 369 | `-FIND` | 未实现 | Searches for the text that you specify in the entire drawing, the current space or layout, or selected objects. | — |
| 370 | `FIND` | 未实现 | Finds the text that you specify, and can optionally replace it with other text. | — |
| 371 | `FINDINCLUDE` | 未实现 | Controls the types of text objects included in a search when using the -FIND command. | — |
| 372 | `FINDMATCH` | 未实现 | Controls how the entered text string is compared with text in objects when using the -FIND command. | — |
| 373 | `FLATSHOT` | 未实现 | Creates a 2D representation of all 3D objects based on the current view. | — |
| 374 | `FLATTEN` | 未实现 | Converts 3D geometry to a projected 2D representation. | — |
| 375 | `FREESPOT` | 未实现 | Creates a free spotlight which is a spotlight without a specified target. | — |
| 376 | `FREEWEB` | 未实现 | Creates a free web light which is a web light without a specified target. | — |
| 377 | `FS` | 部分实现 | Creates a selection set of all objects that touch the selected object. | Express 工具、仅命令行；当前唯一选择为种子，并入**直接**接触（相交或相切）的可见模型空间对象（默认 FSMODE=OFF；链式 FSMODE=ON 未实现）；不展开块实例成员 |
| 378 | `GATTE` | 未实现 | Globally changes attribute values for all instances of a specified block. | — |
| 379 | `GCCOINCIDENT` | 未实现 | Constrains two points together or a point to a curve (or an extension of a curve). | — |
| 380 | `GCCOLLINEAR` | 未实现 | Causes two or more line segments to lie along the same line. | — |
| 381 | `GCCONCENTRIC` | 未实现 | Constrains two arcs, circles, or ellipses to the same center point. | — |
| 382 | `GCEQUAL` | 未实现 | Resizes selected arcs and circles to the same radius, or selected lines to the same length. | — |
| 383 | `GCFIX` | 未实现 | Locks points and curves in position. | — |
| 384 | `GCHORIZONTAL` | 未实现 | Causes lines or pairs of points to lie parallel to the X axis of the current coordinate system. | — |
| 385 | `GCPARALLEL` | 未实现 | Causes selected lines to lie parallel to each other. | — |
| 386 | `GCPERPENDICULAR` | 未实现 | Causes selected lines to lie 90 degrees to one another. | — |
| 387 | `GCSMOOTH` | 未实现 | Constrains a spline to be contiguous and maintain G2 continuity with another spline, line, arc, or polyline. | — |
| 388 | `GCSYMMETRIC` | 未实现 | Causes selected objects to become symmetrically constrained about a selected line. | — |
| 389 | `GCTANGENT` | 未实现 | Constrains two curves to maintain a point of tangency to each other or their extensions. | — |
| 390 | `GCVERTICAL` | 未实现 | Causes lines or pairs of points to lie parallel to the Y axis of the current coordinate system. | — |
| 391 | `GEOGRAPHICLOCATION` | 未实现 | Assigns geographic location information to a drawing file. | — |
| 392 | `GEOMAP` | 未实现 | Displays a map from an online maps service, in the current viewport. | — |
| 393 | `GEOMAPIMAGE` | 未实现 | Captures a portion of the online map to an object known as a map image, and embeds it in the drawing area. | — |
| 394 | `GEOMAPIMAGEUPDATE` | 未实现 | Updates map images from the online maps service and optionally resets their resolution for optimal on-screen viewing. | — |
| 395 | `GEOMARKLATLONG` | 未实现 | Places a position marker at a location defined by latitude and longitude. | — |
| 396 | `GEOMARKME` | 未实现 | Places a position marker in the drawing area at the coordinates corresponding to your current position. | — |
| 397 | `GEOMARKPOINT` | 未实现 | Places a position marker at a specified point in model space. | — |
| 398 | `GEOMARKPOSITION` | 未实现 | Places a position marker at a location you specify. | — |
| 399 | `GEOMCONSTRAINT` | 未实现 | Applies or persists geometric relationships between objects or points on objects. | — |
| 400 | `GEOREMOVE` | 未实现 | Removes all geographic location information from the drawing file. | — |
| 401 | `GEOREORIENTMARKER` | 未实现 | Changes the north direction and position of the geographic marker in model space, without changing its latitude and longitude. | — |
| 402 | `GETSEL` | 未实现 | Creates a selection set of objects based on layer and object type filters. | — |
| 403 | `GOTOSTART` | 未实现 | Switches from the current drawing to the Start tab. | — |
| 404 | `GOTOURL` | 未实现 | Opens the file or web page associated with the hyperlink attached to an object. | — |
| 405 | `GRADIENT` | 未实现 | Fills an enclosed area or selected objects with a gradient fill. | — |
| 406 | `-GRAPHICSCONFIG` | 未实现 | Sets hardware acceleration on or off and provides access to display performance options on the command line. | — |
| 407 | `GRAPHICSCONFIG` | 未实现 | Sets hardware acceleration on or off and provides access to display performance options. | — |
| 408 | `GRAPHSCR` | 未实现 | Causes the text window to display behind the application window. | — |
| 409 | `GRID` | 已实现 | Displays a grid pattern in the current viewport. | — |
| 410 | `-GROUP` | 未实现 | Creates and manages saved sets of objects called groups. | — |
| 411 | `GROUP` | 未实现 | Creates and manages saved sets of objects called groups. | — |
| 412 | `GROUPEDIT` | 未实现 | Adds and removes objects from the selected group, or renames a selected group. | — |
| 413 | `-HATCH` | 未实现 | Uses command prompts to fill an enclosed area or selected objects with a hatch pattern, solid fill, or gradient fill. | — |
| 414 | `HATCH` | 部分实现 | Fills an enclosed area or selected objects with a hatch pattern, solid fill, or gradient fill. | 整实体闭合边界 + LINE/ARC 边链；无 .pat/关联性/实时预览 |
| 415 | `-HATCHEDIT` | 未实现 | From the Command prompt, modifies existing hatches or fills. | — |
| 416 | `HATCHEDIT` | 未实现 | Modifies an existing hatch or fill. | — |
| 417 | `HATCHGENERATEBOUNDARY` | 部分实现 | Creates a non-associated polyline around a selected hatch. | 预选择输入（不弹 Select objects 选项）；每个 hatch loop 生成一条同层同空间的非关联闭合多段线（含内岛、保留 bulge）；源 hatch 保留；无关联性 |
| 418 | `HATCHSETBOUNDARY` | 未实现 | Redefines a selected hatch or fill to conform to a different closed boundary. | — |
| 419 | `HATCHSETORIGIN` | 未实现 | Controls the starting location of hatch pattern generation for a selected hatch. | — |
| 420 | `HATCHTOBACK` | 部分实现 | Sets the draw order for all hatches in the drawing to be behind all other objects. | 仅模型空间：全部 hatch 连续放到最低非 hatch draw order 之下并保持相对次序；i64 下溢按名拒绝；图纸空间/块内不重排；一次事务可撤销 |
| 421 | `HELIX` | 未实现 | Creates a 2D spiral or 3D spring. | — |
| 422 | `HELP` | 未实现 | Displays the online or offline Help system. | — |
| 423 | `HIDE` | 未实现 | Displays a 3D model with hidden lines suppressed for the 2D Wireframe visual style. | — |
| 424 | `HIDEOBJECTS` | 未实现 | Temporarily suppresses the display of selected objects. | — |
| 425 | `HIDEPALETTES` | 未实现 | Hides all displayed palettes, along with the ribbon and the drawing tabs. | — |
| 426 | `HIGHLIGHTNEW` | 未实现 | Controls whether new and changed features in product updates are highlighted in the user interface with an orange dot. | — |
| 427 | `HLSETTINGS` | 未实现 | Sets the display of such properties as hidden lines. | — |
| 428 | `-HYPERLINK` | 未实现 | From the Command prompt, inserts and removes hyperlinks from areas or to selected objects. | — |
| 429 | `HYPERLINK` | 未实现 | Attaches a hyperlink to an object or modifies an existing hyperlink. | — |
| 430 | `HYPERLINKOPTIONS` | 未实现 | Controls the display of the hyperlink cursor, tooltips, and shortcut menu. | — |
| 431 | `ID` | 已实现 | Displays the UCS coordinate values of a specified location. | — |
| 432 | `IGESEXPORT` | 未实现 | Saves selected objects in the current drawing to a new IGES (*.igs, *.iges) file. | — |
| 433 | `IGESIMPORT` | 未实现 | Imports data from an IGES (*.igs or *.iges) file into the current drawing. | — |
| 434 | `-IMAGE` | 未实现 | Attaches a raster image to the drawing. | — |
| 435 | `IMAGE` | 未实现 | Displays the External References palette. | — |
| 436 | `-IMAGEADJUST` | 未实现 | Controls the brightness, contrast, and fade values of images. | — |
| 437 | `IMAGEADJUST` | 未实现 | Controls the brightness, contrast, and fade values of images. | — |
| 438 | `IMAGEAPP` | 未实现 | Specifies the image editing program for IMAGEEDIT. | — |
| 439 | `IMAGEASYNCWAIT` | 未实现 | Ensures that raster images being loaded in the background during file opening are complete before proceeding. | — |
| 440 | `IMAGEATTACH` | 未实现 | Inserts a reference to an image file. | — |
| 441 | `IMAGECLIP` | 未实现 | Crops the display of a selected image to a specified boundary. | — |
| 442 | `IMAGEEDIT` | 未实现 | Launches the image editing program specified by IMAGEAPP for the selected image. | — |
| 443 | `IMAGEQUALITY` | 未实现 | Controls the display quality of images. | — |
| 444 | `IMPORT` | 未实现 | Imports files of different formats into the current drawing. | — |
| 445 | `IMPRINT` | 未实现 | Imprints 2D geometry on a 3D solid or surface, creating additional edges on planar faces. | — |
| 446 | `-INPUTSEARCHOPTIONS` | 未实现 | From the command line, controls settings for display of the command line suggestion list for commands, system variables, and named objects. | — |
| 447 | `INPUTSEARCHOPTIONS` | 未实现 | Opens a dialog box that controls settings for display of the command line suggestion list for commands, system variables, and named objects. | — |
| 448 | `-INSERT` | 未实现 | Inserts a block or drawing into the current drawing. | — |
| 449 | `INSERT` | 已实现 | Displays the Blocks palette, which you can use to insert blocks and drawings into the current drawing. | — |
| 450 | `-INSERTCONTENT` | 未实现 | Inserts a drawing or block into the current drawing. | — |
| 451 | `INSERTOBJ` | 未实现 | Inserts a linked or embedded object. | — |
| 452 | `-INTERFERE` | 未实现 | At the Command prompt, creates a temporary 3D solid from the interferences between two sets of selected 3D solids | — |
| 453 | `INTERFERE` | 未实现 | Creates a temporary 3D solid from the interferences between two sets of selected 3D solids. | — |
| 454 | `INTERSECT` | 未实现 | Creates a 3D solid, surface, or 2D region from overlapping solids, surfaces, or regions. | — |
| 455 | `ISODRAFT` | 未实现 | Turns isometric drafting settings on or off, and specifies the current 2D isometric drafting plane. | — |
| 456 | `ISOLATEOBJECTS` | 未实现 | Temporarily suppresses the display of all objects except those you select. | — |
| 457 | `ISOPLANE` | 未实现 | Specifies the current plane for 2D isometric drawing. | — |
| 458 | `JOIN` | 部分实现 | Joins the endpoints of linear and curved objects to create a single object. | 仅共线 LINE 或同心相续 ARC |
| 459 | `JPGOUT` | 未实现 | Saves selected objects to a file in JPEG file format. | — |
| 460 | `JULIAN` | 未实现 | Contains the DATE tool and several AutoCAD Julian date and calendar date conversion routines. | — |
| 461 | `JUSTIFYTEXT` | 未实现 | Changes the justification point of selected text objects without changing their locations. | — |
| 462 | `LAYCUR` | 部分实现 | Changes the layer property of selected objects to the current layer. | 预选择输入（不弹 Select objects）；把选中对象移到当前层；当前层缺失/目标层锁定显式拒绝；实体写入可撤销 |
| 463 | `-LAYDEL` | 未实现 | At the Command prompt, deletes all objects on a layer and purges the layer. | — |
| 464 | `LAYDEL` | 未实现 | Deletes all objects on a layer and purges the layer. | — |
| 465 | `-LAYER` | 未实现 | Manages layers and layer properties. | — |
| 466 | `LAYER` | 已实现 | Manages layers and layer properties. | chrome 本地选择子：打开图层面板 |
| 467 | `LAYERCLOSE` | 已实现 | Closes the Layer Properties Manager. | chrome 本地选择子：关闭图层面板（面板关闭按钮走同一命令） |
| 468 | `LAYERP` | 部分实现 | Undoes the last change or set of changes made to layer settings. | 会话内有界 16 步；不持久化 |
| 469 | `LAYERPALETTE` | 未实现 | Opens the modeless Layer Properties Manager. | — |
| 470 | `LAYERPMODE` | 未实现 | Turns on and off the tracking of changes made to layer settings for use by the LAYERP command. | — |
| 471 | `LAYERSTATE` | 未实现 | Saves, restores, and manages sets of layer settings that are called layer states. | — |
| 472 | `LAYERSTATESAVE` | 未实现 | Displays the New Layer State to Save dialog box, where you can provide a name and a description for a new layer state. | — |
| 473 | `LAYFRZ` | 部分实现 | Freezes the layer of selected objects. | 预选择输入（不弹 Select objects）；仅 Entity 选择，无 Settings/Viewports/Block 选项；v1 在全部视口冻结；当前层整命令拒绝；图层表写入不可撤销 |
| 474 | `LAYISO` | 已实现 | Hides or locks all layers except those of the selected objects. | — |
| 475 | `LAYLCK` | 部分实现 | Locks the layer of a selected object. | 预选择输入（不弹 Select objects）；锁定选中对象所在的不同图层（含当前层）；图层表写入不可撤销 |
| 476 | `-LAYMCH` | 未实现 | Changes the layer of a selected object to match the layer of a selected object. | — |
| 477 | `LAYMCH` | 未实现 | Changes the layer of a selected object to match the destination layer. | — |
| 478 | `LAYMCUR` | 部分实现 | Sets the current layer to that of a selected object. | 预选择输入（不弹 Select objects）；取第一个选中对象的图层并用既有的锁定/冻结/关闭当前层拒绝；仅会话状态 |
| 479 | `-LAYMRG` | 未实现 | At the Command prompt, merges selected layers into a target layer. | — |
| 480 | `LAYMRG` | 未实现 | Merges selected layers into a target layer, removing them from the drawing. | — |
| 481 | `LAYOFF` | 已实现 | Turns off the layer of a selected object. | — |
| 482 | `LAYON` | 已实现 | Turns on all layers in the drawing. | — |
| 483 | `LAYOUT` | 未实现 | Creates and modifies drawing layouts. | — |
| 484 | `LAYOUTMERGE` | 未实现 | Combines specified layouts into a single layout. | — |
| 485 | `LAYOUTWIZARD` | 未实现 | Creates a new layout tab and specifies page and plot settings. | — |
| 486 | `LAYTHW` | 部分实现 | Thaws all layers in the drawing. | 无选择输入（载荷即拒绝）；解冻全部冻结图层；不处理视口级冻结（本构建无视口模型）；图层表写入不可撤销 |
| 487 | `LAYTRANS` | 未实现 | Translates the layers in the current drawing to specified layer standards. | — |
| 488 | `LAYULK` | 部分实现 | Unlocks the layer of a selected object. | 预选择输入（不弹 Select objects）；解锁选中对象所在的不同图层；图层表写入不可撤销 |
| 489 | `LAYUNISO` | 已实现 | Restores all layers that were hidden or locked with the LAYISO command. | — |
| 490 | `LAYVPI` | 未实现 | Freezes selected layers in all layout viewports except the current viewport. | — |
| 491 | `LAYWALK` | 未实现 | Displays objects on selected layers and hides objects on all other layers. | — |
| 492 | `LEADER` | 未实现 | Creates a line that connects annotation to a feature. | — |
| 493 | `LENGTHEN` | 部分实现 | Changes the length of objects and the included angle of arcs. | 仅 LINE/ARC，只改末端 |
| 494 | `LIGHT` | 未实现 | Creates a light. | — |
| 495 | `LIGHTLIST` | 未实现 | Displays the Lights in Model palette that lists all lights in the model. | — |
| 496 | `LIGHTLISTCLOSE` | 未实现 | Closes the Lights in Model palette. | — |
| 497 | `LIMITS` | 未实现 | Sets an invisible rectangular boundary in the drawing area that can limit the grid display and limit clicking or entering point locations. | — |
| 498 | `LINE` | 已实现 | Create a series of contiguous line segments. Each segment is a line object that can be edited separately. | — |
| 499 | `-LINETYPE` | 未实现 | At the Command prompt, loads, sets, and modifies linetypes. | — |
| 500 | `LINETYPE` | 未实现 | Loads, sets, and modifies linetypes. | — |
| 501 | `LIST` | 部分实现 | Displays property data for selected objects. | 选择驱动（非交互式选取）；逐对象报告空间、图层、标准显示属性与几何摘要；厚度/拉伸方向不建模，显式说明 |
| 502 | `LIVESECTION` | 未实现 | Turns on live sectioning for a selected section object. | — |
| 503 | `LOAD` | 未实现 | Makes the symbols in compiled shape (SHX) files available for use by the SHAPE command. | — |
| 504 | `LOFT` | 未实现 | Creates a 3D solid or surface in the space between several cross sections. | — |
| 505 | `LOGFILEOFF` | 未实现 | Closes the command history log file opened by LOGFILEON. | — |
| 506 | `LOGFILEON` | 未实现 | Writes the contents of the command history to a file. | — |
| 507 | `LSP` | 未实现 | Displays a list of all available AutoLISP commands, functions, and variables. | — |
| 508 | `LSPSURF` | 未实现 | Displays the contents of an AutoLISP file by individual functions. | — |
| 509 | `LTSCALE` | 未实现 | Sets the global linetype scale factor. | — |
| 510 | `-LWEIGHT` | 未实现 | Sets the current lineweight, lineweight display options, and lineweight units. | — |
| 511 | `LWEIGHT` | 未实现 | Sets the current lineweight, lineweight display options, and lineweight units. | — |
| 512 | `MAKELISPAPP` | 未实现 | Compiles one or more AutoLISP (LSP) source files into an application (VLX) file that can be distributed to users and protect your code. | — |
| 513 | `MARKUP` | 未实现 | Opens the Markup Set Manager. | — |
| 514 | `-MARKUPASSIST` | 未实现 | At the Command prompt, analyzes an imported markup and can help place text callouts and revision clouds faster and with less manual effort. | — |
| 515 | `MARKUPASSIST` | 未实现 | Analyzes an imported markup and can help place text callouts and revision clouds faster and with less manual effort. | — |
| 516 | `MARKUPCLOSE` | 未实现 | Closes the Markup Set Manager. | — |
| 517 | `-MARKUPIMPORT` | 未实现 | At the command prompt, imports a marked up drawing (image/pdf) in-place into your DWG as a new trace. | — |
| 518 | `MARKUPIMPORT` | 未实现 | Imports a marked up drawing (image/pdf) in-place into your DWG as a new trace. | — |
| 519 | `MASSPROP` | 未实现 | Calculates the mass properties of selected 2D regions or 3D solids. | — |
| 520 | `MATBROWSERCLOSE` | 未实现 | Closes the Materials Browser. | — |
| 521 | `MATBROWSEROPEN` | 未实现 | Opens the Materials Browser. | — |
| 522 | `MATCHCELL` | 未实现 | Applies the properties of a selected table cell to other table cells. | — |
| 523 | `MATCHPROP` | 部分实现 | Applies the properties of a selected object to other objects. | 只转移 layer/color/linetype/lineweight/transparency |
| 524 | `MATEDITORCLOSE` | 未实现 | Closes the Materials Editor. | — |
| 525 | `MATEDITOROPEN` | 未实现 | Opens the Materials Editor. | — |
| 526 | `MATERIALASSIGN` | 未实现 | Assigns the material defined in the CMATERIAL system variable to the objects you select. | — |
| 527 | `MATERIALATTACH` | 未实现 | Associates materials with layers. | — |
| 528 | `MATERIALMAP` | 未实现 | Adjusts how a texture is mapped to a face or an object. | — |
| 529 | `MATERIALS` | 未实现 | Opens the Materials Browser. | — |
| 530 | `MATERIALSCLOSE` | 未实现 | Closes the Materials Browser. | — |
| 531 | `MEASURE` | 未实现 | Creates point objects or blocks at measured intervals along the length or perimeter of an object. | 本项目 MEASURE 是测量入口（默认 DIST），不是沿对象放置点/块 |
| 532 | `MEASUREGEOM` | 未实现 | Measures the distance, radius, angle, area, and volume of selected objects, a sequence of points, or dynamically. | — |
| 533 | `MENU` | 未实现 | Obsolete. Loads a customization file. | — |
| 534 | `MESH` | 未实现 | Creates a 3D mesh primitive object such as a box, cone, cylinder, pyramid, sphere, wedge, or torus. | — |
| 535 | `MESHCAP` | 未实现 | Creates a mesh face that connects open edges. | — |
| 536 | `MESHCOLLAPSE` | 未实现 | Merges the vertices of selected mesh faces or edges. | — |
| 537 | `MESHCREASE` | 未实现 | Sharpens the edges of selected mesh subobjects. | — |
| 538 | `MESHEXTRUDE` | 未实现 | Extends a mesh face into 3D space. | — |
| 539 | `MESHMERGE` | 未实现 | Merges adjacent faces into a single face. | — |
| 540 | `MESHOPTIONS` | 未实现 | Displays the Mesh Tessellation Options dialog box, which controls default settings for converting existing objects to mesh objects. | — |
| 541 | `MESHPRIMITIVEOPTIONS` | 未实现 | Displays the Mesh Primitive Options dialog box, which sets the tessellation defaults for primitive mesh objects. | — |
| 542 | `MESHREFINE` | 未实现 | Multiplies the number of faces in selected mesh objects or faces. | — |
| 543 | `MESHSMOOTH` | 未实现 | Converts 3D objects such as polygon meshes, surfaces, and solids to mesh objects. | — |
| 544 | `MESHSMOOTHLESS` | 未实现 | Decreases the level of smoothness for mesh objects by one level. | — |
| 545 | `MESHSMOOTHMORE` | 未实现 | Increases the level of smoothness for mesh objects by one level. | — |
| 546 | `MESHSPIN` | 未实现 | Spins the adjoining edge of two triangular mesh faces. | — |
| 547 | `MESHSPLIT` | 未实现 | Splits a mesh face into two faces. | — |
| 548 | `MESHUNCREASE` | 未实现 | Removes the crease from selected mesh faces, edges, or vertices. | — |
| 549 | `MIGRATEMATERIALS` | 未实现 | Finds any legacy materials in Tool Palettes and converts them to generic type. | — |
| 550 | `MINSERT` | 未实现 | Inserts multiple instances of a block in a rectangular array. | — |
| 551 | `MIRROR` | 部分实现 | Creates a mirrored copy of selected objects. | 保留原件；拒绝 Text/MultiLeader |
| 552 | `MIRROR3D` | 未实现 | Creates a mirrored copy of selected 3D objects across a mirroring plane. | — |
| 553 | `MKLTYPE` | 未实现 | Creates a linetype definition based on selected objects and stores them in a specified linetype definition (LIN) file. | — |
| 554 | `MKSHAPE` | 未实现 | Creates a shape definition based on selected objects. | — |
| 555 | `MLEADER` | 部分实现 | Creates a multileader object. | 单簇/2 顶点/仅直线；无 Options |
| 556 | `MLEADERALIGN` | 未实现 | Aligns and spaces selected multileader objects. | — |
| 557 | `MLEADERCOLLECT` | 未实现 | Organizes selected multileaders that contain blocks into rows or columns, and displays the result with a single leader. | — |
| 558 | `MLEADEREDIT` | 未实现 | Adds leader lines to, or removes leader lines from, a multileader object. | — |
| 559 | `MLEADERSTYLE` | 未实现 | Creates and modifies multileader styles. | — |
| 560 | `-MLEDIT` | 未实现 | Edits multiline intersections, breaks, and vertices. | — |
| 561 | `MLEDIT` | 未实现 | Edits multiline intersections, breaks, and vertices. | — |
| 562 | `MLINE` | 未实现 | Creates multiple parallel lines. | — |
| 563 | `MLSTYLE` | 未实现 | Creates, modifies, and manages multiline styles. | — |
| 564 | `MOCORO` | 未实现 | Moves, copies, rotates, and scales selected objects with a single command. | — |
| 565 | `MODEL` | 已实现 | Switches from a named layout tab to the Model tab. | payload-free 选择子，复用 SPACE MODEL 的校验 + 自动取景；Model 标签页走同一命令 |
| 566 | `MOVE` | 已实现 | Moves objects a specified distance in a specified direction. | — |
| 567 | `MOVEBAK` | 未实现 | Changes the destination folder for drawing backup (BAK) files. | — |
| 568 | `MPEDIT` | 未实现 | Edits multiple polylines; also converts multiple line and arc objects to polyline objects. | — |
| 569 | `MREDO` | 未实现 | Reverses the effects of several previous UNDO or U commands. | — |
| 570 | `MSLIDE` | 未实现 | Creates a slide file of the current model viewport or the current layout. | — |
| 571 | `MSPACE` | 未实现 | In a layout, switches from paper space to model space in a layout viewport. | — |
| 572 | `MSTRETCH` | 未实现 | Stretches objects with multiple crossing windows and crossing polygons. | — |
| 573 | `MTEDIT` | 未实现 | Edits multiline text. | — |
| 574 | `-MTEXT` | 未实现 | Creates a multiline text object. | — |
| 575 | `MTEXT` | 部分实现 | Creates a multiline text object. | 两角宽度模型；无行距/分栏/背景填充 |
| 576 | `MULTIPLE` | 未实现 | Repeats the next command until canceled. | — |
| 577 | `MVIEW` | 未实现 | Creates and controls layout viewports. | — |
| 578 | `MVSETUP` | 未实现 | Sets up the specifications of a drawing. | — |
| 579 | `NAVBAR` | 未实现 | Provides access to viewing tools from a unified interface. | — |
| 580 | `NAVSMOTION` | 未实现 | Provides an on-screen display for creating and playing back cinematic camera animations for design review, presentation, and bookmark-style navigation. | — |
| 581 | `NAVSMOTIONCLOSE` | 未实现 | Closes the ShowMotion interface where you can navigate within a drawing by selecting a named view. | — |
| 582 | `NAVSWHEEL` | 未实现 | Provides access to enhanced navigation tools that are quickly accessible from the cursor. | — |
| 583 | `NAVVCUBE` | 未实现 | Indicates the current viewing direction. Dragging or clicking the ViewCube rotates the scene. | — |
| 584 | `NCOPY` | 未实现 | Copies objects that are contained in an xref, block, or DGN underlay. | — |
| 585 | `NETLOAD` | 未实现 | Loads a .NET application. | — |
| 586 | `NEW` | 已实现 | Starts a new drawing. | — |
| 587 | `NEWSHEETSET` | 未实现 | Creates a new sheet set data file that manages drawing layouts, file paths, and project data. | — |
| 588 | `NEWSHOT` | 未实现 | Creates a named view with motion that is played back when viewed with ShowMotion. | — |
| 589 | `NEWVIEW` | 未实现 | Saves a new, named view from the display in the current viewport, or by defining a rectangular window. | — |
| 590 | `-OBJECTSCALE` | 未实现 | From the Command prompt, adds or deletes supported scales for annotative objects. | — |
| 591 | `OBJECTSCALE` | 未实现 | Adds or deletes supported scales for annotative objects. | — |
| 592 | `OFFSET` | 部分实现 | Creates concentric circles, parallel lines, and parallel curves. | LINE/CIRCLE/ARC/LWPOLYLINE；ELLIPSE 拒绝；无全局自交修剪 |
| 593 | `OFFSETEDGE` | 未实现 | Creates a closed polyline or spline object that is offset at a specified distance from the edges of a selected planar face on a 3D solid or surface. | — |
| 594 | `OLECONVERT` | 未实现 | Specifies a different source application for an embedded OLE object, and controls whether the OLE object is represented by an icon. | — |
| 595 | `OLELINKS` | 未实现 | Updates, changes, and cancels a selected linked OLE object. | — |
| 596 | `OLEOPEN` | 未实现 | Opens the selected OLE object in its source application. | — |
| 597 | `OLERESET` | 未实现 | Restores the selected OLE object to its original size and shape. | — |
| 598 | `OLESCALE` | 未实现 | Controls the size, scale, and other properties of a selected OLE object. | — |
| 599 | `OOPS` | 部分实现 | Restores erased objects. | v1 只恢复最近一次 ERASE（不覆盖 BLOCK/WBLOCK 删除）；恢复实体用新 id；撤销恢复副本但不恢复快照 |
| 600 | `OPEN` | 已实现 | Opens an existing drawing file. | — |
| 601 | `OPENDWFMARKUP` | 未实现 | Opens a DWF or DWFx file that contains markups. | — |
| 602 | `OPENFROMWEBMOBILE` | 未实现 | Opens a drawing file from your online Autodesk Account. | — |
| 603 | `OPENSHEETSET` | 未实现 | Opens a selected sheet set. | — |
| 604 | `OPTIONS` | 已实现 | Customizes the program settings. | chrome 本地选择子：打开选项对话框 |
| 605 | `ORTHO` | 已实现 | Constrains cursor movement to the horizontal or vertical direction. | — |
| 606 | `OSNAP` | 已实现 | Sets running object snap modes. | — |
| 607 | `-OVERKILL` | 未实现 | Removes duplicate or overlapping lines, arcs, and polylines. Also, combines partially overlapping or contiguous ones. | — |
| 608 | `OVERKILL` | 部分实现 | Removes duplicate or overlapping lines, arcs, and polylines. Also, combines those that are partially overlapping or contiguous. | v1 仅删除完全相同的重复对象（同空间/图层/几何，1e-9）；无部分重叠、共线合并、圆弧覆盖圆、折线段优化、容差与图层/颜色忽略 |
| 609 | `-PAGESETUP` | 未实现 | Modifies the current page layout settings for the plotting device, paper size, plot scale, and several other settings in the command line. | — |
| 610 | `PAGESETUP` | 未实现 | Controls the page layout, plotting device, paper size, and other settings for each new layout. | — |
| 611 | `-PAN` | 未实现 | Shifts the view by specifying up to two points to determine a distance and direction. | — |
| 612 | `PAN` | 已实现 | Shifts the view without changing the viewing direction or magnification. | — |
| 613 | `-PARAMETERS` | 未实现 | Opens the Parameters Manager palette that includes all dimensional constraint parameters, reference parameters, and user variables in the current drawing. | — |
| 614 | `PARAMETERS` | 未实现 | Opens the Parameters Manager palette that includes all dimensional constraint parameters, reference parameters, and user variables in the current drawing. | — |
| 615 | `PARAMETERSCLOSE` | 未实现 | Closes the Parameters Manager palette. | — |
| 616 | `-PARTIALOAD` | 未实现 | Loads additional geometry into a partially opened drawing from the command line. | — |
| 617 | `PARTIALOAD` | 未实现 | Loads additional geometry into a partially opened drawing. | — |
| 618 | `PARTIALOPEN` | 未实现 | Loads geometry and named objects from a selected view or layer into a drawing. | — |
| 619 | `PASTEASHYPERLINK` | 未实现 | Creates a hyperlink to a file, and associates it with a selected object. | — |
| 620 | `PASTEBLOCK` | 部分实现 | Pastes objects from the Clipboard into the current drawing as a block. | 应用内剪贴板 → 自动命名块；不是 OS 剪贴板 |
| 621 | `PASTECLIP` | 部分实现 | Pastes objects from the Clipboard into the current drawing. | 应用内剪贴板；不是 OS 剪贴板，不跨应用 |
| 622 | `PASTEORIG` | 部分实现 | Pastes objects from the Clipboard into the current drawing using the original coordinates. | 应用内剪贴板；不是 OS 剪贴板，不跨应用 |
| 623 | `PASTESPEC` | 未实现 | Pastes objects from the Clipboard into the current drawing and controls the format of the data. | — |
| 624 | `PCEXTRACTCENTERLINE` | 未实现 | Creates a line through the center axis of a cylindrical segment in a point cloud. | — |
| 625 | `PCEXTRACTCORNER` | 未实现 | Creates a point object at the intersection of three planar segments in a point cloud. | — |
| 626 | `PCEXTRACTEDGE` | 未实现 | Infers the intersection between two adjacent planar segments, and creates a line along the edge. | — |
| 627 | `PCEXTRACTSECTION` | 未实现 | Generates 2D geometry from a section through a point cloud. | — |
| 628 | `PDFADJUST` | 未实现 | Adjust the fade, contrast, and monochrome settings of a PDF underlay. | — |
| 629 | `-PDFATTACH` | 未实现 | Attach a PDF underlay from the command line. | — |
| 630 | `PDFATTACH` | 未实现 | Inserts a PDF file as an underlay into the current drawing. | — |
| 631 | `PDFCLIP` | 未实现 | Crops the display of a selected PDF underlay to a specified boundary. | — |
| 632 | `-PDFIMPORT` | 未实现 | Imports the objects in a PDF file or an attached PDF underlay from the command line. | — |
| 633 | `PDFIMPORT` | 未实现 | Imports the geometry, fills, raster images, and TrueType text objects from a specified PDF file. | — |
| 634 | `PDFLAYERS` | 未实现 | Controls the display of layers in a PDF underlay. | — |
| 635 | `-PDFSHXTEXT` | 未实现 | Converts the SHX geometry imported from PDF files into individual multiline text objects from the command line. | — |
| 636 | `PDFSHXTEXT` | 未实现 | Converts the SHX geometry imported from PDF files into individual multiline text objects. | — |
| 637 | `PEDIT` | 未实现 | Edits polylines, objects to be joined to polylines, and related objects. | — |
| 638 | `PERFANALYZER` | 未实现 | Opens the Performance Analyzer palette from which you can diagnose operations in AutoCAD that seem slow or unresponsive. | — |
| 639 | `PERFANALYZERCLOSE` | 未实现 | Closes the Performance Analyzer palette. | — |
| 640 | `PFACE` | 未实现 | Creates a 3D polyface mesh, vertex by vertex. | — |
| 641 | `PLAN` | 部分实现 | Displays an orthographic view of the XY plane of a specified user coordinate system. | payload-free 选择子：Top/plan 2D 视图；无 UCS/World/Current 选项（本构建无 UCS 模型） |
| 642 | `PLANESURF` | 未实现 | Creates a planar surface. | — |
| 643 | `PLINE` | 已实现 | Creates a 2D polyline, a single object that is composed of line and arc segments. | — |
| 644 | `-PLOT` | 未实现 | Plots a drawing to a plotter, printer, or file in the command line. | — |
| 645 | `PLOT` | 部分实现 | Plots a drawing to a plotter, printer, or file. | 仅矢量 SVG/PDF；无 CTB/STB、无批量多布局 |
| 646 | `-PLOTSTAMP` | 未实现 | Places a plot stamp on a specified corner of each drawing and logs it to a file. | — |
| 647 | `PLOTSTAMP` | 未实现 | Places a plot stamp with information such as date, time, and scale on a specified corner of each drawing and logs it to a file. | — |
| 648 | `-PLOTSTYLE` | 未实现 | Lists all available plotstyles in the current drawing and to set a plotstyle current. | — |
| 649 | `PLOTSTYLE` | 未实现 | Controls the named plot styles that are attached to the current layout and can be assigned to objects. | — |
| 650 | `PLOTTERMANAGER` | 未实现 | Displays the Plotter Manager, where you can add or edit a plotter configuration. | — |
| 651 | `PLT2DWG` | 未实现 | Imports legacy HPGL files into the current drawing, retaining all colors. | — |
| 652 | `PMTOGGLE` | 未实现 | Controls whether the Performance Recorder is turned on or off. | — |
| 653 | `PNGOUT` | 未实现 | Saves selected objects to a file in a Portable Network Graphics format. | — |
| 654 | `POINT` | 已实现 | Creates a point object. | — |
| 655 | `-POINTCLOUDATTACH` | 未实现 | Inserts a point cloud file into the current drawing from the Command prompt. | — |
| 656 | `POINTCLOUDATTACH` | 未实现 | Inserts a point cloud scan (RCS) or project file (RCP) into the current drawing. | — |
| 657 | `POINTCLOUDCOLORMAP` | 未实现 | Displays the Point Cloud Color Map dialog box, used to define settings for Intensity, Elevation, and Classification point cloud stylizations. | — |
| 658 | `POINTCLOUDCROP` | 未实现 | Crops a selected point cloud to a specified polygonal, rectangular, or circular boundary. | — |
| 659 | `POINTCLOUDCROPSTATE` | 未实现 | Saves, restores, and deletes point cloud crop states. | — |
| 660 | `-POINTCLOUDMANAGER` | 未实现 | Allows use of automation to control display of regions and scans in point cloud objects. | — |
| 661 | `POINTCLOUDMANAGER` | 未实现 | Displays the Point Cloud Manager palette, used to control display of point cloud projects, regions, and scans. | — |
| 662 | `POINTCLOUDMANAGERCLOSE` | 未实现 | Closes the Point Cloud Manager. | — |
| 663 | `POINTCLOUDSTYLIZE` | 未实现 | Controls the coloration of point clouds. | — |
| 664 | `POINTCLOUDUNCROP` | 未实现 | Removes all cropped areas from selected point clouds. | — |
| 665 | `POINTLIGHT` | 未实现 | Creates a point light that radiates light in all directions from its location. | — |
| 666 | `POLYGON` | 已实现 | Creates an equilateral closed polyline. | 默认内接；外切变体是独立多词选择子 `POLYGON CIRCUMSCRIBED`（不是别名，见 §已实现索引） |
| 667 | `POLYSOLID` | 未实现 | Creates 3D solid in the shape of a wall or series of walls. | — |
| 668 | `PRESSPULL` | 未实现 | Dynamically modifies objects by extrusion and offset. | — |
| 669 | `PREVIEW` | 未实现 | Displays the drawing as it will be plotted. | — |
| 670 | `PROJECTGEOMETRY` | 未实现 | Projects points, lines, or curves onto a 3D solid or surface from different directions. | — |
| 671 | `PROPERTIES` | 已实现 | Controls properties of existing objects. | chrome 本地选择子：打开特性面板 |
| 672 | `PROPERTIESCLOSE` | 已实现 | Closes the Properties palette. | chrome 本地选择子：关闭特性面板（面板关闭按钮走同一命令） |
| 673 | `PROPULATE` | 未实现 | Update, list, or clear Drawing Properties data. | — |
| 674 | `PSBSCALE` | 未实现 | Specifies or updates the scale of block objects relative to paper space. | — |
| 675 | `-PSETUPIN` | 未实现 | Imports a user-defined page setup into a new drawing layout using the command line. | — |
| 676 | `PSETUPIN` | 未实现 | Imports a user-defined page setup into a new drawing layout. | — |
| 677 | `PSOUT` | 未实现 | Creates a PostScript file from DWG file. | — |
| 678 | `PSPACE` | 未实现 | In a layout, switches from model space in a layout viewport to paper space. | — |
| 679 | `PSTSCALE` | 未实现 | Specifies or updates the scale of text objects relative to paper space. | — |
| 680 | `PTYPE` | 未实现 | Specifies the display style and size of point objects. | — |
| 681 | `-PUBLISH` | 未实现 | Publishes drawings to DWF, DWFx, and PDF files, or to printers or plotters using the command line. | — |
| 682 | `PUBLISH` | 未实现 | Publishes drawings to DWF, DWFx, and PDF files, or to printers or plotters. | — |
| 683 | `-PURGE` | 未实现 | Removes unused named objects, such as block definitions and layers, from the current drawing using the command line. | — |
| 684 | `PURGE` | 未实现 | Removes unused items, such as block definitions and layers, from the drawing. | — |
| 685 | `PURGEAECDATA` | 未实现 | Removes the invisible AEC data (AutoCAD Architecture and AutoCAD Civil 3D custom objects) in the drawing at the Command prompt. | — |
| 686 | `PUSHTODOCSCLOSE` | 未实现 | Closes the Push to Autodesk Docs palette. | — |
| 687 | `PUSHTODOCSOPEN` | 未实现 | Opens the Push to Autodesk Docs palette where you can select AutoCAD layouts to upload as PDFs to Autodesk Docs. | — |
| 688 | `PYRAMID` | 未实现 | Creates a 3D solid pyramid. | — |
| 689 | `QCCLOSE` | 未实现 | Closes the QuickCalc calculator. | — |
| 690 | `QDIM` | 未实现 | Creates a series of dimensions quickly from selected objects. | — |
| 691 | `QLATTACH` | 未实现 | Attaches a leader line to an mtext, tolerance, or block reference object. | — |
| 692 | `QLATTACHSET` | 未实现 | Globally attaches leader lines to mtext, tolerance, or block reference objects. | — |
| 693 | `QLDETACHSET` | 未实现 | Detaches a leader line from an mtext, tolerance, or block reference object. | — |
| 694 | `QLEADER` | 未实现 | Creates a leader and leader annotation. | — |
| 695 | `QNEW` | 未实现 | Starts a new drawing from a specified drawing template file. | — |
| 696 | `QQUIT` | 未实现 | Closes all open drawings and exit. | — |
| 697 | `QSAVE` | 部分实现 | Saves the current drawing using the specified default file format. | SAVE 别名：有损 DXF 另存；源为 .dwg 时可实验性导出 DWG（新文件、默认 <stem>-export.dwg、绝不覆盖源；仅合成 self-roundtrip 证据，vendor/AutoCAD NOT VERIFIED） |
| 698 | `QSELECT` | 未实现 | Creates a selection set based on filtering criteria. | — |
| 699 | `QTEXT` | 未实现 | Controls the display and plotting of text and attribute objects. | — |
| 700 | `QUICKCALC` | 未实现 | Opens the QuickCalc calculator. | — |
| 701 | `QUICKCUI` | 未实现 | Displays the Customize User Interface Editor in a collapsed state. | — |
| 702 | `QUICKPROPERTIES` | 未实现 | Displays quick property data for selected objects. | — |
| 703 | `QUIT` | 未实现 | Exits the program. | — |
| 704 | `QVDRAWING` | 未实现 | Displays open drawings and layouts in a drawing using preview images. | — |
| 705 | `QVDRAWINGCLOSE` | 未实现 | Closes preview images of open drawings and their layouts. | — |
| 706 | `QVLAYOUT` | 未实现 | Displays preview images of model space and layouts for the current drawing. | — |
| 707 | `QVLAYOUTCLOSE` | 未实现 | Closes preview images of model space and layouts in the current drawing | — |
| 708 | `RAY` | 部分实现 | Creates a linear object that starts at a point and continues to infinity. | 两点创建，但落库为裁剪到模型包围盒的有限线段 |
| 709 | `RECOVER` | 未实现 | Repairs and then opens a damaged drawing file. | — |
| 710 | `RECOVERALL` | 未实现 | Repairs a damaged drawing file along with all attached xrefs. | — |
| 711 | `RECTANG` | 已实现 | Creates a rectangular polyline. | — |
| 712 | `REDEFINE` | 未实现 | Restores AutoCAD internal commands overridden by UNDEFINE. | — |
| 713 | `REDIR` | 未实现 | Redefines hard-coded paths in xrefs, images, shapes, styles, and rtext. | — |
| 714 | `REDIRMODE` | 未实现 | Sets options for the REDIR Express Tool by specifying which object types are included. | — |
| 715 | `REDO` | 已实现 | Reverses the effects of previous UNDO or U command. | — |
| 716 | `REDRAW` | 未实现 | Refreshes the display in the current viewport. | — |
| 717 | `REDRAWALL` | 未实现 | Refreshes the display in all viewports. | — |
| 718 | `REFCLOSE` | 未实现 | Saves back or discards changes made during in-place editing of a reference, either an xref or a block definition. | — |
| 719 | `-REFEDIT` | 未实现 | Edits an xref or a block definition directly within the current drawing. | — |
| 720 | `REFEDIT` | 未实现 | Edits an xref or a block definition directly within the current drawing. | — |
| 721 | `REFSET` | 未实现 | Adds or removes objects from a working set during in-place editing of a reference, either an xref or a block definition. | — |
| 722 | `REGEN` | 未实现 | Regenerates the drawing from within the current viewport. | — |
| 723 | `REGEN3` | 未实现 | Regenerates the views in a drawing to repair anomalies in the display of 3D solids and surfaces. | — |
| 724 | `REGENALL` | 未实现 | Regenerates the entire drawing and refreshes all viewports. | — |
| 725 | `REGENAUTO` | 未实现 | Obsolete. Controls automatic regeneration of a drawing. | — |
| 726 | `REGION` | 未实现 | Converts objects that enclose an area into a 2D region object. | — |
| 727 | `REINIT` | 未实现 | Reinitializes the digitizer, digitizer input/output port, and program parameters file. | — |
| 728 | `-RENAME` | 未实现 | Changes the names assigned to named objects from the Command prompt. | — |
| 729 | `RENAME` | 未实现 | Changes the names assigned to items such as layers and dimension styles. | — |
| 730 | `-RENDER` | 未实现 | Creates a photorealistic or realistically shaded image of a 3D solid or surface model from the Command prompt. | — |
| 731 | `RENDER` | 未实现 | Creates a photorealistic or realistically shaded image of a 3D solid or surface model. | — |
| 732 | `RENDERCROP` | 未实现 | Renders a specified rectangular area, called a crop window, within a viewport. | — |
| 733 | `RENDERENVIRONMENT` | 未实现 | Controls the settings related to the rendering environment. | — |
| 734 | `RENDERENVIRONMENTCLOSE` | 未实现 | Closes the Render Environment & Exposure palette. | — |
| 735 | `RENDEREXPOSURE` | 未实现 | Controls the settings related to the rendering environment. | — |
| 736 | `RENDEREXPOSURECLOSE` | 未实现 | Closes the Render Environment & Exposure palette. | — |
| 737 | `RENDERONLINE` | 未实现 | Uses the Autodesk online resources to create an image of a 3D solid or surface model. | — |
| 738 | `RENDERPRESETS` | 未实现 | Specifies render presets, reusable rendering parameters, for rendering an image. | — |
| 739 | `RENDERPRESETSCLOSE` | 未实现 | Closes the Render Presets Manager palette. | — |
| 740 | `RENDERWIN` | 未实现 | Obsolete. Displays the Render window without starting a rendering operation. | — |
| 741 | `RENDERWINDOW` | 未实现 | Displays the Render window without starting a rendering operation. | — |
| 742 | `RENDERWINDOWCLOSE` | 未实现 | Closes the Render window. | — |
| 743 | `REPURLS` | 未实现 | Finds and replaces a specified text string in a URL used in hyperlinks that are attached to all selected objects. | — |
| 744 | `RESETBLOCK` | 未实现 | Resets one or more dynamic block references to the default values of the block definition. | — |
| 745 | `RESUME` | 未实现 | Continues an interrupted script. | — |
| 746 | `REVCLOUD` | 部分实现 | Creates or modifies a revision cloud. | v1 对象模式：把一条选中的闭合直线段 LWPOLYLINE 的每条边换成一段向外弧；无矩形/多边形绘制、无 Arc length |
| 747 | `REVCLOUDPROPERTIES` | 未实现 | Controls the approximate chord length for the arcs in a selected revision cloud. | — |
| 748 | `REVERSE` | 部分实现 | Reverses the vertices of selected lines, polylines, splines, and helixes, which is useful for linetypes with included text, or wide polylines with differing beginning and ending widths. | 仅 LINE/LWPOLYLINE/SPLINE，当前选择为全部输入；无交互拾取提示，无螺旋/3D 变体 |
| 749 | `REVERT` | 未实现 | Closes and then reopens the current drawing. | — |
| 750 | `REVOLVE` | 未实现 | Creates a 3D solid or surface by sweeping an object around an axis. | — |
| 751 | `REVSURF` | 未实现 | Creates a mesh by revolving a profile about an axis. | — |
| 752 | `RIBBON` | 未实现 | Displays the ribbon. | ribbon 恒显示；无 RIBBON 命令 |
| 753 | `RIBBONCLOSE` | 未实现 | Hides the ribbon. | — |
| 754 | `ROTATE` | 已实现 | Rotates objects around a base point. | — |
| 755 | `ROTATE3D` | 未实现 | Moves objects about a 3D axis. | — |
| 756 | `RPREF` | 未实现 | Displays the Render Presets Manager palette used to configure rendering settings. | — |
| 757 | `RPREFCLOSE` | 未实现 | Closes the Render Settings Manager palette. | — |
| 758 | `RSCRIPT` | 未实现 | Repeats a script file. | — |
| 759 | `RTEDIT` | 未实现 | Edits existing remote text (rtext) objects. | — |
| 760 | `RTEXT` | 未实现 | Creates remote text (rtext) objects. | — |
| 761 | `RTUCS` | 未实现 | Rotates the UCS dynamically with your pointing device. | — |
| 762 | `RULESURF` | 未实现 | Creates a mesh that represents the surface between two lines or curves. | — |
| 763 | `SAVE` | 部分实现 | Saves the current drawing under a different file name or location without changing which drawing file is the current one. | 有损 DXF 另存；无 DWG 回写 |
| 764 | `SAVEALL` | 未实现 | Save all open drawings. | — |
| 765 | `SAVEAS` | 部分实现 | Saves a copy of the current drawing under a new file name or location. | SAVE 别名：有损 DXF 另存；源为 .dwg 时可实验性导出 DWG（新文件、默认 <stem>-export.dwg、绝不覆盖源；仅合成 self-roundtrip 证据，vendor/AutoCAD NOT VERIFIED） |
| 766 | `SAVEIMG` | 未实现 | Saves a rendered image to a file. | — |
| 767 | `SAVETOWEBMOBILE` | 未实现 | Saves a copy of the current drawing to your Autodesk Account. | — |
| 768 | `SCALE` | 已实现 | Enlarges or reduces selected objects, keeping the proportions of the object the same after scaling. | — |
| 769 | `-SCALELISTEDIT` | 未实现 | Controls the list of scales available for layout viewports, page layouts, and plotting. | — |
| 770 | `SCALELISTEDIT` | 未实现 | Controls the list of scales available for layout viewports, page layouts, and plotting. | — |
| 771 | `SCALETEXT` | 已实现 | Enlarges or reduces selected text objects without changing their locations. | — |
| 772 | `SCRIPT` | 未实现 | Executes a sequence of commands from a script file. | — |
| 773 | `SCRIPTCALL` | 未实现 | Executes a sequence of commands and nested scripts from a script file. | — |
| 774 | `SECTION` | 未实现 | Uses the intersection of a plane with 3D solids, surfaces, or meshes to create a 2D region object. | — |
| 775 | `SECTIONPLANE` | 未实现 | Creates a section object that acts as a cutting plane through 3D objects, and point clouds. | — |
| 776 | `SECTIONPLANEJOG` | 未实现 | Adds a jogged segment to a section object. | — |
| 777 | `SECTIONPLANESETTINGS` | 未实现 | Sets display options for a selected section plane. | — |
| 778 | `SECTIONPLANETOBLOCK` | 未实现 | Saves selected section planes as 2D or 3D blocks. | — |
| 779 | `SECTIONSPINNERS` | 未实现 | Displays the dialog box to set the increment value for the Section Object Offset and Slice Thickness controls in the Section Plane ribbon contextual tab. | — |
| 780 | `SECURITYOPTIONS` | 未实现 | Controls the security restrictions for running executable files in the product. | — |
| 781 | `SELECT` | 部分实现 | Places selected objects in the Previous selection set. | 以选择载荷设置会话选择；不是 AutoCAD 的交互式 SELECT |
| 782 | `SELECTCOUNT` | 未实现 | Finds all objects within the current count that match the properties of the selected objects, and then adds them to the selection set. | — |
| 783 | `SELECTSIMILAR` | 部分实现 | Finds all objects within the current drawing that match the properties of selected objects, and then adds them to the selection set. | v1 仅按几何类型匹配并并入当前选择；无 layer/颜色/线型等属性匹配；仅模型空间 |
| 784 | `SETBYLAYER` | 部分实现 | Changes the property overrides of selected objects to ByLayer. | 预选择输入（不弹 Select objects）；只重置颜色/线型/线宽为随层；不重置透明度与材质，文字/标注/填充/多段线厚度未建模；Include blocks 未实现；锁定图层整命令拒绝；显示写入可撤销 |
| 785 | `SETVAR` | 未实现 | Lists or changes the values of system variables. | — |
| 786 | `-SHADEMODE` | 未实现 | Provides simple shading for 3D solids and surfaces. | — |
| 787 | `SHADEMODE` | 未实现 | Controls the display of 3D objects. | — |
| 788 | `SHAPE` | 未实现 | Inserts a shape from a shape file (SHX file) that has been loaded using LOAD. | — |
| 789 | `SHARE` | 未实现 | Share a link to a copy of the current drawing to view or edit in AutoCAD on the web (web.autocad.com) or AutoCAD on mobile. The drawing copy includes all external references and images. | — |
| 790 | `SHAREDVIEWS` | 未实现 | Opens the Shared Views palette. | — |
| 791 | `SHAREDVIEWSCLOSE` | 未实现 | Closes the Shared Views palette. | — |
| 792 | `-SHAREVIEW` | 未实现 | Publishes a representation of the current space or the entire drawing for online viewing and sharing. | — |
| 793 | `SHAREVIEW` | 未实现 | Publishes a representation of the current space or the entire drawing for online viewing and sharing. | — |
| 794 | `SHEETSET` | 未实现 | Opens the Sheet Set Manager. | — |
| 795 | `SHEETSETHIDE` | 未实现 | Closes the Sheet Set Manager. | — |
| 796 | `SHELL` | 未实现 | Accesses operating system commands. | — |
| 797 | `SHOWPALETTES` | 未实现 | Restores the display of hidden palettes, the ribbon, and the drawing tabs. | — |
| 798 | `SHOWRENDERGALLERY` | 未实现 | Displays the images that were rendered and stored in your Autodesk account. | — |
| 799 | `SHOWURLS` | 未实现 | Displays all attached URLs contained in a drawing, and allows them to edited. | — |
| 800 | `SHP2BLK` | 未实现 | Converts all instances of a selected shape object with an equivalent block. | — |
| 801 | `SIGVALIDATE` | 未实现 | Displays information about the digital signature attached to a drawing file. | — |
| 802 | `SKETCH` | 未实现 | Creates a series of freehand line segments. | — |
| 803 | `SLICE` | 未实现 | Creates new 3D solids and surfaces by slicing, or dividing, existing objects. | — |
| 804 | `SNAP` | 已实现 | Restricts cursor movement to specified intervals. | — |
| 805 | `SOLDRAW` | 未实现 | Generates profiles and sections in layout viewports created with SOLVIEW. | — |
| 806 | `SOLID` | 未实现 | Creates solid-filled triangles and quadrilaterals. | — |
| 807 | `SOLIDEDIT` | 未实现 | Edits faces and edges of 3D solid objects. | — |
| 808 | `SOLPROF` | 未实现 | Creates 2D profile images of 3D solids for display in a layout viewport. | — |
| 809 | `SOLVIEW` | 未实现 | Creates orthographic views, layers, and layout viewports automatically for 3D solids. | — |
| 810 | `SPACETRANS` | 未实现 | Calculates equivalent model space and paper space distances in a layout. | — |
| 811 | `SPELL` | 未实现 | Checks spelling in a drawing. | — |
| 812 | `SPHERE` | 未实现 | Creates a 3D solid sphere. | — |
| 813 | `SPLINE` | 已实现 | Creates a smooth curve that passes through or near a set of fit points, or that is defined by the vertices in a control frame. | — |
| 814 | `SPLINEDIT` | 未实现 | Modifies the parameters of a spline or converts a spline-fit polyline to a spline. | — |
| 815 | `SPOTLIGHT` | 未实现 | Creates a spotlight that emits a directional cone of light. | — |
| 816 | `SSX` | 未实现 | Creates a selection set based on a selected object. | — |
| 817 | `STANDARDS` | 未实现 | Manages the association of standards files with drawings. | — |
| 818 | `STATUS` | 部分实现 | Displays drawing statistics, modes, and extents. | 报告统计/模式/模型空间范围；创建/编辑时间、内存、图形界限与基点显式 `status.not_tracked`（无图形头模型） |
| 819 | `STLOUT` | 未实现 | Stores 3D solids and watertight meshes in a format that can be used with stereolithography apparatus. | — |
| 820 | `STRETCH` | 部分实现 | Stretches objects crossed by a selection window or polygon. | 恒交叉窗口；TEXT/MultiLeader 整命令拒绝 |
| 821 | `-STYLE` | 未实现 | Creates, modifies, or specifies text styles. | — |
| 822 | `STYLE` | 未实现 | Creates, modifies, or specifies text styles. | — |
| 823 | `STYLESMANAGER` | 未实现 | Displays the Plot Style Manager, where you can revise plot style tables. | — |
| 824 | `SUBTRACT` | 未实现 | Creates as a new object by subtracting one overlapping region or 3D solid from another. | — |
| 825 | `SUNPROPERTIES` | 未实现 | Displays the Sun Properties palette. | — |
| 826 | `SUNPROPERTIESCLOSE` | 未实现 | Closes the Sun Properties palette | — |
| 827 | `SUPERHATCH` | 未实现 | Hatches an area using a selected image, block, xref, or wipeout object. | — |
| 828 | `SURFBLEND` | 未实现 | Creates a continuous blend surface between two existing surfaces. | — |
| 829 | `SURFEXTEND` | 未实现 | Lengthens a surface by a specified distance. | — |
| 830 | `SURFEXTRACTCURVE` | 未实现 | Creates curves on surfaces and 3D solids. | — |
| 831 | `SURFFILLET` | 未实现 | Creates a filleted surface between two other surfaces. | — |
| 832 | `SURFNETWORK` | 未实现 | Creates a surface in the space between several curves in the U and V directions, including surface and solid edge subobjects. | — |
| 833 | `SURFOFFSET` | 未实现 | Creates a parallel surface a specified distance from the original surface. | — |
| 834 | `SURFPATCH` | 未实现 | Creates a new surface by fitting a cap over a surface edge that forms a closed loop. | — |
| 835 | `SURFSCULPT` | 未实现 | Trims and combines a set of surfaces or meshes that completely enclose a volume to create a 3D solid. | — |
| 836 | `SURFTRIM` | 未实现 | Trims portions of a surface where it meets another surface or type of geometry. | — |
| 837 | `SURFUNTRIM` | 未实现 | Replaces surface areas removed by the SURFTRIM command. | — |
| 838 | `SWEEP` | 未实现 | Creates a 3D solid or 3D surface by sweeping a 2D object or subobject along an open or closed path. | — |
| 839 | `SYSVARMONITOR` | 未实现 | Monitors a list of system variables and sends notifications when there are changes to any one in the list. | — |
| 840 | `SYSVDLG` | 未实现 | Allows you to view, edit, save, and restore system variable settings. | — |
| 841 | `SYSWINDOWS` | 未实现 | Arranges windows and icons when the application window is shared with external applications. | — |
| 842 | `-TABLE` | 未实现 | Creates an empty table object. | — |
| 843 | `TABLE` | 未实现 | Creates an empty table object. | — |
| 844 | `TABLEDIT` | 未实现 | Edits text in a table cell. | — |
| 845 | `TABLEEXPORT` | 未实现 | Exports data from a table object in CSV file format. | — |
| 846 | `TABLESTYLE` | 未实现 | Creates, modifies, or specifies table styles. | — |
| 847 | `TABLET` | 未实现 | Calibrates, configures, and turns on and off an attached digitizing tablet. | — |
| 848 | `TABSURF` | 未实现 | Creates a mesh from a line or curve that is swept along a straight path. | — |
| 849 | `TARGETPOINT` | 未实现 | Creates a target point light. | — |
| 850 | `TASKBAR` | 未实现 | Controls the Peek/Preview behavior of the application's button in the Windows taskbar when multiple drawings are open. | — |
| 851 | `TCASE` | 部分实现 | Changes the case of selected text, mtext, attributes, and dimension text. | 选择式；只处理 Text/MText（MULTILEADER 走 with_text）；裸命令默认 Title，键入 `TCASE UPPER/LOWER/TITLE/SENTENCE/TOGGLE` 或 `U/L/T/S/G`；MTEXT 格式转义逐字保留；属性/标注文字未建模（`tcase.not_reset`） |
| 852 | `TCIRCLE` | 未实现 | Creates a circle, a slot, or a rectangle around each selected text or mtext object. | — |
| 853 | `TCOUNT` | 未实现 | Adds sequential numbering to text and mtext objects as a prefix, suffix, or replacement text. | — |
| 854 | `TEXT` | 已实现 | Creates a single-line text object. | — |
| 855 | `TEXTALIGN` | 未实现 | Aligns multiple text objects vertically, horizontally, or obliquely. | — |
| 856 | `TEXTEDIT` | 部分实现 | Edits a selected multiline or single-line text object, or the text in a dimension object. | 整串替换；无光标级编辑、不解析 MTEXT 标记 |
| 857 | `TEXTFIT` | 未实现 | Expands or compresses the width of text objects based on new starting and ending points. | — |
| 858 | `TEXTMASK` | 未实现 | Creates a blank area behind selected text or mtext objects. | — |
| 859 | `TEXTSCR` | 未实现 | Opens a text window that displays a history of prompts and command line entries for the current session. | — |
| 860 | `TEXTTOFRONT` | 未实现 | Brings text, leaders, and dimensions in front of all other objects in the drawing. | — |
| 861 | `TEXTUNMASK` | 未实现 | Removes the mask from selected text or mtext that were masked with TEXTMASK. | — |
| 862 | `TFRAMES` | 未实现 | Toggles the display of frames for all wipeout and image objects. | — |
| 863 | `THICKEN` | 未实现 | Converts a surface into a 3D solid with a specified thickness. | — |
| 864 | `TIFOUT` | 未实现 | Saves selected objects to a file in TIFF file format. | — |
| 865 | `TIME` | 未实现 | Displays the date and time statistics of a drawing. | — |
| 866 | `TINSERT` | 未实现 | Inserts a block in a table cell. | — |
| 867 | `TJUST` | 未实现 | Changes the justification point of a text object without moving the text. Works with text, mtext, and attribute definition objects. | — |
| 868 | `TOLERANCE` | 未实现 | Creates geometric tolerances contained in a feature control frame. | — |
| 869 | `-TOOLBAR` | 未实现 | Displays, hides, and customizes toolbars from the Command prompt. | — |
| 870 | `TOOLBAR` | 未实现 | Displays, hides, and customizes toolbars. | — |
| 871 | `TOOLPALETTES` | 未实现 | Opens the Tool Palettes window. | — |
| 872 | `TOOLPALETTESCLOSE` | 未实现 | Closes the Tool Palettes window. | — |
| 873 | `TORIENT` | 部分实现 | Rotates text, mtext, attribute definitions, and blocks with attributes for readability. | 选择式；裸命令 Most Readable（归一到 `(-π/2, π/2]`），`TORIENT <角度>` 绝对旋转；只改 rotation；非 Text/属性/块属性报告 `torient.not_reset`；MULTILEADER 文本旋转无 setter |
| 874 | `TORUS` | 未实现 | Creates a donut-shaped 3D solid. | — |
| 875 | `TPNAVIGATE` | 未实现 | Displays a specified tool palette or palette group. | — |
| 876 | `TRACE` | 未实现 | Opens and manages traces from the command prompt. | — |
| 877 | `TRACEBACK` | 未实现 | Changes the active trace to view mode so you can edit the parent drawing while the trace is still visible. | — |
| 878 | `TRACEEDIT` | 未实现 | Changes the active trace to edit mode so you can contribute to the trace. | — |
| 879 | `TRACEFRONT` | 未实现 | Changes the active trace to edit mode so you can contribute to the trace. | — |
| 880 | `TRACEPALETTECLOSE` | 未实现 | Closes the Trace palette where you view and manage traces in the current drawing. | — |
| 881 | `TRACEPALETTEOPEN` | 未实现 | Opens the Trace palette where you can view and manage traces in the current drawing. | — |
| 882 | `TRACEVIEW` | 未实现 | Changes the active trace to view mode so you can edit the parent drawing while the trace is still visible. | — |
| 883 | `TRANSPARENCY` | 未实现 | Controls whether background pixels in an image are transparent or opaque. | — |
| 884 | `TRAYSETTINGS` | 未实现 | Controls the display of icons and notifications in the status bar tray. | — |
| 885 | `TREESTAT` | 未实现 | Displays information about the drawing's current spatial index. | — |
| 886 | `TREX` | 未实现 | Combines the TRIM and EXTEND commands. | — |
| 887 | `TRIM` | 部分实现 | Trims objects to meet the edges of other objects. | 目标 LINE/ARC/CIRCLE/SPLINE；边界含折线/样条采样 |
| 888 | `TSCALE` | 未实现 | Scales text, mtext, attributes, and attribute definitions. | — |
| 889 | `TXT2MTXT` | 部分实现 | Converts or combines single-line or multiline text objects into one or more multiline text objects. | 选择式 v1：仅模型空间 Text/MText；上→下、左→右稳定排序，`\P` 原样拼接为一个 `AcDbMText`；沿用首源样式/位置/字高/旋转/对齐/层/绘制顺序/`wrap_width`；同事务删除源；设置对话框/强制统一行距未实现 |
| 890 | `TXTEXP` | 未实现 | Explodes text or mtext objects into polyline objects. | — |
| 891 | `U` | 已实现 | Reverses the most recent operation. | — |
| 892 | `UCS` | 未实现 | Sets the origin and orientation of the current user coordinate system (UCS). | — |
| 893 | `UCSICON` | 未实现 | Controls the visibility, placement, appearance, and selectability of the UCS icon. | — |
| 894 | `UCSMAN` | 未实现 | Manages UCS definitions. | — |
| 895 | `ULAYERS` | 未实现 | Controls the display of layers in a DWF, DWFx, PDF, or DGN underlay. | — |
| 896 | `UNDEFINE` | 未实现 | Allows an application-defined command to override an internal command. | — |
| 897 | `UNDO` | 已实现 | Reverses the effect of commands. | — |
| 898 | `UNGROUP` | 未实现 | Disassociates the objects from a group. | — |
| 899 | `UNION` | 未实现 | Combines two or more 3D solids, surfaces, or 2D regions into a single, composite 3D solid, surface, or region. | — |
| 900 | `UNISOLATEOBJECTS` | 未实现 | Displays objects previously hidden with the ISOLATEOBJECTS or HIDEOBJECTS command. | — |
| 901 | `-UNITS` | 未实现 | Controls the precision and display formats for coordinates, distances, and angles. | — |
| 902 | `UNITS` | 未实现 | Controls the precision and display formats for coordinates, distances, and angles. | — |
| 903 | `UPDATEFIELD` | 未实现 | Updates fields in selected objects. | — |
| 904 | `UPDATETHUMBSNOW` | 未实现 | Manually updates thumbnail previews for named views, drawings, and layouts. | — |
| 905 | `VBAIDE` | 未实现 | Displays the Visual Basic Editor. | — |
| 906 | `VBALOAD` | 未实现 | Loads a global VBA project into the current work session. | — |
| 907 | `VBAMAN` | 未实现 | Manages VBA project operations using a dialog box. | — |
| 908 | `VBAPREF` | 未实现 | Provides access to some of the VBA environment settings. | — |
| 909 | `VBARUN` | 未实现 | Runs a VBA macro. | — |
| 910 | `VBASTMT` | 未实现 | Executes a VBA statement at the AutoCAD Command prompt. | — |
| 911 | `VBAUNLOAD` | 未实现 | Unloads a global VBA project. | — |
| 912 | `-VIEW` | 未实现 | Saves and restores named model space views, layout views, and preset views from the Command prompt. | — |
| 913 | `VIEW` | 未实现 | Saves and restores named model space views, layout views, and preset views. | — |
| 914 | `VIEWBACK` | 未实现 | Restores sequential views backward after changing the view. | — |
| 915 | `VIEWBASE` | 未实现 | Creates a base view from model space or Autodesk Inventor models. | — |
| 916 | `VIEWCOMPONENT` | 未实现 | Selects components from a model documentation drawing view for editing. | — |
| 917 | `VIEWDETAIL` | 未实现 | Creates a detail view of a portion of a model documentation drawing view. | — |
| 918 | `VIEWDETAILSTYLE` | 未实现 | Creates and modifies detail view styles. | — |
| 919 | `VIEWEDIT` | 未实现 | Edits an existing model documentation drawing view. | — |
| 920 | `VIEWFORWARD` | 未实现 | Restores sequential views forward after displaying a previous view with VIEWBACK or Zoom Previous. | — |
| 921 | `VIEWGO` | 未实现 | Restores a named view. | — |
| 922 | `VIEWPLAY` | 未实现 | Plays the animation associated to a named view. | — |
| 923 | `VIEWPLOTDETAILS` | 未实现 | Displays information about completed plot and publish jobs. | — |
| 924 | `VIEWPROJ` | 未实现 | Creates one or more projected views from an existing model documentation drawing view. | — |
| 925 | `VIEWRES` | 未实现 | Sets the tessellation for objects in the current viewport when WHIPARC is turned off. | — |
| 926 | `VIEWSECTION` | 未实现 | Creates a section view of a 3D model that was created in AutoCAD or in Autodesk Inventor. | — |
| 927 | `VIEWSECTIONSTYLE` | 未实现 | Creates and modifies section view styles. | — |
| 928 | `VIEWSETPROJ` | 未实现 | Specifies the active project file for drawings containing model documentation drawing views from Inventor models. | — |
| 929 | `VIEWSKETCHCLOSE` | 未实现 | Exits the symbol sketch mode. | — |
| 930 | `VIEWSTD` | 未实现 | Defines the default settings for model documentation drawing views. | — |
| 931 | `VIEWSYMBOLSKETCH` | 未实现 | Opens an editing environment so you can constrain section lines or detail boundaries to drawing view geometry. | — |
| 932 | `VIEWUPDATE` | 未实现 | Updates drawing views that have become out-of-date because the source model has changed. | — |
| 933 | `-VISUALSTYLES` | 未实现 | Creates and modifies visual styles from the command line. | — |
| 934 | `VISUALSTYLES` | 未实现 | Creates and modifies visual styles and applies a visual style to a viewport. | — |
| 935 | `VISUALSTYLESCLOSE` | 未实现 | Closes the Visual Styles Manager. | — |
| 936 | `VLISP` | 未实现 | Displays the AutoLISP development environment. | — |
| 937 | `VPCLIP` | 未实现 | Redefines a layout viewport object while retaining its properties. | — |
| 938 | `VPLAYER` | 未实现 | Sets layer visibility within viewports. | — |
| 939 | `VPMAX` | 未实现 | Expands the current layout viewport for editing. | — |
| 940 | `VPMIN` | 未实现 | Restores the current layout viewport. | — |
| 941 | `VPOINT` | 未实现 | Sets the viewing direction for a 3D visualization of the drawing. | — |
| 942 | `-VPORTS` | 未实现 | Creates multiple viewports in model space or paper space. | — |
| 943 | `VPORTS` | 未实现 | Creates multiple viewports in model space or in a layout (paper space). | — |
| 944 | `VPSCALE` | 未实现 | On a layout, displays the scale of the current viewport or of a selected layout viewport. | — |
| 945 | `VPSYNC` | 未实现 | Aligns the views in one or more adjacent layout viewports with a master layout viewport. | — |
| 946 | `VSCURRENT` | 未实现 | Sets the visual style in the current viewport. | — |
| 947 | `VSLIDE` | 未实现 | Displays an image slide file in the current viewport. | — |
| 948 | `VSSAVE` | 未实现 | Saves the current visual style under a new name. | — |
| 949 | `VTOPTIONS` | 未实现 | Displays a change in view as a smooth transition. | — |
| 950 | `WALKFLYSETTINGS` | 未实现 | Controls the walk and fly navigation settings. | — |
| 951 | `-WBLOCK` | 未实现 | Saves selected objects or converts a block to a specified drawing file. | — |
| 952 | `WBLOCK` | 未实现 | Saves selected objects or converts a block to a specified drawing file. | — |
| 953 | `WEBLIGHT` | 未实现 | Creates a precise 3D representation of the light intensity distribution of a light source. | — |
| 954 | `WEBLOAD` | 未实现 | Loads a JavaScript file from a URL, and then executes the JavaScript code contained in the file. | — |
| 955 | `WEDGE` | 未实现 | Creates a 3D solid wedge. | — |
| 956 | `WHOHAS` | 未实现 | Displays information about an open drawing file. | — |
| 957 | `WIPEOUT` | 部分实现 | Creates a wipeout object, and controls whether wipeout frames are displayed in the drawing. | v1 多边形遮罩：拾取边界点（≥3，回车结束）→ 一个 `Mask`（type-key `AcDbWipeout`，`inverted: false`）。**无**边框模型（Frames ON/OFF/不打印）、**无** `Polyline`/`Erase Polyline` 选项、**不检查**自交、不能拾取已有遮罩 |
| 958 | `WMFIN` | 未实现 | Imports a Windows metafile. | — |
| 959 | `WMFOPTS` | 未实现 | Sets options for WMFIN. | — |
| 960 | `WMFOUT` | 未实现 | Saves objects to a Windows metafile. | — |
| 961 | `WORKSPACE` | 未实现 | Creates, modifies, and saves workspaces and makes a workspace current. | — |
| 962 | `WSSAVE` | 未实现 | Saves a workspace. | — |
| 963 | `WSSETTINGS` | 未实现 | Sets options for workspaces. | — |
| 964 | `XATTACH` | 未实现 | Attaches selected DWG files as external references (xrefs). | — |
| 965 | `-XBIND` | 未实现 | Binds one or more definitions of named objects in an xref to the current drawing. | — |
| 966 | `XBIND` | 未实现 | Binds one or more definitions of named objects in an xref to the current drawing. | — |
| 967 | `XCLIP` | 未实现 | Crops the display of a selected external reference or block reference to a specified boundary. | — |
| 968 | `XCOMPARE` | 未实现 | Compares an attached xref with the latest state of the referenced drawing file, highlighting the differences with color within revision clouds. | — |
| 969 | `XCOMPARECLOSE` | 未实现 | Closes the Xref Compare toolbar and ends the comparison. | — |
| 970 | `XCOMPARERCNEXT` | 未实现 | Zooms to the next change set of the xref comparison result. | — |
| 971 | `XCOMPARERCPREV` | 未实现 | Zooms to the previous change set of the xref comparison result. | — |
| 972 | `XDATA` | 未实现 | Attaches extended object data (xdata) to a selected object. | — |
| 973 | `XDLIST` | 未实现 | Lists the xdata attached to an object. | — |
| 974 | `XEDGES` | 未实现 | Creates wireframe geometry from the edges of a 3D solid, surface, mesh, region, or subobject. | — |
| 975 | `XLINE` | 部分实现 | Creates a construction line of infinite length. | 两点创建，但落库为裁剪到模型包围盒的有限线段 |
| 976 | `XLIST` | 未实现 | Lists the type, block name, layer name, color and linetype of a nested object in a block or an xref. | — |
| 977 | `XOPEN` | 未实现 | Opens a selected drawing reference (xref) in a new window. | — |
| 978 | `XPLODE` | 未实现 | Breaks a compound object into its component objects, with specified properties for the resulting objects. | — |
| 979 | `-XREF` | 未实现 | From the Command prompt, manages drawings inserted as external references (xrefs). | — |
| 980 | `XREF` | 未实现 | Starts the EXTERNALREFERENCES command. | — |
| 981 | `ZOOM` | 部分实现 | Increases or decreases the magnification of the view in the current viewport. | 无 OBJECT/DYNAMIC/图纸空间 nXP；WINDOW/PREVIOUS 已实现 |

## 复跑

```bash
# 官方清单的抓取是本次一次性研究（见上方 URL 与日期）；仓库内不保存 981 行原始 JSON。
# 本项目词表（源码是唯一权威）：
python3 - <<'PY'
import re
src = open('crates/cad-ui-core/src/session/commands.rs').read()
def rows(pat):
    m = re.search(pat, src, re.S)
    return re.findall(r'\("([^"]+)"', m.group(1))
table = rows(r'pub const COMMAND_TABLE: &\[\(&str, CommandId\)\] = &\[(.*?)\n\];')
meas  = rows(r'pub const MEASUREMENT_SELECTORS: &\[\(&str, MeasurementToolKind\)\] = &\[(.*?)\n\];')
zoom  = rows(r'pub const ZOOM_STEP_SELECTORS: &\[\(&str, f64\)\] =\s*&\[(.*?)\];')
aliases = [a for a, _ in re.findall(
    r'\("([^"]+)",\s*"([^"]+)"\)',
    re.search(r'pub const COMMAND_ALIASES: &\[\(&str, &str\)\] = &\[(.*?)\n\];', src, re.S).group(1))]
print(len(table), len(meas), len(zoom), len(aliases))
PY
```

> 数量一变，本文件的 `总计` 与映射必须同步；**权威是源码表**，不是这里的手抄值。
