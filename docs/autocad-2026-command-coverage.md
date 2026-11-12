# AutoCAD 2026 官方命令覆盖清单（按字母序）

> **这是清单审计，不是验收。** 「已实现」只表示**命令路径存在**并有契约/单测覆盖；不表示与
> AutoCAD 行为等价，也不表示在真实窗口/真实 GPU/真实图纸上验证过。逐条能力边界以
> `docs/compatibility.md` 为权威。
>
> **本文是覆盖事实的唯一文档**：官方 981 条逐条状态 + 下一批 100 条的计划与进度 +
> 项目词表/命令边界/缺口规模审计。原 `docs/autocad-command-coverage.md` 与
> `docs/autocad-2026-next-100-plan.md` 已于 2026-11-07 合并进本文并删除；旧文档里的路径与
> 章节引用均指本文对应内容。
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
| 完整实现 | **107** |
| 部分实现（有写明的界） | **872** |
| **已实现合计（完整 + 部分）** | **979** |
| 未实现 | **2** |

> **禁止的读法**：上述百分比/计数只是本清单的逐条计数，**不是**「AutoCAD 兼容率」。
> 同名命令的选项、提示顺序、系统变量副作用基本没有逐条对照；`部分实现` 的界之外一律
> 显式 `Unsupported`/`InvalidInput`。

## 状态口径

- **已实现**：可键入的命令路径存在，行为落库/落会话状态；未支持选项显式拒绝。
- **部分实现**：命令可用，但有下表写明的界；界之外显式拒绝。
- **未实现**：键入该名得到 `command.unsupported`（或 chrome 不承认该名字）。
- **同名不同义**：`MEASURE`（AutoCAD 沿对象放点/块；本项目是测量入口）与 `TRACE`
  （AutoCAD 2026 是协作追踪；本项目同名命令是旧式宽线带）都**不计**为已覆盖，
  表中状态写成「未实现（同名不同义）」并注明。

## 下一批 100 条命令实施计划与进度（2026-11-07）

> **这是排期与范围记录，不是验收。** 每条命令落地时必须在本文与 `docs/compatibility.md`
> 写明真实边界；“已实现/部分实现”只表示命令路径存在且有契约测试，不表示与 AutoCAD 行为等价，
> 也没有 GUI/GPU/像素/真机/vendor 证据。

> **进度（2026-11-07）**：**99/100 计入官方覆盖** + **1 条同名不同义不计入**。
> 官方清单从 123 提升到 **222 已实现（50 完整 + 172 部分）/ 759 未实现**。
> 100 个名字中 `TRACE` 是 AutoCAD 2026 的协作追踪，本构建同名命令是旧式宽线带，
> 按“同名不同义”排除（与 `MEASURE` 同一口径）。

> **2026-11-08 增补**：`CLOSE` 以“关闭当前唯一文档”的部分实现计入

> **2026-11-09 增补（pilot）**：`QTEXT`/`FILL`/`TFRAMES`/`REGENAUTO`/`ISODRAFT` 五个会话令牌（无渲染器效果，如实标为部分）与 `BCOUNT`/`COUNT`/`EDITTIME`/`JULIAN`/`XLIST` 五个只读报告计入 **233 已实现（50 完整 + 183 部分）/ 748 未实现**；全部 Viewer-legal、无事务/修订/历史。

> **2026-11-09 增补（Wave 2）**：`DRAGMODE`/`APERTURE`/`ISOPLANE`/`HIGHLIGHTNEW`/`TRAYSETTINGS`/`TEXTSCR`/`GRAPHSCR`/`FINDINCLUDE`/`FINDMATCH`/`REVCLOUDPROPERTIES` 十个会话令牌（无渲染器消费，两个只读形式明说设置器未实现）、`SYSVARMONITOR`/`SYSVDLG`/`VTOPTIONS`/`ALIASEDIT`/`WHOHAS`/`ACADINFO`/`PTYPE`/`SCALELISTEDIT` 八个只读报告，以及 `HYPERLINKOPTIONS`…`MVSETUP` 十二个**具名拒绝**（`CadError::Unsupported` + 稳定原因串，不写库/不升修订/不入历史）计入 **263 已实现（50 完整 + 213 部分）/ 718 未实现**；全部 Viewer-legal。具名拒绝是「命令路径存在且如实拒绝」的部分实现，不是可用功能。

> **2026-11-09 增补（Wave 3）**：`MSPACE`/`PSPACE` 两个会话空间切换（复用现有 `MODEL`/`SPACE` 路径，无第二套空间权威；`PSPACE` 无图纸布局时 `pspace.no_layout`）、`STYLE`/`-STYLE` 文字样式只读列表、`MLEADERSTYLE` 多重引线样式只读列表、`BLOCK?` 块定义只读列表（复用 `XLIST` 的 `drawing.blocks()`），以及 `DIMREASSOC`/`DIMREASSOCIATE` 两个非关联 no-op 报告；另加 `DIMJOGGED`…`CONSTRAINTSETTINGS` 二十二个**具名拒绝**（`CadError::Unsupported` + 稳定原因串，不写库/不升修订/不入历史）计入 **293 已实现（50 完整 + 243 部分）/ 688 未实现**；全部 Viewer-legal。具名拒绝是「命令路径存在且如实拒绝」的部分实现，不是可用功能。
> **2026-11-09 增补（Wave 4）**：`CENTERMARK`/`CENTERLINE`/`NCOPY`/`COPYTOLAYER`/`-COPYTOLAYER`/`CDORDER`/`FLATTEN`/`XPLODE`/`BTRIM`/`BEXTEND`/`CHANGE`/`CHPROP` 十二个**真实选择式 2D 编辑**（复用既有创建漏斗、`plan_trim`/`plan_extend`、`plan_entity` 与 EXPLODE 路径；均为 Work-only，一条事务/一步撤销；各自写明部分边界），以及 `BSCALE`…`CHSPACE` 十八个**具名拒绝**（`CadError::Unsupported` + 稳定原因串，不写库/不升修订/不入历史）计入 **323 已实现（50 完整 + 273 部分）/ 658 未实现**。具名拒绝是「命令路径存在且如实拒绝」的部分实现，不是可用功能。
> **2026-11-09 增补（Wave 5）**：`ATTDISP` 一个会话令牌（默认开；本构建无块属性模型，不渲染属性，不写图/不升修订），`BATTMAN`/`BATTORDER` 两个块定义只读列表（复用 `drawing.blocks()`，并明说 `*.attributes_not_modeled`），`XREF`/`-XREF`/`EXTERNALREFERENCES` 三个外部参照**导入期溯源**只读报告（真实读取 `drawing.external_references()`，不虚构行；`EXTERNALREFERENCES` 另报 `palette_not_implemented`），`EXTERNALREFERENCESCLOSE` 一个具名 no-op（无选项板可关）；另加 `ATTDEF`…`CLASSICXREF` 二十三个**具名拒绝**（`CadError::Unsupported` + 稳定原因串，不写库/不升修订/不入历史）计入 **353 已实现（50 完整 + 303 部分）/ 628 未实现**；全部 Viewer-legal。具名拒绝是「命令路径存在且如实拒绝」的部分实现，不是可用功能。
> **2026-11-09 增补（Wave 6）**：`DWGLOG`/`LOGFILEON`/`LOGFILEOFF` 三个名字共用**一个会话图形日志令牌**（默认关；本构建不写日志文件、不写图/不升修订），`DWGPROPS` 一个只读文档报告（文档 id/修订/实体数；标题、作者、自定义特性**未建模**），`DWFFORMAT`/`EXPORTPDF` 两个**只读能力报告**（DWF/DWFx 不导出；PDF 由宿主经 `PLOT` 负责，二者都不是错误）；另加 `SAVEALL`…`AUTOPUBLISH` 二十四个**具名拒绝**（`CadError::Unsupported` + 稳定原因串，不写库/不升修订/不入历史）计入 **383 已实现（50 完整 + 333 部分）/ 598 未实现**；全部 Viewer-legal。具名拒绝是「命令路径存在且如实拒绝」的部分实现，不是可用功能。
> **2026-11-09 增补（Wave 7）**：`BVSTATE`/`-BVSTATE` 两个**只读动态块可见性报告**（逐选中 INSERT 读取 `DrawingDatabase::block_dynamic_visibility` 的活动状态；无 INSERT 报 `*.no_selection`，无描述符或活动状态未解析报 `*.not_set`，绝不猜测；不设置状态/不写库/不升修订）；另加 `BACTION`…`DCLINEAR` 二十八个**具名拒绝**（`CadError::Unsupported` + 稳定原因串，覆盖动态块动作/参数/查找表/构造几何/按状态隐藏显示、块编写选项板、参数重置与几何/尺寸约束；不写库/不升修订/不入历史）计入 **413 已实现（50 完整 + 363 部分）/ 568 未实现**；全部 Viewer-legal。具名拒绝是「命令路径存在且如实拒绝」的部分实现，不是可用功能。
> **2026-11-09 增补（Wave 8）**：`ELEV` 一个**只读会话标高令牌报告**（默认 0.0；`ELEV <height>` 设置形式未实现，不按标高创建对象），`UCSICON` 一个会话开关（默认关；不渲染任何 UCS 图标）；另加 `UCS`/`UCSMAN`/`TABLET`/`TASKBAR`/`REINIT`/`SYSWINDOWS`/`MENU`/`RIBBON` 八个**具名拒绝**（`CadError::Unsupported` + 稳定原因串：无 UCS 模型、无数字化仪/设备初始化模型、无菜单文件模型，窗口装饰/布局/功能区可见性由宿主负责；不写库/不升修订/不入历史）计入 **423 已实现（50 完整 + 373 部分）/ 558 未实现**；全部 Viewer-legal。具名拒绝是「命令路径存在且如实拒绝」的部分实现，不是可用功能。
> **2026-11-09 增补（Wave 9）**：`ARRAY`/`-ARRAY`/`ARRAYCLASSIC` 三个阵列路由（`ArrayRect`/`ArrayPolar` 载荷转交现有 `ARRAYRECT`/`ARRAYPOLAR`）、`BLEND` 与 `BREAKLINE` 两个真实 2D 编辑（三次样条桥接、锯齿折断线；符号/切线规则为声明式）、`-BLOCK`/`-INSERT` 两个已实现别名、`-BOUNDARY`/`-HATCH`/`-HATCHEDIT`/`EXOFFSET` 四个部分别名、`DIM` 只读路由报告、`UNITS` 只读单位报告，以及 `ARCTEXT`…`UPDATEFIELD` 十二个具名拒绝计入 **448 已实现（66 完整 + 382 部分）/ 533 未实现**；全部 Viewer-legal 或 Work-only 边界已写明，无 GUI/GPU/像素证据。
> **2026-11-09 增补（Wave 10）**：`-LAYER` 只读图层表、`-SCALELISTEDIT` 只读真实比例表、`LAYERSTATESAVE`/`NEWVIEW` 两个载荷路由、`LWEIGHT`/`-LWEIGHT`/`COLOR`/`-COLOR`/`-UNITS`/`COPYHIST` 六个只读会话/日志报告、`HELP`…`TRANSPARENCY` 十个具名拒绝，以及 `DSETTINGS`/`QUICKPROPERTIES`/`CLASSICLAYER`/`COMMANDLINE`/`COMMANDLINEHIDE` 五个 chrome 本地选择子计入 **473 已实现（66 完整 + 407 部分）/ 508 未实现**；无 GUI/GPU/像素证据。
> **2026-11-09 增补（Wave 11）**：`3DALIGN`…`3DZOOM` 二十五个 3D/网格/曲面/3D 导航/打印名全部按**具名拒绝**计入（`CadError::Unsupported` + 稳定原因串，不写库/不升修订/不入历史）；3D 优先级低于 2D，本批只提供诚实命令路径，不计为可用 3D 功能。计入 **498 已实现（66 完整 + 432 部分）/ 483 未实现**。
> **2026-11-09 增补（Wave 12）**：`3DSIN`…`SCRIPT` 二十五个文件/导入导出/扩展运行时名全部按**具名拒绝**计入（`CadError::Unsupported` + 稳定原因串；不写库/不升修订/不入历史）；唯一矢量输出仍是 `PLOT`（SVG/PDF）。计入 **523 已实现（66 完整 + 457 部分）/ 458 未实现**。
> **2026-11-09 增补（Wave 13）**：`PAGESETUP`/`-PAGESETUP` 只读页面设置报告、`CONVERTPOLY` 只读单一多段线表示报告，以及 `PSETUPIN`…`LAYTRANS` 二十二个具名拒绝（页面设置导入/打印样式/表格/字段/远程文字/拼写/超级填充/视口对齐同步/图层转换；表格已真实落地（模型 + 创建 + TABLEDIT）；TABLESTYLE/TINSERT/MATCHCELL/TABLEEXPORT 仍拒绝）计入 **548 已实现（66 完整 + 482 部分）/ 433 未实现**；无 GUI/GPU/像素证据。
> **2026-11-09 增补（Wave 14）**：`BOX`…`ROTATE3D` 二十五个 3D 实体图元/布尔/剖切/边编辑名全部按**具名拒绝**计入（稳定原因串，不写库/不升修订/不入历史）；3D 仍低于 2D，不计为可用 3D 功能。计入 **573 已实现（66 完整 + 507 部分）/ 408 未实现**。
> **2026-11-09 增补（Wave 15）**：`SURFBLEND`…`MESHUNCREASE` 二十五个曲面/网格名全部按**具名拒绝**计入（稳定原因串，不写库/不升修订/不入历史）；3D 仍低于 2D。计入 **598 已实现（66 完整 + 532 部分）/ 383 未实现**。
> **2026-11-09 增补（Wave 16）**：`IMAGE`…`DGNADJUST` 二十五个图像/PDF/DGN 底图名全部按**具名拒绝**计入（`IMAGE`/`-IMAGE`/`IMAGEATTACH`/`IMAGECLIP`/`IMAGEADJUST`/`-IMAGEADJUST` 已于 2026-11-10 晋升：前两者只读报告，其余真实写入；其余仍拒绝）（稳定原因串，不写库/不升修订/不入历史）。计入 **623 已实现（66 完整 + 557 部分）/ 358 未实现**。
> **2026-11-10 增补（Wave 17）**：24 个名字按**具名拒绝**计入 + `ARRAYPATH` 真实（`CadError::Unsupported` + 稳定原因串，不写库/不升修订/不入历史；`ACTBASEPOINT`…`ASSISTANTOPEN`）：`ACTBASEPOINT`/`ACTIVITYINSIGHTSCLOSE`/`ACTIVITYINSIGHTSOPEN`/`ACTMANAGER`/`ACTRECORD`/`-ACTSTOP`/`ACTSTOP`/`ACTUSERINPUT`/`-ACTUSERMESSAGE`/`ACTUSERMESSAGE`/`ADCCLOSE`/`ADCENTER`/`ADCNAVIGATE`/`ADJUST`/`AMECONVERT`/`ANALYSISCURVATURE`/`ANALYSISDRAFT`/`ANALYSISOPTIONS`/`ANALYSISZEBRA`/`ANIPATH`/`APPAUTOLOADER`/`APPSTORE`/`ASSISTANTCLOSE`/`ASSISTANTOPEN`（`ARRAYPATH` 在同日晋升为真实 path array，见其命令行）。计入 **648 已实现（67 完整 + 581 部分）/ 333 未实现**。具名拒绝是「命令路径存在且如实拒绝」的部分实现，不是可用功能。
> **2026-11-10 增补（Wave 18）**：25 个名字按**具名拒绝**计入（`CadError::Unsupported` + 稳定原因串，不写库/不升修订/不入历史；`ATTACHURL`…`COMPILE`）：`ATTACHURL`/`BDETECT`/`BLOCKICON`/`BLOCKSDATAOPTION`/`BLOCKSPALETTE`/`BLOCKSPALETTECLOSE`/`BLOCKTOXREF`/`BREP`/`BROWSER`/`BSEARCH`/`CAMERA`/`CENTERDISASSOCIATE`/`CENTERREASSOCIATE`/`CENTERRESET`/`CHECKSTANDARDS`/`CHURLS`/`COMMANDMACROS`/`COMMANDMACROSCLOSE`/`-COMPARE`/`COMPARE`/`COMPARECLOSE`/`COMPAREEXPORT`/`COMPAREIMPORT`/`COMPAREINFO`/`COMPILE`。计入 **673 已实现（67 完整 + 606 部分）/ 308 未实现**。具名拒绝是「命令路径存在且如实拒绝」的部分实现，不是可用功能。
> **2026-11-10 增补（Wave 19）**：25 个名字按**具名拒绝**计入（`CadError::Unsupported` + 稳定原因串，不写库/不升修订/不入历史；`CONVERTOLDLIGHTS`…`CUSTOMIZE`）：`CONVERTOLDLIGHTS`/`CONVERTOLDMATERIALS`/`CONVTOMESH`/`CONVTONURBS`/`CONVTOSOLID`/`CONVTOSURFACE`/`COORDINATIONMODELATTACH`/`COPYFROMTRACE`/`COPYLINK`/`COPYM`/`COUNTAREA`/`COUNTAREACLOSE`/`COUNTCLOSE`/`COUNTFIELD`/`COUNTLIST`/`COUNTLISTCLOSE`/`COUNTNAVNEXT`/`COUNTNAVPREV`/`COUNTTABLE`/`CUI`/`CUIEXPORT`/`CUIIMPORT`/`CUILOAD`/`CUIUNLOAD`/`CUSTOMIZE`。计入 **698 已实现（67 完整 + 631 部分）/ 283 未实现**。具名拒绝是「命令路径存在且如实拒绝」的部分实现，不是可用功能。
> **2026-11-10 增补（Wave 20）**：25 个名字按**具名拒绝**计入（`CadError::Unsupported` + 稳定原因串，不写库/不升修订/不入历史；`CVADD`…`-DGNEXPORT`）：`CVADD`/`CVHIDE`/`CVREBUILD`/`CVREMOVE`/`CVSHOW`/`-DATAEXTRACTION`/`DATAEXTRACTION`/`DATALINK`/`DATALINKUPDATE`/`DBCCLOSE`/`DBCCONFIGURE`/`DBCDEFINELLT`/`DBCDEFINELT`/`DBCONNECT`/`DCRADIUS`/`DCVERTICAL`/`DELAY`/`DETACHURL`/`DETECTCLOSE`/`DETECTCONVERT`/`DETECTPRIMARY`/`DETECTREMOVE`/`DETECTREVIEWNEXT`/`DETECTREVIEWPREV`/`-DGNEXPORT`。计入 **723 已实现（67 完整 + 656 部分）/ 258 未实现**。具名拒绝是「命令路径存在且如实拒绝」的部分实现，不是可用功能。
> **2026-11-10 增补（Wave 21）**：25 个名字按**具名拒绝**计入（`CadError::Unsupported` + 稳定原因串，不写库/不升修订/不入历史；`DGNEXPORT`…`FREESPOT`）：`DGNEXPORT`/`-DGNIMPORT`/`DGNIMPORT`/`DGNMAPPING`/`DIGITALSIGN`/`DISTANTLIGHT`/`DOWNLOADMANAGER`/`DRAWINGRECOVERY`/`DRAWINGRECOVERYHIDE`/`DVIEW`/`DWFADJUST`/`-DWFATTACH`/`DWFATTACH`/`DWFCLIP`/`DWFLAYERS`/`EDGESURF`/`EDITSHOT`/`EXPLAN`/`EXPORTDWF`/`EXPORTDWFX`/`EXPRESSMENU`/`EXPRESSTOOLS`/`FADEMARKUP`/`FLATSHOT`/`FREESPOT`。计入 **748 已实现（67 完整 + 681 部分）/ 233 未实现**。具名拒绝是「命令路径存在且如实拒绝」的部分实现，不是可用功能。
> **2026-11-10 增补（Wave 22）**：25 个名字按**具名拒绝**计入（`CadError::Unsupported` + 稳定原因串，不写库/不升修订/不入历史；`FREEWEB`…`-GRAPHICSCONFIG`）：`FREEWEB`/`GCCOINCIDENT`/`GCCOLLINEAR`/`GCCONCENTRIC`/`GCEQUAL`/`GCFIX`/`GCHORIZONTAL`/`GCPARALLEL`/`GCPERPENDICULAR`/`GCSMOOTH`/`GCSYMMETRIC`/`GCTANGENT`/`GCVERTICAL`/`GEOGRAPHICLOCATION`/`GEOMAP`/`GEOMAPIMAGE`/`GEOMAPIMAGEUPDATE`/`GEOMARKLATLONG`/`GEOMARKME`/`GEOMARKPOINT`/`GEOMARKPOSITION`/`GEOREMOVE`/`GEOREORIENTMARKER`/`GOTOURL`/`-GRAPHICSCONFIG`。计入 **773 已实现（67 完整 + 706 部分）/ 208 未实现**。具名拒绝是「命令路径存在且如实拒绝」的部分实现，不是可用功能。
> **2026-11-10 增补（Wave 23）**：25 个名字按**具名拒绝**计入（`CadError::Unsupported` + 稳定原因串，不写库/不升修订/不入历史；`GRAPHICSCONFIG`…`MATBROWSEROPEN`）：`GRAPHICSCONFIG`/`HIDE`/`HLSETTINGS`/`-HYPERLINK`/`HYPERLINK`/`-INSERTCONTENT`/`INSERTOBJ`/`-INTERFERE`/`INTERFERE`/`LIGHT`/`LIGHTLIST`/`LIGHTLISTCLOSE`/`LIVESECTION`/`LOAD`/`LSP`/`LSPSURF`/`MAKELISPAPP`/`MARKUP`/`-MARKUPASSIST`/`MARKUPASSIST`/`MARKUPCLOSE`/`-MARKUPIMPORT`/`MARKUPIMPORT`/`MATBROWSERCLOSE`/`MATBROWSEROPEN`。计入 **798 已实现（67 完整 + 731 部分）/ 183 未实现**。具名拒绝是「命令路径存在且如实拒绝」的部分实现，不是可用功能。
> **2026-11-10 增补（Wave 24）**：25 个名字按**具名拒绝**计入（`CadError::Unsupported` + 稳定原因串，不写库/不升修订/不入历史；`MATEDITORCLOSE`…`OPENDWFMARKUP`）：`MATEDITORCLOSE`/`MATEDITOROPEN`/`MATERIALASSIGN`/`MATERIALATTACH`/`MATERIALMAP`/`MATERIALS`/`MATERIALSCLOSE`/`MIGRATEMATERIALS`/`MKLTYPE`/`MKSHAPE`/`MOVEBAK`/`MSLIDE`/`NAVBAR`/`NAVSMOTION`/`NAVSMOTIONCLOSE`/`NAVSWHEEL`/`NAVVCUBE`/`NEWSHEETSET`/`NEWSHOT`/`OLECONVERT`/`OLELINKS`/`OLEOPEN`/`OLERESET`/`OLESCALE`/`OPENDWFMARKUP`。计入 **823 已实现（67 完整 + 756 部分）/ 158 未实现**。具名拒绝是「命令路径存在且如实拒绝」的部分实现，不是可用功能。
>
> **第二批 100 条（Wave 13–16）已交付**：页面设置/表格/字段/转换报告 + 3D 实体/曲面/网格/底图的诚实拒绝；全部结论仍仅 Linux 编译 + 契约测试，GUI/GPU/像素/真机 NOT VERIFIED。
>
> **本轮 100 条（Wave 9–12）已交付**：2D 真实编辑/别名/路由/报告 + 3D/文件扩展的诚实拒绝；全部结论仅 Linux 编译 + 契约测试，GUI/GPU/像素/真机 NOT VERIFIED。
> **2026-11-09 增补（P1 晋升批）**：按 AutoCAD 2026 ENU 帮助页逐条核对后，`ALIGN`、`BREAK`、`DIVIDE`、`SELECTSIMILAR`、`MATCHPROP`、`SETBYLAYER`、`LIST`、`DIMSPACE`、`TORIENT` 九条从**部分实现**晋升为**完整实现**：补齐 1/3 对点 ALIGN（3 对点为平面刚性、不缩放）、椭圆 BREAK、全曲线 DIVIDE + 块对齐、SELECTSIMILAR 默认「图层+类型」与 `Name`/`Object style`、MATCHPROP 的 Text/Dimension/Hatch 特殊组、SETBYLAYER 透明度与 Include blocks、LIST 的纸空间/块空间与样式名、DIMSPACE 的 Auto=2×文字高度/`0` 对齐/角度标注、TORIENT Reverse（本项目扩展）。`LENGTHEN` 补了近端语义、开放多段线/椭圆弧/开放样条与 DY，但**开放样条延长**仍是具名拒绝，故保留**部分实现**。三处实现偏差已按官方语义修正：ALIGN 三对点禁缩放、SELECTSIMILAR 默认集、DIMSPACE Auto 公式。计入 **423 已实现（59 完整 + 364 部分）/ 558 未实现**；全部结论仍只覆盖 Linux 编译与契约测试，GUI/GPU/像素 NOT VERIFIED。
>
> **2026-11-11 增补（Wave 25）**：`-LAYER` 从只读报告晋升为**完整实现**（一行一个选项的写形式：`NEW/N`、`MAKE/M`、`SET/S`、`RENAME/R`、`DELETE/D`、`ON`/`OFF`、`FREEZE/F`/`THAW/T`、`LOCK/LO`/`UNLOCK/U`、`COLOR/C`、`LTYPE/L`、`LWEIGHT/LW`、`PLOT|P`；名单逗号分隔，含空格的值/名称可用双引号；整行原子、一条事务/一步撤销；写形式经 Work-only `SetLayerProperty` 载体，Viewer 拒绝，裸命令/`?` 仍 Viewer-legal）；`ZOOM` 补齐 `OBJECT`/`nX`/图纸空间 `nXP`（View → navigate 增加 `ZOOM OBJECT` 按钮；`nX`/`nXP` 仅命令行），仅 `DYNAMIC` 仍具名未实现；DWG 保存选择器在写入前提示实验性/新文件/不覆盖源图（`save.picker_waiting_dwg`）。计入 **823 已实现（68 完整 + 755 部分）/ 158 未实现**；全部结论仅 Linux 编译 + 契约测试，GUI/GPU/像素/真机 NOT VERIFIED。

> **2026-11-12 增补（Wave 26，目标 100 条）**：本轮已推进 **100 个命令名**（其中 12 个是之前未实现的 chrome/QuickView/QuickCalc 名字，`SHAPE`/`SHP2BLK`、`REDEFINE`/`UNDEFINE` 也由未实现转为已实现；其余为部分实现补强或已实现命令的行为扩展）。真实 2D：编辑/绘制 `OFFSET`（闭合环 gap）、`FILLET`（椭圆对）、`MPEDIT`（LINE/ARC→POLYLINE）、`MOCORO`（预校验）、`BREAK`（椭圆弧）、`DIVIDE`（椭圆弧等弧长）、`JOIN`（SPLINE 源混合焊接）、`PEDIT`（顶点子菜单）、`PLINE`（bulge+Close）、`ELLIPSE`（弧+旋转）、`SPLINE`（CV/degree）、`POINT`（Multiple payload）、`TRIM`/`EXTEND`（椭圆/椭圆弧）、`ARRAYRECT`（有符号间距）、`ARRAYPOLAR`（AutoCAD fill 规则）、`HATCH`（`.pat` 解析）、`HATCHEDIT`（CUSTOM/边界）；标注/文字 `TXTEXP`、`ARCTEXT`、`QLEADER`/`LEADER`/`MLEADER`（引线选项）、`TEXTMASK`/`TEXTUNMASK`、`TEXTALIGN`、`TEXTFIT`、`TXT2MTXT`；布局/视口 `VPLAYER`、`MVSETUP`、`PSBSCALE`、`PSTSCALE`、`VPMAX`（真实相机快照/恢复）、`ALIGNSPACE`、`VPSYNC`、`VPCLIP`/`MVIEW`（多边形写入）、`LAYVPI`；查询 `MASSPROP`（闭合样条/填充环）、`MEASUREGEOM`（RADIUS 弧/圆）、`STATUS`/`TIME`；命令/块/图像 `SHAPE`、`SHP2BLK`、`MINSERT`、`ADDSELECTED`、`IMAGECLIP`（多边形）；会话/UI `APERTURE`（真实拾取孔径）、`PTYPE`、`REVCLOUDPROPERTIES`，以及 `RIBBON`/`RIBBONCLOSE`/`HIDEPALETTES`/`SHOWPALETTES`/`FILETAB`/`FILETABCLOSE`/`QVDRAWING(+CLOSE)`/`QVLAYOUT(+CLOSE)`/`QUICKCALC`/`QCCLOSE`/`UPDATETHUMBSNOW`/`WORKSPACE`/`WSSAVE`/`WSSETTINGS`/`TOOLBAR`/`-TOOLBAR`（chrome 本地选择子）。
>
> **不得读成完整**：多边形视口/图像 clip 数据真实但当前表示层只接受 3/4 点 clip（`*.polygon_not_renderable`，按 partial 计）；`MVSETUP` 是简单等分网格而非 AutoCAD Standard 配置；`PSBSCALE`/`PSTSCALE` 为单方向/单视口声明规则；`TRIM` 的 POLYLINE/INSERT/HATCH 目标与 `EXTEND` 的 SPLINE 目标仍具名拒绝；`HATCH` 自定义图案无命令行选择通道；`TEXTALIGN` 分布/斜排、`TEXTFIT` 真实宽度拉伸、`TXT2MTXT` UNIFORM 行距、`PEDIT`/`PLINE` 宽度仍受域限制；`SHAPE` 按 name 只建行不解析；`MINSERT`/`ARRAYPOLAR`/`ARRAYRECT` 非关联。证据仅 Linux 编译 + 契约测试，GUI/GPU/像素/真机 NOT VERIFIED；Windows/macOS/Android/Web NOT VERIFIED。计入 **840 已实现（105 完整 + 735 部分）/ 141 未实现**。
>
> **840 已实现（105 完整 + 735 部分）/ 141 未实现**；无未保存修改提示，无多文档。

> **2026-11-12 增补（Wave 27，目标 100 条）**：本轮推进 **100 个命令名**：4 个真实只读报告（`RECOVER`/`RECOVERALL` 逐实体 `validate_entity` 完整性扫描并明示磁盘 DWG 修复未建模；`PURGEAECDATA` 明示本构建无 AEC 模型；`SHOWURLS` 明示不保留超链接数据）、5 个 chrome 本地选择子（`TOOLPALETTES`/`TOOLPALETTESCLOSE`/`TPNAVIGATE` 仅 Blocks 页的工具选项板浮窗、`PERFANALYZER`/`PERFANALYZERCLOSE` 只读宿主性能面板），以及 91 个具名拒绝（3D 实体/曲面/剖切/点云、render/灯光/视觉样式/视图基、云共享/图纸集/外部参照比较/部分打开/`XCLIP`/`PASTESPEC`；`CadError::Unsupported` + 稳定原因串）。全部新名字 Viewer-legal，无事务/修订/历史；报告与面板只读真实数据，缺失指标显示 N/A，不伪造 0。
>
> **不得读成完整**：`RECOVER` 不做磁盘 DWG 修复与重新打开（`recover.file_level_not_modeled`）；`RECOVERALL` 只扫描单文档（`recoverall.single_document`）；`PURGEAECDATA`/`SHOWURLS` 只报缺模型、不清理/不列 URL；`TOOLPALETTES` 只有 Blocks 页，`TPNAVIGATE` 只接受 `Blocks`；`PERFANALYZER` 无命令录制/采样，面板指标全来自宿主 `cad_status.performance`。计入 **940 已实现（107 完整 + 833 部分）/ 41 未实现**；证据仅 Linux 编译 + Rust 契约测试 + React 单测，GUI/GPU/像素/真机 NOT VERIFIED。

> **2026-11-12 增补（Wave 28，官方名收尾）**：最后 **39 个**未实现名按**具名拒绝**计入
> （`CadError::Unsupported` + 稳定原因串）：Parameters Manager（`PARAMETERS`/`-PARAMETERS`/`PARAMETERSCLOSE`）、
> in-place 参照编辑（`REFEDIT`/`-REFEDIT`/`REFSET`/`REFCLOSE`）、协作追踪（`TRACEBACK`/`TRACEEDIT`/`TRACEFRONT`/
> `TRACEPALETTEOPEN`/`TRACEPALETTECLOSE`/`TRACEVIEW`）、脚本引擎（`RESUME`/`RSCRIPT`/`SCRIPTCALL`）、
> VBA/LISP/slide（`VBAMAN`/`VBAPREF`/`VBASTMT`/`VLISP`/`VSLIDE`/`VSSAVE`）、云文件交换
> （`OPENFROMWEBMOBILE`/`SAVETOWEBMOBILE`）、`PSOUT`/`SAVEIMG`/`WMFOPTS`/`ULAYERS`/`REDIR`/`REDIRMODE`/
> `REPURLS`/`PASTEASHYPERLINK`/`STANDARDS`/`QUICKCUI`/`RTUCS`/`VIEWPLAY`/`WALKFLYSETTINGS`/`WEBLOAD`/
> `-PARTIALOAD`。全部 Viewer-legal：不写库、不升修订、不入历史。
>
> **官方 981 条已无「未实现」的可用命令名**：仅 `MEASURE` 与 `TRACE` 两条按**同名不同义**不计入
> （本仓库同名命令分别表示测量入口与旧式宽线带），它们的官方语义在本构建仍无对应实现。计入
> **979 已实现（107 完整 + 872 部分）/ 2 未实现（同名不同义）**；证据仅 Linux 编译 + Rust 契约测试，
> GUI/GPU/像素/真机 NOT VERIFIED。

> **2026-11-12 增补（Wave 29，真实 2D 升级，无新命令名）**：`TRIM` 新增 **LWPOLYLINE 目标**（开放/闭合；bulge 还原为精确 ARC 后求交，保留拾取投影所在窗口，子弧 bulge 精确保留；一条事务/一步撤销；EXTEND 的 POLYLINE 目标仍按名拒绝）。该升级不改变 981 条状态总计，仍计入 **979 已实现（107 完整 + 872 部分）/ 2 未实现（同名不同义）**；证据仅 Linux 契约测试，GUI/GPU/像素 NOT VERIFIED。

> **2026-11-12 增补（Wave 29b）**：`HATCHEDIT` 新增 **`PATTERN <内置图案名>`**（按名称从现有 `.pat` 库取图案，保留 hatch 的角度/比例/double；未知名 `hatchedit.unknown_pattern`、缺名 `hatchedit.needs_pattern_name`、实心/渐变 `hatchedit.pattern_requires_pattern` 均具名拒绝；一条事务/一步撤销）。任意用户 `.pat` 图案文件仍需宿主文件 I/O，未接入。不影响 981 条状态总计。

### 纪律

- 2D 优先；3D/实体/约束/表格/外部参照/点云/渲染/发布类命令不在本 100 条内。
- 每条命令：Rust 命令路径 + `COMMAND_TABLE` + dispatch + 能力门 + React 目录/gating/i18n
  + 契约测试；未支持选项一律具名拒绝。
- 每 wave 独立 worktree/分支，隔离 `CARGO_TARGET_DIR`；合并后由 orchestrator 统一更新本文、
  `docs/compatibility.md`、`docs/handoff.md`。
- 共享单表（`CommandId`、`COMMAND_TABLE`、dispatch、UI catalog、i18n）冲突由 orchestrator
  按显式路径合并；合并后重跑 cad-app/cad-ui-core/前端全量/门禁。
- 本轮所有结论仅 Linux 本机编译/契约；Windows/macOS/Android/Web 运行时 NOT VERIFIED。

### 已交付（按批次，100 个名字）

| 批次 | 数量 | 名字 |
|---|---:|---|
| 文字 | 10 | `JUSTIFYTEXT`/`TJUST`、`TSCALE`、`TEXTALIGN`、`TCOUNT`、`TCIRCLE`、`TEXTMASK`、`TEXTUNMASK`、`MTEDIT`、`-MTEXT` |
| 图层 | 10 | `LAYMCH`/`-LAYMCH`、`LAYMRG`/`-LAYMRG`、`LAYDEL`/`-LAYDEL`、`LAYERSTATE`、`LAYERPMODE`、`LTSCALE`、`LAYWALK` |
| 选择 | 10 | `QSELECT`、`FILTER`、`GETSEL`、`SSX`、`SELECTCOUNT`、`-FIND`、`FIND`、`HIDEOBJECTS`、`ISOLATEOBJECTS`、`UNISOLATEOBJECTS` |
| 视图 | 10 | `REDRAW`、`REDRAWALL`、`REGEN`、`REGENALL`、`VIEW`、`-VIEW`、`VIEWGO`、`VIEWBACK`、`VIEWFORWARD`、`VPOINT` |
| 实用工具 | 10 | `PURGE`/`-PURGE`、`RENAME`/`-RENAME`、`SETVAR`、`TIME`、`DBLIST`、`TREESTAT`、`MREDO`、`MULTIPLE` |
| 绘制/修改 | 10 | `DONUT`、`SOLID`、`HELIX`、`MLINE`、`MLSTYLE`、`TREX`、`EXTRIM`、`MPEDIT`、`-OVERKILL`、`DRAWORDER` |
| 布局/视口 | 10 | `LAYOUT`、`LAYOUTMERGE`、`LAYOUTWIZARD`、`VPORTS`、`-VPORTS`、`VPMAX`、`VPMIN`、`VPCLIP`、`VPSCALE`、`LAYVPI` |
| 标注 | 10 | `DIMSTYLE`、`-DIMSTYLE`、`DIMTEDIT`、`DIMEDIT`、`DIMOVERRIDE`、`DIMREGEN`、`DIMDISASSOCIATE`、`QDIM`、`DIMJOGLINE`、`DIMBREAK` |
| 组/注释 | 10 | `GROUP`、`-GROUP`、`UNGROUP`、`GROUPEDIT`、`QLEADER`、`LEADER`、`TEXTFIT`、`HATCHSETBOUNDARY`、`HATCHSETORIGIN`、`XDLIST` |
| 绘制/视图补充 | 10（计 9） | `SKETCH`、`MOCORO`、`MSTRETCH`、`-PAN`、`VIEWRES`、`VSCURRENT`、`REGEN3`、`LINETYPE`、`-LINETYPE`；`TRACE` 同名不同义，不计入 |

### 计划中被替换/未交付的名字

初版 wave 中的 `DIMARC`、`DIMJOGGED`、`DIMREASSOCIATE`、`WHOHAS`、`-BLOCK`、`BMAKE`、
`WBLOCK`、`-WBLOCK`、`-INSERT`、`MINSERT`、`ATTDEF`、`ATTEDIT`、`-ATTEDIT`、`ATTSYNC`、
`-HATCH`、`-HATCHEDIT`、`TOLERANCE`、`FIELD`、`RTEXT`、`RTEDIT`、`DIMINSPECT`、
`-DIMINSPECT`、`DIMEX`、`DIMIM`、`SPACETRANS`、`MVIEW`、`MVSETUP`、`SAVEALL`、`QNEW`、
`REVERT`、`SCRIPT`、`RESUME`、`RSCRIPT`、`DELAY`、`LOGFILEON`、`LOGFILEOFF`、`TXTEXP`
**未实施**；由同域的已交付名字替换。**未实现不等于永远不做**，它们仍在官方清单里保持
「未实现」，缺口规模见 §5。

> **2026-11-09 更正（Wave 2）**：上段列出的 `WHOHAS`、`MVIEW`、`MVSETUP` 已在 Wave 2 以
> **部分实现**计入：`WHOHAS` 是单文档只读报告，`MVIEW`/`MVSETUP` 是具名拒绝（`Unsupported` +
> 稳定原因串）。「部分实现」只表示命令路径存在并如实回答，不等于功能可用。

## 部分实现 → 完整实现 晋升计划（50 条，2026-11-09 起）

> **这不是验收，是排期与口径。** 目标是把 50 条已经存在的 `部分实现` 命令在**支持域内**补到
> `完整实现`。晋升后该命令行不再写任何「界」；做不到的保留 `部分实现` 并缩小界，绝不把缺口
> 改写成空成功。

**晋升口径**

- `完整实现` = 该命令在其支持域内没有剩余的可写明缺口（覆盖表与 `docs/compatibility.md` 都不再
  列界）。
- **全局范围声明**（不再逐条计入某命令的界）：3D/实体/曲面/B-rep、UCS、无人值守对话框/逐选项
  提示、多文档、OS 级剪贴板、真实打印样式/绘图仪、协作/云端、**块属性模型**（ATTDEF/ATTRIB）、
  **关联性/反应器**（标注与填充的双向关联）、**厚度/拉伸方向**、**材质与命名打印样式**。这些是
  构建级范围，不是单条命令的晋升目标。
- **未建模能力（会阻止相关命令晋升，不得当作豁免）**：多段线宽度/`plinegen`/顶点子菜单、
  文字字形镜像（`MIRRTEXT=1`）、分类/自定义对象特性、MULTILEADER 导入未覆盖的内容（背景/分栏/
  自定义箭头/打断/非默认平面）、实体标志位/挤出矢量、标注文字的字形级对齐/排版、`3DFACE` /
  `2D SOLID` 实体、文字行距/段落间距、多段线样式（REVCLOUD Calligraphy）。落在这些能力的命令
  **保留 `部分实现`**，并在界里写明具体缺的能力。
- 命令自身的选项顺序、提示文案与系统变量副作用仍按 AutoCAD 2026 帮助页核对；**已核对过**
  `ALIGN`/`LENGTHEN`/`BREAK`/`DIVIDE`/`SELECTSIMILAR`/`MATCHPROP`/`SETBYLAYER`/`LIST`/
  `DIMSPACE`/`TORIENT` 的 2026 ENU 页面，并据此修正了三处实现偏差（ALIGN 三对点禁缩放、
  SELECTSIMILAR 默认仅图层+类型、DIMSPACE Auto = 2×标注文字高度）。
- 支持域内做不到的能力必须保留**具名拒绝**，该命令继续算 `部分实现`。
- 晋升不新增命令名，因此不改 `CommandId`/`COMMAND_TABLE`/UI catalog/i18n；只改实现、契约测试、
  本条文档与 `docs/compatibility.md`。

**分批与退出条件**

| 批次 | 命令 | 退出条件（支持域内） |
| --- | --- | --- |
| P1 | `ALIGN` `LENGTHEN` `BREAK` `DIVIDE` `SELECTSIMILAR` `MATCHPROP` `SETBYLAYER` `LIST` `DIMSPACE` `TORIENT` | 1/3 对点对齐、四种 LENGTHEN 模式、`F` 首点与多段线打断、椭圆/样条/含 bulge 折线等分与块等分、按属性相似选择、MATCHPROP 设置子集、SETBYLAYER 透明度与块内、LIST 全部已建模属性、DIMSPACE 间距值、TORIENT 全模式 |
| P2 | `CHAMFER` `FILLET` `PEDIT` `JOIN` `OFFSET` `STRETCH` `MIRROR` `EXPLODE` `QSELECT` `FILTER` | Angle/Trim/Multiple/Undo 与多段线、PEDIT Join/Width/Fit/Spline/Decurve/Edit vertex、折线/样条连接、椭圆偏移、文本整体拉伸与多边形窗口、全部实体镜像、标注/多重引线/非均匀块分解、QSELECT/FILTER 全属性与 Append/Exclude |
| P3 | `OVERKILL` `AUDIT` `QDIM` `DIMTEDIT` `REVCLOUD` `WIPEOUT` `TEXTMASK` `TXT2MTXT` `HATCHEDIT` `SSX` | 共线/部分重叠去重、AUDIT 可修复项、QDIM 圆/弧、DIMTEDIT 全选项、矩形/多边形修订云线、WIPEOUT/TEXTMASK 边框模型、TXT2MTXT 选项、HATCHEDIT 图案/颜色、SSX 全属性 |
| P4 | `LAYDEL` `-LAYDEL` `LAYMRG` `-LAYMRG` `LAYFRZ` `LAYLCK` `LAYULK` `LAYTHW` `RENAME` `-RENAME` | 图层表写入纳入 `DrawingUndoRecord`（一条事务/一步撤销），并补齐图纸空间/块内实体与 Settings 选项；基础改动是 `cad-history` 的 `LayerPatch` + 应用器 |
| P5 | `CLIP` `CLIPIT` `VPORTS` `VPSCALE` `VPCLIP` `LAYVPI` `LAYOUTMERGE` `LAYOUTWIZARD` `MVIEW` `MVSETUP` | `cad-db` 新增 image clip 写路径与 `Layout`/`PaperViewport` 变更 API；`LAYVPI` 另需 per-viewport 图层状态模型 |

**进度**：

- P1 **已完成**（2026-11-09）：9 条晋升（`ALIGN` `BREAK` `DIVIDE` `SELECTSIMILAR` `MATCHPROP`
  `SETBYLAYER` `LIST` `DIMSPACE` `TORIENT`），`LENGTHEN` 因开放样条延长未实现保留 `部分实现`。
- P2 **已完成**（2026-11-09）：按严格口径 2 条晋升（`CHAMFER` `OFFSET`）；其余保留 `部分实现`：
  `FILLET`（椭圆/样条曲线对）、`JOIN`（`JOIN CLOSE` 混合 SPLINE）、`STRETCH`（多边形/lasso 捕获未接线）、
  `PEDIT`（多段线宽度/`plinegen`/顶点子菜单）、`MIRROR`（`MIRRTEXT=1` 字形镜像）、
  `EXPLODE`（MULTILEADER 未导入内容）、`QSELECT`/`FILTER`（分类/自定义特性）。
- P3 **已完成**（2026-11-09）：按严格口径 3 条晋升（`OVERKILL` `AUDIT` `QDIM`）；其余保留
  `部分实现`：`HATCHEDIT`（`Add/Remove Boundaries`）、`WIPEOUT`（`FRAME` 覆盖）、
  `SSX`（实体标志位/厚度/挤出）、`DIMTEDIT`（字形级对齐）、`REVCLOUD`（`Calligraphy`）、
  `TEXTMASK`（`3dface`/`Solid`）、`TXT2MTXT`（强制行距）。
- P4 **基础完成**（2026-11-10）：`cad-history` 的 `LayerPatch` + 应用器已落地；`LAYDEL`/`-LAYDEL`、`LAYMRG`/`-LAYMRG`、`LAYFRZ`、`LAYLCK`、`LAYULK`、`LAYTHW`、`RENAME`/`-RENAME`、`LAYERSTATE RESTORE` 的图层表写入均纳入 `DrawingUndoRecord`（一条事务/一步撤销）。仍保留图纸空间/块内实体与 Settings 选项边界，故仍为**部分实现**，没有把边界算作完成。
- P5 **部分完成**（2026-11-10）：`cad-db` 布局/视口写 API + `LayoutPatch` 已落地；`LAYOUT` 的 `NEW/RENAME/DELETE`、`LAYOUTMERGE`（升为**已实现**）、`LAYOUTWIZARD` 的布局创建、`VPORTS`/`-VPORTS`、`VPCLIP RECT`、`VPSCALE`、`PAGESETUP`/`-PAGESETUP`、`PSETUPIN`/`-PSETUPIN`、`MVIEW`、`LAYOUTWIZARD` 纸张已真实写入并一条事务/一步撤销。仍保留 `MVSETUP`、跨文件页面设置、视口选择与多边形裁剪边界；`VPMAX`/`VPMIN` 仍是会话标记。每批落地后更新本表、总计与 `docs/compatibility.md`，并单独提交。

## 已实现 / 部分实现命令索引（按字母序）

> 本索引覆盖官方清单里所有当前为「已实现」或「部分实现」的名字（含 `-` 别名）；
> 状态与说明与下方全量清单逐条一致，边界细节以 `docs/compatibility.md` 为权威。
> 另列一个**项目自己的多词选择子** `POLYGON CIRCUMSCRIBED`（不属于官方 981 条，不计入总计）。

| Command | 状态 | 说明 |
| --- | --- | --- |
| `-3DOSNAP` | 部分实现 | 具名拒绝（`-3dosnap.unsupported: no 3D object-snap model`）。 |
| `-ACTSTOP` | 部分实现 | 具名拒绝（`-actstop.unsupported: action-recorder macros are not modeled (no recorder state, file, or playback)`）。 |
| `-ACTUSERMESSAGE` | 部分实现 | 具名拒绝（`-actusermessage.unsupported: action-recorder macros are not modeled (no recorder state, file, or playback)`）。 |
| `-ARCHIVE` | 部分实现 | 具名拒绝（`-archive.unsupported`）：本构建无图纸集归档打包模型；命令行拼写。 |
| `-ARRAY` | 部分实现 | `ARRAY` 的命令行拼写，同一路由与边界。 |
| `-ATTACH` | 部分实现 | 具名拒绝（`-attach.unsupported`）：不附着外部参照或底图；命令行拼写。 |
| `-ATTDEF` | 部分实现 | 具名拒绝（`-attdef.unsupported`）：本构建无块属性模型；命令行拼写。 |
| `-ATTEDIT` | 部分实现 | 具名拒绝（`-attedit.unsupported`）：本构建无块属性模型；命令行拼写。 |
| `-ATTEXT` | 部分实现 | 具名拒绝（`-attext.unsupported`）：本构建无块属性模型；命令行拼写。 |
| `-BCPARAMETER` | 部分实现 | 具名拒绝（`-bcparameter.unsupported`）：本构建无动态块参数模型；命令行拼写。 |
| `-BEDIT` | 部分实现 | 具名拒绝（`-bedit.unsupported`）：本构建无块编辑器；命令行拼写。 |
| `-BLOCK` | 已实现 | `BLOCK` 的命令行拼写；同一实现/写入路径。 |
| `-BOUNDARY` | 部分实现 | `BOUNDARY` 的命令行拼写；同一实现与边界。 |
| `-BREPLACE` | 部分实现 | 具名拒绝（`-breplace.unsupported`）：BREPLACE 的命令行拼写。 |
| `-BVSTATE` | 部分实现 | 只读：每个选中 INSERT 的活动动态块可见性状态（`-bvstate.report`）；无 INSERT 报 `-bvstate.no_selection`，无状态报 `-bvstate.not_set`；不设置状态。 |
| `-COLOR` | 部分实现 | 同一当前颜色报告的命令行拼写。 |
| `-COMPARE` | 部分实现 | 具名拒绝（`-compare.unsupported: this build has no drawing-compare/diff model`）。 |
| `-COPYTOLAYER` | 部分实现 | 真实编辑：同 COPYTOLAYER，命令行拼写；一条事务/一步撤销。 |
| `-DATAEXTRACTION` | 部分实现 | 具名拒绝（`-dataextraction.unsupported: this build has no data-extraction/attribute-template model`）。 |
| `-DGNATTACH` | 部分实现 | 具名拒绝（`-dgnattach.unsupported: DGN underlays are not implemented`）。 |
| `-DGNEXPORT` | 部分实现 | 具名拒绝（`-dgnexport.unsupported: this build has no DGN exporter`）。 |
| `-DGNIMPORT` | 部分实现 | 具名拒绝（`-dgnimport.unsupported: this build has no DGN importer/exporter or mapping model`）。 |
| `-DIMINSPECT` | 部分实现 | 具名拒绝（`-diminspect.unsupported`）：同一具名拒绝的命令行拼写。 |
| `-DIMSTYLE` | 部分实现 | `DIMSTYLE` 的别名，同一实现 |
| `-DWFATTACH` | 部分实现 | 具名拒绝（`-dwfattach.unsupported: this build has no DWF/DWFx underlay or export model`）。 |
| `-EATTEXT` | 部分实现 | 具名拒绝（`-eatttext.unsupported`）：本构建无块属性模型；命令行拼写。 |
| `-ETRANSMIT` | 部分实现 | 具名拒绝（`-etransmit.unsupported`）：本构建无 eTransmit/zip 打包模型；命令行拼写。 |
| `-EXPORT` | 部分实现 | 具名拒绝（`-export.unsupported`）：请用 PLOT/EXPORT；本构建只有一条矢量导出路径；命令行拼写。 |
| `-FIND` | 部分实现 | `-FIND <needle>`；在模型空间 Text/MText/MULTILEADER 中做 ASCII 不区分大小写子串搜索并报告；不写库 |
| `-GRAPHICSCONFIG` | 部分实现 | 具名拒绝（`-graphicsconfig.unsupported: renderer/graphics configuration is owned by the host`）。 |
| `-GROUP` | 部分实现 | `GROUP` 的别名（同一 `CommandId`/会话组表路径）；无 DWG group 对象/修订/历史 |
| `-HATCH` | 部分实现 | `HATCH` 的命令行拼写；同一实现与边界。 |
| `-HATCHEDIT` | 部分实现 | `HATCHEDIT` 的命令行拼写；同一实现与边界。 |
| `-HYPERLINK` | 部分实现 | 具名拒绝（`-hyperlink.unsupported: this build has no hyperlink storage model`）。 |
| `-IMAGE` | 部分实现 | 只读报告所有 IMAGE 实体（id/空间/file key/pixels/visible/clip/origin，`-image.*` 诊断码）；Viewer-legal，不写库。 |
| `-IMAGEADJUST` | 部分实现 | 真实：选中一个 IMAGE，`IMAGEADJUST <brightness> <contrast> <fade> [MONO|COLOR]` 或 `RESET`（各 0..=100，50/50/0 中性）；一条事务/一步撤销；渲染顺序 mono→contrast→brightness→fade-to-background（声明近似）。 |
| `-INPUTSEARCHOPTIONS` | 部分实现 | 具名拒绝（`-inputsearchoptions.unsupported`）：同一具名拒绝的命令行拼写。 |
| `-INSERT` | 已实现 | `INSERT` 的命令行拼写；同一实现/写入路径。 |
| `-INSERTCONTENT` | 部分实现 | 具名拒绝（`-insertcontent.unsupported: this build has no content-palette insertion model`）。 |
| `-INTERFERE` | 部分实现 | 具名拒绝（`-interfere.unsupported: 3D solids are not modeled, so interference has nothing to compute`）。 |
| `-LAYDEL` | 部分实现 | `LAYDEL` 的别名，同一实现；实体与层行同一事务/一步撤销。 |
| `-LAYER` | 已实现 | 裸命令/`? [pattern]` 为只读图层表报告（名称/可见/off/frozen/locked/当前层，`layer.entry`；`*`/`?` 通配符过滤，无匹配报 `layer.no_match`）；写形式一行一个选项（大小写不敏感）：`NEW/N`、`MAKE/M`、`SET/S`、`RENAME/R`、`DELETE/D`、`ON`/`OFF`、`FREEZE/F`/`THAW/T`、`LOCK/LO`/`UNLOCK/U`、`COLOR/C`（ACI 数字/七个标准色名/`aci:N`/`#RRGGBB`/`rgb:…`）、`LTYPE/L`、`LWEIGHT/LW`、`PLOT/P Y/N`；名单逗号分隔，含空格的值/名称可用双引号；整行原子（先解析全部名称与值再发布候选库），一条事务/一步撤销（多行 patch 同一 `DrawingUndoRecord`），无变化为 `layer.unchanged` 且不升修订；`MAKE`/`SET` 复用当前层规则（locked/frozen/off 先拒绝），`OFF`/`FREEZE` 对当前层按 `layer.current_*` 拒绝；写形式经 Work-only `SetLayerProperty` 载体，Viewer 拒绝。逐选项交互重提示属全局范围。 |
| `-LAYMCH` | 部分实现 | `LAYMCH` 的别名，同一实现 |
| `-LAYMRG` | 部分实现 | `LAYMRG` 的别名，同一实现；实体与层行同一事务/一步撤销。 |
| `-LINETYPE` | 部分实现 | `LINETYPE` 的别名（同一 `CommandId`/路径）；`LIST` 只读、`SET` 复用 `SETENTITYPROPERTY` |
| `-LWEIGHT` | 部分实现 | 同一线宽显示报告的命令行拼写。 |
| `-MARKUPASSIST` | 部分实现 | 具名拒绝（`-markupassist.unsupported: this build has no collaboration markup/trace model`）。 |
| `-MARKUPIMPORT` | 部分实现 | 具名拒绝（`-markupimport.unsupported: this build has no collaboration markup/trace model`）。 |
| `-MLEDIT` | 部分实现 | `MLEDIT` 的命令行拼写，同一实现与边界 |
| `-MTEXT` | 部分实现 | `MTEXT` 创建工具的别名（同一两角 + 终端文本捕获）；无第二实现 |
| `-OBJECTSCALE` | 部分实现 | 具名拒绝（`-objectscale.unsupported`）：同一具名拒绝的命令行拼写。 |
| `-OVERKILL` | 部分实现 | `OVERKILL` 的别名，同一去重实现 |
| `-PAGESETUP` | 部分实现 | 裸命令只读报告；typed 形式写入 Authored PLOTSETTINGS（纸张/比例/旋转），一条事务/一步撤销。 |
| `-PAN` | 部分实现 | `PAN` 的别名；`dx dy` 或两点，走既有相机路径；只改视图（不写库/不 bump 修订/不入历史） |
| `-PARAMETERS` | 部分实现 | 具名拒绝（`-parameters.unsupported: no dynamic-block parameter/lookup model; the Parameters Manager palette is not implemented`）。 |
| `-PARTIALOAD` | 部分实现 | 具名拒绝（`-partiaload.unsupported: no partial layer-index open/load model`）。 |
| `-PDFATTACH` | 部分实现 | 具名拒绝（`-pdfattach.unsupported: PDF underlays are not implemented`）。 |
| `-PDFIMPORT` | 部分实现 | 具名拒绝（`-pdfimport.unsupported: PDF geometry import is not implemented`）。 |
| `-PDFSHXTEXT` | 部分实现 | 具名拒绝（`-pdfshxtext.unsupported: PDF SHX text conversion is not implemented`）。 |
| `-PLOT` | 部分实现 | 具名拒绝（`-plot.unsupported`）：请用 PLOT；本构建只有一条矢量出图路径；命令行拼写。 |
| `-PLOTSTAMP` | 部分实现 | 具名拒绝（`-plotstamp.unsupported`）：本构建无打印戳记模型；命令行拼写。 |
| `-PLOTSTYLE` | 部分实现 | 具名拒绝（`-plotstyle.unsupported`）：打印样式列表的命令行拼写。 |
| `-POINTCLOUDATTACH` | 部分实现 | 具名拒绝（`-pointcloudattach.unsupported: no point-cloud attachment/crop/style model`）。 |
| `-POINTCLOUDMANAGER` | 部分实现 | 具名拒绝（`-pointcloudmanager.unsupported: no point-cloud attachment/crop/style model`）。 |
| `-PSETUPIN` | 部分实现 | 同一图纸内页面设置复制的命令行拼写；一条事务/一步撤销。 |
| `-PUBLISH` | 部分实现 | 具名拒绝（`-publish.unsupported`）：本构建无批量发布模型；命令行拼写。 |
| `-PURGE` | 部分实现 | `PURGE` 的别名，同一实现 |
|  `-REFEDIT` | 部分实现 | 真实：`REFEDIT`/`-REFEDIT` 仅接受一个相似变换模型空间 INSERT，把可见块成员按 INSERT 变换克隆到模型空间（一条事务），源 INSERT 走会话隐藏；`REFSET ADD|REMOVE` 只改会话工作集；`REFCLOSE SAVE` 逆变换写回块定义（成员更新/新增/删除与克隆删除，一条事务/一步撤销），`DISCARD` 删除克隆、块定义不变。仅提取可见成员；无参照淡显/锁定（其他宿主对象仍可选，REFSET 是显式选择）；成员图层/绘图次序不回写；QSAVE 应用层拒绝（`refedit.save_blocked`）。 |
| `-RENAME` | 部分实现 | `RENAME` 的别名，同一实现；图层重命名一条事务/一步撤销。 |
| `-RENDER` | 部分实现 | 具名拒绝（`-render.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| `-SCALELISTEDIT` | 部分实现 | 只读报告真实 `ACAD_SCALELIST`（`scalelistedit.entry`）；创建/编辑/删除/排序比例未实现（`scalelistedit.write_unsupported`）。 |
| `-SHADEMODE` | 部分实现 | 具名拒绝（`-shademode.unsupported: no 3D shaded display mode; VSCURRENT owns the 2D display token`）。 |
| `-SHAREVIEW` | 部分实现 | 具名拒绝（`-shareview.unsupported: no Autodesk cloud sharing/view model`）。 |
| `-STYLE` | 已实现 | `STYLE` 的命令行拼写，同一 grammar/写入路径与边界 |
| `-TABLE` | 部分实现 | 真实创建（与 `TABLE` 同实现）：`-TABLE <rows> <cols> <col_width> <row_height> <x> <y>`，+X/+Z、空单元格/Standard、一条事务/一步撤销；无对话框。
| `-UNITS` | 部分实现 | 只读报告固定单位显示规则（`-units.*` 诊断码）；Units 对话框/逐图纸单位表未实现。 |
| `-VIEW` | 部分实现 | `VIEW` 的别名，同一实现 |
| `-VISUALSTYLES` | 部分实现 | 具名拒绝（`-visualstyles.unsupported: no visual-style/3D display model; VSCURRENT owns the 2D display token`）。 |
| `-VPORTS` | 部分实现 | `VPORTS` 的别名，同一实现；同一边界 |
| `-WBLOCK` | 部分实现 | 具名拒绝（`-wblock.unsupported`）：将块定义写入独立图形文件未实现。 |
| `-XBIND` | 部分实现 | 具名拒绝（`-xbind.unsupported`）：本构建无外部参照绑定/插入模型；命令行拼写。 |
| `-XREF` | 部分实现 | 只读：外部参照导入溯源报告（`-xref.entry`/`-xref.summary`）；附着/卸载/重载/绑定未实现。 |
| `3DALIGN` | 部分实现 | 具名拒绝（`3dalign.unsupported: 3D alignment is not modeled; use the 2D ALIGN path`）。 |
| `3DARRAY` | 部分实现 | 具名拒绝（`3darray.unsupported: 3D arrays are not modeled; use ARRAY/ARRAYRECT/ARRAYPOLAR`）。 |
| `3DCLIP` | 部分实现 | 具名拒绝（`3dclip.unsupported: no 3D clipping-plane model`）。 |
| `3DCORBIT` | 部分实现 | 具名拒绝（`3dcorbit.unsupported: no continuous 3D orbit controller`）。 |
| `3DDISTANCE` | 部分实现 | 具名拒绝（`3ddistance.unsupported: no interactive 3D distance navigation`）。 |
| `3DDWF` | 部分实现 | 具名拒绝（`3ddwf.unsupported: DWF/DWFx export is not implemented; use PLOT for SVG/PDF`）。 |
| `3DEDITBAR` | 部分实现 | 具名拒绝（`3deditbar.unsupported: no NURBS/spline edit bar`）。 |
| `3DFACE` | 部分实现 | 具名拒绝（`3dface.unsupported: no 3DFACE entity model`）。 |
| `3DFLY` | 部分实现 | 具名拒绝（`3dfly.unsupported: no 3D fly navigation`）。 |
| `3DFORBIT` | 部分实现 | 具名拒绝（`3dforbit.unsupported: no free 3D orbit controller`）。 |
| `3DMESH` | 部分实现 | 具名拒绝（`3dmesh.unsupported: no free-form mesh creation model`）。 |
| `3DMOVE` | 部分实现 | 具名拒绝（`3dmove.unsupported: no 3D move gizmo`）。 |
| `3DORBIT` | 部分实现 | 具名拒绝（`3dorbit.unsupported: no interactive 3D orbit controller; use ORBIT`）。 |
| `3DORBITCTR` | 部分实现 | 具名拒绝（`3dorbitctr.unsupported: no 3D orbit-center model`）。 |
| `3DOSNAP` | 部分实现 | 具名拒绝（`3dosnap.unsupported: no 3D object-snap model`）。 |
| `3DPAN` | 部分实现 | 具名拒绝（`3dpan.unsupported: no 3D pan navigation`）。 |
| `3DPOLY` | 部分实现 | 具名拒绝（`3dpoly.unsupported: no 3D polyline entity model`）。 |
| `3DPRINT` | 部分实现 | 具名拒绝（`3dprint.unsupported: no 3D print model`）。 |
| `3DPRINTSERVICE` | 部分实现 | 具名拒绝（`3dprintservice.unsupported: no STL export or 3D print service`）。 |
| `3DROTATE` | 部分实现 | 具名拒绝（`3drotate.unsupported: no 3D rotate gizmo`）。 |
| `3DSCALE` | 部分实现 | 具名拒绝（`3dscale.unsupported: no 3D scale gizmo`）。 |
| `3DSIN` | 部分实现 | 具名拒绝（`3dsin.unsupported: 3DS import is not implemented`）。 |
| `3DSWIVEL` | 部分实现 | 具名拒绝（`3dswivel.unsupported: no 3D swivel navigation`）。 |
| `3DWALK` | 部分实现 | 具名拒绝（`3dwalk.unsupported: no 3D walk navigation`）。 |
| `3DZOOM` | 部分实现 | 具名拒绝（`3dzoom.unsupported: no perspective 3D zoom navigation`）。 |
| `ABOUT` | 已实现 | chrome 本地选择子：打开 About 对话框（与 File > About 同一路径） |
| `ACADINFO` | 部分实现 | 只读：产品版本/构建配置 + 文档修订/实体数；许可证/硬件/模块数据不收集。 |
| `ACISIN` | 部分实现 | 具名拒绝（`acisin.unsupported: ACIS (SAT) import is not implemented`）。 |
| `ACISOUT` | 部分实现 | 具名拒绝（`acisout.unsupported: ACIS (SAT) export is not implemented`）。 |
| `ACTBASEPOINT` | 部分实现 | 具名拒绝（`actbasepoint.unsupported: action-recorder macros are not modeled (no recorder state, file, or playback)`）。 |
| `ACTIVITYINSIGHTSCLOSE` | 部分实现 | 具名拒绝（`activityinsightsclose.unsupported: this build has no Activity Insights data model`）。 |
| `ACTIVITYINSIGHTSOPEN` | 部分实现 | 具名拒绝（`activityinsightsopen.unsupported: this build has no Activity Insights data model`）。 |
| `ACTMANAGER` | 部分实现 | 具名拒绝（`actmanager.unsupported: action-recorder macros are not modeled (no recorder state, file, or playback)`）。 |
| `ACTRECORD` | 部分实现 | 具名拒绝（`actrecord.unsupported: action-recorder macros are not modeled (no recorder state, file, or playback)`）。 |
| `ACTSTOP` | 部分实现 | 具名拒绝（`actstop.unsupported: action-recorder macros are not modeled (no recorder state, file, or playback)`）。 |
| `ACTUSERINPUT` | 部分实现 | 具名拒绝（`actuserinput.unsupported: action-recorder macros are not modeled (no recorder state, file, or playback)`）。 |
| `ACTUSERMESSAGE` | 部分实现 | 具名拒绝（`actusermessage.unsupported: action-recorder macros are not modeled (no recorder state, file, or playback)`）。 |
| `ADCCLOSE` | 部分实现 | 具名拒绝（`adcclose.unsupported: this build has no DesignCenter content palette`）。 |
| `ADCENTER` | 部分实现 | 具名拒绝（`adcenter.unsupported: this build has no DesignCenter content palette`）。 |
| `ADCNAVIGATE` | 部分实现 | 具名拒绝（`adcnavigate.unsupported: this build has no DesignCenter content palette`）。 |
| `ADDSELECTED` | 部分实现 | `ADDSELECTED <x> <y>` 按选中类型/属性新建一个实体（LINE/CIRCLE/POINT/TEXT/MTEXT），保留图层/颜色/线型/线宽/透明度；ARC/POLYLINE/INSERT 等按名拒绝 |
| `ADJUST` | 部分实现 | 具名拒绝（`adjust.unsupported: this build has no underlay/image fade-contrast-monochrome model`）。 |
| `ALIASEDIT` | 部分实现 | `ADD <alias> <target>`/`REMOVE <alias>`/`LIST`/bare；运行时别名先于静态表解析（可遮蔽内建），仅删除运行时行；无交互对话框（声明），会话状态不持久化 |
| `ALIGN` | 已实现 | 1 对点移动、2 对点平移+旋转+可选均匀缩放、3 对点平面**刚性**（旋转+平移，不缩放）；退化/非共面/非全等/镜像按名拒绝；3 对点的 3D 平面旋转属全局范围 |
| `ALIGNSPACE` | 部分实现 | `model_a paper_a model_b paper_b` 4 坐标→均匀 `paper_per_model` + 平移，真实写 `model_to_paper`；CLI 尚无 4 坐标拼写；旋转对报告 `rotation_ignored` |
| `AMECONVERT` | 部分实现 | 具名拒绝（`ameconvert.unsupported: this build has no AME solid model`）。 |
| `ANALYSISCURVATURE` | 部分实现 | 具名拒绝（`analysiscurvature.unsupported: this build has no 3D curvature/draft/zebra analysis display`）。 |
| `ANALYSISDRAFT` | 部分实现 | 具名拒绝（`analysisdraft.unsupported: this build has no 3D curvature/draft/zebra analysis display`）。 |
| `ANALYSISOPTIONS` | 部分实现 | 具名拒绝（`analysisoptions.unsupported: this build has no 3D curvature/draft/zebra analysis display`）。 |
| `ANALYSISZEBRA` | 部分实现 | 具名拒绝（`analysiszebra.unsupported: this build has no 3D curvature/draft/zebra analysis display`）。 |
| `ANIPATH` | 部分实现 | 具名拒绝（`anipath.unsupported: this build has no camera-animation model`）。 |
| `ANNORESET` | 部分实现 | 具名拒绝（`annoreset.unsupported`）：本构建无注释性对象模型。 |
| `ANNOUPDATE` | 部分实现 | 具名拒绝（`annoupdate.unsupported`）：本构建无注释性对象模型。 |
| `APERTURE` | 已实现 | `APERTURE <n>`（整像素 1..=50）写会话孔径并让 snap/候选拾取真实使用（overlay 于配置默认值；PICKBOX 保持独立）；bare 为只读报告 |
| `APPAUTOLOADER` | 部分实现 | 具名拒绝（`appautoloader.unsupported: this build has no plug-in autoloader registry`）。 |
| `APPLOAD` | 部分实现 | 具名拒绝（`appload.unsupported: no plug-in/application loader model`）。 |
| `APPSTORE` | 部分实现 | 具名拒绝（`appstore.unsupported: the host owns this browser/store/chrome action`）。 |
| `ARC` | 已实现 | 三点弧；payload 可写 `(center,radius,start,sweep)`（负 sweep = CW），S-C-E/S-C-A/S-E-A 的几何都可表达；交互关键字 SCE/SCA/SEA/Continue 尚未接线 |
| `ARCHIVE` | 部分实现 | 具名拒绝（`archive.unsupported`）：本构建无图纸集归档打包模型。 |
| `ARCTEXT` | 已实现 | 沿选中 ARC 等角步放置逐字符 `TEXT`（切线旋转、倒置翻转），保留高度/样式/字体/图层，源文字被消费；每字符一个 TEXT 是声明表示偏离；+Z、单行 |
| `AREA` | 已实现 | — |
| `ARRAY` | 部分实现 | 路由：`ArrayRect`/`ArrayPolar` 载荷转交现有 `ARRAYRECT`/`ARRAYPOLAR` 规划器；无载荷只报告，不创建；无旧式对话框/关联阵列。 |
| `ARRAYCLASSIC` | 部分实现 | 无旧式对话框；带阵列载荷时转交 `ARRAY` 规划器，否则报告缺少载荷。 |
| `ARRAYCLOSE` | 部分实现 | 具名拒绝（`arrayclose.unsupported`）：本构建无关联阵列编辑状态。 |
| `ARRAYEDIT` | 部分实现 | 具名拒绝（`arrayedit.unsupported`）：本构建无关联阵列编辑状态。 |
| `ARRAYPATH` | 部分实现 | 真实 v1：当前选择第一个实体为路径、其余为项目，等弧长分布，`ARRAYPATH <count> [ALIGN]`；一条事务/一步撤销；无关联性/持久路径参数。 |
| `ARRAYPOLAR` | 部分实现 | 独立、非关联副本；角度步长改为 AutoCAD 规则：半圈 `fill/(items−1)`、整圈 `fill/items`；负 fill 支持；关联编辑未建模 |
| `ARRAYRECT` | 部分实现 | 独立、非关联副本；**有符号行/列间距**（负值向负方向阵列）；`Levels`/网格 `angle` 无工具参数通道；`Path`/关联编辑未建模 |
| `ARX` | 部分实现 | 具名拒绝（`arx.unsupported: no ObjectARX loader model`）。 |
| `ASSISTANTCLOSE` | 部分实现 | 具名拒绝（`assistantclose.unsupported: this build has no Autodesk Assistant model`）。 |
| `ASSISTANTOPEN` | 部分实现 | 具名拒绝（`assistantopen.unsupported: this build has no Autodesk Assistant model`）。 |
| `ATTACH` | 部分实现 | 具名拒绝（`attach.unsupported`）：不附着外部参照或底图。 |
| `ATTACHURL` | 部分实现 | 具名拒绝（`attachurl.unsupported: this build has no hyperlink storage model`）。 |
| `ATTDEF` | 部分实现 | 具名拒绝（`attdef.unsupported`）：本构建无块属性模型。 |
| `ATTDISP` | 部分实现 | 会话令牌（默认开）：翻转 `attdisp`；无块属性模型，不渲染属性、不写图。 |
| `ATTEDIT` | 部分实现 | 具名拒绝（`attedit.unsupported`）：本构建无块属性模型。 |
| `ATTEXT` | 部分实现 | 具名拒绝（`attext.unsupported`）：本构建无块属性模型。 |
| `ATTIN` | 部分实现 | 具名拒绝（`attin.unsupported`）：本构建无块属性模型。 |
| `ATTIPEDIT` | 部分实现 | 具名拒绝（`attipedit.unsupported`）：本构建无块属性模型。 |
| `ATTOUT` | 部分实现 | 具名拒绝（`attout.unsupported`）：本构建无块属性模型。 |
| `ATTREDEF` | 部分实现 | 具名拒绝（`attredef.unsupported`）：本构建无块属性模型。 |
| `ATTSYNC` | 部分实现 | 具名拒绝（`attsync.unsupported`）：本构建无块属性模型。 |
| `AUDIT` | 已实现 | `Fix any errors detected? [Yes/No]`（默认 No）+ 分段报告（头变量/块定义/Pass 1/2/`Total errors found N fixed M`）；只修复 `validate_entity` 可证明可修复的行（ObjectId 归一、非法 TEXT wrap→None），一条事务；修复不可撤销（`audit.repairs_not_undoable`）；`AUDITCTL` 属宿主文件 I/O 全局 |
| `AUTOCONSTRAIN` | 部分实现 | 具名拒绝（`autoconstrain.unsupported`）：本构建无几何约束模型。 |
| `AUTOPUBLISH` | 部分实现 | 具名拒绝（`autopublish.unsupported`）：本构建无自动发布模型。 |
| `BACTION` | 部分实现 | 具名拒绝（`baction.unsupported`）：本构建无动态块动作模型。 |
| `BACTIONBAR` | 部分实现 | 具名拒绝（`bactionbar.unsupported`）：本构建无动态块动作模型。 |
| `BACTIONSET` | 部分实现 | 具名拒绝（`bactionset.unsupported`）：本构建无动态块动作模型。 |
| `BACTIONTOOL` | 部分实现 | 具名拒绝（`bactiontool.unsupported`）：本构建无动态块动作模型。 |
| `BASE` | 部分实现 | 具名拒绝（`base.unsupported`）：图形不保存插入基点。 |
| `BASSOCIATE` | 部分实现 | 具名拒绝（`bassociate.unsupported`）：本构建无动态块动作模型。 |
| `BATTMAN` | 部分实现 | 只读：块定义与成员数（`battman.entry`/`battman.summary`）；块属性未建模（`battman.attributes_not_modeled`）。 |
| `BATTORDER` | 部分实现 | 只读：块定义列表（`battorder.entry`/`battorder.summary`）；属性顺序未建模（`battorder.attributes_not_modeled`）。 |
| `BAUTHORPALETTE` | 部分实现 | 具名拒绝（`bauthorpalette.unsupported`）：本构建无块编辑器。 |
| `BAUTHORPALETTECLOSE` | 部分实现 | 具名拒绝（`bauthorpaletteclose.unsupported`）：本构建无块编辑器。 |
| `BCLOSE` | 部分实现 | 具名拒绝（`bclose.unsupported`）：本构建无块编辑器。 |
| `BCONSTRUCTION` | 部分实现 | 具名拒绝（`bconstruction.unsupported`）：本构建无动态块构造几何。 |
| `BCOUNT` | 部分实现 | 只读：统计整个模型空间的块参照实例数（按块名分组）；无 AutoCAD 的 Select objects 提示、不展开块成员、不统计图纸空间。 |
| `BCPARAMETER` | 部分实现 | 具名拒绝（`bcparameter.unsupported`）：本构建无动态块参数模型。 |
| `BCYCLEORDER` | 部分实现 | 具名拒绝（`bcycleorder.unsupported`）：本构建无动态块夹点模型。 |
| `BDETECT` | 部分实现 | 具名拒绝（`bdetect.unsupported: this build has no object-detection model`）。 |
| `BEDIT` | 部分实现 | 具名拒绝（`bedit.unsupported`）：本构建无块编辑器。 |
| `BESETTINGS` | 部分实现 | 具名拒绝（`besettings.unsupported`）：本构建无块编辑器。 |
| `BEXTEND` | 部分实现 | 真实编辑：首个选中 INSERT 成员为边界，其余选中对象复用 EXTEND 规划器延伸；一条事务/一步撤销；无交互边界提示。 |
| `BGRIPSET` | 部分实现 | 具名拒绝（`bgripset.unsupported`）：本构建无动态块参数模型。 |
| `BLEND` | 部分实现 | 真实编辑：两条开放 `LINE`/`ARC`/`LWPOLYLINE` 最近端点之间一条三次 `SPLINE`；内部控制点沿源切线取间隙 1/3（声明规则）；无曲线连续性选项。 |
| `BLOCK` | 已实现 | — |
| `BLOCK?` | 部分实现 | 只读列出块定义与成员数（复用 `XLIST` 的 `drawing.blocks()`）；不是块编辑器。 |
| `BLOCKICON` | 部分实现 | 具名拒绝（`blockicon.unsupported: this build has no block thumbnail/preview model`）。 |
| `BLOCKREPLACE` | 部分实现 | 具名拒绝（`blockreplace.unsupported`）：需要两个块定义；无块替换编辑器。 |
| `BLOCKSDATAOPTION` | 部分实现 | 具名拒绝（`blocksdataoption.unsupported: this build has no data-collection consent dialog model`）。 |
| `BLOCKSPALETTE` | 部分实现 | 具名拒绝（`blockspalette.unsupported: this build has no Blocks palette; block insertion is the INSERT path`）。 |
| `BLOCKSPALETTECLOSE` | 部分实现 | 具名拒绝（`blockspaletteclose.unsupported: this build has no Blocks palette; block insertion is the INSERT path`）。 |
| `BLOCKTOXREF` | 部分实现 | 具名拒绝（`blocktoxref.unsupported: block-to-xref replacement needs a writable block table and xref attachment, neither modeled`）。 |
| `BLOOKUPTABLE` | 部分实现 | 具名拒绝（`blookuptable.unsupported`）：本构建无动态块查找表。 |
| `BMPOUT` | 部分实现 | 具名拒绝（`bmpout.unsupported: raster BMP export is not implemented; use PLOT for vector SVG/PDF`）。 |
| `BOUNDARY` | 部分实现 | 一个内部点 → 每个发现的闭合环（含嵌套岛）一条闭合非关联多段线；仅整实体圆/完整椭圆/闭合多段线、同 `+Z` 平面；`-BOUNDARY` 选项（边界集/关闭岛检测/面域输出/对象类型）未实现；无 `BO`/`BPOLY` 别名 |
| `BOX` | 部分实现 | 具名拒绝（`box.unsupported: 3D solid primitives are not implemented`）。 |
| `BPARAMETER` | 部分实现 | 具名拒绝（`bparameter.unsupported`）：本构建无动态块参数模型。 |
| `BREAK` | 已实现 | 两断点 + `F` 首点覆盖；`LINE`/`ARC`/`CIRCLE`/`SPLINE`/整椭圆/**椭圆弧**/`LWPOLYLINE`；圆→弧；第二点可投影；**无多段打断**（AutoCAD 无此模式） |
| `BREAKATPOINT` | 部分实现 | 交互工具：先拾取对象再指定断点（断点可投影到对象上）；仅 LINE/ARC/开放 LWPOLYLINE；圆/闭合多段线/SPLINE 按名拒绝（闭合对象不拆分） |
| `BREAKLINE` | 部分实现 | 真实编辑：两点之间一条带声明式锯齿折断符号的开放折线；符号宽 15%、深 5%（声明规则）；无块式折断符号。 |
| `BREP` | 部分实现 | 具名拒绝（`brep.unsupported: this build has no 3D solid history/brep model`）。 |
| `BREPLACE` | 部分实现 | 具名拒绝（`breplace.unsupported`）：需要两个块定义；无块替换编辑器。 |
| `BROWSER` | 部分实现 | 具名拒绝（`browser.unsupported: the host owns this browser/store/chrome action`）。 |
| `BSAVE` | 部分实现 | 具名拒绝（`bsave.unsupported`）：本构建无块编辑器。 |
| `BSAVEAS` | 部分实现 | 具名拒绝（`bsaveas.unsupported`）：本构建无块编辑器。 |
| `BSCALE` | 部分实现 | 具名拒绝（`bscale.unsupported`）：需要比例因子与插入参照；请对选中块参照用 SCALE。 |
| `BSEARCH` | 部分实现 | 具名拒绝（`bsearch.unsupported: this build has no convert-to-block search model`）。 |
| `BTABLE` | 部分实现 | 具名拒绝（`btable.unsupported`）：本构建无动态块查找表。 |
| `BTESTBLOCK` | 部分实现 | 具名拒绝（`btestblock.unsupported`）：本构建无块编辑器。 |
| `BTRIM` | 部分实现 | 真实编辑：首个选中 INSERT 成员为边界，其余选中对象复用 TRIM 规划器修剪；一条事务/一步撤销；无交互边界提示。 |
| `BURST` | 部分实现 | 具名拒绝（`burst.unsupported`）：无块属性模型可保留。 |
| `BVHIDE` | 部分实现 | 具名拒绝（`bvhide.unsupported`）：本构建无按可见性状态隐藏实体的 API。 |
| `BVSHOW` | 部分实现 | 具名拒绝（`bvshow.unsupported`）：本构建无按可见性状态显示实体的 API。 |
| `BVSTATE` | 部分实现 | 只读：每个选中 INSERT 的活动动态块可见性状态（`bvstate.report`）；无 INSERT 报 `bvstate.no_selection`，无状态报 `bvstate.not_set`；不设置状态。 |
| `CAL` | 部分实现 | 具名拒绝（`cal.unsupported`）：未实现表达式求值器；不写图。 |
| `CAMERA` | 部分实现 | 具名拒绝（`camera.unsupported: this build has no 3D camera/target view model`）。 |
| `CDORDER` | 部分实现 | 真实编辑：按颜色索引升序重写选中（空选择=全部模型空间）绘制顺序；一条事务/一步撤销；无对话框。 |
| `CENTERDISASSOCIATE` | 部分实现 | 具名拒绝（`centerdisassociate.unsupported: center marks/centerlines are not modeled, so associativity/reset has nothing to edit`）。 |
| `CENTERLINE` | 部分实现 | 真实编辑：恰好两条平行直线之间创建一条中间直线，覆盖投影重叠；一条事务/一步撤销；无延伸/间隙选项。 |
| `CENTERMARK` | 部分实现 | 真实编辑：每个选中圆/圆弧圆心两条交叉直线，臂长=半径 10%（声明规则）；一条事务/一步撤销；无关联对象/无可配置尺寸。 |
| `CENTERREASSOCIATE` | 部分实现 | 具名拒绝（`centerreassociate.unsupported: center marks/centerlines are not modeled, so associativity/reset has nothing to edit`）。 |
| `CENTERRESET` | 部分实现 | 具名拒绝（`centerreset.unsupported: center marks/centerlines are not modeled, so associativity/reset has nothing to edit`）。 |
| `CHAMFER` | 已实现 | `Distance`/`Angle`/`Method`/`Trim`/No-Trim/`Polyline` 全顶点；双零距离/角度=仅延伸相交；`Angle=0` 且距离>0 按名拒绝；`Multiple`/命令内 `Undo` 属逐选项提示全局范围；负距离按名拒绝 |
| `CHAMFEREDGE` | 部分实现 | 具名拒绝（`chamferedge.unsupported: no 3D edge chamfer model`）。 |
| `CHANGE` | 部分实现 | 真实编辑：选中模型空间实体移到当前层并重置颜色/线型/线宽为 ByLayer；一条事务/一步撤销；无交互点/特性提示。 |
| `CHECKSTANDARDS` | 部分实现 | 具名拒绝（`checkstandards.unsupported: this build has no CAD standards (DWS) model`）。 |
| `CHPROP` | 部分实现 | 真实编辑：同 CHANGE（移到当前层并重置颜色/线型/线宽为 ByLayer）；一条事务/一步撤销；无交互提示。 |
| `CHSPACE` | 部分实现 | 具名拒绝（`chspace.unsupported`）：模型/图纸空间之间移动实体未实现。 |
| `CHURLS` | 部分实现 | 具名拒绝（`churls.unsupported: this build has no hyperlink storage model`）。 |
| `CIRCLE` | 已实现 | — |
| `CLASSICGROUP` | 部分实现 | chrome 面板：列出会话 `EntityGroups` 与成员数，行聚焦/展开成员引用/关闭；“按组选择成员”因无宿主替换选择路径而禁用 |
| `CLASSICIMAGE` | 部分实现 | chrome 面板：只读列出 IMAGE 实体（id/空间/file key/pixels/可见/clip 摘要），行聚焦/关闭；attach/clip/adjust 仍是独立命令 |
| `CLASSICINSERT` | 已实现 | chrome 面板：列出真实块定义（名称/成员数/基点），插入动作走既有 `INSERT` 点位+名称流程（面板在宿主确认文字步后提交块名）；无块元数据的定义不可插入（不伪造基点） |
| `CLASSICLAYER` | 部分实现 | chrome 本地选择子：打开现有图层面板；紧凑布局像 LAYER 一样拒绝。 |
| `CLASSICXREF` | 部分实现 | 真实只读外部参照报告 + chrome 面板（`cad_status.references` 的 name/status/reason），行聚焦/关闭/Esc；attach/detach/reload 仍按名拒绝 |
| `CLEANSCREENOFF` | 部分实现 | chrome 本地选择子：显式退出 clean-screen；只恢复 chrome 可见性，不重开各面板 |
| `CLEANSCREENON` | 部分实现 | chrome 本地选择子：显式进入 clean-screen；只隐藏 chrome，不记录各面板的独立开关 |
| `CLIP` | 部分实现 | 具名拒绝（`clip.unsupported`）：无图像/外部参照裁剪模型。 |
| `CLIPIT` | 部分实现 | 具名拒绝（`clipit.unsupported`）：无图像/外部参照裁剪模型。 |
| `CLOSE` | 部分实现 | 关闭当前文档并回到开始页；无未保存修改提示（没有 dirty 模型），无多文档。 |
| `CLOSEALL` | 部分实现 | 具名拒绝（`closeall.single_document`）：本构建只有一个文档；请用 CLOSE。 |
| `CLOSEALLOTHER` | 部分实现 | 具名拒绝（`closeallother.single_document`）：只存在一个文档。 |
| `COLOR` | 部分实现 | 只读报告会话当前颜色 ACI（默认 256=ByLayer，`color.report`）；无创建路径消费该令牌，颜色对话框未实现。 |
| `COMMANDLINE` | 部分实现 | chrome 本地选择子：显示并聚焦命令行。 |
| `COMMANDLINEHIDE` | 部分实现 | chrome 本地选择子：隐藏命令行（与状态栏开关同一状态）。 |
| `COMMANDMACROS` | 部分实现 | 具名拒绝（`commandmacros.unsupported: this build has no command-macro palette model`）。 |
| `COMMANDMACROSCLOSE` | 部分实现 | 具名拒绝（`commandmacrosclose.unsupported: this build has no command-macro palette model`）。 |
| `COMPARE` | 部分实现 | 具名拒绝（`compare.unsupported: this build has no drawing-compare/diff model`）。 |
| `COMPARECLOSE` | 部分实现 | 具名拒绝（`compareclose.unsupported: this build has no drawing-compare/diff model`）。 |
| `COMPAREEXPORT` | 部分实现 | 具名拒绝（`compareexport.unsupported: this build has no drawing-compare/diff model`）。 |
| `COMPAREIMPORT` | 部分实现 | 具名拒绝（`compareimport.unsupported: this build has no drawing-compare/diff model`）。 |
| `COMPAREINFO` | 部分实现 | 具名拒绝（`compareinfo.unsupported: this build has no drawing-compare/diff model`）。 |
| `COMPILE` | 部分实现 | 具名拒绝（`compile.unsupported: this build has no SHX shape/font compiler or loader`）。 |
| `CONE` | 部分实现 | 具名拒绝（`cone.unsupported: 3D solid primitives are not implemented`）。 |
| `CONSTRAINTBAR` | 部分实现 | 具名拒绝（`constraintbar.unsupported`）：本构建无约束模型。 |
| `CONSTRAINTSETTINGS` | 部分实现 | 具名拒绝（`constraintsettings.unsupported`）：本构建无约束模型。 |
| `CONVERT` | 部分实现 | 具名拒绝（`convert.unsupported`）：通用对象转换未实现。 |
| `CONVERTCTB` | 部分实现 | 具名拒绝（`convertctb.unsupported`）：CTB 转换未实现（无 CTB/STB 模型）。 |
| `CONVERTOLDLIGHTS` | 部分实现 | 具名拒绝（`convertoldlights.unsupported: this build has no legacy light/material conversion model`）。 |
| `CONVERTOLDMATERIALS` | 部分实现 | 具名拒绝（`convertoldmaterials.unsupported: this build has no legacy light/material conversion model`）。 |
| `CONVERTPOLY` | 部分实现 | 只读报告（`convertpoly.single_representation`）：本构建只存一种 `LWPOLYLINE` 表示，无旧式多段线可转换。 |
| `CONVERTPSTYLES` | 部分实现 | 具名拒绝（`convertpstyles.unsupported`）：打印样式转换未实现（无 CTB/STB 模型）。 |
| `CONVTOMESH` | 部分实现 | 具名拒绝（`convtomesh.unsupported: 3D solids, surfaces and meshes are not modeled`）。 |
| `CONVTONURBS` | 部分实现 | 具名拒绝（`convtonurbs.unsupported: 3D solids, surfaces and meshes are not modeled`）。 |
| `CONVTOSOLID` | 部分实现 | 具名拒绝（`convtosolid.unsupported: 3D solids, surfaces and meshes are not modeled`）。 |
| `CONVTOSURFACE` | 部分实现 | 具名拒绝（`convtosurface.unsupported: 3D solids, surfaces and meshes are not modeled`）。 |
| `COORDINATIONMODELATTACH` | 部分实现 | 具名拒绝（`coordinationmodelattach.unsupported: this build has no Navisworks coordination-model attachment model`）。 |
| `COPY` | 已实现 | — |
| `COPYBASE` | 部分实现 | 应用内剪贴板 + 基点；不是 OS 剪贴板，不跨应用 |
| `COPYCLIP` | 部分实现 | 应用内剪贴板；不是 OS 剪贴板，不跨应用 |
| `COPYFROMTRACE` | 部分实现 | 具名拒绝（`copyfromtrace.unsupported: this build has no collaboration trace model`）。 |
| `COPYHIST` | 部分实现 | 只读报告（`copyhist.not_tracked`）：会话不存命令历史文本，界面命令日志自管；未复制任何内容。 |
| `COPYLINK` | 部分实现 | 具名拒绝（`copylink.unsupported: OLE/clipboard linking is owned by the host`）。 |
| `COPYM` | 部分实现 | 具名拒绝（`copym.unsupported: this build has no multi-copy (Repeat/Array/Divide/Measure) session model`）。 |
| `COPYTOLAYER` | 部分实现 | 真实编辑：首个选中实体图层为目标，其余实体以新 id 复制到该层；原实体不变；一条事务/一步撤销；无图层提示。 |
| `COUNT` | 部分实现 | 只读：统计当前选择；空选择时统计全部模型空间顶层实体（按类型分组）；不修改选择、不开事务。 |
| `COUNTAREA` | 部分实现 | 具名拒绝（`countarea.unsupported: this build has no object-count session model`）。 |
| `COUNTAREACLOSE` | 部分实现 | 具名拒绝（`countareaclose.unsupported: this build has no object-count session model`）。 |
| `COUNTCLOSE` | 部分实现 | 具名拒绝（`countclose.unsupported: this build has no object-count session model`）。 |
| `COUNTFIELD` | 部分实现 | 具名拒绝（`countfield.unsupported: this build has no object-count session model`）。 |
| `COUNTLIST` | 部分实现 | 具名拒绝（`countlist.unsupported: this build has no object-count session model`）。 |
| `COUNTLISTCLOSE` | 部分实现 | 具名拒绝（`countlistclose.unsupported: this build has no object-count session model`）。 |
| `COUNTNAVNEXT` | 部分实现 | 具名拒绝（`countnavnext.unsupported: this build has no object-count session model`）。 |
| `COUNTNAVPREV` | 部分实现 | 具名拒绝（`countnavprev.unsupported: this build has no object-count session model`）。 |
| `COUNTTABLE` | 部分实现 | 具名拒绝（`counttable.unsupported: this build has no object-count session model`）。 |
| `CUI` | 部分实现 | 具名拒绝（`cui.unsupported: this build has no CUIx customization model`）。 |
| `CUIEXPORT` | 部分实现 | 具名拒绝（`cuiexport.unsupported: this build has no CUIx customization model`）。 |
| `CUIIMPORT` | 部分实现 | 具名拒绝（`cuiimport.unsupported: this build has no CUIx customization model`）。 |
| `CUILOAD` | 部分实现 | 具名拒绝（`cuiload.unsupported: this build has no CUIx customization model`）。 |
| `CUIUNLOAD` | 部分实现 | 具名拒绝（`cuiunload.unsupported: this build has no CUIx customization model`）。 |
| `CUSTOMIZE` | 部分实现 | 具名拒绝（`customize.unsupported: this build has no tool-palette customization model`）。 |
| `CUTBASE` | 部分实现 | 应用内剪贴板 + 基点；不是 OS 剪贴板，不跨应用 |
| `CUTCLIP` | 部分实现 | 应用内剪贴板；不是 OS 剪贴板，不跨应用 |
| `CVADD` | 部分实现 | 具名拒绝（`cvadd.unsupported: NURBS control-vertex editing is not modeled`）。 |
| `CVHIDE` | 部分实现 | 具名拒绝（`cvhide.unsupported: NURBS control-vertex editing is not modeled`）。 |
| `CVREBUILD` | 部分实现 | 具名拒绝（`cvrebuild.unsupported: NURBS control-vertex editing is not modeled`）。 |
| `CVREMOVE` | 部分实现 | 具名拒绝（`cvremove.unsupported: NURBS control-vertex editing is not modeled`）。 |
| `CVSHOW` | 部分实现 | 具名拒绝（`cvshow.unsupported: NURBS control-vertex editing is not modeled`）。 |
| `CYLINDER` | 部分实现 | 具名拒绝（`cylinder.unsupported: 3D solid primitives are not implemented`）。 |
| `DATAEXTRACTION` | 部分实现 | 具名拒绝（`dataextraction.unsupported: this build has no data-extraction/attribute-template model`）。 |
| `DATALINK` | 部分实现 | 具名拒绝（`datalink.unsupported: this build has no external data-link model`）。 |
| `DATALINKUPDATE` | 部分实现 | 具名拒绝（`datalinkupdate.unsupported: this build has no external data-link model`）。 |
| `DBCCLOSE` | 部分实现 | 具名拒绝（`dbcclose.unsupported: this build has no dbConnect/external database model`）。 |
| `DBCCONFIGURE` | 部分实现 | 具名拒绝（`dbcconfigure.unsupported: this build has no dbConnect/external database model`）。 |
| `DBCDEFINELLT` | 部分实现 | 具名拒绝（`dbcdefinellt.unsupported: this build has no dbConnect/external database model`）。 |
| `DBCDEFINELT` | 部分实现 | 具名拒绝（`dbcdefinelt.unsupported: this build has no dbConnect/external database model`）。 |
| `DBCONNECT` | 部分实现 | 具名拒绝（`dbconnect.unsupported: this build has no dbConnect/external database model`）。 |
| `DBLIST` | 部分实现 | 只读列出所有顶层模型/图纸实体（id/type/layer/space/draw order）；块定义成员计数并跳过 |
| `DCALIGNED` | 部分实现 | 具名拒绝（`dcaligned.unsupported`）：本构建无尺寸约束模型。 |
| `DCANGULAR` | 部分实现 | 具名拒绝（`dcangular.unsupported`）：本构建无尺寸约束模型。 |
| `DCCONVERT` | 部分实现 | 具名拒绝（`dcconvert.unsupported`）：本构建无尺寸约束模型。 |
| `DCDIAMETER` | 部分实现 | 具名拒绝（`dcdiameter.unsupported`）：本构建无尺寸约束模型。 |
| `DCDISPLAY` | 部分实现 | 具名拒绝（`dcdisplay.unsupported`）：本构建无尺寸约束模型。 |
| `DCFORM` | 部分实现 | 具名拒绝（`dcform.unsupported`）：本构建无尺寸约束模型。 |
| `DCHORIZONTAL` | 部分实现 | 具名拒绝（`dchorizontal.unsupported`）：本构建无尺寸约束模型。 |
| `DCLINEAR` | 部分实现 | 具名拒绝（`dclinear.unsupported`）：本构建无尺寸约束模型。 |
| `DCRADIUS` | 部分实现 | 具名拒绝（`dcradius.unsupported: this build has no dimensional-constraint model`）。 |
| `DCVERTICAL` | 部分实现 | 具名拒绝（`dcvertical.unsupported: this build has no dimensional-constraint model`）。 |
| `DELAY` | 部分实现 | 具名拒绝（`delay.unsupported: this build has no script runner, so a timed pause has no effect`）。 |
| `DELCONSTRAINT` | 部分实现 | 具名拒绝（`delconstraint.unsupported`）：本构建无几何约束模型。 |
| `DETACHURL` | 部分实现 | 具名拒绝（`detachurl.unsupported: this build has no hyperlink storage model`）。 |
| `DETECTCLOSE` | 部分实现 | 具名拒绝（`detectclose.unsupported: this build has no object-detection model`）。 |
| `DETECTCONVERT` | 部分实现 | 具名拒绝（`detectconvert.unsupported: this build has no object-detection model`）。 |
| `DETECTPRIMARY` | 部分实现 | 具名拒绝（`detectprimary.unsupported: this build has no object-detection model`）。 |
| `DETECTREMOVE` | 部分实现 | 具名拒绝（`detectremove.unsupported: this build has no object-detection model`）。 |
| `DETECTREVIEWNEXT` | 部分实现 | 具名拒绝（`detectreviewnext.unsupported: this build has no object-detection model`）。 |
| `DETECTREVIEWPREV` | 部分实现 | 具名拒绝（`detectreviewprev.unsupported: this build has no object-detection model`）。 |
| `DGNADJUST` | 部分实现 | 具名拒绝（`dgnadjust.unsupported: no DGN underlay adjustment model`）。 |
| `DGNATTACH` | 部分实现 | 具名拒绝（`dgnattach.unsupported: DGN underlays are not implemented`）。 |
| `DGNCLIP` | 部分实现 | 具名拒绝（`dgnclip.unsupported: no DGN underlay clip model`）。 |
| `DGNEXPORT` | 部分实现 | 具名拒绝（`dgnexport.unsupported: this build has no DGN exporter`）。 |
| `DGNIMPORT` | 部分实现 | 具名拒绝（`dgnimport.unsupported: this build has no DGN importer/exporter or mapping model`）。 |
| `DGNLAYERS` | 部分实现 | 具名拒绝（`dgnlayers.unsupported: no DGN underlay layer model`）。 |
| `DGNMAPPING` | 部分实现 | 具名拒绝（`dgnmapping.unsupported: this build has no DGN importer/exporter or mapping model`）。 |
| `DIGITALSIGN` | 部分实现 | 具名拒绝（`digitalsign.unsupported: this build has no digital-signature model`）。 |
| `DIM` | 部分实现 | 只读路由报告（`dim.available`/`dim.router_only`）：列出已实现的 `DIM*` 创建选择子；不创建标注，无交互式类型提示。 |
| `DIMALIGNED` | 已实现 | — |
| `DIMANGULAR` | 部分实现 | 仅两线形式可交互；三点形式只到载荷层 |
| `DIMARC` | 部分实现 | 具名拒绝（`dimarc.unsupported`）：标注域无弧长类型。 |
| `DIMBASELINE` | 已实现 | — |
| `DIMBREAK` | 部分实现 | 按 `dimbreak.unsupported` 拒绝（无标注线打断记录）；绝不删除/裁剪标注本身 |
| `DIMCENTER` | 已实现 | — |
| `DIMCONSTRAINT` | 部分实现 | 具名拒绝（`dimconstraint.unsupported`）：本构建无尺寸约束模型。 |
| `DIMCONTINUE` | 已实现 | — |
| `DIMDIAMETER` | 已实现 | — |
| `DIMDISASSOCIATE` | 部分实现 | payload-free；无关联模型，`dimdisassociate.already_non_associative` 且不改变任何状态 |
| `DIMEDIT` | 部分实现 | 对所有选中标注 `TEXT <override>`（组 1，`<>` 占位，≤1024）或 `RESET`；一条事务/一步撤销；延伸线编辑未实现 |
| `DIMEX` | 部分实现 | 具名拒绝（`dimex.unsupported`）：未建模标注样式导出文件。 |
| `DIMIM` | 部分实现 | 具名拒绝（`dimim.unsupported`）：未建模标注样式导入文件。 |
| `DIMINSPECT` | 部分实现 | 具名拒绝（`diminspect.unsupported`）：标注域不存储检验数据。 |
| `DIMJOGGED` | 部分实现 | 具名拒绝（`dimjogged.unsupported`）：标注域不存储折弯字段。 |
| `DIMJOGLINE` | 部分实现 | `ADD`/`REMOVE` 解析后按 `dimjogline.unsupported` 拒绝（域无 jog 字段/存储） |
| `DIMLINEAR` | 已实现 | — |
| `DIMORDINATE` | 已实现 | — |
| `DIMOVERRIDE` | 部分实现 | 闭集的实体快照变量；未知变量/仅表变量/坏值/未实现值分别具名拒绝；一条事务/一步撤销；不改样式表 |
| `DIMRADIUS` | 已实现 | — |
| `DIMREASSOC` | 部分实现 | 非关联标注的具名 no-op（`dimreassoc.non_associative`）；不改变任何状态。 |
| `DIMREASSOCIATE` | 部分实现 | 非关联标注的具名 no-op（`dimreassociate.non_associative`）；不改变任何状态。 |
| `DIMREGEN` | 部分实现 | payload-free；本构建非关联，`dimregen.non_associative` 且不改变任何状态 |
| `DIMROTATED` | 部分实现 | 三点拾取（两尺寸界线原点 + 尺寸线位置）；尺寸线角度来自会话 `dimRotation` 参数（度，任意有限值，不回绕/不归一化）；无交互角度提示、无对象选择/文字覆盖/关联 |
| `DIMSPACE` | 已实现 | 首个线性/对齐/角度标注为基准；显式间距（`0` = 对齐）或 Auto = **2 × 基准样式文字高度**；角度标注按同心弧半径步进；模型空间；关联重排属全局范围 |
| `DIMSTYLE` | 已实现 | `LIST`/`CURRENT`/`SET <name>`/`NEW`/`RENAME`/`DELETE`/`MODIFY <field> <value>`；一条 dim-style patch/一步撤销；`Standard` 不可删、被标注引用的行不可改删；`MODIFY` 不回写已烘焙的标注快照（无 DIMREGEN 级联） |
| `DIMTEDIT` | 部分实现 | `MOVE`/`LEFT`/`RIGHT`/`CENTER`/`HOME`/`ANGLE`（`ROTATE`→`ANGLE`）；Left/Right/Center 同时移动锚点并设置字形级 `h_align`（仅线性/对齐/半径/直径）；Angle 固定文字中心、`0` 复位；一条事务/一步撤销；角/坐标标注的 justify 与 Oblique 未实现 |
| `DIST` | 已实现 | — |
| `DISTANTLIGHT` | 部分实现 | 具名拒绝（`distantlight.unsupported: 3D light objects are not modeled`）。 |
| `DIVIDE` | 已实现 | `LINE`/`ARC`/`CIRCLE`/`SPLINE`/`ELLIPSE`（含椭圆弧）/含 bulge 折线，按**等弧长**；开放 `N−1`、闭合 `N`；块对齐 Yes/No |
| `DONUT` | 部分实现 | 类型化 `DONUT <cx> <cy> <inner> <outer>`；两个同心圆；`inner=0` 只建外圆并诊断；无中心拾取提示 |
| `DOWNLOADMANAGER` | 部分实现 | 具名拒绝（`downloadmanager.unsupported: this build has no download-manager model`）。 |
| `DRAGMODE` | 部分实现 | 会话令牌 `dragmode`（默认 ON）；渲染器不消费，无拖动预览。 |
| `DRAWINGRECOVERY` | 部分实现 | 具名拒绝（`drawingrecovery.unsupported: this build has no drawing-recovery manager model`）。 |
| `DRAWINGRECOVERYHIDE` | 部分实现 | 具名拒绝（`drawingrecoveryhide.unsupported: this build has no drawing-recovery manager model`）。 |
| `DRAWORDER` | 部分实现 | `FRONT\ |
| `DSETTINGS` | 部分实现 | chrome 本地选择子：打开现有 Options 对话框（Drafting 页），不读写图纸。 |
| `DVIEW` | 部分实现 | 具名拒绝（`dview.unsupported: this build has no 3D camera/target view model`）。 |
| `DWFADJUST` | 部分实现 | 具名拒绝（`dwfadjust.unsupported: this build has no DWF/DWFx underlay or export model`）。 |
| `DWFATTACH` | 部分实现 | 具名拒绝（`dwfattach.unsupported: this build has no DWF/DWFx underlay or export model`）。 |
| `DWFCLIP` | 部分实现 | 具名拒绝（`dwfclip.unsupported: this build has no DWF/DWFx underlay or export model`）。 |
| `DWFFORMAT` | 部分实现 | 只读能力报告（`dwfformat.not_supported`）：本构建不导出 DWF/DWFx；请用 PLOT 输出 SVG/PDF。 |
| `DWFLAYERS` | 部分实现 | 具名拒绝（`dwflayers.unsupported: this build has no DWF/DWFx underlay or export model`）。 |
| `DWGCONVERT` | 部分实现 | 具名拒绝（`dwgconvert.unsupported: batch DWG conversion is not implemented`）。 |
| `DWGLOG` | 部分实现 | 会话令牌（默认关）：翻转 `dwglog`；无日志文件、不写图/不升修订。 |
| `DWGPROPS` | 部分实现 | 只读：文档 id/修订/实体数（`dwgprops.report`）；标题/作者/自定义特性未建模（`dwgprops.metadata_not_tracked`）。 |
| `DXBIN` | 部分实现 | 具名拒绝（`dxbin.unsupported: DXB import is not implemented`）。 |
| `EATTEDIT` | 部分实现 | 具名拒绝（`eattedit.unsupported`）：本构建无块属性模型。 |
| `EATTEXT` | 部分实现 | 具名拒绝（`eatttext.unsupported`）：本构建无块属性模型。 |
| `EDGE` | 部分实现 | 具名拒绝（`edge.unsupported: no 3D face-edge visibility model`）。 |
| `EDGESURF` | 部分实现 | 具名拒绝（`edgesurf.unsupported: 3D mesh surfaces are not modeled`）。 |
| `EDITSHOT` | 部分实现 | 具名拒绝（`editshot.unsupported: this build has no ShowMotion named-view/motion model`）。 |
| `EDITTIME` | 部分实现 | 只读：当前 UTC 时钟 + 文档修订/实体数；创建/修改时间与会话用时 NOT TRACKED。 |
| `ELEV` | 部分实现 | 只读报告会话标高档位（默认 0.0）；`ELEV <height>` 设置形式未实现，不按标高创建对象。 |
| `ELLIPSE` | 已实现 | 轴端点整椭圆；payload 新增 **弧模式**（`start`/`sweep`，≤一圈）与**绕长轴旋转**（`normal` + `ratio·cos R` 的 DXF 投影规则）；点击臂仍只产整椭圆 |
| `ERASE` | 已实现 | — |
| `ETRANSMIT` | 部分实现 | 具名拒绝（`etransmit.unsupported`）：本构建无 eTransmit/zip 打包模型。 |
| `EXOFFSET` | 部分实现 | Express 偏移名；转交唯一 `OFFSET` 规划器；边界同 `OFFSET`。 |
| `EXPLAN` | 部分实现 | 具名拒绝（`explan.unsupported: this build has no UCS model; the world +Z plan path is PLAN`）。 |
| `EXPLODE` | 部分实现 | 一层分解；标注→线/弧/箭头/文字、多重引线→引线/样条/箭头/MTEXT、非均匀块可表示子集；`EXPMODE` 无会话通道 |
| `EXPORT` | 部分实现 | PLOT 别名：仅矢量 SVG/PDF |
| `EXPORTDWF` | 部分实现 | 具名拒绝（`exportdwf.unsupported: this build has no DWF/DWFx exporter`）。 |
| `EXPORTDWFX` | 部分实现 | 具名拒绝（`exportdwfx.unsupported: this build has no DWF/DWFx exporter`）。 |
| `EXPORTLAYOUT` | 部分实现 | 具名拒绝（`exportlayout.unsupported`）：请用 PLOT（单一布局，SVG/PDF）。 |
| `EXPORTPDF` | 部分实现 | 只读能力报告（`exportpdf.use_plot`）：PDF 导出由宿主经 PLOT 负责，本命令自身不写文件。 |
| `EXPORTSETTINGS` | 部分实现 | 具名拒绝（`exportsettings.unsupported`）：本构建无导出设置模型。 |
| `EXPORTTOAUTOCAD` | 部分实现 | 具名拒绝（`exporttoautocad.unsupported: exporting to another AutoCAD version is not implemented`）。 |
| `EXPRESSMENU` | 部分实现 | 具名拒绝（`expressmenu.unsupported: this build has no Express Tools menu/library loader`）。 |
| `EXPRESSTOOLS` | 部分实现 | 具名拒绝（`expresstools.unsupported: this build has no Express Tools menu/library loader`）。 |
| `EXTEND` | 部分实现 | 目标 LINE/ARC/**ELLIPSE（含椭圆弧）**；弧可超过半圈（至整圈前）；`SPLINE` 目标与 `CIRCLE`/整椭圆按名拒绝；折线/INSERT/HATCH 等目标按名拒绝 |
| `EXTERNALREFERENCES` | 部分实现 | 只读：外部参照导入溯源报告（`externalreferences.entry`/`.summary`）；无选项板（`.palette_not_implemented`）。 |
| `EXTERNALREFERENCESCLOSE` | 部分实现 | 具名 no-op（`externalreferences.close_not_implemented`）：无外部参照选项板可关闭。 |
| `EXTRIM` | 部分实现 | 第一个选中为边界，其余为被裁对象；复用 trim 规划；不支持边界/目标按名拒绝 |
| `EXTRUDE` | 部分实现 | 具名拒绝（`extrude.unsupported: 3D extrusion is not implemented`）。 |
| `FADEMARKUP` | 部分实现 | 具名拒绝（`fademarkup.unsupported: this build has no collaboration markup/trace model`）。 |
| `FIELD` | 部分实现 | 具名拒绝（`field.unsupported`）：字段未实现（无字段模型）。 |
| `FILETAB` | 已实现 | chrome 本地选择子：显示文件/图形标签栏（单文档构建仍有该标签） |
| `FILETABCLOSE` | 已实现 | chrome 本地选择子：隐藏文件/图形标签栏 |
| `FILL` | 部分实现 | 会话令牌 `fill`（默认 ON）；渲染器不消费该标志，无视觉变化；不写库/不入历史。 |
| `FILLET` | 部分实现 | `Radius`/`Trim`/`Polyline`、零半径尖角、平行线、圆/圆弧对曲线；本轮新增 **ELLIPSE ↔ LINE/CIRCLE/ARC/ELLIPSE** 数值圆角（采样+Newton+投影抛光，椭圆按参数区间修剪）；`SPLINE` 对按名拒绝；`Multiple`/多解循环未实现 |
| `FILLETEDGE` | 部分实现 | 具名拒绝（`filletedge.unsupported: no 3D edge fillet model`）。 |
| `FILTER` | 部分实现 | `AND`/`OR`/`XOR`/`NOT` 递归逻辑树（括号、元数校验）、相对运算符、已建模属性集；只读；**分类/自定义特性未建模**；**命名 `.nfl` 过滤器未实现** |
| `FIND` | 部分实现 | `FIND <needle> [=> <replacement>]`；替换形式一条事务/一步撤销；不解析 MTEXT 格式 run；TEXT 单行拒绝换行 |
| `FINDINCLUDE` | 部分实现 | 会话令牌（默认 ON）；跨块/外部参照文字搜索未实现。 |
| `FINDMATCH` | 部分实现 | 会话令牌（默认 OFF）；已实现的 -FIND/FIND 恒按 ASCII 不区分大小写比较。 |
| `FLATSHOT` | 部分实现 | 具名拒绝（`flatshot.unsupported: 3D solids are not modeled, so there is nothing to flatten`）。 |
| `FLATTEN` | 部分实现 | 真实编辑：选中 LINE/POLYLINE/CIRCLE/ARC/ELLIPSE/POINT/SPLINE 的 z 置 0；一条事务/一步撤销；MESH/INSERT/3D 按名拒绝（Express 行为简化为 z=0）。 |
| `FREESPOT` | 部分实现 | 具名拒绝（`freespot.unsupported: 3D light objects are not modeled`）。 |
| `FREEWEB` | 部分实现 | 具名拒绝（`freeweb.unsupported: 3D light objects are not modeled`）。 |
| `FS` | 部分实现 | Express 工具、仅命令行（无功能区按钮）；取当前唯一选择为种子，把**直接**接触（相交或相切都算）的可见模型空间对象并入选择。默认 `FSMODE=OFF`：链式接触（`FSMODE=ON`）未实现；不展开块实例成员 |
| `GATTE` | 部分实现 | 具名拒绝（`gatte.unsupported`）：本构建无块属性模型。 |
| `GCCOINCIDENT` | 部分实现 | 具名拒绝（`gccoincident.unsupported: this build has no geometric-constraint model`）。 |
| `GCCOLLINEAR` | 部分实现 | 具名拒绝（`gccollinear.unsupported: this build has no geometric-constraint model`）。 |
| `GCCONCENTRIC` | 部分实现 | 具名拒绝（`gcconcentric.unsupported: this build has no geometric-constraint model`）。 |
| `GCEQUAL` | 部分实现 | 具名拒绝（`gcequal.unsupported: this build has no geometric-constraint model`）。 |
| `GCFIX` | 部分实现 | 具名拒绝（`gcfix.unsupported: this build has no geometric-constraint model`）。 |
| `GCHORIZONTAL` | 部分实现 | 具名拒绝（`gchorizontal.unsupported: this build has no geometric-constraint model`）。 |
| `GCPARALLEL` | 部分实现 | 具名拒绝（`gcparallel.unsupported: this build has no geometric-constraint model`）。 |
| `GCPERPENDICULAR` | 部分实现 | 具名拒绝（`gcperpendicular.unsupported: this build has no geometric-constraint model`）。 |
| `GCSMOOTH` | 部分实现 | 具名拒绝（`gcsmooth.unsupported: this build has no geometric-constraint model`）。 |
| `GCSYMMETRIC` | 部分实现 | 具名拒绝（`gcsymmetric.unsupported: this build has no geometric-constraint model`）。 |
| `GCTANGENT` | 部分实现 | 具名拒绝（`gctangent.unsupported: this build has no geometric-constraint model`）。 |
| `GCVERTICAL` | 部分实现 | 具名拒绝（`gcvertical.unsupported: this build has no geometric-constraint model`）。 |
| `GEOGRAPHICLOCATION` | 部分实现 | 具名拒绝（`geographiclocation.unsupported: this build has no geographic/GIS or online-map model`）。 |
| `GEOMAP` | 部分实现 | 具名拒绝（`geomap.unsupported: this build has no geographic/GIS or online-map model`）。 |
| `GEOMAPIMAGE` | 部分实现 | 具名拒绝（`geomapimage.unsupported: this build has no geographic/GIS or online-map model`）。 |
| `GEOMAPIMAGEUPDATE` | 部分实现 | 具名拒绝（`geomapimageupdate.unsupported: this build has no geographic/GIS or online-map model`）。 |
| `GEOMARKLATLONG` | 部分实现 | 具名拒绝（`geomarklatlong.unsupported: this build has no geographic/GIS or online-map model`）。 |
| `GEOMARKME` | 部分实现 | 具名拒绝（`geomarkme.unsupported: this build has no geographic/GIS or online-map model`）。 |
| `GEOMARKPOINT` | 部分实现 | 具名拒绝（`geomarkpoint.unsupported: this build has no geographic/GIS or online-map model`）。 |
| `GEOMARKPOSITION` | 部分实现 | 具名拒绝（`geomarkposition.unsupported: this build has no geographic/GIS or online-map model`）。 |
| `GEOMCONSTRAINT` | 部分实现 | 具名拒绝（`geomconstraint.unsupported`）：本构建无几何约束模型。 |
| `GEOREMOVE` | 部分实现 | 具名拒绝（`georemove.unsupported: this build has no geographic/GIS or online-map model`）。 |
| `GEOREORIENTMARKER` | 部分实现 | 具名拒绝（`georeorientmarker.unsupported: this build has no geographic/GIS or online-map model`）。 |
| `GETSEL` | 部分实现 | `GETSEL LAYER\ |
| `GOTOSTART` | 部分实现 | 具名拒绝（`gotostart.unsupported`）：切换到开始页由宿主负责。 |
| `GOTOURL` | 部分实现 | 具名拒绝（`gotourl.unsupported: this build has no hyperlink storage model`）。 |
| `GRADIENT` | 部分实现 | 预选择闭合边界（Circle/完整 Ellipse/闭合 Polyline/LINE+ARC 边链）且必须位于 +Z 平面；复用 HATCH 创建路径，强制渐变填充（会话 kind/角度/双色 ACI，默认线性 0°、ACI 1→5），一个 `AcDbHatch`、一条事务/一步撤销；边界实体保留 |
| `GRAPHICSCONFIG` | 部分实现 | 具名拒绝（`graphicsconfig.unsupported: renderer/graphics configuration is owned by the host`）。 |
| `GRAPHSCR` | 部分实现 | 置会话 `text_screen`=false；无文本屏幕窗口可隐藏。 |
| `GRID` | 已实现 | — |
| `GROUP` | 部分实现 | 会话内组表（上限 64、名字大小写不敏感）；裸命令/`LIST` 只读、`CREATE` 从选择集建组、`DELETE` 只移除名字；不写 DWG group 对象/修订/历史/事务 |
| `GROUPEDIT` | 部分实现 | `ADD`/`REMOVE` 从选择集或键入实体 id 增删、`RENAME`（允许仅改大小写）；单次会话变更；不写实体/数据库 |
| `HATCH` | 部分实现 | 整实体闭合边界 + LINE/ARC 边链；**本轮新增 `.pat` 解析器/名称解析**（内置三项同源解析）；自定义图案仍无命令行选择通道（`hatchPattern` 闭集 1..=3）；无关联性/实时预览 |
|  `HATCHEDIT` | 部分实现 | 多目标；`Style`/`PatternType`（含 **CUSTOM 76=2**）/**`PATTERN <内置图案名>`**（现有 `.pat` 库；任意用户 `.pat` 文件需宿主 I/O）/`Angle`/`Scale`/`Solid`/`Gradient`/`Color`/`Transparency`/`Layer`/`DrawOrder`/`Recreate Boundary`/`Separate`/**Add-Remove Boundaries**；`Origin`/`Associate`/`Annotative` 未建模 |
| `HATCHGENERATEBOUNDARY` | 部分实现 | 预选择 hatch；每个 loop 一条非关联闭合多段线（同层同空间、保留 bulge、含内岛）；无关联/无选项提示 |
| `HATCHSETBOUNDARY` | 部分实现 | 恰好一个 hatch + 一条闭合 Polyline/Circle/完整 Ellipse，共面性校验；用 `HatchGeometry::new` 重建、丢弃旧种子；失败 `hatchsetboundary.rebuild_failed` |
| `HATCHSETORIGIN` | 部分实现 | 恒按 `hatchsetorigin.missing_field` 拒绝（`HatchGeometry` 无图案原点字段）；不写入 |
| `HATCHTOBACK` | 部分实现 | 全部模型空间 hatch 置底（仅改写存储 draw order、保持相对次序、可撤销）；图纸空间/块内不重排 |
| `HELIX` | 部分实现 | 平面螺旋 `Polyline`；`height>0` 按 `helix.3d_not_implemented` 拒绝 |
| `HELP` | 部分实现 | 具名拒绝（`help.unsupported`）：帮助系统由宿主负责，未实现。 |
| `HIDE` | 部分实现 | 具名拒绝（`hide.unsupported: the renderer has no 3D hidden-line removal`）。 |
| `HIDEOBJECTS` | 部分实现 | 会话隐藏集；经共享可见性谓词影响拾取/渲染（FLAG_HIDDEN）；无数据库写/撤销；隐藏点标记 overlay 仍可能绘制（声明） |
| `HIDEPALETTES` | 已实现 | chrome 本地选择子：隐藏功能区+选项板+文件标签（同一 clean-screen 形态状态） |
| `HIGHLIGHTNEW` | 部分实现 | 会话令牌（默认 ON）；不渲染新功能高亮。 |
| `HLSETTINGS` | 部分实现 | 具名拒绝（`hlsettings.unsupported: this build has no hidden-line display settings model`）。 |
| `HYPERLINK` | 部分实现 | 具名拒绝（`hyperlink.unsupported: this build has no hyperlink storage model`）。 |
| `HYPERLINKOPTIONS` | 部分实现 | 具名拒绝（`hyperlinkoptions.unsupported`）：无超链接存储模型。 |
| `ID` | 已实现 | — |
| `IGESEXPORT` | 部分实现 | 具名拒绝（`igesexport.unsupported: IGES export is not implemented`）。 |
| `IGESIMPORT` | 部分实现 | 具名拒绝（`igesimport.unsupported: IGES import is not implemented`）。 |
| `IMAGE` | 部分实现 | 只读报告所有 IMAGE 实体（id/空间/file key/pixels/visible/clip/origin，`image.*` 诊断码）；Viewer-legal，不写库；无 External References 面板。 |
| `IMAGEADJUST` | 部分实现 | 真实：选中一个 IMAGE，`IMAGEADJUST <brightness> <contrast> <fade> [MONO|COLOR]` 或 `RESET`（各 0..=100，50/50/0 中性）；一条事务/一步撤销；渲染顺序 mono→contrast→brightness→fade-to-background（声明近似）。 |
| `IMAGEAPP` | 部分实现 | 具名拒绝（`imageapp.unsupported: no external image-editor association model`）。 |
| `IMAGEASYNCWAIT` | 部分实现 | 具名拒绝（`imageasyncwait.unsupported: no asynchronous image loading model`）。 |
| `IMAGEATTACH` | 部分实现 | 真实创建 `SemanticGeometry::Image`：`IMAGEATTACH <file> <x> <y> <w_px> <h_px> <units_per_pixel>`；逻辑 key 经 `image_resource_key` 规范化（拒绝绝对路径/`..`/URL），+Z 平面、`u=(upp,0,0)`/`v=(0,upp,0)`，一条事务/一步撤销；宿主在提交后按 key 集变化重载纹理（大小写敏感文件系统用逐段不敏感回退），未解析 key 进入宿主图像报告。 |
| `IMAGECLIP` | 部分实现 | `IMAGECLIP <minx> <miny> <maxx> <maxy>` / `IMAGECLIP POLY <x1> <y1> …` / `IMAGECLIP OFF`：图像 clip 已存 `Vec<[f64;2]>`，世界坐标经 u/v 映射；非凸环存储合法但当前渲染 fan-triangulate，表示层报告 partial |
| `IMAGEEDIT` | 部分实现 | 具名拒绝（`imageedit.unsupported: no external image-editor model`）。 |
| `IMAGEQUALITY` | 部分实现 | 具名拒绝（`imagequality.unsupported: no image display-quality model`）。 |
| `IMPORT` | 部分实现 | 具名拒绝（`import.unsupported: the generic IMPORT picker is not implemented`）。 |
| `IMPRINT` | 部分实现 | 具名拒绝（`imprint.unsupported: no 3D imprint model`）。 |
| `INPUTSEARCHOPTIONS` | 部分实现 | 具名拒绝（`inputsearchoptions.unsupported`）：输入搜索设置未建模。 |
| `INSERT` | 已实现 | — |
| `INSERTOBJ` | 部分实现 | 具名拒绝（`insertobj.unsupported: this build has no OLE object storage model`）。 |
| `INTERFERE` | 部分实现 | 具名拒绝（`interfere.unsupported: 3D solids are not modeled, so interference has nothing to compute`）。 |
| `INTERSECT` | 部分实现 | 具名拒绝（`intersect.unsupported: no 3D boolean model`）。 |
| `ISODRAFT` | 部分实现 | 会话令牌 `isodraft`；不切换等轴测平面/光标，无视觉变化。 |
| `ISOLATEOBJECTS` | 部分实现 | 会话隐藏集：隐藏除当前选择外的模型空间对象；无数据库写/撤销 |
| `ISOPLANE` | 部分实现 | 循环会话令牌 LEFT→TOP→RIGHT；无等轴测光标/捕捉变化。 |
| `JOIN` | 部分实现 | 混合 LINE/ARC/多段线/SPLINE/椭圆弧精确焊接为有理 B 样条；**SPLINE 源对象接受接触的 LINE/ARC/椭圆弧**；`JOIN CLOSE` 的混合 SPLINE 仍拒绝；空隙桥接不臆造 |
| `JPGOUT` | 部分实现 | 具名拒绝（`jpgout.unsupported: raster JPEG export is not implemented; use PLOT for vector SVG/PDF`）。 |
| `JULIAN` | 部分实现 | 只读：当前 UTC 时刻的儒略日（整数日 + 小数）；不解析日期参数。 |
| `JUSTIFYTEXT` | 部分实现 | `JUSTIFYTEXT <H> <V>`（H∈LEFT/CENTER/RIGHT，V∈BASELINE/BOTTOM/MIDDLE/TOP）；只改 `h_align`/`v_align`，位置不动；非文字目标逐条 `justifytext.not_text` 跳过；空选择拒绝；`TJUST` 为同一实现 |
| `LAYCUR` | 部分实现 | 预选择输入（不弹 Select objects）；把选中对象移到当前层；当前层缺失/目标层锁定显式拒绝；实体写入可撤销 |
| `LAYDEL` | 部分实现 | 选择式：删除选中对象所在层的模型空间实体 + 层行，同一事务/一步撤销；当前层/0 层/仍有图纸或块内实体的层拒绝（图纸/块内实体仍未纳入） |
| `LAYER` | 已实现 | chrome 本地选择子：打开图层面板 |
| `LAYERCLOSE` | 已实现 | chrome 本地选择子：关闭图层面板（面板关闭按钮走同一命令） |
| `LAYERP` | 部分实现 | 会话内有界 16 步；不持久化 |
| `LAYERPALETTE` | 部分实现 | 具名拒绝（`layerpalette.unsupported`）：无独立图层调色板，请用 `LAYER`。 |
| `LAYERPMODE` | 部分实现 | `ON\ |
| `LAYERSTATE` | 部分实现 | `LAYERSTATE SAVE\|RESTORE\|LIST\|DELETE <name>`；会话内最多 32 个命名状态；RESTORE 的图层属性写入一条事务/一步撤销；不持久化 DWG |
| `LAYERSTATESAVE` | 部分实现 | 带 `LayerState` 载荷时转交既有 `LAYERSTATE` 会话存储；无载荷只报告缺少 New Layer State 对话框（`layerstatesave.needs_name`）。 |
| `LAYFRZ` | 部分实现 | 预选择输入（不弹 Select objects）；仅 Entity 选择，无 Settings/Viewports/Block 选项；v1 在全部视口冻结；当前层整命令拒绝；图层表写入一条事务/一步撤销 |
| `LAYISO` | 已实现 | — |
| `LAYLCK` | 部分实现 | 预选择输入（不弹 Select objects）；锁定选中对象所在的不同图层（含当前层）；图层表写入一条事务/一步撤销 |
| `LAYMCH` | 部分实现 | 选择式：第一个选中对象的图层为目标，其余移入（一条事务/一步撤销）；空选择/<2 目标/非模型空间/块实例按名拒绝；无交互 Select objects |
| `LAYMCUR` | 部分实现 | 预选择输入（不弹 Select objects）；取第一个选中对象的图层并用既有的锁定/冻结/关闭当前层拒绝；仅会话状态 |
| `LAYMRG` | 部分实现 | 选择式：选中对象的层合并到当前层（重写实体 + 删除源层），同一事务/一步撤销；当前层/0 层不能作源；无交互 Settings |
| `LAYOFF` | 已实现 | — |
| `LAYON` | 已实现 | — |
| `LAYOUT` | 部分实现 | `LIST`/`SET` 真实；`NEW/RENAME/DELETE` 真实写布局表（名称校验、重复名拒绝、删除级联 plot settings），一条事务/一步撤销；`COPY`/`TEMPLATE` 未实现 |
| `LAYOUTMERGE` | 已实现 | 把源布局的图纸空间实体移入目标并删除源布局（plot settings 级联），同一事务/一步撤销；同一布局拒绝 |
| `LAYOUTWIZARD` | 部分实现 | 真实创建布局并应用 Authored 纸张设置（一条事务/一步撤销）；无视口向导/打印设备 |
| `LAYTHW` | 部分实现 | 无选择输入（载荷即拒绝）；解冻全部冻结图层；不处理视口级冻结（本构建无视口模型）；图层表写入一条事务/一步撤销 |
| `LAYTRANS` | 部分实现 | 具名拒绝（`laytrans.unsupported`）：图层转换标准未实现。 |
| `LAYULK` | 部分实现 | 预选择输入（不弹 Select objects）；解锁选中对象所在的不同图层；图层表写入一条事务/一步撤销 |
| `LAYUNISO` | 已实现 | — |
| `LAYVPI` | 部分实现 | 全视口冻结 + **指定视口索引形式**；AutoCAD 的“除当前视口”需要当前视口标记，本构建不存储 |
| `LAYWALK` | 部分实现 | 逗号分隔层名；只显示命名层+当前层（会话覆盖，同 LAYISO 原子路径）；未知/空拒绝 |
| `LEADER` | 部分实现 | 同一共享引线计划（创建单个 MULTILEADER）；无引线样式对话框/块/公差注释 |
| `LENGTHEN` | 部分实现 | `DE`/`PER`/`TOT` 从**靠近拾取点**的端点改长度；`DY` 拖一端；`LINE`/`ARC`/开放多段线/椭圆弧/开放样条；闭合对象拒绝；开放样条**延长**未实现（`lengthen.spline_extend_unsupported`），缩短精确 |
| `LIGHT` | 部分实现 | 具名拒绝（`light.unsupported: 3D light objects are not modeled`）。 |
| `LIGHTLIST` | 部分实现 | 具名拒绝（`lightlist.unsupported: 3D light objects are not modeled`）。 |
| `LIGHTLISTCLOSE` | 部分实现 | 具名拒绝（`lightlistclose.unsupported: 3D light objects are not modeled`）。 |
| `LIMITS` | 部分实现 | 具名拒绝（`limits.unsupported`）：无图形界限模型。 |
| `LINE` | 已实现 | — |
| `LINETYPE` | 部分实现 | `LIST` 只读（Viewer 合法）；`SET <name>` 复用 `SETENTITYPROPERTY`、重新授权工作模式、一条事务/一步撤销；未知名 `property.unknown_linetype`；无线型表加载/新建/删除/改名 |
| `LIST` | 已实现 | 读取当前选择；报告模型/图纸/块空间、图层、draw order、颜色/线型/线宽/透明度/线型比例/可见性、文字样式/标注样式/块名与几何摘要；厚度/拉伸方向属未建模的全局范围 |
| `LIVESECTION` | 部分实现 | 具名拒绝（`livesection.unsupported: this build has no section-object model`）。 |
| `LOAD` | 部分实现 | 具名拒绝（`load.unsupported: this build has no SHX shape-file loader`）。 |
| `LOFT` | 部分实现 | 具名拒绝（`loft.unsupported: 3D loft is not implemented`）。 |
| `LOGFILEOFF` | 部分实现 | 会话令牌置关（`logfileoff.toggled`）；无日志文件。 |
| `LOGFILEON` | 部分实现 | 会话令牌置开（`logfileon.toggled`）；无日志文件。 |
| `LSP` | 部分实现 | 具名拒绝（`lsp.unsupported: this build has no AutoLISP runtime or compiler`）。 |
| `LSPSURF` | 部分实现 | 具名拒绝（`lspsurf.unsupported: this build has no AutoLISP runtime or compiler`）。 |
| `LTSCALE` | 部分实现 | 一个正有限小数；写内存 `$LTSCALE`（一次 revision，无撤销）；当前导出器不写该变量，SAVE→OPEN 不往返 |
| `LWEIGHT` | 部分实现 | 只读报告会话线宽显示令牌（默认 ON，`lweight.report`）；渲染器不消费该标志，LWEIGHT 对话框未实现。 |
| `MAKELISPAPP` | 部分实现 | 具名拒绝（`makelispapp.unsupported: this build has no AutoLISP runtime or compiler`）。 |
| `MARKUP` | 部分实现 | 具名拒绝（`markup.unsupported: this build has no collaboration markup/trace model`）。 |
| `MARKUPASSIST` | 部分实现 | 具名拒绝（`markupassist.unsupported: this build has no collaboration markup/trace model`）。 |
| `MARKUPCLOSE` | 部分实现 | 具名拒绝（`markupclose.unsupported: this build has no collaboration markup/trace model`）。 |
| `MARKUPIMPORT` | 部分实现 | 具名拒绝（`markupimport.unsupported: this build has no collaboration markup/trace model`）。 |
| `MASSPROP` | 部分实现 | 闭合圆/整椭圆/闭合折线 + **闭合 SPLINE（采样）** + **HATCH 环嵌套净面积**；样条/导入填充边界为近似（打印弦差/样本数）；无惯性矩/主轴/体积/密度 |
| `MATBROWSERCLOSE` | 部分实现 | 具名拒绝（`matbrowserclose.unsupported: this build has no material/texture model`）。 |
| `MATBROWSEROPEN` | 部分实现 | 具名拒绝（`matbrowseropen.unsupported: this build has no material/texture model`）。 |
| `MATCHCELL` | 部分实现 | **无单元格样式复制面**（`matchcell.unsupported`）。
| `MATCHPROP` | 已实现 | Basic 六项 + Settings 子集；Text 组复制文字样式与字体、Dimension 组复制标注样式、Hatch 组复制填充样式/图案/渐变；**不**复制字高、标注覆盖值与填充原点（与 AutoCAD 一致）；异类目标跳过该组并报告 |
| `MATEDITORCLOSE` | 部分实现 | 具名拒绝（`mateditorclose.unsupported: this build has no material/texture model`）。 |
| `MATEDITOROPEN` | 部分实现 | 具名拒绝（`mateditoropen.unsupported: this build has no material/texture model`）。 |
| `MATERIALASSIGN` | 部分实现 | 具名拒绝（`materialassign.unsupported: this build has no material/texture model`）。 |
| `MATERIALATTACH` | 部分实现 | 具名拒绝（`materialattach.unsupported: this build has no material/texture model`）。 |
| `MATERIALMAP` | 部分实现 | 具名拒绝（`materialmap.unsupported: this build has no material/texture model`）。 |
| `MATERIALS` | 部分实现 | 具名拒绝（`materials.unsupported: this build has no material/texture model`）。 |
| `MATERIALSCLOSE` | 部分实现 | 具名拒绝（`materialsclose.unsupported: this build has no material/texture model`）。 |
| `MEASUREGEOM` | 部分实现 | DISTANCE/ANGLE/AREA/POLYLINE 复用工具；**RADIUS 完成弧长/扫掠角/弦长/圆面积**；`VOLUME` 无 3D 体积模型，按名拒绝 |
| `MENU` | 部分实现 | 具名拒绝（`menu.unsupported`）：本构建无菜单文件模型，界面固定。 |
| `MESH` | 部分实现 | 具名拒绝（`mesh.unsupported: free-form mesh creation is not implemented`）。 |
| `MESHCAP` | 部分实现 | 具名拒绝（`meshcap.unsupported: mesh capping is not implemented`）。 |
| `MESHCOLLAPSE` | 部分实现 | 具名拒绝（`meshcollapse.unsupported: mesh collapse is not implemented`）。 |
| `MESHCREASE` | 部分实现 | 具名拒绝（`meshcrease.unsupported: mesh creasing is not implemented`）。 |
| `MESHEXTRUDE` | 部分实现 | 具名拒绝（`meshextrude.unsupported: mesh extrusion is not implemented`）。 |
| `MESHMERGE` | 部分实现 | 具名拒绝（`meshmerge.unsupported: mesh merge is not implemented`）。 |
| `MESHOPTIONS` | 部分实现 | 具名拒绝（`meshoptions.unsupported: no mesh tessellation options model`）。 |
| `MESHPRIMITIVEOPTIONS` | 部分实现 | 具名拒绝（`meshprimitiveoptions.unsupported: no mesh primitive options model`）。 |
| `MESHREFINE` | 部分实现 | 具名拒绝（`meshrefine.unsupported: mesh refinement is not implemented`）。 |
| `MESHSMOOTH` | 部分实现 | 具名拒绝（`meshsmooth.unsupported: mesh smoothing conversion is not implemented`）。 |
| `MESHSMOOTHLESS` | 部分实现 | 具名拒绝（`meshsmoothless.unsupported: no mesh smoothness level model`）。 |
| `MESHSMOOTHMORE` | 部分实现 | 具名拒绝（`meshsmoothmore.unsupported: no mesh smoothness level model`）。 |
| `MESHSPIN` | 部分实现 | 具名拒绝（`meshspin.unsupported: mesh edge spin is not implemented`）。 |
| `MESHSPLIT` | 部分实现 | 具名拒绝（`meshsplit.unsupported: mesh face split is not implemented`）。 |
| `MESHUNCREASE` | 部分实现 | 具名拒绝（`meshuncrease.unsupported: mesh uncreasing is not implemented`）。 |
| `MIGRATEMATERIALS` | 部分实现 | 具名拒绝（`migratematerials.unsupported: this build has no material/texture model`）。 |
| `MINSERT` | 已实现 | `MINSERT <block> <rows> <cols> <row_spacing> <col_spacing> <x> <y>`：矩形 INSERT 阵列，ARRAYRECT 单元规则 + INSERT `T(i)·T(-base)`，一条事务；独立副本（域无 MINSERT 实体，非关联） |
| `MIRROR` | 部分实现 | `MIRRTEXT=0` 可读镜像（位置镜像、文字保持可读）、`MIRROR ERASE YES/NO`、标注反射后重合成；**`MIRRTEXT=1` 字形镜像未实现**（文字实体无镜像标志） |
| `MIRROR3D` | 部分实现 | 具名拒绝（`mirror3d.unsupported: no 3D mirror-plane model`）。 |
| `MKLTYPE` | 部分实现 | 具名拒绝（`mkltype.unsupported: this build has no .lin definition-file writer`）。 |
| `MKSHAPE` | 部分实现 | 具名拒绝（`mkshape.unsupported: this build has no SHX shape compiler`）。 |
| `MLEADER` | 部分实现 | 引线路径类型直线/样条（样式表决定）、注释文本；2 顶点样条不可弯折（报告）；无 dogleg/attachment/角度约束覆盖 |
| `MLEADERALIGN` | 部分实现 | `LEFT/RIGHT/CENTER [spacing]` 对齐多条 MULTILEADER 的文字 X 或等距纵排；箭头锚定，其余顶点/根落点平移；一条事务；无 distribute/column 选项 |
| `MLEADERCOLLECT` | 部分实现 | 合并 ≥2 MULTILEADER 为一条（根/线拼接、文本以 `\P` 连接、首样式/平面保留、根落点重定向），删除源；一条事务；要求样式/覆盖掩码/平面一致且无保留数据 |
| `MLEADEREDIT` | 部分实现 | `ADD <x> <y>` 给根 0 增加一条引线（箭头到既有 dogleg 顶点）；`REMOVE <index>` 删除一条（不可清空根）；一条事务；无逐线覆盖/箭头编辑 |
| `MLEADERSTYLE` | 已实现 | `?`/`LIST`/`SET`/`NEW`/`RENAME`/`DELETE`/`MODIFY`；一条 mleader-style patch/一步撤销；`Standard` 不可删；新 MLEADER 解析当前样式；既有引线保留烘焙快照 |
| `MLEDIT` | 部分实现 | 两独立 LINE 的可表示子集：`MERGE/CLOSED/CROSS/TEE` 清理（把各自最近端点 trim/extend 到交点）、`OPEN` 在交点断开、`BREAK` 移除两条平行线之间的带；一条事务；导入的 `AcDbMline` 复合体按名拒绝；无 join/cap/fill/顶点模型（`mledit.residual`） |
| `MLINE` | 部分实现 | 恰好两条平行 `LINE`；无 join/cap/多段基线 |
| `MLSTYLE` | 部分实现 | 会话内最多 16 个命名样式，喂 MLINE；不持久化 |
| `MOCORO` | 部分实现 | `MOVE`→`COPY`→`ROTATE`→`SCALE` 链式复用既有变换漏斗；写入前预校验全部数值；一条事务/一步撤销；无命令内 `Undo`/交互 gizmo（全局 `UNDO` 等价） |
| `MODEL` | 已实现 | payload-free 选择子，复用 SPACE MODEL 的校验 + 自动取景；Model 标签页走同一命令 |
| `MOVE` | 已实现 | — |
| `MOVEBAK` | 部分实现 | 具名拒绝（`movebak.unsupported: the backup-folder location is owned by the host filesystem`）。 |
| `MPEDIT` | 部分实现 | 批量 `CLOSE`/`OPEN`/`REVERSE` + **LINE/ARC→POLYLINE 精确转换**；`JOIN`/`FIT`/`SPLINE`/`DECURVE` 委托 PEDIT；`WIDTH`/`LTYPE GEN` 仍拒绝；异质批按 PEDIT 选择规则拒绝 |
| `MREDO` | 部分实现 | 重做 N 步（默认 1），每步一条历史移动；空栈拒绝；不足时 `mredo.partial` |
| `MSLIDE` | 部分实现 | 具名拒绝（`mslide.unsupported: this build has no slide (SLD) writer`）。 |
| `MSPACE` | 部分实现 | 会话切换到模型空间，复用现有 `MODEL` 路径（仅会话状态）；已在模型空间时为具名 no-op。 |
| `MSTRETCH` | 部分实现 | 仅 payload 多窗口列表（无命令行语法）；复用 STRETCH planner 的窗口并集；一条事务/一步撤销；无多边形窗口 |
| `MTEDIT` | 部分实现 | `TEXTEDIT` 的别名（同一 `CommandId`/路径）；编辑当前单选的 Text/MText/MULTILEADER 字符串；无第二编辑路径 |
| `MTEXT` | 部分实现 | 两角宽度模型；无行距/分栏/背景填充 |
| `MULTIPLE` | 部分实现 | 设置会话 `MultipleRepeat` 标志；本宿主无重复循环，报告 `multiple.repeat_not_implemented`，不伪造重复 |
| `MVIEW` | 部分实现 | 矩形（2 点）+ **多边形（≥3 点）视口创建**；多边形 clip 数据真实但渲染受同一 3/4 点限制（`mview.polygon_not_renderable`）；无视口孔洞 |
| `MVSETUP` | 部分实现 | `MVSETUP <rows> <cols> <scale>` 在活动图纸布局建等分视口网格并设比例；非 AutoCAD Standard 配置；无 Align/Create/Options/title-block 提示 |
| `NAVBAR` | 部分实现 | 真实设置写入 `NAVBAR [ON\|OFF]`（持久化导航栏可见性）；无 SteeringWheels/ShowMotion/3Dconnexion。 |
| `NAVSMOTION` | 部分实现 | 具名拒绝（`navsmotion.unsupported: this build has no 3D navigation/ViewCube model`）。 |
| `NAVSMOTIONCLOSE` | 部分实现 | 具名拒绝（`navsmotionclose.unsupported: this build has no 3D navigation/ViewCube model`）。 |
| `NAVSWHEEL` | 部分实现 | 具名拒绝（`navswheel.unsupported: this build has no 3D navigation/ViewCube model`）。 |
| `NAVVCUBE` | 部分实现 | 真实设置写入 `NAVVCUBE [ON\|OFF]`（按当前 2D/3D 视图类别切显示位）；`PROPERTIES` 具名拒绝；无罗盘/边角目标/动画。 |
| `NCOPY` | 部分实现 | 真实编辑：选中块参照成员按世界变换复制到模型空间；一条事务/一步撤销；无嵌套选择界面。 |
| `NETLOAD` | 部分实现 | 具名拒绝（`netload.unsupported: no .NET application loader model`）。 |
| `NEW` | 已实现 | — |
| `NEWSHEETSET` | 部分实现 | 具名拒绝（`newsheetset.unsupported: this build has no sheet-set model`）。 |
| `NEWSHOT` | 部分实现 | 具名拒绝（`newshot.unsupported: this build has no ShowMotion named-view/motion model`）。 |
| `NEWVIEW` | 部分实现 | 带 `View` 载荷时转交既有命名视图存储；无载荷只报告缺少窗口/名称提示（`newview.needs_name`）。 |
| `OBJECTSCALE` | 部分实现 | 具名拒绝（`objectscale.unsupported`）：本构建无注释性对象模型。 |
| `OFFSET` | 已实现 | `Through`/`Erase`/`Layer`/`Multiple`(单步)/固定距离；椭圆与椭圆弧→**样条**；`OFFSETGAPTYPE` Sharp/Fillet/Chamfer（含闭合环全部 wrap 角）；闭合环自交拒绝；开放折线远端自交未检查（AutoCAD 未文档化） |
| `OFFSETEDGE` | 部分实现 | 具名拒绝（`offsetedge.unsupported: no 3D face/edge offset model`）。 |
| `OLECONVERT` | 部分实现 | 具名拒绝（`oleconvert.unsupported: this build has no OLE object storage model`）。 |
| `OLELINKS` | 部分实现 | 具名拒绝（`olelinks.unsupported: this build has no OLE object storage model`）。 |
| `OLEOPEN` | 部分实现 | 具名拒绝（`oleopen.unsupported: this build has no OLE object storage model`）。 |
| `OLERESET` | 部分实现 | 具名拒绝（`olereset.unsupported: this build has no OLE object storage model`）。 |
| `OLESCALE` | 部分实现 | 具名拒绝（`olescale.unsupported: this build has no OLE object storage model`）。 |
| `OOPS` | 部分实现 | v1 只恢复最近一次 ERASE（不覆盖 AutoCAD 的 BLOCK/WBLOCK 删除）；恢复实体用新 id；撤销恢复副本但不恢复快照 |
| `OPEN` | 已实现 | — |
| `OPENDWFMARKUP` | 部分实现 | 具名拒绝（`opendwfmarkup.unsupported: this build has no DWF markup model`）。 |
| `OPENFROMWEBMOBILE` | 部分实现 | 具名拒绝（`openfromwebmobile.unsupported: no Autodesk Account (web/mobile) file-exchange model`）。 |
| `OPENSHEETSET` | 部分实现 | 具名拒绝（`opensheetset.unsupported: no sheet-set (DST) model`）。 |
| `OPTIONS` | 已实现 | chrome 本地选择子：打开选项对话框 |
| `ORTHO` | 已实现 | — |
| `OSNAP` | 已实现 | — |
| `OVERKILL` | 已实现 | 容差（`0`=精确）、忽略 Color/Layer/Linetype/LTScale/Lineweight/Transparency、段优化与 `NOBREAK`、共线部分重叠/端到端合并、弧覆盖圆、重复段删除；保留最低 `(draw_order,id)`；一条事务/一步撤销 |
| `PAGESETUP` | 部分实现 | 裸命令只读报告每个布局的入库 `PLOTSETTINGS`；`PAGESETUP <layout> <paper> <width> <height> [<num>:<den>] [0|90|180|270]` 写入 Authored 记录，一条事务/一步撤销；无绘图仪/CTB/STB。 |
| `PAN` | 已实现 | — |
| `PARAMETERS` | 部分实现 | 具名拒绝（`parameters.unsupported: no dynamic-block parameter/lookup model; the Parameters Manager palette is not implemented`）。 |
| `PARAMETERSCLOSE` | 部分实现 | 具名拒绝（`parametersclose.unsupported: no dynamic-block parameter/lookup model; the Parameters Manager palette is not implemented`）。 |
| `PARTIALOAD` | 部分实现 | 具名拒绝（`partiaload.unsupported: no partial layer-index open/load model`）。 |
| `PARTIALOPEN` | 部分实现 | 具名拒绝（`partialopen.unsupported: no partial layer-index open/load model`）。 |
| `PASTEASHYPERLINK` | 部分实现 | 具名拒绝（`pasteashyperlink.unsupported: no hyperlink storage model`）。 |
| `PASTEBLOCK` | 部分实现 | 应用内剪贴板 → 自动命名块；不是 OS 剪贴板 |
| `PASTECLIP` | 部分实现 | 应用内剪贴板；不是 OS 剪贴板，不跨应用 |
| `PASTEORIG` | 部分实现 | 应用内剪贴板；不是 OS 剪贴板，不跨应用 |
| `PASTESPEC` | 部分实现 | 具名拒绝（`pastespec.unsupported: clipboard special-paste (OLE/format selection) is not modeled; use PASTECLIP or PASTEBLOCK`）。 |
| `PCEXTRACTCENTERLINE` | 部分实现 | 具名拒绝（`pcextractcenterline.unsupported: no point-cloud extraction model`）。 |
| `PCEXTRACTCORNER` | 部分实现 | 具名拒绝（`pcextractcorner.unsupported: no point-cloud extraction model`）。 |
| `PCEXTRACTEDGE` | 部分实现 | 具名拒绝（`pcextractedge.unsupported: no point-cloud extraction model`）。 |
| `PCEXTRACTSECTION` | 部分实现 | 具名拒绝（`pcextractsection.unsupported: no point-cloud extraction model`）。 |
| `PDFADJUST` | 部分实现 | 具名拒绝（`pdfadjust.unsupported: no PDF underlay adjustment model`）。 |
| `PDFATTACH` | 部分实现 | 具名拒绝（`pdfattach.unsupported: PDF underlays are not implemented`）。 |
| `PDFCLIP` | 部分实现 | 具名拒绝（`pdfclip.unsupported: no PDF underlay clip model`）。 |
| `PDFIMPORT` | 部分实现 | 具名拒绝（`pdfimport.unsupported: PDF geometry import is not implemented`）。 |
| `PDFLAYERS` | 部分实现 | 具名拒绝（`pdflayers.unsupported: no PDF underlay layer model`）。 |
| `PDFSHXTEXT` | 部分实现 | 具名拒绝（`pdfshxtext.unsupported: PDF SHX text conversion is not implemented`）。 |
| `PEDIT` | 部分实现 | `Close`/`Open`/`Join`/`Fit`/`Spline`/`Decurve`/`Reverse`/`Undo` + LINE/ARC/SPLINE 转换；本轮新增 **Edit vertex 子菜单**（NEXT/PREVIOUS/INSERT/MOVE/STRAIGHTEN；TANGENT/WIDTH 拒绝）；`Width`/`Ltype gen` 未建模 |
| `PERFANALYZER` | 部分实现 | chrome 本地选择子：非模态性能面板，只读显示宿主 `cad_status.performance` 与文档/会话计数；缺失指标显示 N/A（不伪造 0），无命令录制/采样。 |
| `PERFANALYZERCLOSE` | 已实现 | chrome 本地选择子：关闭性能分析浮窗（幂等成功）。 |
| `PFACE` | 部分实现 | 具名拒绝（`pface.unsupported: no 3DFACE/mesh entity model`）。 |
| `PLAN` | 部分实现 | payload-free 选择子：Top/plan 2D 视图；无 UCS/World/Current 选项（本构建无 UCS 模型） |
| `PLANESURF` | 部分实现 | 具名拒绝（`planesurf.unsupported: no planar-surface entity model`）。 |
| `PLINE` | 已实现 | `Geometry(Polyline)` 现在接受 **bulge 弧段**与 **Close**（含隐式闭合段），逐位保留顶点/凸度；`Width`/交互逐顶点 `Undo` 未建模（全局范围） |
| `PLOT` | 部分实现 | 仅矢量 SVG/PDF；无 CTB/STB、无批量多布局 |
| `PLOTSTAMP` | 部分实现 | 具名拒绝（`plotstamp.unsupported`）：本构建无打印戳记模型。 |
| `PLOTSTYLE` | 部分实现 | 具名拒绝（`plotstyle.unsupported`）：命名打印样式未实现（无 CTB/STB 表）。 |
| `PLOTTERMANAGER` | 部分实现 | 具名拒绝（`plottermanager.unsupported`）：本构建无绘图仪配置模型。 |
| `PLT2DWG` | 部分实现 | 具名拒绝（`plt2dwg.unsupported: HPGL (PLT) import is not implemented`）。 |
| `PNGOUT` | 部分实现 | 具名拒绝（`pngout.unsupported: raster PNG export is not implemented; use PLOT for vector SVG/PDF`）。 |
| `POINT` | 已实现 | 单点 + **payload 多点 `Multiple`**（n 个 `AcDbPoint`，一条事务/一步撤销）；交互重复捕获（Exact(1)）未接线 |
| `POINTCLOUDATTACH` | 部分实现 | 具名拒绝（`pointcloudattach.unsupported: no point-cloud attachment/crop/style model`）。 |
| `POINTCLOUDCOLORMAP` | 部分实现 | 具名拒绝（`pointcloudcolormap.unsupported: no point-cloud attachment/crop/style model`）。 |
| `POINTCLOUDCROP` | 部分实现 | 具名拒绝（`pointcloudcrop.unsupported: no point-cloud attachment/crop/style model`）。 |
| `POINTCLOUDCROPSTATE` | 部分实现 | 具名拒绝（`pointcloudcropstate.unsupported: no point-cloud attachment/crop/style model`）。 |
| `POINTCLOUDMANAGER` | 部分实现 | 具名拒绝（`pointcloudmanager.unsupported: no point-cloud attachment/crop/style model`）。 |
| `POINTCLOUDMANAGERCLOSE` | 部分实现 | 具名拒绝（`pointcloudmanagerclose.unsupported: no point-cloud attachment/crop/style model`）。 |
| `POINTCLOUDSTYLIZE` | 部分实现 | 具名拒绝（`pointcloudstylize.unsupported: no point-cloud attachment/crop/style model`）。 |
| `POINTCLOUDUNCROP` | 部分实现 | 具名拒绝（`pointclouduncrop.unsupported: no point-cloud attachment/crop/style model`）。 |
| `POINTLIGHT` | 部分实现 | 具名拒绝（`pointlight.unsupported: no 3D light model`）。 |
| `POLYGON` | 已实现 | 默认内接 + 外切多词选择子；`Edge` 法与数字半径无输入通道；边数取会话 `POLYSIDES` |
| `POLYGON CIRCUMSCRIBED` | 已实现 | **本项目多词选择子**（不属于官方 981 条，不计入上方总计）：`POLYGON` 的 `Circumscribed about circle`（`C`）构造；第二点落在**内切（相切）圆**上，拾取方向为第一条边中点，`R = r / cos(π/n)`；共享会话 `POLYSIDES`；无 Edge/数字半径定向 |
| `POLYSOLID` | 部分实现 | 具名拒绝（`polysolid.unsupported: 3D solid primitives are not implemented`）。 |
| `PRESSPULL` | 部分实现 | 具名拒绝（`presspull.unsupported: no press/pull model`）。 |
| `PREVIEW` | 部分实现 | 具名拒绝（`preview.unsupported`）：本构建无打印预览窗口。 |
| `PROJECTGEOMETRY` | 部分实现 | 具名拒绝（`projectgeometry.unsupported: no 3D solid edge/geometry projection model`）。 |
| `PROPERTIES` | 已实现 | chrome 本地选择子：打开特性面板 |
| `PROPERTIESCLOSE` | 已实现 | chrome 本地选择子：关闭特性面板（面板关闭按钮走同一命令） |
| `PROPULATE` | 部分实现 | 具名拒绝（`propulate.unsupported`）：本构建无特性传播模型。 |
| `PSBSCALE` | 部分实现 | 按活动视口 paper-per-model 因子缩放选定图纸空间块参照（插入点固定）；单方向/单视口是声明规则；无像素证据 |
| `PSETUPIN` | 部分实现 | 从同一图纸的源布局复制已存 `PLOTSETTINGS` 到目标布局，一条事务/一步撤销；跨文件/无源记录拒绝。 |
| `PSOUT` | 部分实现 | 具名拒绝（`psout.unsupported: PostScript output is not implemented; use PLOT for SVG/PDF`）。 |
| `PSPACE` | 部分实现 | 会话切换到第一个图纸布局，复用现有 `SPACE` 路径；无图纸布局时报 `pspace.no_layout`。 |
| `PSTSCALE` | 部分实现 | 按同一因子缩放图纸空间 TEXT/MTEXT 存储字高（及 MTEXT wrap 宽度）；单方向/单视口；不处理 MULTILEADER/TABLE 文本 |
| `PTYPE` | 已实现 | `PTYPE <code> [size]` 委托既有 `PDMODE`/`PDSIZE` 头部写入（两次 header 写、无 header undo 补丁）；bare 为只读报告 |
| `PUBLISH` | 部分实现 | 具名拒绝（`publish.unsupported`）：本构建无批量发布模型。 |
| `PURGE` | 部分实现 | 只清未使用的**图层**（复用 `DELETELAYER` 的引用拒绝）；BLOCK/STYLE/LINETYPE 无删除 API，按名拒绝；PURGE 的层删除仍**有意不记历史**（其他图层写已可撤销） |
| `PURGEAECDATA` | 部分实现 | 只读能力报告：本构建不导入/建模 AEC 对象，无 AEC 数据可清理（`purgeaecdata.no_aec_model`）；不写库。 |
| `PUSHTODOCSCLOSE` | 部分实现 | 具名拒绝（`pushtodocsclose.unsupported: no Autodesk Docs connector model`）。 |
| `PUSHTODOCSOPEN` | 部分实现 | 具名拒绝（`pushtodocsopen.unsupported: no Autodesk Docs connector model`）。 |
| `PYRAMID` | 部分实现 | 具名拒绝（`pyramid.unsupported: 3D solid primitives are not implemented`）。 |
| `QDIM` | 已实现 | `Continuous`/`Staggered`/`Baseline`/`Ordinate`/`Radius`/`Diameter` + `Datum Point`/`Edit`/`Settings`；Staggered 步长 = `DIMDLI`（声明）；Ordinate 基准默认 `(0,0,0)`；不支持项逐条 `qdim.skipped`；一条事务/一步撤销；角度不是 QDIM 模式 |
| `QLATTACH` | 部分实现 | 具名拒绝（`qlattach.unsupported`）：本构建无 QLEADER 注释附着模型。 |
| `QLATTACHSET` | 部分实现 | 具名拒绝（`qlattachset.unsupported`）：本构建无 QLEADER 注释附着模型。 |
| `QLDETACHSET` | 部分实现 | 具名拒绝（`qldetchset.unsupported`）：本构建无 QLEADER 注释附着模型。 |
| `QLEADER` | 部分实现 | 与 LEADER/MLEADER 共享引线选项计划：直线/样条引线、注释类型 text、箭头开；箭头关/最大点数按名拒绝；无设置对话框/块/公差注释 |
| `QNEW` | 部分实现 | 具名拒绝（`qnew.single_document`）：本构建只有一个文档；请用 NEW。 |
| `QQUIT` | 部分实现 | 具名拒绝（`qquit.unsupported`）：应用退出由宿主负责。 |
| `QSAVE` | 部分实现 | SAVE 别名：有损 DXF 另存；源为 .dwg 时可实验性导出 DWG（新文件、默认 <stem>-export.dwg、绝不覆盖源；仅合成 self-roundtrip 证据，vendor/AutoCAD NOT VERIFIED） |
| `QSELECT` | 部分实现 | `Equals`/`Not Equal`/`>`/`<`/`>=`/`<=`/通配符/`Select All`；整图（当前空间）或当前选择；`Include`/`Exclude`/`Append`；嵌套块不参与；只读；**分类/自定义对象特性未建模**，属性集为基础子集 |
| `QTEXT` | 部分实现 | 会话令牌 `qtext`；仍按几何绘制文字，不显示占位框。 |
| `QUICKCUI` | 部分实现 | 具名拒绝（`quickcui.unsupported: no CUI (menu-customization) model`）。 |
| `QUICKPROPERTIES` | 部分实现 | chrome 本地选择子：打开现有特性面板；紧凑布局像 PROPERTIES 一样拒绝。 |
| `QUIT` | 部分实现 | 具名拒绝（`quit.unsupported`）：应用退出由宿主负责。 |
| `RAY` | 部分实现 | 两点创建，但落库为裁剪到模型包围盒的有限线段 |
| `RECOVER` | 部分实现 | 只读：对当前文档逐实体跑 `validate_entity`（`recover.summary`/`recover.invalid_entity`）；文件级 DWG 修复/重新打开由宿主负责（`recover.file_level_not_modeled`）；不写库/不升修订/不入历史。 |
| `RECOVERALL` | 部分实现 | 只读：同一完整性扫描 + `recoverall.single_document`（本构建单文档，无多文档/外部参照批处理）；不写库。 |
| `RECTANG` | 已实现 | 两角→闭合四顶点 LWPOLYLINE；`payload_length` 守卫；`Fillet`/`Chamfer`/`Rotation`/`Area` 无输入通道（payload 仅两角），矩形可由 `PLINE` 表达 |
| `REDIR` | 部分实现 | 具名拒绝（`redir.unsupported: no xref/image path-redirection model`）。 |
| `REDIRMODE` | 部分实现 | 具名拒绝（`redirmode.unsupported: no xref/image path-redirection model`）。 |
| `REDO` | 已实现 | — |
| `REDRAW` | 部分实现 | 当前视口刷新（session generation）；不改相机、不写库、不进 `ZOOM PREVIOUS` |
| `REDRAWALL` | 部分实现 | 所有视口刷新；同 REDRAW 的只读边界 |
|  `REFCLOSE` | 部分实现 | 真实：`REFCLOSE SAVE|DISCARD`（裸命令 `refclose.needs_option`）；SAVE 逆变换写回块定义并删除克隆（一条事务/一步撤销），DISCARD 删除克隆且块定义不变。 |
|  `REFEDIT` | 部分实现 | 真实：`REFEDIT`/`-REFEDIT` 仅接受一个相似变换模型空间 INSERT，把可见块成员按 INSERT 变换克隆到模型空间（一条事务），源 INSERT 走会话隐藏；`REFSET ADD|REMOVE` 只改会话工作集；`REFCLOSE SAVE` 逆变换写回块定义（成员更新/新增/删除与克隆删除，一条事务/一步撤销），`DISCARD` 删除克隆、块定义不变。仅提取可见成员；无参照淡显/锁定（其他宿主对象仍可选，REFSET 是显式选择）；成员图层/绘图次序不回写；QSAVE 应用层拒绝（`refedit.save_blocked`）。 |
|  `REFSET` | 部分实现 | 真实：`REFSET ADD|REMOVE`（裸命令=ADD）把当前选择加入/移出 in-place 块编辑工作集；会话状态，不写图；无活动会话 `refedit.no_session`、空选择 `refedit.selection_empty`；自引用实例拒绝。 |
| `REGEN` | 部分实现 | 推进当前视口 regeneration stamp，重建派生显示缓存；不写库 |
| `REGEN3` | 部分实现 | `REGEN` 的别名（同一 `CommandId`/路径），无第二套实现；不修复 3D 实体显示异常 |
| `REGENALL` | 部分实现 | 所有视口 regeneration；不写库 |
| `REGENAUTO` | 部分实现 | 会话令牌 `regenauto`；本构建无自动重生成开关，需显式 REGEN。 |
| `REGION` | 部分实现 | 具名拒绝（`region.unsupported`）：无面域几何模型。 |
| `REINIT` | 部分实现 | 具名拒绝（`reinit.unsupported`）：本构建无设备/数字化仪初始化模型。 |
| `RENAME` | 部分实现 | `RENAME <kind> <old> <new>`：`LAYER`/`STYLE`/`DIMSTYLE`/`MLEADERSTYLE`/`BLOCK` 真实写；一条表 patch/一步撤销；`LINETYPE` 仍无 rename API（具名拒绝） |
| `RENDER` | 部分实现 | 具名拒绝（`render.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| `RENDERCROP` | 部分实现 | 具名拒绝（`rendercrop.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| `RENDERENVIRONMENT` | 部分实现 | 具名拒绝（`renderenvironment.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| `RENDERENVIRONMENTCLOSE` | 部分实现 | 具名拒绝（`renderenvironmentclose.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| `RENDEREXPOSURE` | 部分实现 | 具名拒绝（`renderexposure.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| `RENDEREXPOSURECLOSE` | 部分实现 | 具名拒绝（`renderexposureclose.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| `RENDERONLINE` | 部分实现 | 具名拒绝（`renderonline.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| `RENDERPRESETS` | 部分实现 | 具名拒绝（`renderpresets.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| `RENDERPRESETSCLOSE` | 部分实现 | 具名拒绝（`renderpresetsclose.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| `RENDERWIN` | 部分实现 | 具名拒绝（`renderwin.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| `RENDERWINDOW` | 部分实现 | 具名拒绝（`renderwindow.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| `RENDERWINDOWCLOSE` | 部分实现 | 具名拒绝（`renderwindowclose.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| `REPURLS` | 部分实现 | 具名拒绝（`repurls.unsupported: no hyperlink storage model`）。 |
| `RESETBLOCK` | 部分实现 | 具名拒绝（`resetblock.unsupported`）：本构建无动态块参数重置 API。 |
| `RESUME` | 部分实现 | 具名拒绝（`resume.unsupported: no script-file engine`）。 |
| `REVCLOUD` | 部分实现 | `Object`/`Rectangular`/`Polygonal`/`Freehand`/`Arc Length`/`Style`/`Modify`；弧长按 payload → 会话 `REVCLOUDPROPERTIES` token → 5% 替代顺序；`Calligraphy` 未建模 |
| `REVCLOUDPROPERTIES` | 已实现 | `REVCLOUDPROPERTIES <arc-length>`（有限 > 0）写会话 token，`REVCLOUD` 真实消费并改变弧长；bare 为只读报告 |
| `REVERSE` | 部分实现 | 仅 LINE/LWPOLYLINE/SPLINE，当前选择为全部输入；无交互拾取提示，无螺旋/3D 变体 |
| `REVERT` | 部分实现 | 具名拒绝（`revert.unsupported`）：本构建无重新打开源文件的路径。 |
| `REVOLVE` | 部分实现 | 具名拒绝（`revolve.unsupported: 3D revolution is not implemented`）。 |
| `REVSURF` | 部分实现 | 具名拒绝（`revsurf.unsupported: no 3D surface (NURBS) model`）。 |
| `RIBBON` | 已实现 | chrome 本地选择子：显示功能区（set，不 toggle）；不进入 `cad_command` |
| `RIBBONCLOSE` | 已实现 | chrome 本地选择子：隐藏功能区 |
| `ROTATE` | 已实现 | — |
| `ROTATE3D` | 部分实现 | 具名拒绝（`rotate3d.unsupported: no 3D rotate-axis model`）。 |
| `RPREF` | 部分实现 | 具名拒绝（`rpref.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| `RPREFCLOSE` | 部分实现 | 具名拒绝（`rprefclose.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| `RSCRIPT` | 部分实现 | 具名拒绝（`rscript.unsupported: no script-file engine`）。 |
| `RTEDIT` | 部分实现 | 具名拒绝（`rtedit.unsupported`）：远程文字未实现（无 RTEXT 模型）。 |
| `RTEXT` | 部分实现 | 具名拒绝（`rtext.unsupported`）：远程文字未实现（无 RTEXT 模型）。 |
| `RTUCS` | 部分实现 | 具名拒绝（`rtucs.unsupported: no dynamic UCS model`）。 |
| `RULESURF` | 部分实现 | 具名拒绝（`rulesurf.unsupported: no 3D surface (NURBS) model`）。 |
| `SAVE` | 部分实现 | 有损 DXF 另存；源为 `.dwg` 时可**实验性导出 DWG**（上游 `acadrust::DwgWriter`，新文件、默认 `<stem>-export.dwg`、绝不覆盖源）；DWG 导出有损/不完整，仅合成 self-roundtrip 证据，vendor/AutoCAD **NOT VERIFIED** |
| `SAVEALL` | 部分实现 | 具名拒绝（`saveall.single_document`）：本构建只有一个文档；请用 SAVE。 |
| `SAVEAS` | 部分实现 | SAVE 别名：有损 DXF 另存；源为 .dwg 时可实验性导出 DWG（新文件、默认 <stem>-export.dwg、绝不覆盖源；仅合成 self-roundtrip 证据，vendor/AutoCAD NOT VERIFIED） |
| `SAVEIMG` | 部分实现 | 具名拒绝（`saveimg.unsupported: raster image export is host-owned; this command layer has no file handle (use PLOT PNG)`）。 |
| `SAVETOWEBMOBILE` | 部分实现 | 具名拒绝（`savetowebmobile.unsupported: no Autodesk Account (web/mobile) file-exchange model`）。 |
| `SCALE` | 已实现 | — |
| `SCALELISTEDIT` | 部分实现 | 只读：第一个图纸布局的页面 + `scalelistedit.write_unsupported`；无比例列表存储。 |
| `SCALETEXT` | 已实现 | — |
| `SCRIPT` | 部分实现 | 具名拒绝（`script.unsupported: no script-file runner`）。 |
| `SCRIPTCALL` | 部分实现 | 具名拒绝（`scriptcall.unsupported: no script-file engine`）。 |
| `SECTION` | 部分实现 | 具名拒绝（`section.unsupported: no 3D section-plane model`）。 |
| `SECTIONPLANE` | 部分实现 | 具名拒绝（`sectionplane.unsupported: no 3D section-plane model`）。 |
| `SECTIONPLANEJOG` | 部分实现 | 具名拒绝（`sectionplanejog.unsupported: no 3D section-plane model`）。 |
| `SECTIONPLANESETTINGS` | 部分实现 | 具名拒绝（`sectionplanesettings.unsupported: no 3D section-plane model`）。 |
| `SECTIONPLANETOBLOCK` | 部分实现 | 具名拒绝（`sectionplanetoblock.unsupported: no 3D section-plane model`）。 |
| `SECTIONSPINNERS` | 部分实现 | 具名拒绝（`sectionspinners.unsupported: no 3D section-plane model`）。 |
| `SECURITYOPTIONS` | 部分实现 | 具名拒绝（`securityoptions.unsupported: no security/executable-trust settings model`）。 |
| `SELECT` | 部分实现 | 以选择载荷设置会话选择；不是 AutoCAD 的交互式 SELECT |
| `SELECTCOUNT` | 部分实现 | 只读统计与第一个选中对象五特性相同的模型空间对象数；不改选择/无事务 |
| `SELECTSIMILAR` | 已实现 | 对象类型强制；默认**图层+类型**；Color/Linetype/Lineweight/Name/Object style 独立开关；透明**不**参与；`Name` 按**根块参照名**匹配展开成员；模型/图纸空间各自匹配；只读 |
| `SETBYLAYER` | 已实现 | 颜色/线型/线宽/透明度复位随层；Include blocks 递归块成员；锁定图层拒绝；材质/命名打印样式属未建模的全局范围（AutoCAD 的 SETBYLAYER 本就不含文字/标注/填充样式） |
| `SETVAR` | 部分实现 | 读/写闭集 `PDMODE/PDSIZE/SNAPUNIT/LTSCALE/GRID/SNAP/ORTHO/TEXTSIZE`；写复用既有命令路径；另可写 `NAVVCUBEDISPLAY/DISPLAYVIEWCUBEIN2D/DISPLAYVIEWCUBEIN3D`（主机外观设置，读形式为 `setvar.write_only_variable`）；未知/越界按名拒绝 |
| `SHADEMODE` | 部分实现 | 具名拒绝（`shademode.unsupported: no 3D shaded display mode; VSCURRENT owns the 2D display token`）。 |
| `SHARE` | 部分实现 | 具名拒绝（`share.unsupported: no Autodesk cloud sharing/view model`）。 |
| `SHAREDVIEWS` | 部分实现 | 具名拒绝（`sharedviews.unsupported: no Autodesk cloud sharing/view model`）。 |
| `SHAREDVIEWSCLOSE` | 部分实现 | 具名拒绝（`sharedviewsclose.unsupported: no Autodesk cloud sharing/view model`）。 |
| `SHAREVIEW` | 部分实现 | 具名拒绝（`shareview.unsupported: no Autodesk cloud sharing/view model`）。 |
| `SHEETSET` | 部分实现 | 具名拒绝（`sheetset.unsupported: no sheet-set (DST) model`）。 |
| `SHEETSETHIDE` | 部分实现 | 具名拒绝（`sheetsethide.unsupported: no sheet-set (DST) model`）。 |
| `SHELL` | 部分实现 | 具名拒绝（`shell.unsupported: the host owns process spawning; this command layer has no OS shell`）。 |
| `SHOWPALETTES` | 已实现 | chrome 本地选择子：恢复功能区+选项板+文件标签 |
| `SHOWRENDERGALLERY` | 部分实现 | 具名拒绝（`showrendergallery.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| `SHOWURLS` | 部分实现 | 只读：本数据库不保留超链接/URL 数据（`showurls.not_retained`），不虚构 URL 行、不写库。 |
| `SIGVALIDATE` | 部分实现 | 具名拒绝（`sigvalidate.unsupported`）：无数字签名模型。 |
| `SKETCH` | 部分实现 | 仅 `CommandPayload::Points`：一条活动层开放 LWPOLYLINE；两点以上且有限；一条事务/一步撤销；交互式定时采样 NOT IMPLEMENTED |
| `SLICE` | 部分实现 | 具名拒绝（`slice.unsupported: no 3D slicing model`）。 |
| `SNAP` | 已实现 | — |
| `SOLDRAW` | 部分实现 | 具名拒绝（`soldraw.unsupported: no 3D solid-to-2D drawing view model`）。 |
| `SOLID` | 部分实现 | 3-4 个 `x,y` 点 → 两三角 `Mesh`（`AcDbSolid`）；DXF 导出写 polyface mesh，不是 SOLID 记录 |
| `SOLIDEDIT` | 部分实现 | 具名拒绝（`solidedit.unsupported: no 3D solid-history/brep model`）。 |
| `SOLPROF` | 部分实现 | 具名拒绝（`solprof.unsupported: no 3D solid-to-2D drawing view model`）。 |
| `SOLVIEW` | 部分实现 | 具名拒绝（`solview.unsupported: no 3D solid-to-2D drawing view model`）。 |
| `SPACETRANS` | 已实现 | `SPACETRANS <distance>` 用活动视口 `model_to_paper` 因子做模型↔图纸换算并报告方向/因子；模型空间/无视口按名拒绝 |
| `SPELL` | 部分实现 | 具名拒绝（`spell.unsupported`）：无拼写检查词典/模型。 |
| `SPHERE` | 部分实现 | 具名拒绝（`sphere.unsupported: 3D solid primitives are not implemented`）。 |
| `SPLINE` | 已实现 | fit 点法 + **CV 法**（degree 2/3，显式或简写 knots/weights）；`Close`/`Tangency`/`Knots` 参数化/`Object` 未建模 |
| `SPLINEDIT` | 部分实现 | 具名拒绝（`splinedit.unsupported`）：无 NURBS 编辑面。 |
| `SPOTLIGHT` | 部分实现 | 具名拒绝（`spotlight.unsupported: no 3D light model`）。 |
| `SSX` | 部分实现 | Express **筛选表构建器**（不是 SELECTSIMILAR）：`BLOCK`/`COLOR`/`ENTITY`/`LAYER`/`LTYPE`/`STYLE`/`PICK`/`CLEAR`，单值覆盖/移除；模板来自当前选择；仅当前空间；替换选择；只读；**`FLAG`/`THICKNESS`/`VECTOR` 未建模**（实体标志位/厚度/挤出）⇒ 具名拒绝 |
| `STANDARDS` | 部分实现 | 具名拒绝（`standards.unsupported: no CAD standards (DWS) model`）。 |
| `STATUS` | 部分实现 | 图形/数据库/修订/实体/图层/块/样式/布局/范围/模式；本轮新增 **current_space/space_usage/layer_usage/modes/selection**；创建/编辑时间、内存、limits/base 明确 `status.not_tracked` |
| `STLOUT` | 部分实现 | 具名拒绝（`stlout.unsupported: STL export is not implemented`）。 |
| `STRETCH` | 部分实现 | 交叉矩形与多边形/lasso payload；**IMAGE/MASK/TABLE 新增移动型分类**；圆按圆心、块/文字按插入点；`MULTILEADER` 整命令拒绝；多边形/lasso 交互捕获未接线 |
| `STYLE` | 已实现 | `NEW <name> [<base>]`/`RENAME <old> <new>`/`DELETE <name>`/`SET <name> <FONT/KEYS/ADDKEY/REMOVEKEY> <value>`/`?`；一条 style-table patch/一步撤销；域 `Style` 只有 name/resource_keys，故只可写这两类字段；无“当前文字样式”会话变量 |
| `STYLESMANAGER` | 部分实现 | 具名拒绝（`stylesmanager.unsupported`）：本构建无 CTB/STB 打印样式模型。 |
| `SUBTRACT` | 部分实现 | 具名拒绝（`subtract.unsupported: no 3D boolean model`）。 |
| `SUNPROPERTIES` | 部分实现 | 具名拒绝（`sunproperties.unsupported: no sun/sky lighting model`）。 |
| `SUNPROPERTIESCLOSE` | 部分实现 | 具名拒绝（`sunpropertiesclose.unsupported: no sun/sky lighting model`）。 |
| `SUPERHATCH` | 部分实现 | 具名拒绝（`superhatch.unsupported`）：基于图像/块的超级填充未实现。 |
| `SURFBLEND` | 部分实现 | 具名拒绝（`surfblend.unsupported: surface blending is not implemented`）。 |
| `SURFEXTEND` | 部分实现 | 具名拒绝（`surfextend.unsupported: surface extension is not implemented`）。 |
| `SURFEXTRACTCURVE` | 部分实现 | 具名拒绝（`surfextractcurve.unsupported: surface curve extraction is not implemented`）。 |
| `SURFFILLET` | 部分实现 | 具名拒绝（`surffillet.unsupported: surface filleting is not implemented`）。 |
| `SURFNETWORK` | 部分实现 | 具名拒绝（`surfnetwork.unsupported: surface network creation is not implemented`）。 |
| `SURFOFFSET` | 部分实现 | 具名拒绝（`surfoffset.unsupported: surface offset is not implemented`）。 |
| `SURFPATCH` | 部分实现 | 具名拒绝（`surfpatch.unsupported: surface patching is not implemented`）。 |
| `SURFSCULPT` | 部分实现 | 具名拒绝（`surfsculpt.unsupported: surface sculpting is not implemented`）。 |
| `SURFTRIM` | 部分实现 | 具名拒绝（`surftrim.unsupported: surface trimming is not implemented`）。 |
| `SURFUNTRIM` | 部分实现 | 具名拒绝（`surfuntrim.unsupported: surface untrimming is not implemented`）。 |
| `SWEEP` | 部分实现 | 具名拒绝（`sweep.unsupported: 3D sweep is not implemented`）。 |
| `SYSVARMONITOR` | 部分实现 | `ADD/REMOVE <name>`、`LIST [<name>]`、bare=LIST；会话监视集 + 查询时快照变化检测；无后台通知、不持久化；仅 SETVAR 闭集变量 |
| `SYSVDLG` | 部分实现 | 只读报告所有已暴露的会话变量当前值 + `sysvdlg.not_tracked`；无编辑/保存/恢复对话框 |
| `SYSWINDOWS` | 部分实现 | 具名拒绝（`syswindows.unsupported`）：应用窗口布局由宿主负责。 |
| `TABLE` | 部分实现 | **导入的 ACAD_TABLE 已建模**：行/列/单元格/合并/文字/边框，可 MOVE/ROTATE/SCALE（相似变换；+Z 表格文字随旋转）、可拾取/显示、FIND 逐单元格报告/替换、真实 DXF 导出（块内表格带 INSERT 变换）；导入文字高度/样式按 `cell→row→base→TableStyle` 解析；创建真实：`TABLE <rows> <cols> <col_width> <row_height> <x> <y>`（空单元格/Standard、+X/+Z、一条事务/一步撤销）；单元格块/字段内容、边框颜色/线宽/填充、对齐/逐格旋转、TABLESTYLE 名称不往返均为声明损失；无对话框/单元格样式/创建时合并。
| `TABLEDIT` | 部分实现 | 真实单元格文本编辑：`TABLEDIT <row> <col> [text]`（1-based；选中一个表格；剩余整行作为文本，空文本清除），FIND 可见；一条事务/一步撤销；无样式/合并/块内容编辑。
| `TABLEEXPORT` | 部分实现 | **无 CSV/文件写出**（`tableexport.unsupported`）；表格单元格数据已在模型里。
| `TABLESTYLE` | 部分实现 | **无表格样式表**（`tablestyle.unsupported`）；导入时只按样式解析边框可见性/双线。
| `TABLET` | 部分实现 | 具名拒绝（`tablet.unsupported`）：本构建无数字化仪模型。 |
| `TABSURF` | 部分实现 | 具名拒绝（`tabsurf.unsupported: no 3D surface (NURBS) model`）。 |
| `TARGETPOINT` | 部分实现 | 具名拒绝（`targetpoint.unsupported: no 3D light model`）。 |
| `TASKBAR` | 部分实现 | 具名拒绝（`taskbar.unsupported`）：应用窗口装饰由宿主负责。 |
| `TCASE` | 部分实现 | Express 工具、选择式：只改 Text/MText 的可见字符大小写；裸命令默认 Title，键入 `TCASE UPPER/LOWER/TITLE/SENTENCE/TOGGLE`（或 `U/L/T/S/G`）；MTEXT 格式转义逐字保留；属性/标注文字未建模（`tcase.not_reset`）；MULTILEADER 走 `with_text` |
| `TCIRCLE` | 部分实现 | `TCIRCLE CIRCLE\ |
| `TCOUNT` | 部分实现 | `TCOUNT <start> <step> [prefix] [suffix]`；按上→下/左→右顺序把每个选中 Text/MText 替换为 `prefix+编号+suffix`；`step` 必须 >0；AutoCAD 的逐项前缀/后缀对话框模式未实现 |
| `TEXT` | 已实现 | — |
|  `TEXTALIGN` | 部分实现 | 15 个对齐拼写（6 个单轴 + 9 个组合，`TEXTALIGN TOPLEFT` 等）按选择并集包围盒在两轴对齐；`DISTRIBUTE [X|Y]` 按阅读序等间隙分布（首尾固定、≥3 个目标，`textalign.distribute_needs_three`）；`SETSPACING`/`CURRENTVERTICAL`/`CURRENTHORIZONTAL` 解析后具名拒绝；只移动参照点，不改内容/字高/旋转/对齐；一条事务/一步撤销；宽高仍为估算值。 |
| `TEXTEDIT` | 部分实现 | 整串替换；无光标级编辑、不解析 MTEXT 标记 |
| `TEXTFIT` | 部分实现 | TEXT/MTEXT 高度拟合到目标宽度，其余字段逐位保留；真实字形宽度拉伸未实现（域无 width factor） |
| `TEXTMASK` | 部分实现 | 偏移（默认 0.35）/Wipeout 遮罩/`*TEXTMASK-*` 会话组/重跑替换/组上限/有限边界守卫；`3dface`/`solid` 遮罩类型无实体模型，按名拒绝 |
| `TEXTSCR` | 部分实现 | 置会话 `text_screen`=true；无文本屏幕窗口、不显示提示历史。 |
| `TEXTTOFRONT` | 部分实现 | 选择式；仅模型空间 Text/MultiLeader/Dimension 上移到所有非合格对象之前；其它选中对象逐条诊断跳过；图纸空间/块内不重排；“全图文字”形式与 Draw Order 对话框未实现 |
| `TEXTUNMASK` | 部分实现 | 组优先删除 + 估计外框启发；清理已擦除文字留下的陈旧组；未分组遮罩来源不可证明（声明）；无匹配为具名无操作成功 |
| `TFRAMES` | 部分实现 | 会话令牌 `tframes`；不改变图像/外部参照边框可见性。 |
| `THICKEN` | 部分实现 | 具名拒绝（`thicken.unsupported: no surface-to-solid model`）。 |
| `TIFOUT` | 部分实现 | 具名拒绝（`tifout.unsupported: raster TIFF export is not implemented; use PLOT for vector SVG/PDF`）。 |
| `TIME` | 部分实现 | 真实 UTC/文档/修订/计数 + 4 个显式 `time.not_tracked`（创建、最后更新、命令耗时、计时器）；未跟踪值不杜撰 |
| `TINSERT` | 部分实现 | **无块进入单元格的创建**（`tinsert.unsupported`）；导入的单元格块内容未建模。
| `TJUST` | 部分实现 | `JUSTIFYTEXT` 的别名，同一语法与实现 |
| `TOLERANCE` | 部分实现 | `TOLERANCE <x> <y> <text>`（`|` 分格）→ 闭合边框 LWPOLYLINE + 分隔 LINE + 每格 TEXT，活动图层，一条事务；是独立实体而非单个 `AcDbFcf`；轴对齐、格宽/行高为声明估计、≤6 格 |
| `TOOLPALETTES` | 部分实现 | chrome 本地选择子：非模态工具选项板浮窗，仅 Blocks 页列出真实块定义并可启动既有 INSERT；无工具内容页，不进入 `cad_command`。 |
| `TOOLPALETTESCLOSE` | 已实现 | chrome 本地选择子：关闭工具选项板浮窗（幂等成功）。 |
| `TORIENT` | 已实现 | 裸命令 Most Readable、`TORIENT <角度>` 绝对旋转、`TORIENT REVERSE|R` 再加 180°（**本项目扩展**，AutoCAD 2026 无此选项）；只改 TEXT/MTEXT 的 rotation，位置不变；ATTDEF/带属性块因无属性模型不适用（全局范围） |
| `TORUS` | 部分实现 | 具名拒绝（`torus.unsupported: 3D solid primitives are not implemented`）。 |
| `TPNAVIGATE` | 部分实现 | chrome 本地选择子：`TPNAVIGATE Blocks` 打开/聚焦 Blocks 页；其它页名或裸词显式拒绝（`toolpalettes.refusal.*`），不转发 `cad_command`。 |
| `TRACEBACK` | 部分实现 | 具名拒绝（`traceback.unsupported: no collaboration trace model`）。 |
| `TRACEEDIT` | 部分实现 | 具名拒绝（`traceedit.unsupported: no collaboration trace model`）。 |
| `TRACEFRONT` | 部分实现 | 具名拒绝（`tracefront.unsupported: no collaboration trace model`）。 |
| `TRACEPALETTECLOSE` | 部分实现 | 具名拒绝（`tracepaletteclose.unsupported: no collaboration trace model`）。 |
| `TRACEPALETTEOPEN` | 部分实现 | 具名拒绝（`tracepaletteopen.unsupported: no collaboration trace model`）。 |
| `TRACEVIEW` | 部分实现 | 具名拒绝（`traceview.unsupported: no collaboration trace model`）。 |
| `TRANSPARENCY` | 部分实现 | 具名拒绝（`transparency.unsupported`）：未建模图像透明度。 |
| `TRAYSETTINGS` | 部分实现 | 会话令牌（默认 ON）；无状态栏托盘 UI。 |
| `TREESTAT` | 部分实现 | 显式 `treestat.not_tracked`；不造假计数 |
| `TREX` | 部分实现 | `TREX TRIM\ |
| `TRIM` | 部分实现 | 目标 LINE/ARC/CIRCLE/SPLINE/**ELLIPSE（含椭圆弧）**/**LWPOLYLINE（开放/闭合，bulge 保留为精确子弧）**；拾取投影所在的段保留（本项目 TRIM 语义，见 `docs/drawing-edit.md` §3）；边界折线/样条采样；**INSERT/HATCH/MESH/OPAQUE/COMPOUND 目标与 EXTEND 的 POLYLINE 目标按名拒绝** |
| `TSCALE` | 部分实现 | `TSCALE <factor>`；复用 SCALETEXT 的唯一 `scaled_text` 规则（height + MText `wrap_width`）；factor=1 为 `tscale.unchanged` 且不开事务 |
| `TXT2MTXT` | 部分实现 | COMBINE/SEPARATE/ORDER/word-wrap；**每个源文字高度以 `\H` 保留**、保留首个已存 wrap 宽度；`UNIFORM` 行距无字段 |
| `TXTEXP` | 已实现 | 字体轮廓→每个轮廓一条 `LWPOLYLINE`（闭合 sfnt 环闭合、SHX 笔画开放），删除源 TEXT/MTEXT，一条事务/一步撤销；字体引擎由宿主安装；MTEXT 宽度换行未应用（声明） |
| `U` | 已实现 | — |
| `UCS` | 部分实现 | 具名拒绝（`ucs.unsupported`）：本构建无 UCS 模型。 |
| `UCSICON` | 部分实现 | 会话开关（默认关）：翻转 `ucsicon`；不渲染任何 UCS 图标。 |
| `UCSMAN` | 部分实现 | 具名拒绝（`ucsman.unsupported`）：本构建无 UCS 模型。 |
| `ULAYERS` | 部分实现 | 具名拒绝（`ulayers.unsupported: no DWF/PDF/DGN underlay layer-display model`）。 |
| `UNDO` | 已实现 | — |
| `UNGROUP` | 部分实现 | 仅移除一个组名并保留全部实体；未知组名拒绝；不写实体/数据库 |
| `UNION` | 部分实现 | 具名拒绝（`union.unsupported: no 3D boolean model`）。 |
| `UNISOLATEOBJECTS` | 部分实现 | 清空会话隐藏集；无数据库写/撤销 |
| `UNITS` | 部分实现 | 只读报告（`units.report`/`units.dialog_not_implemented`）：图纸单位 4 位小数、角度用度；UNITS 对话框与逐图纸单位表未实现。 |
| `UPDATEFIELD` | 部分实现 | 具名拒绝（`updatefield.unsupported`）：未建模字段。 |
| `VBAIDE` | 部分实现 | 具名拒绝（`vbaide.unsupported: no VBA IDE model`）。 |
| `VBALOAD` | 部分实现 | 具名拒绝（`vbaload.unsupported: no VBA loader model`）。 |
| `VBAMAN` | 部分实现 | 具名拒绝（`vbaman.unsupported: no VBA runtime/IDE`）。 |
| `VBAPREF` | 部分实现 | 具名拒绝（`vbapref.unsupported: no VBA runtime/IDE`）。 |
| `VBARUN` | 部分实现 | 具名拒绝（`vbarun.unsupported: no VBA runtime`）。 |
| `VBASTMT` | 部分实现 | 具名拒绝（`vbastmt.unsupported: no VBA runtime/IDE`）。 |
| `VBAUNLOAD` | 部分实现 | 具名拒绝（`vbaunload.unsupported: no VBA runtime`）。 |
| `VIEW` | 部分实现 | `SAVE/RESTORE/LIST/DELETE <name>`；会话内最多 32 个命名视图（相机+视图模式+工作平面+活动空间）；不持久化 DWG |
| `VIEWBACK` | 部分实现 | 复用每视口 `ZOOM PREVIOUS` 栈；空栈为 `view.back.empty` no-op |
| `VIEWBASE` | 部分实现 | 具名拒绝（`viewbase.unsupported: no 3D model-to-drawing base/detail/section view model`）。 |
| `VIEWCOMPONENT` | 部分实现 | 具名拒绝（`viewcomponent.unsupported: no 3D model-to-drawing base/detail/section view model`）。 |
| `VIEWDETAIL` | 部分实现 | 具名拒绝（`viewdetail.unsupported: no 3D model-to-drawing base/detail/section view model`）。 |
| `VIEWDETAILSTYLE` | 部分实现 | 具名拒绝（`viewdetailstyle.unsupported: no 3D model-to-drawing base/detail/section view model`）。 |
| `VIEWEDIT` | 部分实现 | 具名拒绝（`viewedit.unsupported: no 3D model-to-drawing base/detail/section view model`）。 |
| `VIEWFORWARD` | 部分实现 | 新的每视口前进栈（上限 16）；空栈 no-op；文档化偏离 |
| `VIEWGO` | 部分实现 | 同 `VIEW RESTORE`；未知名称按名拒绝 |
| `VIEWPLAY` | 部分实现 | 具名拒绝（`viewplay.unsupported: no named-view animation model`）。 |
| `VIEWPLOTDETAILS` | 部分实现 | 具名拒绝（`viewplotdetails.unsupported`）：无打印详细信息模型。 |
| `VIEWPROJ` | 部分实现 | 具名拒绝（`viewproj.unsupported: no 3D model-to-drawing base/detail/section view model`）。 |
| `VIEWRES` | 部分实现 | 整数 `1..=1000` 仅存会话 token；越界 `viewres.out_of_range`；渲染器消费/像素效果 NOT IMPLEMENTED / NOT VERIFIED |
| `VIEWSECTION` | 部分实现 | 具名拒绝（`viewsection.unsupported: no 3D model-to-drawing base/detail/section view model`）。 |
| `VIEWSECTIONSTYLE` | 部分实现 | 具名拒绝（`viewsectionstyle.unsupported: no 3D model-to-drawing base/detail/section view model`）。 |
| `VIEWSETPROJ` | 部分实现 | 具名拒绝（`viewsetproj.unsupported: no 3D model-to-drawing base/detail/section view model`）。 |
| `VIEWSKETCHCLOSE` | 部分实现 | 具名拒绝（`viewsketchclose.unsupported: no 3D model-to-drawing base/detail/section view model`）。 |
| `VIEWSTD` | 部分实现 | 具名拒绝（`viewstd.unsupported: no 3D model-to-drawing base/detail/section view model`）。 |
| `VIEWSYMBOLSKETCH` | 部分实现 | 具名拒绝（`viewsymbolsketch.unsupported: no 3D model-to-drawing base/detail/section view model`）。 |
| `VIEWUPDATE` | 部分实现 | 具名拒绝（`viewupdate.unsupported: no 3D model-to-drawing base/detail/section view model`）。 |
| `VISUALSTYLES` | 部分实现 | 具名拒绝（`visualstyles.unsupported: no visual-style/3D display model; VSCURRENT owns the 2D display token`）。 |
| `VISUALSTYLESCLOSE` | 部分实现 | 具名拒绝（`visualstylesclose.unsupported: no visual-style/3D display model; VSCURRENT owns the 2D display token`）。 |
| `VLISP` | 部分实现 | 具名拒绝（`vlisp.unsupported: no AutoLISP runtime/IDE`）。 |
| `VPCLIP` | 部分实现 | 矩形 + **多边形 clip 真实写入**（N 点、共面/自交校验）；representation 仅接受 3/4 点 clip ⇒ 多边形按 `*.polygon_not_renderable` 诚实报告，不伪称可渲染 |
| `VPLAYER` | 已实现 | `VPLAYER FREEZE/THAW <layer,...> [<index>]` 写单视口 `layer_overrides`（全局层表不动），bare 为只读报告；命令行 grammar 已接；一条事务/一步撤销 |
| `VPMAX` | 已实现 | 真实最大化编辑态：保存相机+视图模式+工作平面+空间的 `ViewSnapshot`，`VPMIN` 位级恢复；一次一个视口；无宿主差异化渲染（无像素证据） |
| `VPMIN` | 部分实现 | 恢复会话标记；空状态 `vpmin.nothing_to_restore` |
| `VPOINT` | 部分实现 | 向量或 `TOP/BOTTOM/FRONT/BACK/LEFT/RIGHT`；零/非有限向量拒绝；不改模型；向量形式进入 3D 透视 |
| `VPORTS` | 部分实现 | 在活动图纸布局创建简单 rows×cols 视口网格（四角 clip、`model_per_paper=1`），一条事务/一步撤销；命名/标准视口配置未实现 |
| `VPSCALE` | 部分实现 | 设置活动布局**第一个**视口比例（存储对角线 `1/factor`，保留模型锚点），一条事务/一步撤销；无视口选择 |
| `VPSYNC` | 部分实现 | 把 viewport 0 的 `model_to_paper` 复制到其余视口；主视口不可选择（无逐视口选择模型） |
| `VSCURRENT` | 部分实现 | 仅 `WIREFRAME` 存会话 token；其余样式 `vscurrent.style_not_implemented`；无渲染器切换、NOT pixel-verified |
| `VSLIDE` | 部分实现 | 具名拒绝（`vslide.unsupported: no slide (.sld) format model`）。 |
| `VSSAVE` | 部分实现 | 具名拒绝（`vssave.unsupported: no slide (.sld) format model`）。 |
| `VTOPTIONS` | 部分实现 | 只读：视图过渡设置未建模（`vtoptions.not_tracked`），不杜撰平滑/时长数值。 |
| `WALKFLYSETTINGS` | 部分实现 | 具名拒绝（`walkflysettings.unsupported: no 3D walk/fly navigation model`）。 |
| `WBLOCK` | 部分实现 | 具名拒绝（`wblock.unsupported`）：将块定义写入独立图形文件未实现。 |
| `WEBLIGHT` | 部分实现 | 具名拒绝（`weblight.unsupported: no 3D light model`）。 |
| `WEBLOAD` | 部分实现 | 具名拒绝（`webload.unsupported: no JavaScript/web-load runtime`）。 |
| `WEDGE` | 部分实现 | 具名拒绝（`wedge.unsupported: 3D solid primitives are not implemented`）。 |
| `WHOHAS` | 部分实现 | 只读：单文档构建，无网络查询，不列其他用户或文件锁。 |
| `WIPEOUT` | 已实现 | `Frames ON|OFF|DISPLAY` 真实（显示/出图分离）；`Polyline`/`Erase`；遮罩几何与边框状态一条事务/一步撤销 |
| `WMFIN` | 部分实现 | 具名拒绝（`wmfin.unsupported: Windows metafile import is not implemented`）。 |
| `WMFOPTS` | 部分实现 | 具名拒绝（`wmfopts.unsupported: no Windows metafile import model (WMFIN is not implemented)`）。 |
| `WMFOUT` | 部分实现 | 具名拒绝（`wmfout.unsupported: Windows metafile export is not implemented`）。 |
| `XATTACH` | 部分实现 | 具名拒绝（`xattach.unsupported`）：不把外部图纸作为外部参照附着。 |
| `XBIND` | 部分实现 | 具名拒绝（`xbind.unsupported`）：本构建无外部参照绑定/插入模型。 |
|  `XCLIP` | 部分实现 | 真实（display-only）：`XCLIP RECT|POLY|ON|OFF|DELETE|?`（一个直接可寻址的模型空间 INSERT；RECT/POLY 为块局部坐标）；clip 存于 INSERT 的 `EntityRenderAttributes`（一条事务/一步撤销），表示层在块展开时映射为世界轴对齐矩形并精确裁剪 LINE/ARC/圆/MESH/IMAGE（TEXT/MASK 保留并报告）；矩形仅 0/90/180/270 世界旋转可渲染，其它旋转 `xclip.rotation_not_renderable`；多边形存而报告 `xclip.polygon_not_renderable`，INVERT 拒绝；拾取/范围仍按未裁剪几何；无 DXF/DWG 持久化。 |
| `XCOMPARE` | 部分实现 | 具名拒绝（`xcompare.unsupported: no xref comparison/diff model`）。 |
| `XCOMPARECLOSE` | 部分实现 | 具名拒绝（`xcompareclose.unsupported: no xref comparison/diff model`）。 |
| `XCOMPARERCNEXT` | 部分实现 | 具名拒绝（`xcomparercnext.unsupported: no xref comparison/diff model`）。 |
| `XCOMPARERCPREV` | 部分实现 | 具名拒绝（`xcomparercprev.unsupported: no xref comparison/diff model`）。 |
| `XDATA` | 部分实现 | 具名拒绝（`xdata.unsupported`）：本数据库不保留实体 XDATA。 |
| `XDLIST` | 部分实现 | 只读；每个选中实体报告 `xdlist.not_retained`；不伪造 xdata |
| `XEDGES` | 部分实现 | 具名拒绝（`xedges.unsupported: no 3D solid/surface wireframe extraction model`）。 |
| `XLINE` | 部分实现 | 两点创建，但落库为裁剪到模型包围盒的有限线段 |
| `XLIST` | 部分实现 | 只读：列出块定义及成员数；实体 XDATA 不保留，不列出。 |
| `XOPEN` | 部分实现 | 具名拒绝（`xopen.unsupported`）：本构建无外部参照编辑会话。 |
| `XPLODE` | 部分实现 | 真实编辑：复用 EXPLODE 分解选中 INSERT；一条事务/一步撤销；属性/特性控制未实现。 |
| `XREF` | 部分实现 | 只读：外部参照导入溯源报告（`xref.entry`/`xref.summary`）；附着/卸载/重载/绑定未实现。 |
| `ZOOM` | 部分实现 | OBJECT（当前选择包围盒，含块展开）/nX/图纸空间 nXP/WINDOW/PREVIOUS 已实现；仅 DYNAMIC（交互式指针跟随缩放）具名未实现（`zoom.dynamic_not_implemented`） |

| # | Command | 状态 | 官方说明 | 说明 / 本项目选择子 |
| ---: | --- | --- | --- | --- |
| `SHAPE` | 已实现 | 真实 `SemanticGeometry::Shape` 字形：`SHAPE <name-or-code> <x> <y> [size] [rotation_deg]`，字体取显式 key 或 `Standard` 样式，缺省 size/rotation 取会话值；一条事务/一步撤销；按 name 建行但本构建 SHX 仅保留编号，表示层保持既有 `Partial` |
| `SHP2BLK` | 已实现 | 选中 SHAPE → 按 `(font, code)` 建块（字形轮廓多段线）+ 每个源位置一个 INSERT（`T·R·S`），删除源，一条事务；需要宿主安装字体引擎，否则 `shp2blk.font_unavailable` |
| `QVDRAWING` | 已实现 | chrome 面板：Quick View Drawings，列出打开的图形（单文档）与预览卡；点击聚焦，Esc 关闭 |
| `QVDRAWINGCLOSE` | 已实现 | chrome 本地选择子：关闭 Quick View Drawings（幂等成功） |
| `QVLAYOUT` | 已实现 | chrome 面板：列出真实布局，点击派发 `SPACE #<id>`/`MODEL` 并关闭 |
| `QVLAYOUTCLOSE` | 已实现 | chrome 本地选择子：关闭 Quick View Layouts（幂等成功） |
| `QUICKCALC` | 已实现 | chrome 非模态可拖动浮窗计算器：+ − × ÷、括号、小数、正负号、清除/退格、M+/MR/MC、复制结果、键盘输入、Esc 关闭；科学模式含三角/反三角/`ln`/`log`/`sqrt`/`abs`/幂/平方/倒数/`π`/`e` 与 DEG/RAD；无伪工程键，域错误不编造数值 |
| `QCCLOSE` | 已实现 | 关闭 QUICKCALC 浮窗（幂等成功） |
| `UPDATETHUMBSNOW` | 已实现 | 立即刷新 Quick View 预览卡（重算预览状态并显示瞬时“已更新”；不做文件写） |
| `WORKSPACE` | 已实现 | chrome 工作空间选择器：功能区/选项板/文件标签可见性与面板宽度预设（内置 Drafting & Annotation） |
| `WSSAVE` | 已实现 | 把当前 UI 状态保存为命名工作空间（会话内存，不落盘） |
| `WSSETTINGS` | 已实现 | chrome 工作空间设置面：列出/重命名/删除/设为当前 |
| `TOOLBAR` | 已实现 | chrome 本地选择子：显示快速访问工具栏 |
| `-TOOLBAR` | 已实现 | chrome 本地选择子：`-TOOLBAR [SHOW|HIDE]` 设置快速访问工具栏可见性 |
| `PMTOGGLE` | 已实现 | chrome 本地选择子：切换与 HIDEPALETTES/SHOWPALETTES 相同的选项板可见性状态（幂等；关闭打开的选项板） |
| `REDEFINE` | 已实现 | `REDEFINE <name>` 重新启用被 `UNDEFINE` 禁用的内建命令；未知/从未定义按名拒绝；essential 命令双向拒绝；会话状态不写图 |
| `UNDEFINE` | 已实现 | `UNDEFINE <name>` 会话内禁用内建命令；被禁用名按 `command.undefined: <NAME>` 拒绝且不派发；静态/运行时别名指向禁用目标时同样禁用；不写图 |

| 1 | `3DALIGN` | 部分实现 | Aligns objects with other objects in 2D and 3D. | 具名拒绝（`3dalign.unsupported: 3D alignment is not modeled; use the 2D ALIGN path`）。 |
| 2 | `3DARRAY` | 部分实现 | Creates nonassociative, 3D rectangular or polar arrays. | 具名拒绝（`3darray.unsupported: 3D arrays are not modeled; use ARRAY/ARRAYRECT/ARRAYPOLAR`）。 |
| 3 | `3DCLIP` | 部分实现 | Opens the Adjust Clipping Planes window, where you can specify what portions of a 3D model to display. | 具名拒绝（`3dclip.unsupported: no 3D clipping-plane model`）。 |
| 4 | `3DCORBIT` | 部分实现 | Rotates the view in 3D space with continuous motion. | 具名拒绝（`3dcorbit.unsupported: no continuous 3D orbit controller`）。 |
| 5 | `3DDISTANCE` | 部分实现 | Starts the interactive 3D view and makes objects appear closer or farther away. | 具名拒绝（`3ddistance.unsupported: no interactive 3D distance navigation`）。 |
| 6 | `3DDWF` | 部分实现 | Creates a 3D DWF or 3D DWFx file of your 3D model and displays it in the DWF Viewer. | 具名拒绝（`3ddwf.unsupported: DWF/DWFx export is not implemented; use PLOT for SVG/PDF`）。 |
| 7 | `3DEDITBAR` | 部分实现 | Reshapes splines and NURBS surfaces, including their tangency properties. | 具名拒绝（`3deditbar.unsupported: no NURBS/spline edit bar`）。 |
| 8 | `3DFACE` | 部分实现 | Creates a three-sided or four-sided surface in 3D space. | 具名拒绝（`3dface.unsupported: no 3DFACE entity model`）。 |
| 9 | `3DFLY` | 部分实现 | Changes the 3D view in a drawing interactively to create the appearance of flying through the model. | 具名拒绝（`3dfly.unsupported: no 3D fly navigation`）。 |
| 10 | `3DFORBIT` | 部分实现 | Rotates the view in 3D space without constraining roll. | 具名拒绝（`3dforbit.unsupported: no free 3D orbit controller`）。 |
| 11 | `3DMESH` | 部分实现 | Creates a free-form polygon mesh. | 具名拒绝（`3dmesh.unsupported: no free-form mesh creation model`）。 |
| 12 | `3DMOVE` | 部分实现 | In a 3D view, displays the 3D Move gizmo to aid in moving 3D objects a specified distance in a specified direction. | 具名拒绝（`3dmove.unsupported: no 3D move gizmo`）。 |
| 13 | `3DORBIT` | 部分实现 | Provides interactive viewing in 3D with the mouse. Right-click to display viewing options. | 具名拒绝（`3dorbit.unsupported: no interactive 3D orbit controller; use ORBIT`）。 |
| 14 | `3DORBITCTR` | 部分实现 | Sets a specific center of rotation in 3D Orbit view. | 具名拒绝（`3dorbitctr.unsupported: no 3D orbit-center model`）。 |
| 15 | `-3DOSNAP` | 部分实现 | Uses command prompts to set running object snap modes for 3D objects. | 具名拒绝（`-3dosnap.unsupported: no 3D object-snap model`）。 |
| 16 | `3DOSNAP` | 部分实现 | Sets the object snap modes for 3D objects. | 具名拒绝（`3dosnap.unsupported: no 3D object-snap model`）。 |
| 17 | `3DPAN` | 部分实现 | Shifts the view without changing the viewing direction or magnification. | 具名拒绝（`3dpan.unsupported: no 3D pan navigation`）。 |
| 18 | `3DPOLY` | 部分实现 | Creates a 3D polyline. | 具名拒绝（`3dpoly.unsupported: no 3D polyline entity model`）。 |
| 19 | `3DPRINT` | 部分实现 | Specifies 3D Plot settings, and prepares your drawing for 3D printing. | 具名拒绝（`3dprint.unsupported: no 3D print model`）。 |
| 20 | `3DPRINTSERVICE` | 部分实现 | Creates an STL file that can be sent to a 3D printing service. | 具名拒绝（`3dprintservice.unsupported: no STL export or 3D print service`）。 |
| 21 | `3DROTATE` | 部分实现 | In a 3D view, displays the 3D Rotate gizmo to aid in revolving 3D objects around a base point. | 具名拒绝（`3drotate.unsupported: no 3D rotate gizmo`）。 |
| 22 | `3DSCALE` | 部分实现 | In a 3D view, displays the 3D Scale gizmo to aid in resizing 3D objects. | 具名拒绝（`3dscale.unsupported: no 3D scale gizmo`）。 |
| 23 | `3DSIN` | 部分实现 | Imports a 3ds Max (3DS) file. | 具名拒绝（`3dsin.unsupported: 3DS import is not implemented`）。 |
| 24 | `3DSWIVEL` | 部分实现 | Changes the target of the view in the direction that you drag. | 具名拒绝（`3dswivel.unsupported: no 3D swivel navigation`）。 |
| 25 | `3DWALK` | 部分实现 | Changes the 3D view in a drawing interactively to create the appearance of walking through the model. | 具名拒绝（`3dwalk.unsupported: no 3D walk navigation`）。 |
| 26 | `3DZOOM` | 部分实现 | Zooms in and out in a perspective view. | 具名拒绝（`3dzoom.unsupported: no perspective 3D zoom navigation`）。 |
| 27 | `ABOUT` | 已实现 | Displays information about your product. | chrome 本地选择子：打开 About 对话框（与 File > About 同一路径） |
| 28 | `ACADINFO` | 部分实现 | Creates a file that stores information about your AutoCAD installation and current setup. | 只读：产品版本/构建配置 + 文档修订/实体数；许可证/硬件/模块数据不收集。 |
| 29 | `ACISIN` | 部分实现 | Imports an ACIS (SAT) file and creates 3D solid, body, or region objects. | 具名拒绝（`acisin.unsupported: ACIS (SAT) import is not implemented`）。 |
| 30 | `ACISOUT` | 部分实现 | Exports 3D solid, region, or body objects to an ACIS file. | 具名拒绝（`acisout.unsupported: ACIS (SAT) export is not implemented`）。 |
| 31 | `ACTBASEPOINT` | 部分实现 | Inserts a base point or base point prompt in an action macro. | 具名拒绝（`actbasepoint.unsupported: action-recorder macros are not modeled (no recorder state, file, or playback)`）。 |
| 32 | `ACTIVITYINSIGHTSCLOSE` | 部分实现 | Closes the Activity Insights palette which displays activities for the current drawing. | 具名拒绝（`activityinsightsclose.unsupported: this build has no Activity Insights data model`）。 |
| 33 | `ACTIVITYINSIGHTSOPEN` | 部分实现 | Opens the Activity Insights palette to view activities for the current drawing. | 具名拒绝（`activityinsightsopen.unsupported: this build has no Activity Insights data model`）。 |
| 34 | `ACTMANAGER` | 部分实现 | Manages action macro files. | 具名拒绝（`actmanager.unsupported: action-recorder macros are not modeled (no recorder state, file, or playback)`）。 |
| 35 | `ACTRECORD` | 部分实现 | Starts the recording an action macro. | 具名拒绝（`actrecord.unsupported: action-recorder macros are not modeled (no recorder state, file, or playback)`）。 |
| 36 | `-ACTSTOP` | 部分实现 | Stops the Action Recorder and, from the command line, provides the option of saving the recorded actions to an action macro file | 具名拒绝（`-actstop.unsupported: action-recorder macros are not modeled (no recorder state, file, or playback)`）。 |
| 37 | `ACTSTOP` | 部分实现 | Stops the Action Recorder and provides the option of saving the recorded actions to an action macro file. | 具名拒绝（`actstop.unsupported: action-recorder macros are not modeled (no recorder state, file, or playback)`）。 |
| 38 | `ACTUSERINPUT` | 部分实现 | Pauses for user input in an action macro. | 具名拒绝（`actuserinput.unsupported: action-recorder macros are not modeled (no recorder state, file, or playback)`）。 |
| 39 | `-ACTUSERMESSAGE` | 部分实现 | From the command line, inserts a user message into an action macro. | 具名拒绝（`-actusermessage.unsupported: action-recorder macros are not modeled (no recorder state, file, or playback)`）。 |
| 40 | `ACTUSERMESSAGE` | 部分实现 | Inserts a user message into an action macro. | 具名拒绝（`actusermessage.unsupported: action-recorder macros are not modeled (no recorder state, file, or playback)`）。 |
| 41 | `ADCCLOSE` | 部分实现 | Closes DesignCenter. | 具名拒绝（`adcclose.unsupported: this build has no DesignCenter content palette`）。 |
| 42 | `ADCENTER` | 部分实现 | Manages and inserts content such as blocks, xrefs, and hatch patterns. | 具名拒绝（`adcenter.unsupported: this build has no DesignCenter content palette`）。 |
| 43 | `ADCNAVIGATE` | 部分实现 | Loads a specified drawing file, folder, or network path in the DesignCenter Folders tab. | 具名拒绝（`adcnavigate.unsupported: this build has no DesignCenter content palette`）。 |
| 44 | `ADDSELECTED` | 部分实现 | Creates a new object of the same type and general properties as a selected object, but with different geometric values. | `ADDSELECTED <x> <y>` 按选中类型/属性新建一个实体（LINE/CIRCLE/POINT/TEXT/MTEXT），保留图层/颜色/线型/线宽/透明度；ARC/POLYLINE/INSERT 等按名拒绝 |
| 45 | `ADJUST` | 部分实现 | Adjusts the fade, contrast and monochrome settings of the selected underlay (DWF, DWFx, PDF, or DGN), or image. | 具名拒绝（`adjust.unsupported: this build has no underlay/image fade-contrast-monochrome model`）。 |
| 46 | `ALIASEDIT` | 部分实现 | Creates, modifies, and deletes AutoCAD command aliases. | `ADD <alias> <target>`/`REMOVE <alias>`/`LIST`/bare；运行时别名先于静态表解析（可遮蔽内建），仅删除运行时行；无交互对话框（声明），会话状态不持久化 |
| 47 | `ALIGN` | 已实现 | Aligns objects with other objects in 2D and 3D. | 1 对点移动、2 对点平移+旋转+可选均匀缩放、3 对点平面**刚性**（旋转+平移，不缩放）；退化/非共面/非全等/镜像按名拒绝；3 对点的 3D 平面旋转属全局范围 |
| 48 | `ALIGNSPACE` | 部分实现 | Adjusts the pan and zoom factor of a view in a layout viewport based on the alignment points specified in model space and paper space. | `model_a paper_a model_b paper_b` 4 坐标→均匀 `paper_per_model` + 平移，真实写 `model_to_paper`；CLI 尚无 4 坐标拼写；旋转对报告 `rotation_ignored` |
| 49 | `AMECONVERT` | 部分实现 | Converts AME solid models to AutoCAD solid objects. | 具名拒绝（`ameconvert.unsupported: this build has no AME solid model`）。 |
| 50 | `ANALYSISCURVATURE` | 部分实现 | Displays a color gradient onto a surface to evaluate different aspects of its curvature. | 具名拒绝（`analysiscurvature.unsupported: this build has no 3D curvature/draft/zebra analysis display`）。 |
| 51 | `ANALYSISDRAFT` | 部分实现 | Displays a color gradient onto a 3D model to evaluate whether there is adequate space between a part and its mold. | 具名拒绝（`analysisdraft.unsupported: this build has no 3D curvature/draft/zebra analysis display`）。 |
| 52 | `ANALYSISOPTIONS` | 部分实现 | Sets the display options for zebra, curvature, and draft analysis. | 具名拒绝（`analysisoptions.unsupported: this build has no 3D curvature/draft/zebra analysis display`）。 |
| 53 | `ANALYSISZEBRA` | 部分实现 | Projects stripes onto a 3D model to analyze surface continuity. | 具名拒绝（`analysiszebra.unsupported: this build has no 3D curvature/draft/zebra analysis display`）。 |
| 54 | `ANIPATH` | 部分实现 | Saves an animation of a camera moving or panning in a 3D model. | 具名拒绝（`anipath.unsupported: this build has no camera-animation model`）。 |
| 55 | `ANNORESET` | 部分实现 | Resets the locations of all alternate scale representations of the selected annotative objects. | 具名拒绝（`annoreset.unsupported`）：无注释性对象模型。 |
| 56 | `ANNOUPDATE` | 部分实现 | Updates existing annotative objects to match the current properties of their styles. | 具名拒绝（`annoupdate.unsupported`）：无注释性对象模型。 |
| 57 | `APERTURE` | 已实现 | Controls the size of the object snap target box. | `APERTURE <n>`（整像素 1..=50）写会话孔径并让 snap/候选拾取真实使用（overlay 于配置默认值；PICKBOX 保持独立）；bare 为只读报告 |
| 58 | `APPAUTOLOADER` | 部分实现 | Lists or reloads all plug-ins in the application plug-in folder. | 具名拒绝（`appautoloader.unsupported: this build has no plug-in autoloader registry`）。 |
| 59 | `APPLOAD` | 部分实现 | Loads and unloads applications and defines which applications to load at startup. | 具名拒绝（`appload.unsupported: no plug-in/application loader model`）。 |
| 60 | `APPSTORE` | 部分实现 | Opens the Autodesk App Store website. | 具名拒绝（`appstore.unsupported: the host owns this browser/store/chrome action`）。 |
| 61 | `ARC` | 已实现 | Creates an arc. | 三点弧；payload 可写 `(center,radius,start,sweep)`（负 sweep = CW），S-C-E/S-C-A/S-E-A 的几何都可表达；交互关键字 SCE/SCA/SEA/Continue 尚未接线 |
| 62 | `-ARCHIVE` | 部分实现 | From the command prompt, packages the current sheet set files for storage. | 具名拒绝（`-archive.unsupported`）：本构建无图纸集归档打包模型；命令行拼写。 |
| 63 | `ARCHIVE` | 部分实现 | Packages the current sheet set files for storage. | 具名拒绝（`archive.unsupported`）：本构建无图纸集归档打包模型。 |
| 64 | `ARCTEXT` | 已实现 | Places text along an arc. | 沿选中 ARC 等角步放置逐字符 `TEXT`（切线旋转、倒置翻转），保留高度/样式/字体/图层，源文字被消费；每字符一个 TEXT 是声明表示偏离；+Z、单行 |
| 65 | `AREA` | 已实现 | Calculates the area and perimeter of objects or of defined areas. | — |
| 66 | `-ARRAY` | 部分实现 | From the command line, creates copies of objects arranged in a pattern. | `ARRAY` 的命令行拼写，同一路由与边界。 |
| 67 | `ARRAY` | 部分实现 | Creates copies of objects arranged in a pattern. | 路由：`ArrayRect`/`ArrayPolar` 载荷转交现有 `ARRAYRECT`/`ARRAYPOLAR` 规划器；无载荷只报告，不创建；无旧式对话框/关联阵列。 |
| 68 | `ARRAYCLASSIC` | 部分实现 | Creates arrays using the legacy dialog box. | 无旧式对话框；带阵列载荷时转交 `ARRAY` 规划器，否则报告缺少载荷。 |
| 69 | `ARRAYCLOSE` | 部分实现 | Saves or discards changes made to an array's source objects and exits the array editing state. | 具名拒绝（`arrayclose.unsupported`）：无关联阵列编辑状态。 |
| 70 | `ARRAYEDIT` | 部分实现 | Edits associative array objects and their source objects. | 具名拒绝（`arrayedit.unsupported`）：无关联阵列编辑状态。 |
| 71 | `ARRAYPATH` | 部分实现 | Evenly distributes object copies along a path or a portion of a path. | 真实 v1：当前选择第一个实体为路径、其余为项目，等弧长分布，`ARRAYPATH <count> [ALIGN]`；一条事务/一步撤销；无关联性/持久路径参数。 |
| 72 | `ARRAYPOLAR` | 部分实现 | Evenly distributes object copies in a circular pattern around a center point or axis of rotation. | 独立、非关联副本；角度步长改为 AutoCAD 规则：半圈 `fill/(items−1)`、整圈 `fill/items`；负 fill 支持；关联编辑未建模 |
| 73 | `ARRAYRECT` | 部分实现 | Distributes object copies into any combination of rows, columns, and levels. | 独立、非关联副本；**有符号行/列间距**（负值向负方向阵列）；`Levels`/网格 `angle` 无工具参数通道；`Path`/关联编辑未建模 |
| 74 | `ARX` | 部分实现 | Loads, unloads, and provides information about ObjectARX applications. | 具名拒绝（`arx.unsupported: no ObjectARX loader model`）。 |
| 75 | `ASSISTANTCLOSE` | 部分实现 | Closes Autodesk Assistant. | 具名拒绝（`assistantclose.unsupported: this build has no Autodesk Assistant model`）。 |
| 76 | `ASSISTANTOPEN` | 部分实现 | Displays Autodesk Assistant. | 具名拒绝（`assistantopen.unsupported: this build has no Autodesk Assistant model`）。 |
| 77 | `-ATTACH` | 部分实现 | From the Command prompt, inserts references to external files such as other drawings, raster images, point clouds, coordination models, and underlays. | 具名拒绝（`-attach.unsupported`）：不附着外部参照或底图。 |
| 78 | `ATTACH` | 部分实现 | Inserts references to external files such as other drawings, raster images, point clouds, coordination models, and underlays. | 具名拒绝（`attach.unsupported`）：不附着外部参照或底图。 |
| 79 | `ATTACHURL` | 部分实现 | Attaches hyperlinks to objects or areas in a drawing. | 具名拒绝（`attachurl.unsupported: this build has no hyperlink storage model`）。 |
| 80 | `-ATTDEF` | 部分实现 | At the Command prompt, creates an attribute definition for storing data in a block. | 具名拒绝（`-attdef.unsupported`）：本构建无块属性模型。 |
| 81 | `ATTDEF` | 部分实现 | Creates an attribute definition for storing data in a block. | 具名拒绝（`attdef.unsupported`）：本构建无块属性模型。 |
| 82 | `ATTDISP` | 部分实现 | Controls the visibility overrides for all block attributes in a drawing. | 会话令牌（默认开）：翻转 `attdisp`；无块属性模型，不渲染属性、不写图、不升修订。 |
| 83 | `-ATTEDIT` | 部分实现 | At the Command prompt, changes attribute information in a block. | 具名拒绝（`-attedit.unsupported`）：本构建无块属性模型。 |
| 84 | `ATTEDIT` | 部分实现 | Changes attribute information in a block. | 具名拒绝（`attedit.unsupported`）：本构建无块属性模型。 |
| 85 | `-ATTEXT` | 部分实现 | From the Command prompt, extracts attribute data, informational text associated with a block, into a file. | 具名拒绝（`-attext.unsupported`）：本构建无块属性模型。 |
| 86 | `ATTEXT` | 部分实现 | Extracts attribute data, informational text associated with a block, into a file. | 具名拒绝（`attext.unsupported`）：本构建无块属性模型。 |
| 87 | `ATTIN` | 部分实现 | Imports block attribute values from an external, tab-delimited ASCII file. | 具名拒绝（`attin.unsupported`）：本构建无块属性模型。 |
| 88 | `ATTIPEDIT` | 部分实现 | Changes the textual content of an attribute within a block. | 具名拒绝（`attipedit.unsupported`）：本构建无块属性模型。 |
| 89 | `ATTOUT` | 部分实现 | Exports block attribute values to an external file in tab-delimited ASCII format. | 具名拒绝（`attout.unsupported`）：本构建无块属性模型。 |
| 90 | `ATTREDEF` | 部分实现 | Redefines a block and updates associated attributes. | 具名拒绝（`attredef.unsupported`）：本构建无块属性模型。 |
| 91 | `ATTSYNC` | 部分实现 | Applies attribute changes in a block definition to all block references. | 具名拒绝（`attsync.unsupported`）：本构建无块属性模型。 |
| 92 | `AUDIT` | 已实现 | Evaluates the integrity of a drawing and corrects some errors. | `Fix any errors detected? [Yes/No]`（默认 No）+ 分段报告（头变量/块定义/Pass 1/2/`Total errors found N fixed M`）；只修复 `validate_entity` 可证明可修复的行（ObjectId 归一、非法 TEXT wrap→None），一条事务；修复不可撤销（`audit.repairs_not_undoable`）；`AUDITCTL` 属宿主文件 I/O 全局 |
| 93 | `AUTOCONSTRAIN` | 部分实现 | Applies geometric constraints to a selection set of objects based on orientation of the objects relative to one another. | 具名拒绝（`autoconstrain.unsupported`）：本构建无几何约束模型。 |
| 94 | `AUTOPUBLISH` | 部分实现 | Publishes drawings to DWF, DWFx, or PDF files automatically to a specified location. | 具名拒绝（`autopublish.unsupported`）：本构建无自动发布模型。 |
| 95 | `BACTION` | 部分实现 | Adds an action to a dynamic block definition. | 具名拒绝（`baction.unsupported`）：本构建无动态块动作模型。 |
| 96 | `BACTIONBAR` | 部分实现 | Displays or hides action bars for a selection set of parameter objects. | 具名拒绝（`bactionbar.unsupported`）：本构建无动态块动作模型。 |
| 97 | `BACTIONSET` | 部分实现 | Specifies the selection set of objects associated with an action in a dynamic block definition. | 具名拒绝（`bactionset.unsupported`）：本构建无动态块动作模型。 |
| 98 | `BACTIONTOOL` | 部分实现 | Adds an action to a dynamic block definition. | 具名拒绝（`bactiontool.unsupported`）：本构建无动态块动作模型。 |
| 99 | `BASE` | 部分实现 | Sets the insertion base point for the current drawing. | 具名拒绝（`base.unsupported`）：图形不保存插入基点。 |
| 100 | `BASSOCIATE` | 部分实现 | Associates an action with a parameter in a dynamic block definition. | 具名拒绝（`bassociate.unsupported`）：本构建无动态块动作模型。 |
| 101 | `BATTMAN` | 部分实现 | Manages the attributes for a selected block definition. | 只读：列出块定义与成员数（复用 `drawing.blocks()`；`battman.entry`/`battman.summary`）；块属性未建模（`battman.attributes_not_modeled`）。 |
| 102 | `BATTORDER` | 部分实现 | Specifies the order of attributes for a block. | 只读：列出块定义（`battorder.entry`/`battorder.summary`）；属性显示顺序未建模（`battorder.attributes_not_modeled`）。 |
| 103 | `BAUTHORPALETTE` | 部分实现 | Opens the Block Authoring Palettes window in the Block Editor. | 具名拒绝（`bauthorpalette.unsupported`）：本构建无块编辑器。 |
| 104 | `BAUTHORPALETTECLOSE` | 部分实现 | Closes the Block Authoring Palettes window in the Block Editor. | 具名拒绝（`bauthorpaletteclose.unsupported`）：本构建无块编辑器。 |
| 105 | `BCLOSE` | 部分实现 | Closes the Block Editor. | 具名拒绝（`bclose.unsupported`）：无块编辑器。 |
| 106 | `BCONSTRUCTION` | 部分实现 | Converts block geometry into construction geometry, which may be hidden or displayed. | 具名拒绝（`bconstruction.unsupported`）：本构建无动态块构造几何。 |
| 107 | `BCOUNT` | 部分实现 | Creates a report of the number of instances of each block in a selection set or in the entire drawing. | 只读：统计整个模型空间的块参照实例数（按块名分组）；无 AutoCAD 的 Select objects 提示、不展开块成员、不统计图纸空间。 |
| 108 | `-BCPARAMETER` | 部分实现 | Applies constraint parameters to selected objects, or converts dimensional constraints to parameter constraints. | 具名拒绝（`-bcparameter.unsupported`）：本构建无动态块参数模型；命令行拼写。 |
| 109 | `BCPARAMETER` | 部分实现 | Applies constraint parameters to selected objects, or converts dimensional constraints to parameter constraints. | 具名拒绝（`bcparameter.unsupported`）：本构建无动态块参数模型。 |
| 110 | `BCYCLEORDER` | 部分实现 | Changes the cycling order of grips for a dynamic block reference. | 具名拒绝（`bcycleorder.unsupported`）：无动态块夹点模型。 |
| 111 | `BDETECT` | 部分实现 | Displays the Detect palette where you can start object detection and manage the objects that are eligible for conversion. | 具名拒绝（`bdetect.unsupported: this build has no object-detection model`）。 |
| 112 | `-BEDIT` | 部分实现 | Opens the block definition in the Block Editor from the Command prompt. | 具名拒绝（`-bedit.unsupported`）：同一拒绝的命令行拼写。 |
| 113 | `BEDIT` | 部分实现 | Opens the block definition in the Block Editor. | 具名拒绝（`bedit.unsupported`）：无块编辑器。 |
| 114 | `BESETTINGS` | 部分实现 | Displays the Block Editor Settings dialog box. | 具名拒绝（`besettings.unsupported`）：无块编辑器。 |
| 115 | `BEXTEND` | 部分实现 | Extends objects to blocks. | 真实编辑：首个选中 INSERT 成员为边界，其余选中对象复用 EXTEND 规划器延伸；一条事务/一步撤销；无交互边界提示。 |
| 116 | `BGRIPSET` | 部分实现 | Creates, deletes, or resets grips associated with a parameter. | 具名拒绝（`bgripset.unsupported`）：无动态块参数模型。 |
| 117 | `BLEND` | 部分实现 | Creates a spline in the gap between two selected lines or curves. | 真实编辑：两条开放 `LINE`/`ARC`/`LWPOLYLINE` 最近端点之间一条三次 `SPLINE`；内部控制点沿源切线取间隙 1/3（声明规则）；无曲线连续性选项。 |
| 118 | `-BLOCK` | 已实现 | At the Command prompt, creates a block definition from selected objects. | `BLOCK` 的命令行拼写；同一实现/写入路径。 |
| 119 | `BLOCK` | 已实现 | Creates a block definition from selected objects. | — |
| 120 | `BLOCK?` | 部分实现 | Lists the objects in a block definition. | 只读列出块定义与成员数（`block.entry`/`block.summary`），复用 `drawing.blocks()`；不是块编辑器。 |
| 121 | `BLOCKICON` | 部分实现 | Generates preview images for blocks displayed in DesignCenter. | 具名拒绝（`blockicon.unsupported: this build has no block thumbnail/preview model`）。 |
| 122 | `BLOCKREPLACE` | 部分实现 | Replaces all instances of a specified block with a different block. | 具名拒绝（`blockreplace.unsupported`）：需要两个块定义；无块替换编辑器。 |
| 123 | `BLOCKSDATAOPTION` | 部分实现 | Displays the data collection consent dialog box. | 具名拒绝（`blocksdataoption.unsupported: this build has no data-collection consent dialog model`）。 |
| 124 | `BLOCKSPALETTE` | 部分实现 | Displays the Blocks palette, which you can use to insert blocks and drawings into the current drawing. | 具名拒绝（`blockspalette.unsupported: this build has no Blocks palette; block insertion is the INSERT path`）。 |
| 125 | `BLOCKSPALETTECLOSE` | 部分实现 | Closes the Blocks palette. | 具名拒绝（`blockspaletteclose.unsupported: this build has no Blocks palette; block insertion is the INSERT path`）。 |
| 126 | `BLOCKTOXREF` | 部分实现 | Replaces all instances of a specified block with an xref. | 具名拒绝（`blocktoxref.unsupported: block-to-xref replacement needs a writable block table and xref attachment, neither modeled`）。 |
| 127 | `BLOOKUPTABLE` | 部分实现 | Displays or creates a lookup table for a dynamic block definition. | 具名拒绝（`blookuptable.unsupported`）：本构建无动态块查找表。 |
| 128 | `BMPOUT` | 部分实现 | Saves selected objects to a file in device-independent bitmap format. | 具名拒绝（`bmpout.unsupported: raster BMP export is not implemented; use PLOT for vector SVG/PDF`）。 |
| 129 | `-BOUNDARY` | 部分实现 | At the Command prompt, creates regions or polylines from enclosed areas. | `BOUNDARY` 的命令行拼写；同一实现与边界。 |
| 130 | `BOUNDARY` | 部分实现 | Creates regions or a polylines from enclosed areas. | 一个内部点 → 复用 HATCHPOINT 发现（最小可见闭合边界 + 嵌套岛）；每个环在当前层创建一条闭合、非关联 `AcDbPolyline`（保留 bulge、丢重复末点、世界 `+Z`）；源对象保留；仅整实体圆/完整椭圆/闭合多段线、同 `+Z` 平面；`-BOUNDARY` 的边界集/关闭岛检测/面域输出/对象类型未实现；不承认 `BO`/`BPOLY` 别名 |
| 131 | `BOX` | 部分实现 | Creates a 3D solid box. | 具名拒绝（`box.unsupported: 3D solid primitives are not implemented`）。 |
| 132 | `BPARAMETER` | 部分实现 | Adds a parameter with grips to a dynamic block definition. | 具名拒绝（`bparameter.unsupported`）：本构建无动态块参数模型。 |
| 133 | `BREAK` | 已实现 | Breaks the selected object between two points. | 两断点 + `F` 首点覆盖；`LINE`/`ARC`/`CIRCLE`/`SPLINE`/整椭圆/**椭圆弧**/`LWPOLYLINE`；圆→弧；第二点可投影；**无多段打断**（AutoCAD 无此模式） |
| 134 | `BREAKATPOINT` | 部分实现 | Breaks the selected object into two objects at a specified point. | 交互工具：先拾取对象再指定断点（断点可投影）；仅 LINE/ARC/开放 LWPOLYLINE；圆/闭合多段线/SPLINE 按名拒绝（闭合对象不拆分） |
| 135 | `BREAKLINE` | 部分实现 | Creates a breakline, a polyline that includes the breakline symbol. | 真实编辑：两点之间一条带声明式锯齿折断符号的开放折线；符号宽 15%、深 5%（声明规则）；无块式折断符号。 |
| 136 | `BREP` | 部分实现 | Removes the history from 3D solids and composite solids, and associativity from surfaces. | 具名拒绝（`brep.unsupported: this build has no 3D solid history/brep model`）。 |
| 137 | `-BREPLACE` | 部分实现 | At the Command prompt, replaces specified block references with a different block. | 具名拒绝（`-breplace.unsupported`）：BREPLACE 的命令行拼写。 |
| 138 | `BREPLACE` | 部分实现 | Replaces specified block references with a different block. | 具名拒绝（`breplace.unsupported`）：需要两个块定义；无块替换编辑器。 |
| 139 | `BROWSER` | 部分实现 | Launches the default web browser defined in your system's registry. | 具名拒绝（`browser.unsupported: the host owns this browser/store/chrome action`）。 |
| 140 | `BSAVE` | 部分实现 | Saves the current block definition. | 具名拒绝（`bsave.unsupported`）：本构建无块编辑器。 |
| 141 | `BSAVEAS` | 部分实现 | Saves a copy of the current block definition under a new name. | 具名拒绝（`bsaveas.unsupported`）：本构建无块编辑器。 |
| 142 | `BSCALE` | 部分实现 | Scales a block reference relative to its insertion point. | 具名拒绝（`bscale.unsupported`）：需要比例因子与插入参照；请对选中块参照用 SCALE。 |
| 143 | `BSEARCH` | 部分实现 | Displays the Convert dialog box, which provides options to convert selected entities and identical instances into blocks. | 具名拒绝（`bsearch.unsupported: this build has no convert-to-block search model`）。 |
| 144 | `BTABLE` | 部分实现 | Stores variations of a block in the Block Properties Table. | 具名拒绝（`btable.unsupported`）：本构建无动态块查找表。 |
| 145 | `BTESTBLOCK` | 部分实现 | Displays a window within the Block Editor to test a dynamic block. | 具名拒绝（`btestblock.unsupported`）：无块编辑器。 |
| 146 | `BTRIM` | 部分实现 | Trim objects to blocks. | 真实编辑：首个选中 INSERT 成员为边界，其余选中对象复用 TRIM 规划器修剪；一条事务/一步撤销；无交互边界提示。 |
| 147 | `BURST` | 部分实现 | Explodes selected blocks, preserving the block layer and converting attribute values to text objects. | 具名拒绝（`burst.unsupported`）：无块属性模型可保留。 |
| 148 | `BVHIDE` | 部分实现 | Makes objects invisible in the current visibility state or all visibility states in a dynamic block definition. | 具名拒绝（`bvhide.unsupported`）：本构建无按可见性状态隐藏实体的 API。 |
| 149 | `BVSHOW` | 部分实现 | Makes objects visible in the current visibility state or all visibility states in a dynamic block definition. | 具名拒绝（`bvshow.unsupported`）：本构建无按可见性状态显示实体的 API。 |
| 150 | `-BVSTATE` | 部分实现 | At the command prompt, creates, sets, or deletes a visibility state in a dynamic block. | 只读：同 `BVSTATE`，使用 `-bvstate.*` 诊断码；无 INSERT 报 `-bvstate.no_selection`，无状态报 `-bvstate.not_set`；不设置状态。 |
| 151 | `BVSTATE` | 部分实现 | Creates, sets, or deletes a visibility state in a dynamic block. | 只读：每个选中 INSERT 的活动动态块可见性状态（`bvstate.report`）；无 INSERT 报 `bvstate.no_selection`，无状态报 `bvstate.not_set`；不设置状态/不写库/不升修订。 |
| 152 | `CAL` | 部分实现 | Evaluates mathematical and geometric expressions at the Command prompt or within a command. | 具名拒绝（`cal.unsupported`）：未实现表达式求值器；不写图。 |
| 153 | `CAMERA` | 部分实现 | Sets a camera and target location to create and save a 3D perspective view of objects. | 具名拒绝（`camera.unsupported: this build has no 3D camera/target view model`）。 |
| 154 | `CDORDER` | 部分实现 | Arranges the drawing order of selected objects by their color number. | 真实编辑：按颜色索引升序重写选中（空选择=全部模型空间）绘制顺序；一条事务/一步撤销；无对话框。 |
| 155 | `CENTERDISASSOCIATE` | 部分实现 | Removes associativity of center marks or centerlines from objects they define. | 具名拒绝（`centerdisassociate.unsupported: center marks/centerlines are not modeled, so associativity/reset has nothing to edit`）。 |
| 156 | `CENTERLINE` | 部分实现 | Creates centerline geometry associated with selected lines and linear polyline segments. | 真实编辑：恰好两条平行直线之间创建一条中间直线，覆盖投影重叠；一条事务/一步撤销；无延伸/间隙选项。 |
| 157 | `CENTERMARK` | 部分实现 | Creates an associative, cross-shaped mark at the center of a selected circle or arc. | 真实编辑：每个选中圆/圆弧圆心两条交叉直线，臂长=半径 10%（声明规则）；一条事务/一步撤销；无关联对象/无可配置尺寸。 |
| 158 | `CENTERREASSOCIATE` | 部分实现 | Associates or reassociates a center mark or centerline object to selected objects. | 具名拒绝（`centerreassociate.unsupported: center marks/centerlines are not modeled, so associativity/reset has nothing to edit`）。 |
| 159 | `CENTERRESET` | 部分实现 | Resets the centerlines to the current value specified in the CENTEREXE system variable. | 具名拒绝（`centerreset.unsupported: center marks/centerlines are not modeled, so associativity/reset has nothing to edit`）。 |
| 160 | `CHAMFER` | 已实现 | Bevels or chamfers the edges of two 2D objects or the adjacent faces of a 3D solid. | `Distance`/`Angle`/`Method`/`Trim`/No-Trim/`Polyline` 全顶点；双零距离/角度=仅延伸相交；`Angle=0` 且距离>0 按名拒绝；`Multiple`/命令内 `Undo` 属逐选项提示全局范围；负距离按名拒绝 |
| 161 | `CHAMFEREDGE` | 部分实现 | Bevels the edges of 3D solids and surfaces. | 具名拒绝（`chamferedge.unsupported: no 3D edge chamfer model`）。 |
| 162 | `CHANGE` | 部分实现 | Changes the properties of existing objects. | 真实编辑：选中模型空间实体移到当前层并重置颜色/线型/线宽为 ByLayer；一条事务/一步撤销；无交互点/特性提示。 |
| 163 | `CHECKSTANDARDS` | 部分实现 | Checks the current drawing for standards violations. | 具名拒绝（`checkstandards.unsupported: this build has no CAD standards (DWS) model`）。 |
| 164 | `CHPROP` | 部分实现 | Changes the properties of an object. | 真实编辑：同 CHANGE（移到当前层并重置颜色/线型/线宽为 ByLayer）；一条事务/一步撤销；无交互提示。 |
| 165 | `CHSPACE` | 部分实现 | On a layout, transfers selected objects between model space and paper space. | 具名拒绝（`chspace.unsupported`）：模型/图纸空间之间移动实体未实现。 |
| 166 | `CHURLS` | 部分实现 | Provides a method to edit a previously attached URL for selected objects. | 具名拒绝（`churls.unsupported: this build has no hyperlink storage model`）。 |
| 167 | `CIRCLE` | 已实现 | Creates a circle. | — |
| 168 | `CLASSICGROUP` | 部分实现 | Opens the legacy Object Grouping dialog box. | chrome 面板：列出会话 `EntityGroups` 与成员数，行聚焦/展开成员引用/关闭；“按组选择成员”因无宿主替换选择路径而禁用 |
| 169 | `CLASSICIMAGE` | 部分实现 | Manages referenced image files in the current drawing. | chrome 面板：只读列出 IMAGE 实体（id/空间/file key/pixels/可见/clip 摘要），行聚焦/关闭；attach/clip/adjust 仍是独立命令 |
| 170 | `CLASSICINSERT` | 已实现 | Inserts a block or drawing into the current drawing using the classic version of the INSERT command. | chrome 面板：列出真实块定义（名称/成员数/基点），插入动作走既有 `INSERT` 点位+名称流程（面板在宿主确认文字步后提交块名）；无块元数据的定义不可插入（不伪造基点） |
| 171 | `CLASSICLAYER` | 部分实现 | Opens the legacy Layer Properties Manager. | chrome 本地选择子：打开现有图层面板；紧凑布局像 LAYER 一样拒绝。 |
| 172 | `CLASSICXREF` | 部分实现 | Manages referenced drawing files in the current drawing. | 真实只读外部参照报告 + chrome 面板（`cad_status.references` 的 name/status/reason），行聚焦/关闭/Esc；attach/detach/reload 仍按名拒绝 |
| 173 | `CLEANSCREENOFF` | 部分实现 | Restores the state of the display before CLEANSCREENON was used. | chrome 本地选择子：显式退出 clean-screen；只恢复 chrome 可见性，不重开各面板 |
| 174 | `CLEANSCREENON` | 部分实现 | Clears the screen of toolbars and dockable windows, excluding the command window. | chrome 本地选择子：显式进入 clean-screen；只隐藏 chrome，不记录各面板的独立开关 |
| 175 | `CLIP` | 部分实现 | Crops selected objects such as blocks, external references, images, viewports, and underlays to a specified boundary. | 具名拒绝（`clip.unsupported`）：无图像/外部参照裁剪模型。 |
| 176 | `CLIPIT` | 部分实现 | Clips xrefs or images with lines and curves. | 具名拒绝（`clipit.unsupported`）：无图像/外部参照裁剪模型。 |
| 177 | `CLOSE` | 部分实现 | Closes the current drawing. | 只关闭当前唯一文档并回到开始页；无未保存修改提示/保存确认（项目没有 dirty 模型），无多文档/CLOSEALL。 |
| 178 | `CLOSEALL` | 部分实现 | Closes all currently open drawings. | 具名拒绝（`closeall.single_document`）：本构建只有一个文档；请用 CLOSE。 |
| 179 | `CLOSEALLOTHER` | 部分实现 | Closes all other open drawings, except the current drawing. | 具名拒绝（`closeallother.single_document`）：只存在一个文档。 |
| 180 | `-COLOR` | 部分实现 | Sets the color for new objects from the Command prompt. | 同一当前颜色报告的命令行拼写。 |
| 181 | `COLOR` | 部分实现 | Sets the color for new objects. | 只读报告会话当前颜色 ACI（默认 256=ByLayer，`color.report`）；无创建路径消费该令牌，颜色对话框未实现。 |
| 182 | `COMMANDLINE` | 部分实现 | Displays the Command window. | chrome 本地选择子：显示并聚焦命令行。 |
| 183 | `COMMANDLINEHIDE` | 部分实现 | Hides the Command window. | chrome 本地选择子：隐藏命令行（与状态栏开关同一状态）。 |
| 184 | `COMMANDMACROS` | 部分实现 | Opens the Command Macros palette from which you can manage and use command macro recommendations. | 具名拒绝（`commandmacros.unsupported: this build has no command-macro palette model`）。 |
| 185 | `COMMANDMACROSCLOSE` | 部分实现 | Closes the Command Macros palette. | 具名拒绝（`commandmacrosclose.unsupported: this build has no command-macro palette model`）。 |
| 186 | `-COMPARE` | 部分实现 | At the Command prompt, compares a specified drawing to the current drawing. | 具名拒绝（`-compare.unsupported: this build has no drawing-compare/diff model`）。 |
| 187 | `COMPARE` | 部分实现 | Compares a specified drawing file with the current drawing file, highlighting the differences with color within revision clouds. | 具名拒绝（`compare.unsupported: this build has no drawing-compare/diff model`）。 |
| 188 | `COMPARECLOSE` | 部分实现 | Closes the DWG Compare toolbar and ends the comparison. | 具名拒绝（`compareclose.unsupported: this build has no drawing-compare/diff model`）。 |
| 189 | `COMPAREEXPORT` | 部分实现 | Exports the results of the comparison into a new drawing file, called a snapshot drawing. | 具名拒绝（`compareexport.unsupported: this build has no drawing-compare/diff model`）。 |
| 190 | `COMPAREIMPORT` | 部分实现 | Imports objects from the compared drawing into the current drawing. | 具名拒绝（`compareimport.unsupported: this build has no drawing-compare/diff model`）。 |
| 191 | `COMPAREINFO` | 部分实现 | Provides a way to insert or copy the property information about the two compared drawing files. | 具名拒绝（`compareinfo.unsupported: this build has no drawing-compare/diff model`）。 |
| 192 | `COMPILE` | 部分实现 | Compiles shape files and PostScript font files into SHX files. | 具名拒绝（`compile.unsupported: this build has no SHX shape/font compiler or loader`）。 |
| 193 | `CONE` | 部分实现 | Creates a 3D solid cone. | 具名拒绝（`cone.unsupported: 3D solid primitives are not implemented`）。 |
| 194 | `CONSTRAINTBAR` | 部分实现 | Displays or hides the geometric constraints on an object. | 具名拒绝（`constraintbar.unsupported`）：无约束模型。 |
| 195 | `CONSTRAINTSETTINGS` | 部分实现 | Controls the display of geometric constraints on constraint bars. | 具名拒绝（`constraintsettings.unsupported`）：无约束模型。 |
| 196 | `CONVERT` | 部分实现 | Converts "legacy" 2D polylines and hatches for use with later product releases. | 具名拒绝（`convert.unsupported`）：通用对象转换未实现。 |
| 197 | `CONVERTCTB` | 部分实现 | Converts a color-dependent plot style table (CTB) to a named plot style table (STB). | 具名拒绝（`convertctb.unsupported`）：CTB 转换未实现（无 CTB/STB 模型）。 |
| 198 | `CONVERTOLDLIGHTS` | 部分实现 | Converts lights created in previous drawing file formats to the current format. | 具名拒绝（`convertoldlights.unsupported: this build has no legacy light/material conversion model`）。 |
| 199 | `CONVERTOLDMATERIALS` | 部分实现 | Converts older materials to use the current materials format. | 具名拒绝（`convertoldmaterials.unsupported: this build has no legacy light/material conversion model`）。 |
| 200 | `CONVERTPOLY` | 部分实现 | Converts 2D polylines between the "legacy" heavy and optimized lightweight formats. | 只读报告（`convertpoly.single_representation`）：本构建只存一种 `LWPOLYLINE` 表示，无旧式多段线可转换。 |
| 201 | `CONVERTPSTYLES` | 部分实现 | Converts the current drawing to either named or color-dependent plot styles. | 具名拒绝（`convertpstyles.unsupported`）：打印样式转换未实现（无 CTB/STB 模型）。 |
| 202 | `CONVTOMESH` | 部分实现 | Converts 3D objects such as polygon meshes, surfaces, and solids to mesh objects. | 具名拒绝（`convtomesh.unsupported: 3D solids, surfaces and meshes are not modeled`）。 |
| 203 | `CONVTONURBS` | 部分实现 | Converts 3D solids and surfaces into NURBS surfaces. | 具名拒绝（`convtonurbs.unsupported: 3D solids, surfaces and meshes are not modeled`）。 |
| 204 | `CONVTOSOLID` | 部分实现 | Converts eligible 3D objects to 3D solids. | 具名拒绝（`convtosolid.unsupported: 3D solids, surfaces and meshes are not modeled`）。 |
| 205 | `CONVTOSURFACE` | 部分实现 | Converts objects to 3D surfaces. | 具名拒绝（`convtosurface.unsupported: 3D solids, surfaces and meshes are not modeled`）。 |
| 206 | `COORDINATIONMODELATTACH` | 部分实现 | Inserts references to coordination models such as NWD and NWC Navisworks files. | 具名拒绝（`coordinationmodelattach.unsupported: this build has no Navisworks coordination-model attachment model`）。 |
| 207 | `COPY` | 已实现 | Copies objects a specified distance in a specified direction. | — |
| 208 | `COPYBASE` | 部分实现 | Copies selected objects to the Clipboard along with a specified base point. | 应用内剪贴板 + 基点；不是 OS 剪贴板，不跨应用 |
| 209 | `COPYCLIP` | 部分实现 | Copies selected objects to the Clipboard. | 应用内剪贴板；不是 OS 剪贴板，不跨应用 |
| 210 | `COPYFROMTRACE` | 部分实现 | Copies objects from a trace into the drawing. | 具名拒绝（`copyfromtrace.unsupported: this build has no collaboration trace model`）。 |
| 211 | `COPYHIST` | 部分实现 | Copies the text in the command line history to the Clipboard. | 只读报告（`copyhist.not_tracked`）：会话不存命令历史文本，界面命令日志自管；未复制任何内容。 |
| 212 | `COPYLINK` | 部分实现 | Copies the current view to the Clipboard for linking to other OLE applications. | 具名拒绝（`copylink.unsupported: OLE/clipboard linking is owned by the host`）。 |
| 213 | `COPYM` | 部分实现 | Copies multiple objects with Repeat, Array, Divide and Measure options. | 具名拒绝（`copym.unsupported: this build has no multi-copy (Repeat/Array/Divide/Measure) session model`）。 |
| 214 | `-COPYTOLAYER` | 部分实现 | Copies one or more objects to another layer from the Command prompt. | 真实编辑：同 COPYTOLAYER，命令行拼写；一条事务/一步撤销。 |
| 215 | `COPYTOLAYER` | 部分实现 | Copies one or more objects to another layer. | 真实编辑：首个选中实体图层为目标，其余实体以新 id 复制到该层；原实体不变；一条事务/一步撤销；无图层提示。 |
| 216 | `COUNT` | 部分实现 | Counts and highlights the instances of the selected object in the drawing. | 只读：统计当前选择；空选择时统计全部模型空间顶层实体（按类型分组）；不修改选择、不开事务。 |
| 217 | `COUNTAREA` | 部分实现 | Defines the area to count the instances of an object or block. | 具名拒绝（`countarea.unsupported: this build has no object-count session model`）。 |
| 218 | `COUNTAREACLOSE` | 部分实现 | Cancels the count selection area. | 具名拒绝（`countareaclose.unsupported: this build has no object-count session model`）。 |
| 219 | `COUNTCLOSE` | 部分实现 | Closes the Count toolbar and exits the count. | 具名拒绝（`countclose.unsupported: this build has no object-count session model`）。 |
| 220 | `COUNTFIELD` | 部分实现 | Creates a field that's set to the value of the current count. | 具名拒绝（`countfield.unsupported: this build has no object-count session model`）。 |
| 221 | `COUNTLIST` | 部分实现 | Opens the Count palette to display and manage the counted blocks. | 具名拒绝（`countlist.unsupported: this build has no object-count session model`）。 |
| 222 | `COUNTLISTCLOSE` | 部分实现 | Closes the Count palette. | 具名拒绝（`countlistclose.unsupported: this build has no object-count session model`）。 |
| 223 | `COUNTNAVNEXT` | 部分实现 | Zooms to the next object in the count result. | 具名拒绝（`countnavnext.unsupported: this build has no object-count session model`）。 |
| 224 | `COUNTNAVPREV` | 部分实现 | Zooms to the previous object in the count result. | 具名拒绝（`countnavprev.unsupported: this build has no object-count session model`）。 |
| 225 | `COUNTTABLE` | 部分实现 | Inserts a table containing the block names and the corresponding count of each block in the drawing. | 具名拒绝（`counttable.unsupported: this build has no object-count session model`）。 |
| 226 | `CUI` | 部分实现 | Manages the customized user interface elements in the product. | 具名拒绝（`cui.unsupported: this build has no CUIx customization model`）。 |
| 227 | `CUIEXPORT` | 部分实现 | Exports customized settings from the main CUIx file to an enterprise or partial CUIx file. | 具名拒绝（`cuiexport.unsupported: this build has no CUIx customization model`）。 |
| 228 | `CUIIMPORT` | 部分实现 | Imports customized settings from an enterprise or partial CUIx file to the main CUIx file. | 具名拒绝（`cuiimport.unsupported: this build has no CUIx customization model`）。 |
| 229 | `CUILOAD` | 部分实现 | Loads a customization file (CUIx). | 具名拒绝（`cuiload.unsupported: this build has no CUIx customization model`）。 |
| 230 | `CUIUNLOAD` | 部分实现 | Unloads a CUIx file. | 具名拒绝（`cuiunload.unsupported: this build has no CUIx customization model`）。 |
| 231 | `CUSTOMIZE` | 部分实现 | Customizes tool palettes and tool palette groups. | 具名拒绝（`customize.unsupported: this build has no tool-palette customization model`）。 |
| 232 | `CUTBASE` | 部分实现 | Copies selected objects to the Clipboard, along with a specified base point, and removes them from the drawing. | 应用内剪贴板 + 基点；不是 OS 剪贴板，不跨应用 |
| 233 | `CUTCLIP` | 部分实现 | Copies selected objects to the Clipboard and removes them from the drawing. | 应用内剪贴板；不是 OS 剪贴板，不跨应用 |
| 234 | `CVADD` | 部分实现 | Adds control vertices to NURBS surfaces and splines. | 具名拒绝（`cvadd.unsupported: NURBS control-vertex editing is not modeled`）。 |
| 235 | `CVHIDE` | 部分实现 | Turns off the display of control vertices for all NURBS surfaces and curves. | 具名拒绝（`cvhide.unsupported: NURBS control-vertex editing is not modeled`）。 |
| 236 | `CVREBUILD` | 部分实现 | Rebuilds the shape of NURBS surfaces and curves. | 具名拒绝（`cvrebuild.unsupported: NURBS control-vertex editing is not modeled`）。 |
| 237 | `CVREMOVE` | 部分实现 | Removes control vertices from NURBS surfaces and curves. | 具名拒绝（`cvremove.unsupported: NURBS control-vertex editing is not modeled`）。 |
| 238 | `CVSHOW` | 部分实现 | Displays the control vertices for specified NURBS surfaces or curves. | 具名拒绝（`cvshow.unsupported: NURBS control-vertex editing is not modeled`）。 |
| 239 | `CYLINDER` | 部分实现 | Creates a 3D solid cylinder. | 具名拒绝（`cylinder.unsupported: 3D solid primitives are not implemented`）。 |
| 240 | `-DATAEXTRACTION` | 部分实现 | Extracts data as specified in an existing attribute extraction template (BLK) file or data extraction (DXE or DXEX) file. | 具名拒绝（`-dataextraction.unsupported: this build has no data-extraction/attribute-template model`）。 |
| 241 | `DATAEXTRACTION` | 部分实现 | Extracts drawing data and merges data from an external source to a data extraction table or external file. | 具名拒绝（`dataextraction.unsupported: this build has no data-extraction/attribute-template model`）。 |
| 242 | `DATALINK` | 部分实现 | Displays the Data Link dialog box. | 具名拒绝（`datalink.unsupported: this build has no external data-link model`）。 |
| 243 | `DATALINKUPDATE` | 部分实现 | Updates data to or from an established external data link. | 具名拒绝（`datalinkupdate.unsupported: this build has no external data-link model`）。 |
| 244 | `DBCCLOSE` | 部分实现 | Closes the Select Data Object dialog box (dbConnect Manager). | 具名拒绝（`dbcclose.unsupported: this build has no dbConnect/external database model`）。 |
| 245 | `DBCCONFIGURE` | 部分实现 | Opens the Configure a Data Source dialog box (dbConnect Manager) | 具名拒绝（`dbcconfigure.unsupported: this build has no dbConnect/external database model`）。 |
| 246 | `DBCDEFINELLT` | 部分实现 | Opens the Select a Database Object dialog box. | 具名拒绝（`dbcdefinellt.unsupported: this build has no dbConnect/external database model`）。 |
| 247 | `DBCDEFINELT` | 部分实现 | Opens the Select Data Object Dialog Box (dbConnect Manager) | 具名拒绝（`dbcdefinelt.unsupported: this build has no dbConnect/external database model`）。 |
| 248 | `DBCONNECT` | 部分实现 | Provides an interface to external database tables. | 具名拒绝（`dbconnect.unsupported: this build has no dbConnect/external database model`）。 |
| 249 | `DBLIST` | 部分实现 | Lists database information for each object in the drawing. | 只读列出所有顶层模型/图纸实体（id/type/layer/space/draw order）；块定义成员计数并跳过 |
| 250 | `DCALIGNED` | 部分实现 | Constrains the distance between two points on different objects. | 具名拒绝（`dcaligned.unsupported`）：本构建无尺寸约束模型。 |
| 251 | `DCANGULAR` | 部分实现 | Constrains the angle between line or polyline segments, the angle swept out by an arc or a polyline arc segment, or the angle between three points on objects. | 具名拒绝（`dcangular.unsupported`）：本构建无尺寸约束模型。 |
| 252 | `DCCONVERT` | 部分实现 | Converts associative dimensions to dimensional constraints. | 具名拒绝（`dcconvert.unsupported`）：本构建无尺寸约束模型。 |
| 253 | `DCDIAMETER` | 部分实现 | Constrains the diameter of a circle or an arc. | 具名拒绝（`dcdiameter.unsupported`）：本构建无尺寸约束模型。 |
| 254 | `DCDISPLAY` | 部分实现 | Displays or hides the dynamic constraints associated with a selection set of objects. | 具名拒绝（`dcdisplay.unsupported`）：本构建无尺寸约束模型。 |
| 255 | `DCFORM` | 部分实现 | Specifies whether the dimensional constraint being created is dynamic or annotational. | 具名拒绝（`dcform.unsupported`）：本构建无尺寸约束模型。 |
| 256 | `DCHORIZONTAL` | 部分实现 | Constrains the X distance between points on an object, or between two points on different objects. | 具名拒绝（`dchorizontal.unsupported`）：本构建无尺寸约束模型。 |
| 257 | `DCLINEAR` | 部分实现 | Creates a horizontal, vertical, or rotated constraint based on the locations of the extension line origins and the dimension line. | 具名拒绝（`dclinear.unsupported`）：本构建无尺寸约束模型。 |
| 258 | `DCRADIUS` | 部分实现 | Constrains the radius of a circle or an arc. | 具名拒绝（`dcradius.unsupported: this build has no dimensional-constraint model`）。 |
| 259 | `DCVERTICAL` | 部分实现 | Constrains the Y distance between points on an object, or between two points on different objects. | 具名拒绝（`dcvertical.unsupported: this build has no dimensional-constraint model`）。 |
| 260 | `DELAY` | 部分实现 | Provides a timed pause within a script. | 具名拒绝（`delay.unsupported: this build has no script runner, so a timed pause has no effect`）。 |
| 261 | `DELCONSTRAINT` | 部分实现 | Removes all geometric and dimensional constraints from a selection set of objects. | 具名拒绝（`delconstraint.unsupported`）：本构建无几何约束模型。 |
| 262 | `DETACHURL` | 部分实现 | Removes hyperlinks in a drawing. | 具名拒绝（`detachurl.unsupported: this build has no hyperlink storage model`）。 |
| 263 | `DETECTCLOSE` | 部分实现 | Ends the detection review. | 具名拒绝（`detectclose.unsupported: this build has no object-detection model`）。 |
| 264 | `DETECTCONVERT` | 部分实现 | Displays the Convert dialog box, which provides options to convert the detected objects and instances into blocks. | 具名拒绝（`detectconvert.unsupported: this build has no object-detection model`）。 |
| 265 | `DETECTPRIMARY` | 部分实现 | Specifies the detected instance from which a new block definition is created. | 具名拒绝（`detectprimary.unsupported: this build has no object-detection model`）。 |
| 266 | `DETECTREMOVE` | 部分实现 | Removes selected instances from the set. | 具名拒绝（`detectremove.unsupported: this build has no object-detection model`）。 |
| 267 | `DETECTREVIEWNEXT` | 部分实现 | Displays the next set of detected objects. | 具名拒绝（`detectreviewnext.unsupported: this build has no object-detection model`）。 |
| 268 | `DETECTREVIEWPREV` | 部分实现 | Displays the previous set of detected objects. | 具名拒绝（`detectreviewprev.unsupported: this build has no object-detection model`）。 |
| 269 | `DGNADJUST` | 部分实现 | Adjust the fade, contrast, and monochrome settings of a DGN underlay. | 具名拒绝（`dgnadjust.unsupported: no DGN underlay adjustment model`）。 |
| 270 | `-DGNATTACH` | 部分实现 | Attach a DGN underlay from the command line. | 具名拒绝（`-dgnattach.unsupported: DGN underlays are not implemented`）。 |
| 271 | `DGNATTACH` | 部分实现 | Inserts a DGN file as an underlay into the current drawing. | 具名拒绝（`dgnattach.unsupported: DGN underlays are not implemented`）。 |
| 272 | `DGNCLIP` | 部分实现 | Crops the display of a selected DGN underlay to a specified boundary. | 具名拒绝（`dgnclip.unsupported: no DGN underlay clip model`）。 |
| 273 | `-DGNEXPORT` | 部分实现 | At the Command prompt, creates one or more DGN files from the current drawing. | 具名拒绝（`-dgnexport.unsupported: this build has no DGN exporter`）。 |
| 274 | `DGNEXPORT` | 部分实现 | Creates one or more DGN files from the current drawing. | 具名拒绝（`dgnexport.unsupported: this build has no DGN exporter`）。 |
| 275 | `-DGNIMPORT` | 部分实现 | At the Command prompt, imports data from DGN files. | 具名拒绝（`-dgnimport.unsupported: this build has no DGN importer/exporter or mapping model`）。 |
| 276 | `DGNIMPORT` | 部分实现 | Imports the data from a DGN file into a new DWG file or the current DWG file, depending on the DGNIMPORTMODE system variable. | 具名拒绝（`dgnimport.unsupported: this build has no DGN importer/exporter or mapping model`）。 |
| 277 | `DGNLAYERS` | 部分实现 | Controls the display of layers in a DGN underlay. | 具名拒绝（`dgnlayers.unsupported: no DGN underlay layer model`）。 |
| 278 | `DGNMAPPING` | 部分实现 | Allows users to create and edit user-defined DGN mapping setups. | 具名拒绝（`dgnmapping.unsupported: this build has no DGN importer/exporter or mapping model`）。 |
| 279 | `DIGITALSIGN` | 部分实现 | Attaches a digital signature to a drawing, which is removed if an unauthorized change is made. | 具名拒绝（`digitalsign.unsupported: this build has no digital-signature model`）。 |
| 280 | `DIM` | 部分实现 | Creates multiple dimensions and types of dimensions with a single command. | 只读路由报告（`dim.available`/`dim.router_only`）：列出已实现的 `DIM*` 创建选择子；不创建标注，无交互式类型提示。 |
| 281 | `DIMALIGNED` | 已实现 | Creates an aligned linear dimension. | — |
| 282 | `DIMANGULAR` | 部分实现 | Creates an angular dimension. | 仅两线形式可交互；三点形式只到载荷层 |
| 283 | `DIMARC` | 部分实现 | Creates an arc length dimension. | 具名拒绝（`dimarc.unsupported`）：标注域无弧长类型。 |
| 284 | `DIMBASELINE` | 已实现 | Creates a linear, angular, or ordinate dimension from the baseline of the previous or selected dimension. | — |
| 285 | `DIMBREAK` | 部分实现 | Breaks or restores dimension and extension lines where they cross other objects. | 按 `dimbreak.unsupported` 拒绝（无标注线打断记录）；绝不删除/裁剪标注本身 |
| 286 | `DIMCENTER` | 已实现 | Creates the non-associative center mark or the centerlines of circles and arcs. | — |
| 287 | `DIMCONSTRAINT` | 部分实现 | Applies dimensional constraints to selected objects or points on objects, or converts associative dimensions to dimensional constraints. | 具名拒绝（`dimconstraint.unsupported`）：本构建无尺寸约束模型。 |
| 288 | `DIMCONTINUE` | 已实现 | Creates a dimension that starts from an extension line of the previous or selected dimension. | — |
| 289 | `DIMDIAMETER` | 已实现 | Creates a diameter dimension for a circle or an arc. | — |
| 290 | `DIMDISASSOCIATE` | 部分实现 | Removes associativity from selected dimensions. | payload-free；无关联模型，`dimdisassociate.already_non_associative` 且不改变任何状态 |
| 291 | `DIMEDIT` | 部分实现 | Edits dimension text and extension lines. | 对所有选中标注 `TEXT <override>`（组 1，`<>` 占位，≤1024）或 `RESET`；一条事务/一步撤销；延伸线编辑未实现 |
| 292 | `DIMEX` | 部分实现 | Exports named dimension styles and their settings to an external file. | 具名拒绝（`dimex.unsupported`）：未建模标注样式导出文件。 |
| 293 | `DIMIM` | 部分实现 | Imports named dimension styles and their settings from an external file. | 具名拒绝（`dimim.unsupported`）：未建模标注样式导入文件。 |
| 294 | `-DIMINSPECT` | 部分实现 | From the Command prompt, adds or removes inspection information from a selected dimension. | 具名拒绝（`-diminspect.unsupported`）：同一拒绝的命令行拼写。 |
| 295 | `DIMINSPECT` | 部分实现 | Adds or removes inspection information for a selected dimension. | 具名拒绝（`diminspect.unsupported`）：标注域无检验数据。 |
| 296 | `DIMJOGGED` | 部分实现 | Creates jogged dimensions for circles and arcs. | 具名拒绝（`dimjogged.unsupported`）：标注域无折弯字段。 |
| 297 | `DIMJOGLINE` | 部分实现 | Adds or removes a jog line on a linear or aligned dimension. | `ADD`/`REMOVE` 解析后按 `dimjogline.unsupported` 拒绝（域无 jog 字段/存储） |
| 298 | `DIMLINEAR` | 已实现 | Creates a linear dimension. | — |
| 299 | `DIMORDINATE` | 已实现 | Creates ordinate dimensions. | — |
| 300 | `DIMOVERRIDE` | 部分实现 | Controls overrides of system variables used in selected dimensions. | 闭集的实体快照变量；未知变量/仅表变量/坏值/未实现值分别具名拒绝；一条事务/一步撤销；不改样式表 |
| 301 | `DIMRADIUS` | 已实现 | Creates a radius dimension for a circle or an arc. | — |
| 302 | `DIMREASSOC` | 部分实现 | Restores the measured value to overridden or modified dimension text. | 非关联标注 no-op（`dimreassoc.non_associative`）；无状态变化。 |
| 303 | `DIMREASSOCIATE` | 部分实现 | Associates or reassociates selected dimensions to objects or points on objects. | 非关联标注 no-op（`dimreassociate.non_associative`）；无状态变化。 |
| 304 | `DIMREGEN` | 部分实现 | Updates the locations of all associative dimensions. | payload-free；本构建非关联，`dimregen.non_associative` 且不改变任何状态 |
| 305 | `DIMROTATED` | 部分实现 | Creates a rotated linear dimension. | 三点拾取（两尺寸界线原点 + 尺寸线位置）；角度为会话 `dimRotation` 参数（度，任意有限值，不回绕/不归一化）；无交互角度提示、无对象选择/文字覆盖/关联 |
| 306 | `DIMSPACE` | 已实现 | Adjusts the spacing between linear dimensions or angular dimensions. | 首个线性/对齐/角度标注为基准；显式间距（`0` = 对齐）或 Auto = **2 × 基准样式文字高度**；角度标注按同心弧半径步进；模型空间；关联重排属全局范围 |
| 307 | `-DIMSTYLE` | 部分实现 | At the Command prompt, creates and modifies dimension styles. | `DIMSTYLE` 的别名，同一实现 |
| 308 | `DIMSTYLE` | 已实现 | Creates and modifies dimension styles. | `LIST`/`CURRENT`/`SET <name>`/`NEW`/`RENAME`/`DELETE`/`MODIFY <field> <value>`；一条 dim-style patch/一步撤销；`Standard` 不可删、被标注引用的行不可改删；`MODIFY` 不回写已烘焙的标注快照（无 DIMREGEN 级联） |
| 309 | `DIMTEDIT` | 部分实现 | Moves and rotates dimension text and relocates the dimension line. | `MOVE`/`LEFT`/`RIGHT`/`CENTER`/`HOME`/`ANGLE`；Left/Right/Center 同时移动锚点并设置字形级 `h_align`（仅线性/对齐/半径/直径）；Angle 固定文字中心，`0` 复位；一条事务/一步撤销；角/坐标标注 justify 与 Oblique 未实现 |
| 310 | `DIST` | 已实现 | Measures the distance and angle between two points. | — |
| 311 | `DISTANTLIGHT` | 部分实现 | Creates a distant light. | 具名拒绝（`distantlight.unsupported: 3D light objects are not modeled`）。 |
| 312 | `DIVIDE` | 已实现 | Creates evenly spaced point objects or blocks along the length or perimeter of an object. | `LINE`/`ARC`/`CIRCLE`/`SPLINE`/`ELLIPSE`（含椭圆弧）/含 bulge 折线，按**等弧长**；开放 `N−1`、闭合 `N`；块对齐 Yes/No |
| 313 | `DONUT` | 部分实现 | Creates a filled circle or a wide ring. | 类型化 `DONUT <cx> <cy> <inner> <outer>`；两个同心圆；`inner=0` 只建外圆并诊断；无中心拾取提示 |
| 314 | `DOWNLOADMANAGER` | 部分实现 | Reports the status of the of the current download. | 具名拒绝（`downloadmanager.unsupported: this build has no download-manager model`）。 |
| 315 | `DRAGMODE` | 部分实现 | Controls the way dragged objects are displayed. | 会话令牌 `dragmode`（默认 ON）；渲染器不消费，无拖动预览。 |
| 316 | `DRAWINGRECOVERY` | 部分实现 | Displays a list of drawing files that can be recovered after a program or system failure. | 具名拒绝（`drawingrecovery.unsupported: this build has no drawing-recovery manager model`）。 |
| 317 | `DRAWINGRECOVERYHIDE` | 部分实现 | Closes the Drawing Recovery Manager. | 具名拒绝（`drawingrecoveryhide.unsupported: this build has no drawing-recovery manager model`）。 |
| 318 | `DRAWORDER` | 部分实现 | Changes the draw order of images and other objects. | `FRONT\|BACK\|ABOVE\|BELOW`；ABOVE/BELOW 第一个选中为参照；一条事务，保持相对顺序 |
| 319 | `DSETTINGS` | 部分实现 | Sets grid and snap, polar and object snap tracking, object snap modes, Dynamic Input, and Quick Properties. | chrome 本地选择子：打开现有 Options 对话框（Drafting 页），不读写图纸。 |
| 320 | `DVIEW` | 部分实现 | Defines parallel projection or perspective views by using a camera and target. | 具名拒绝（`dview.unsupported: this build has no 3D camera/target view model`）。 |
| 321 | `DWFADJUST` | 部分实现 | Adjust the fade, contrast, and monochrome settings of a DWF or DWFx underlay. | 具名拒绝（`dwfadjust.unsupported: this build has no DWF/DWFx underlay or export model`）。 |
| 322 | `-DWFATTACH` | 部分实现 | From the Command prompt, attach a DWF underlay. | 具名拒绝（`-dwfattach.unsupported: this build has no DWF/DWFx underlay or export model`）。 |
| 323 | `DWFATTACH` | 部分实现 | Inserts a DWF or DWFx file as an underlay into the current drawing. | 具名拒绝（`dwfattach.unsupported: this build has no DWF/DWFx underlay or export model`）。 |
| 324 | `DWFCLIP` | 部分实现 | Crops the display of a selected DWF or DWFx underlay to a specified boundary. | 具名拒绝（`dwfclip.unsupported: this build has no DWF/DWFx underlay or export model`）。 |
| 325 | `DWFFORMAT` | 部分实现 | Sets the default format to DWF or DWFx for output in specific commands. | 只读能力报告（`dwfformat.not_supported`）：本构建不导出 DWF/DWFx；请用 PLOT 输出 SVG/PDF。 |
| 326 | `DWFLAYERS` | 部分实现 | Controls the display of layers in a DWF or DWFx underlay. | 具名拒绝（`dwflayers.unsupported: this build has no DWF/DWFx underlay or export model`）。 |
| 327 | `DWGCONVERT` | 部分实现 | Converts drawing format version for selected drawing files. | 具名拒绝（`dwgconvert.unsupported: batch DWG conversion is not implemented`）。 |
| 328 | `DWGLOG` | 部分实现 | Creates and maintains an individual log file for each drawing file as it is accessed. | 会话令牌（默认关）：翻转 `dwglog`；无日志文件、不写图/不升修订。 |
| 329 | `DWGPROPS` | 部分实现 | Sets and displays the file properties of the current drawing. | 只读：文档 id/修订/实体数（`dwgprops.report`）；标题/作者/自定义特性未建模（`dwgprops.metadata_not_tracked`）。 |
| 330 | `DXBIN` | 部分实现 | Imports an AutoCAD DXB (drawing interchange binary) file. | 具名拒绝（`dxbin.unsupported: DXB import is not implemented`）。 |
| 331 | `EATTEDIT` | 部分实现 | Edits attributes in a block reference. | 具名拒绝（`eattedit.unsupported`）：本构建无块属性模型。 |
| 332 | `-EATTEXT` | 部分实现 | From the Command line, extracts data as specified in an existing attribute extraction template (BLK) file or data extraction (DXE or DXEX) file. | 具名拒绝（`-eatttext.unsupported`）：本构建无块属性模型。 |
| 333 | `EATTEXT` | 部分实现 | Exports block attribute information to a table or to an external file. | 具名拒绝（`eatttext.unsupported`）：本构建无块属性模型。 |
| 334 | `EDGE` | 部分实现 | Changes the visibility of 3D face edges. | 具名拒绝（`edge.unsupported: no 3D face-edge visibility model`）。 |
| 335 | `EDGESURF` | 部分实现 | Creates a mesh between four contiguous edges or curves. | 具名拒绝（`edgesurf.unsupported: 3D mesh surfaces are not modeled`）。 |
| 336 | `EDITSHOT` | 部分实现 | Edits a saved named view with or without motion. | 具名拒绝（`editshot.unsupported: this build has no ShowMotion named-view/motion model`）。 |
| 337 | `EDITTIME` | 部分实现 | Tracks the amount of active editing time for a drawing. | 只读：当前 UTC 时钟 + 文档修订/实体数；创建/修改时间与会话用时 NOT TRACKED。 |
| 338 | `ELEV` | 部分实现 | Sets elevation and extrusion thickness of new objects. | 只读报告会话标高档位（默认 0.0）；`ELEV <height>` 设置形式未实现，不按标高创建对象。 |
| 339 | `ELLIPSE` | 已实现 | Creates an ellipse or an elliptical arc. | 轴端点整椭圆；payload 新增 **弧模式**（`start`/`sweep`，≤一圈）与**绕长轴旋转**（`normal` + `ratio·cos R` 的 DXF 投影规则）；点击臂仍只产整椭圆 |
| 340 | `ERASE` | 已实现 | Removes objects from a drawing. | — |
| 341 | `-ETRANSMIT` | 部分实现 | From the Command line, packages a set of files for internet transmission. | 具名拒绝（`-etransmit.unsupported`）：本构建无 eTransmit/zip 打包模型；命令行拼写。 |
| 342 | `ETRANSMIT` | 部分实现 | Packages a set of files for Internet transmission. | 具名拒绝（`etransmit.unsupported`）：本构建无 eTransmit/zip 打包模型。 |
| 343 | `EXOFFSET` | 部分实现 | Offsets selected objects. | Express 偏移名；转交唯一 `OFFSET` 规划器；边界同 `OFFSET`。 |
| 344 | `EXPLAN` | 部分实现 | Displays an orthographic view of the XY plane of a specified UCS without changing the view magnification. | 具名拒绝（`explan.unsupported: this build has no UCS model; the world +Z plan path is PLAN`）。 |
| 345 | `EXPLODE` | 部分实现 | Breaks a compound object into its component objects. | 一层分解；标注→线/弧/箭头/文字、多重引线→引线/样条/箭头/MTEXT、非均匀块可表示子集；`EXPMODE` 无会话通道 |
| 346 | `-EXPORT` | 部分实现 | Creates a DWF, DWFx, or PDF file from the command line. | 具名拒绝（`-export.unsupported`）：请用 PLOT/EXPORT；本构建只有一条矢量导出路径；命令行拼写。 |
| 347 | `EXPORT` | 部分实现 | Saves the objects in a drawing to a different file format. | PLOT 别名：仅矢量 SVG/PDF |
| 348 | `EXPORTDWF` | 部分实现 | Creates a DWF file and allows you to set individual page setup overrides on a sheet-by-sheet basis. | 具名拒绝（`exportdwf.unsupported: this build has no DWF/DWFx exporter`）。 |
| 349 | `EXPORTDWFX` | 部分实现 | Creates a DWFx file where you can set individual page setup overrides on a sheet-by-sheet basis. | 具名拒绝（`exportdwfx.unsupported: this build has no DWF/DWFx exporter`）。 |
| 350 | `EXPORTLAYOUT` | 部分实现 | Creates a visual representation of the current layout in the model space of a new drawing. | 具名拒绝（`exportlayout.unsupported`）：请用 PLOT（单一布局，SVG/PDF）。 |
| 351 | `EXPORTPDF` | 部分实现 | Generates a PDF file from a single layout, all layouts, or a specified area in model space. | 只读能力报告（`exportpdf.use_plot`）：PDF 导出由宿主经 PLOT 负责，本命令自身不写文件。 |
| 352 | `EXPORTSETTINGS` | 部分实现 | Adjusts the page setup and drawing selection when exporting to a DWF, DWFx, or PDF file. | 具名拒绝（`exportsettings.unsupported`）：本构建无导出设置模型。 |
| 353 | `EXPORTTOAUTOCAD` | 部分实现 | Creates a version of a drawing file that can be opened in products such as AutoCAD and previous releases of a toolset. | 具名拒绝（`exporttoautocad.unsupported: exporting to another AutoCAD version is not implemented`）。 |
| 354 | `EXPRESSMENU` | 部分实现 | Loads the AutoCAD Express Tools menu and displays the Express menu on the menu bar. | 具名拒绝（`expressmenu.unsupported: this build has no Express Tools menu/library loader`）。 |
| 355 | `EXPRESSTOOLS` | 部分实现 | Loads the AutoCAD Express Tools libraries, places the Express folder in the search path, and loads and places the Express menu on the menu bar. | 具名拒绝（`expresstools.unsupported: this build has no Express Tools menu/library loader`）。 |
| 356 | `EXTEND` | 部分实现 | Extends objects to meet the edges of other objects. | 目标 LINE/ARC/**ELLIPSE（含椭圆弧）**；弧可超过半圈（至整圈前）；`SPLINE` 目标与 `CIRCLE`/整椭圆按名拒绝；折线/INSERT/HATCH 等目标按名拒绝 |
| 357 | `EXTERNALREFERENCES` | 部分实现 | Opens the External References palette. | 只读：外部参照导入溯源报告（`externalreferences.entry`/`externalreferences.summary`）；本构建无选项板（`externalreferences.palette_not_implemented`）。 |
| 358 | `EXTERNALREFERENCESCLOSE` | 部分实现 | Closes the External References palette. | 具名 no-op（`externalreferences.close_not_implemented`）：本构建无外部参照选项板，无窗口可关闭。 |
| 359 | `EXTRIM` | 部分实现 | Trims all objects at the cutting edge specified by a selected polyline, line, circle, arc, ellipse, text, mtext, or attribute definition. | 第一个选中为边界，其余为被裁对象；复用 trim 规划；不支持边界/目标按名拒绝 |
| 360 | `EXTRUDE` | 部分实现 | Creates a 3D solid from an object that encloses an area, or a 3D surface from an object with open ends. | 具名拒绝（`extrude.unsupported: 3D extrusion is not implemented`）。 |
| 361 | `FADEMARKUP` | 部分实现 | Fade individual markups so they are less visible on a trace. | 具名拒绝（`fademarkup.unsupported: this build has no collaboration markup/trace model`）。 |
| 362 | `FIELD` | 部分实现 | Creates a multiline text object with a field that can be updated automatically as the field value changes. | 具名拒绝（`field.unsupported`）：字段未实现（无字段模型）。 |
| 363 | `FILETAB` | 已实现 | Displays the file tabs at the top of the drawing area. | chrome 本地选择子：显示文件/图形标签栏（单文档构建仍有该标签） |
| 364 | `FILETABCLOSE` | 已实现 | Hides the file tabs at the top of the drawing area. | chrome 本地选择子：隐藏文件/图形标签栏 |
| 365 | `FILL` | 部分实现 | Controls the display of filled objects such as hatches, 2D solids, and wide polylines. | 会话令牌 `fill`（默认 ON）；渲染器不消费该标志，无视觉变化；不写库/不入历史。 |
| 366 | `FILLET` | 部分实现 | Rounds or fillets the edges of two 2D objects or the adjacent faces of a 3D solid. | `Radius`/`Trim`/`Polyline`、零半径尖角、平行线、圆/圆弧对曲线；本轮新增 **ELLIPSE ↔ LINE/CIRCLE/ARC/ELLIPSE** 数值圆角（采样+Newton+投影抛光，椭圆按参数区间修剪）；`SPLINE` 对按名拒绝；`Multiple`/多解循环未实现 |
| 367 | `FILLETEDGE` | 部分实现 | Rounds and fillets the edges of solid objects. | 具名拒绝（`filletedge.unsupported: no 3D edge fillet model`）。 |
| 368 | `FILTER` | 部分实现 | Creates a list of requirements that an object must meet to be included in a selection set. | `AND`/`OR`/`XOR`/`NOT` 递归逻辑树（括号、元数校验）、相对运算符、已建模属性集；只读；**分类/自定义特性未建模**；**命名 `.nfl` 过滤器未实现** |
| 369 | `-FIND` | 部分实现 | Searches for the text that you specify in the entire drawing, the current space or layout, or selected objects. | `-FIND <needle>`；在模型空间 Text/MText/MULTILEADER 中做 ASCII 不区分大小写子串搜索并报告；不写库 |
| 370 | `FIND` | 部分实现 | Finds the text that you specify, and can optionally replace it with other text. | `FIND <needle> [=> <replacement>]`；替换形式一条事务/一步撤销；不解析 MTEXT 格式 run；TEXT 单行拒绝换行 |
| 371 | `FINDINCLUDE` | 部分实现 | Controls the types of text objects included in a search when using the -FIND command. | 会话令牌（默认 ON）；跨块/外部参照文字搜索未实现。 |
| 372 | `FINDMATCH` | 部分实现 | Controls how the entered text string is compared with text in objects when using the -FIND command. | 会话令牌（默认 OFF）；已实现的 -FIND/FIND 恒按 ASCII 不区分大小写比较。 |
| 373 | `FLATSHOT` | 部分实现 | Creates a 2D representation of all 3D objects based on the current view. | 具名拒绝（`flatshot.unsupported: 3D solids are not modeled, so there is nothing to flatten`）。 |
| 374 | `FLATTEN` | 部分实现 | Converts 3D geometry to a projected 2D representation. | 真实编辑：选中 LINE/POLYLINE/CIRCLE/ARC/ELLIPSE/POINT/SPLINE 的 z 置 0；一条事务/一步撤销；MESH/INSERT/3D 按名拒绝（Express 行为简化为 z=0）。 |
| 375 | `FREESPOT` | 部分实现 | Creates a free spotlight which is a spotlight without a specified target. | 具名拒绝（`freespot.unsupported: 3D light objects are not modeled`）。 |
| 376 | `FREEWEB` | 部分实现 | Creates a free web light which is a web light without a specified target. | 具名拒绝（`freeweb.unsupported: 3D light objects are not modeled`）。 |
| 377 | `FS` | 部分实现 | Creates a selection set of all objects that touch the selected object. | Express 工具、仅命令行；当前唯一选择为种子，并入**直接**接触（相交或相切）的可见模型空间对象（默认 FSMODE=OFF；链式 FSMODE=ON 未实现）；不展开块实例成员 |
| 378 | `GATTE` | 部分实现 | Globally changes attribute values for all instances of a specified block. | 具名拒绝（`gatte.unsupported`）：本构建无块属性模型。 |
| 379 | `GCCOINCIDENT` | 部分实现 | Constrains two points together or a point to a curve (or an extension of a curve). | 具名拒绝（`gccoincident.unsupported: this build has no geometric-constraint model`）。 |
| 380 | `GCCOLLINEAR` | 部分实现 | Causes two or more line segments to lie along the same line. | 具名拒绝（`gccollinear.unsupported: this build has no geometric-constraint model`）。 |
| 381 | `GCCONCENTRIC` | 部分实现 | Constrains two arcs, circles, or ellipses to the same center point. | 具名拒绝（`gcconcentric.unsupported: this build has no geometric-constraint model`）。 |
| 382 | `GCEQUAL` | 部分实现 | Resizes selected arcs and circles to the same radius, or selected lines to the same length. | 具名拒绝（`gcequal.unsupported: this build has no geometric-constraint model`）。 |
| 383 | `GCFIX` | 部分实现 | Locks points and curves in position. | 具名拒绝（`gcfix.unsupported: this build has no geometric-constraint model`）。 |
| 384 | `GCHORIZONTAL` | 部分实现 | Causes lines or pairs of points to lie parallel to the X axis of the current coordinate system. | 具名拒绝（`gchorizontal.unsupported: this build has no geometric-constraint model`）。 |
| 385 | `GCPARALLEL` | 部分实现 | Causes selected lines to lie parallel to each other. | 具名拒绝（`gcparallel.unsupported: this build has no geometric-constraint model`）。 |
| 386 | `GCPERPENDICULAR` | 部分实现 | Causes selected lines to lie 90 degrees to one another. | 具名拒绝（`gcperpendicular.unsupported: this build has no geometric-constraint model`）。 |
| 387 | `GCSMOOTH` | 部分实现 | Constrains a spline to be contiguous and maintain G2 continuity with another spline, line, arc, or polyline. | 具名拒绝（`gcsmooth.unsupported: this build has no geometric-constraint model`）。 |
| 388 | `GCSYMMETRIC` | 部分实现 | Causes selected objects to become symmetrically constrained about a selected line. | 具名拒绝（`gcsymmetric.unsupported: this build has no geometric-constraint model`）。 |
| 389 | `GCTANGENT` | 部分实现 | Constrains two curves to maintain a point of tangency to each other or their extensions. | 具名拒绝（`gctangent.unsupported: this build has no geometric-constraint model`）。 |
| 390 | `GCVERTICAL` | 部分实现 | Causes lines or pairs of points to lie parallel to the Y axis of the current coordinate system. | 具名拒绝（`gcvertical.unsupported: this build has no geometric-constraint model`）。 |
| 391 | `GEOGRAPHICLOCATION` | 部分实现 | Assigns geographic location information to a drawing file. | 具名拒绝（`geographiclocation.unsupported: this build has no geographic/GIS or online-map model`）。 |
| 392 | `GEOMAP` | 部分实现 | Displays a map from an online maps service, in the current viewport. | 具名拒绝（`geomap.unsupported: this build has no geographic/GIS or online-map model`）。 |
| 393 | `GEOMAPIMAGE` | 部分实现 | Captures a portion of the online map to an object known as a map image, and embeds it in the drawing area. | 具名拒绝（`geomapimage.unsupported: this build has no geographic/GIS or online-map model`）。 |
| 394 | `GEOMAPIMAGEUPDATE` | 部分实现 | Updates map images from the online maps service and optionally resets their resolution for optimal on-screen viewing. | 具名拒绝（`geomapimageupdate.unsupported: this build has no geographic/GIS or online-map model`）。 |
| 395 | `GEOMARKLATLONG` | 部分实现 | Places a position marker at a location defined by latitude and longitude. | 具名拒绝（`geomarklatlong.unsupported: this build has no geographic/GIS or online-map model`）。 |
| 396 | `GEOMARKME` | 部分实现 | Places a position marker in the drawing area at the coordinates corresponding to your current position. | 具名拒绝（`geomarkme.unsupported: this build has no geographic/GIS or online-map model`）。 |
| 397 | `GEOMARKPOINT` | 部分实现 | Places a position marker at a specified point in model space. | 具名拒绝（`geomarkpoint.unsupported: this build has no geographic/GIS or online-map model`）。 |
| 398 | `GEOMARKPOSITION` | 部分实现 | Places a position marker at a location you specify. | 具名拒绝（`geomarkposition.unsupported: this build has no geographic/GIS or online-map model`）。 |
| 399 | `GEOMCONSTRAINT` | 部分实现 | Applies or persists geometric relationships between objects or points on objects. | 具名拒绝（`geomconstraint.unsupported`）：本构建无几何约束模型。 |
| 400 | `GEOREMOVE` | 部分实现 | Removes all geographic location information from the drawing file. | 具名拒绝（`georemove.unsupported: this build has no geographic/GIS or online-map model`）。 |
| 401 | `GEOREORIENTMARKER` | 部分实现 | Changes the north direction and position of the geographic marker in model space, without changing its latitude and longitude. | 具名拒绝（`georeorientmarker.unsupported: this build has no geographic/GIS or online-map model`）。 |
| 402 | `GETSEL` | 部分实现 | Creates a selection set of objects based on layer and object type filters. | `GETSEL LAYER\|TYPE <name>`；替换选择 |
| 403 | `GOTOSTART` | 部分实现 | Switches from the current drawing to the Start tab. | 具名拒绝（`gotostart.unsupported`）：切换到开始页由宿主负责。 |
| 404 | `GOTOURL` | 部分实现 | Opens the file or web page associated with the hyperlink attached to an object. | 具名拒绝（`gotourl.unsupported: this build has no hyperlink storage model`）。 |
| 405 | `GRADIENT` | 部分实现 | Fills an enclosed area or selected objects with a gradient fill. | 当前选择为闭合边界（整实体 Circle/完整 Ellipse/闭合 Polyline/LINE+ARC 边链），复用 HATCH 创建路径并强制渐变填充（会话 kind/角度/双色 ACI，默认线性 0°、ACI 1→5）；边界必须在 +Z 平面（否则 `gradient.not_plus_z`）；一个 `AcDbHatch`、一条事务/一步撤销；无预览、无 `-GRADIENT` 选项提示、无 .pat/关联性 |
| 406 | `-GRAPHICSCONFIG` | 部分实现 | Sets hardware acceleration on or off and provides access to display performance options on the command line. | 具名拒绝（`-graphicsconfig.unsupported: renderer/graphics configuration is owned by the host`）。 |
| 407 | `GRAPHICSCONFIG` | 部分实现 | Sets hardware acceleration on or off and provides access to display performance options. | 具名拒绝（`graphicsconfig.unsupported: renderer/graphics configuration is owned by the host`）。 |
| 408 | `GRAPHSCR` | 部分实现 | Causes the text window to display behind the application window. | 置会话 `text_screen`=false；无文本屏幕窗口可隐藏。 |
| 409 | `GRID` | 已实现 | Displays a grid pattern in the current viewport. | — |
| 410 | `-GROUP` | 部分实现 | Creates and manages saved sets of objects called groups. | `GROUP` 的别名（同一 `CommandId`/会话组表路径）；无 DWG group 对象/修订/历史 |
| 411 | `GROUP` | 部分实现 | Creates and manages saved sets of objects called groups. | 会话内组表（上限 64、名字大小写不敏感）；裸命令/`LIST` 只读、`CREATE` 从选择集建组、`DELETE` 只移除名字；不写 DWG group 对象/修订/历史/事务 |
| 412 | `GROUPEDIT` | 部分实现 | Adds and removes objects from the selected group, or renames a selected group. | `ADD`/`REMOVE` 从选择集或键入实体 id 增删、`RENAME`（允许仅改大小写）；单次会话变更；不写实体/数据库 |
| 413 | `-HATCH` | 部分实现 | Uses command prompts to fill an enclosed area or selected objects with a hatch pattern, solid fill, or gradient fill. | `HATCH` 的命令行拼写；同一实现与边界。 |
| 414 | `HATCH` | 部分实现 | Fills an enclosed area or selected objects with a hatch pattern, solid fill, or gradient fill. | 整实体闭合边界 + LINE/ARC 边链；**本轮新增 `.pat` 解析器/名称解析**（内置三项同源解析）；自定义图案仍无命令行选择通道（`hatchPattern` 闭集 1..=3）；无关联性/实时预览 |
| 415 | `-HATCHEDIT` | 部分实现 | From the Command prompt, modifies existing hatches or fills. | `HATCHEDIT` 的命令行拼写；同一实现与边界。 |
|  416 | `HATCHEDIT` | 部分实现 | Modifies an existing hatch or fill. | 多目标；`Style`/`PatternType`/**`PATTERN <内置图案名>`**（按名称取现有 `.pat` 库，保留角度/比例/double；任意用户 `.pat` 文件未接入）/`Angle`/`Scale`/`Solid`/`Gradient`/`Color`/`Transparency`/`Layer`/`DrawOrder Behind|Front`/`Recreate Boundary`/`Separate Hatches`；`Add Boundaries`/`Remove Boundaries` 真实（一个 hatch + 其余选择为边界，按 points+bulges 匹配移除），一条事务/一步撤销；Origin 无字段、Associate/Annotative 无模型、Boundary draw order 拒绝 |
| 417 | `HATCHGENERATEBOUNDARY` | 部分实现 | Creates a non-associated polyline around a selected hatch. | 预选择输入（不弹 Select objects 选项）；每个 hatch loop 生成一条同层同空间的非关联闭合多段线（含内岛、保留 bulge）；源 hatch 保留；无关联性 |
| 418 | `HATCHSETBOUNDARY` | 部分实现 | Redefines a selected hatch or fill to conform to a different closed boundary. | 恰好一个 hatch + 一条闭合 Polyline/Circle/完整 Ellipse，共面性校验；用 `HatchGeometry::new` 重建、丢弃旧种子；失败 `hatchsetboundary.rebuild_failed` |
| 419 | `HATCHSETORIGIN` | 部分实现 | Controls the starting location of hatch pattern generation for a selected hatch. | 恒按 `hatchsetorigin.missing_field` 拒绝（`HatchGeometry` 无图案原点字段）；不写入 |
| 420 | `HATCHTOBACK` | 部分实现 | Sets the draw order for all hatches in the drawing to be behind all other objects. | 仅模型空间：全部 hatch 连续放到最低非 hatch draw order 之下并保持相对次序；i64 下溢按名拒绝；图纸空间/块内不重排；一次事务可撤销 |
| 421 | `HELIX` | 部分实现 | Creates a 2D spiral or 3D spring. | 平面螺旋 `Polyline`；`height>0` 按 `helix.3d_not_implemented` 拒绝 |
| 422 | `HELP` | 部分实现 | Displays the online or offline Help system. | 具名拒绝（`help.unsupported`）：帮助系统由宿主负责，未实现。 |
| 423 | `HIDE` | 部分实现 | Displays a 3D model with hidden lines suppressed for the 2D Wireframe visual style. | 具名拒绝（`hide.unsupported: the renderer has no 3D hidden-line removal`）。 |
| 424 | `HIDEOBJECTS` | 部分实现 | Temporarily suppresses the display of selected objects. | 会话隐藏集；经共享可见性谓词影响拾取/渲染（FLAG_HIDDEN）；无数据库写/撤销；隐藏点标记 overlay 仍可能绘制（声明） |
| 425 | `HIDEPALETTES` | 已实现 | Hides all displayed palettes, along with the ribbon and the drawing tabs. | chrome 本地选择子：隐藏功能区+选项板+文件标签（同一 clean-screen 形态状态） |
| 426 | `HIGHLIGHTNEW` | 部分实现 | Controls whether new and changed features in product updates are highlighted in the user interface with an orange dot. | 会话令牌（默认 ON）；不渲染新功能高亮。 |
| 427 | `HLSETTINGS` | 部分实现 | Sets the display of such properties as hidden lines. | 具名拒绝（`hlsettings.unsupported: this build has no hidden-line display settings model`）。 |
| 428 | `-HYPERLINK` | 部分实现 | From the Command prompt, inserts and removes hyperlinks from areas or to selected objects. | 具名拒绝（`-hyperlink.unsupported: this build has no hyperlink storage model`）。 |
| 429 | `HYPERLINK` | 部分实现 | Attaches a hyperlink to an object or modifies an existing hyperlink. | 具名拒绝（`hyperlink.unsupported: this build has no hyperlink storage model`）。 |
| 430 | `HYPERLINKOPTIONS` | 部分实现 | Controls the display of the hyperlink cursor, tooltips, and shortcut menu. | 具名拒绝（`hyperlinkoptions.unsupported`）：无超链接存储模型。 |
| 431 | `ID` | 已实现 | Displays the UCS coordinate values of a specified location. | — |
| 432 | `IGESEXPORT` | 部分实现 | Saves selected objects in the current drawing to a new IGES (*.igs, *.iges) file. | 具名拒绝（`igesexport.unsupported: IGES export is not implemented`）。 |
| 433 | `IGESIMPORT` | 部分实现 | Imports data from an IGES (*.igs or *.iges) file into the current drawing. | 具名拒绝（`igesimport.unsupported: IGES import is not implemented`）。 |
| 434 | `-IMAGE` | 部分实现 | Attaches a raster image to the drawing. | 只读报告所有 IMAGE 实体（id/空间/file key/pixels/visible/clip/origin，`-image.*` 诊断码）；Viewer-legal，不写库。 |
| 435 | `IMAGE` | 部分实现 | Displays the External References palette. | 只读报告所有 IMAGE 实体（id/空间/file key/pixels/visible/clip/origin，`image.*` 诊断码）；Viewer-legal，不写库；无 External References 面板。 |
| 436 | `-IMAGEADJUST` | 部分实现 | Controls the brightness, contrast, and fade values of images. | 真实：选中一个 IMAGE，`IMAGEADJUST <brightness> <contrast> <fade> [MONO|COLOR]` 或 `RESET`（各 0..=100，50/50/0 中性）；一条事务/一步撤销；渲染顺序 mono→contrast→brightness→fade-to-background（声明近似）。
| 437 | `IMAGEADJUST` | 部分实现 | Controls the brightness, contrast, and fade values of images. | 真实：选中一个 IMAGE，`IMAGEADJUST <brightness> <contrast> <fade> [MONO|COLOR]` 或 `RESET`（各 0..=100，50/50/0 中性）；一条事务/一步撤销；渲染顺序 mono→contrast→brightness→fade-to-background（声明近似）。
| 438 | `IMAGEAPP` | 部分实现 | Specifies the image editing program for IMAGEEDIT. | 具名拒绝（`imageapp.unsupported: no external image-editor association model`）。 |
| 439 | `IMAGEASYNCWAIT` | 部分实现 | Ensures that raster images being loaded in the background during file opening are complete before proceeding. | 具名拒绝（`imageasyncwait.unsupported: no asynchronous image loading model`）。 |
| 440 | `IMAGEATTACH` | 部分实现 | Inserts a reference to an image file. | 真实创建 `SemanticGeometry::Image`：`IMAGEATTACH <file> <x> <y> <w_px> <h_px> <units_per_pixel>`；逻辑 key 经 `image_resource_key` 规范化（拒绝绝对路径/`..`/URL），+Z 平面、`u=(upp,0,0)`/`v=(0,upp,0)`，一条事务/一步撤销；宿主在提交后按 key 集变化重载纹理（大小写敏感文件系统用逐段不敏感回退），未解析 key 进入宿主图像报告。 |
| 441 | `IMAGECLIP` | 部分实现 | Crops the display of a selected image to a specified boundary. | `IMAGECLIP <minx> <miny> <maxx> <maxy>` / `IMAGECLIP POLY <x1> <y1> …` / `IMAGECLIP OFF`：图像 clip 已存 `Vec<[f64;2]>`，世界坐标经 u/v 映射；非凸环存储合法但当前渲染 fan-triangulate，表示层报告 partial |
| 442 | `IMAGEEDIT` | 部分实现 | Launches the image editing program specified by IMAGEAPP for the selected image. | 具名拒绝（`imageedit.unsupported: no external image-editor model`）。 |
| 443 | `IMAGEQUALITY` | 部分实现 | Controls the display quality of images. | 具名拒绝（`imagequality.unsupported: no image display-quality model`）。 |
| 444 | `IMPORT` | 部分实现 | Imports files of different formats into the current drawing. | 具名拒绝（`import.unsupported: the generic IMPORT picker is not implemented`）。 |
| 445 | `IMPRINT` | 部分实现 | Imprints 2D geometry on a 3D solid or surface, creating additional edges on planar faces. | 具名拒绝（`imprint.unsupported: no 3D imprint model`）。 |
| 446 | `-INPUTSEARCHOPTIONS` | 部分实现 | From the command line, controls settings for display of the command line suggestion list for commands, system variables, and named objects. | 具名拒绝（`-inputsearchoptions.unsupported`）：同一具名拒绝的命令行拼写。 |
| 447 | `INPUTSEARCHOPTIONS` | 部分实现 | Opens a dialog box that controls settings for display of the command line suggestion list for commands, system variables, and named objects. | 具名拒绝（`inputsearchoptions.unsupported`）：输入搜索设置未建模。 |
| 448 | `-INSERT` | 已实现 | Inserts a block or drawing into the current drawing. | `INSERT` 的命令行拼写；同一实现/写入路径。 |
| 449 | `INSERT` | 已实现 | Displays the Blocks palette, which you can use to insert blocks and drawings into the current drawing. | — |
| 450 | `-INSERTCONTENT` | 部分实现 | Inserts a drawing or block into the current drawing. | 具名拒绝（`-insertcontent.unsupported: this build has no content-palette insertion model`）。 |
| 451 | `INSERTOBJ` | 部分实现 | Inserts a linked or embedded object. | 具名拒绝（`insertobj.unsupported: this build has no OLE object storage model`）。 |
| 452 | `-INTERFERE` | 部分实现 | At the Command prompt, creates a temporary 3D solid from the interferences between two sets of selected 3D solids | 具名拒绝（`-interfere.unsupported: 3D solids are not modeled, so interference has nothing to compute`）。 |
| 453 | `INTERFERE` | 部分实现 | Creates a temporary 3D solid from the interferences between two sets of selected 3D solids. | 具名拒绝（`interfere.unsupported: 3D solids are not modeled, so interference has nothing to compute`）。 |
| 454 | `INTERSECT` | 部分实现 | Creates a 3D solid, surface, or 2D region from overlapping solids, surfaces, or regions. | 具名拒绝（`intersect.unsupported: no 3D boolean model`）。 |
| 455 | `ISODRAFT` | 部分实现 | Turns isometric drafting settings on or off, and specifies the current 2D isometric drafting plane. | 会话令牌 `isodraft`；不切换等轴测平面/光标，无视觉变化。 |
| 456 | `ISOLATEOBJECTS` | 部分实现 | Temporarily suppresses the display of all objects except those you select. | 会话隐藏集：隐藏除当前选择外的模型空间对象；无数据库写/撤销 |
| 457 | `ISOPLANE` | 部分实现 | Specifies the current plane for 2D isometric drawing. | 循环会话令牌 LEFT→TOP→RIGHT；无等轴测光标/捕捉变化。 |
| 458 | `JOIN` | 部分实现 | Joins the endpoints of linear and curved objects to create a single object. | 混合 LINE/ARC/多段线/SPLINE/椭圆弧精确焊接为有理 B 样条；**SPLINE 源对象接受接触的 LINE/ARC/椭圆弧**；`JOIN CLOSE` 的混合 SPLINE 仍拒绝；空隙桥接不臆造 |
| 459 | `JPGOUT` | 部分实现 | Saves selected objects to a file in JPEG file format. | 具名拒绝（`jpgout.unsupported: raster JPEG export is not implemented; use PLOT for vector SVG/PDF`）。 |
| 460 | `JULIAN` | 部分实现 | Contains the DATE tool and several AutoCAD Julian date and calendar date conversion routines. | 只读：当前 UTC 时刻的儒略日（整数日 + 小数）；不解析日期参数。 |
| 461 | `JUSTIFYTEXT` | 部分实现 | Changes the justification point of selected text objects without changing their locations. | `JUSTIFYTEXT <H> <V>`（H∈LEFT/CENTER/RIGHT，V∈BASELINE/BOTTOM/MIDDLE/TOP）；只改 `h_align`/`v_align`，位置不动；非文字目标逐条 `justifytext.not_text` 跳过；空选择拒绝；`TJUST` 为同一实现 |
| 462 | `LAYCUR` | 部分实现 | Changes the layer property of selected objects to the current layer. | 预选择输入（不弹 Select objects）；把选中对象移到当前层；当前层缺失/目标层锁定显式拒绝；实体写入可撤销 |
| 463 | `-LAYDEL` | 部分实现 | At the Command prompt, deletes all objects on a layer and purges the layer. | `LAYDEL` 的命令行拼写；实体与层行同一事务/一步撤销。 |
| 464 | `LAYDEL` | 部分实现 | Deletes all objects on a layer and purges the layer. | 选择式：删除选中对象所在层的模型空间实体 + 层行，同一事务/一步撤销；当前层/0 层/仍有图纸或块内实体的层拒绝（图纸/块内实体仍未纳入） |
| 465 | `-LAYER` | 已实现 | Manages layers and layer properties. | 裸命令/`? [pattern]` 为只读图层表报告（名称/可见/off/frozen/locked/当前层，`layer.entry`；`*`/`?` 通配符过滤，无匹配报 `layer.no_match`）；写形式一行一个选项（大小写不敏感）：`NEW/N`、`MAKE/M`、`SET/S`、`RENAME/R`、`DELETE/D`、`ON`/`OFF`、`FREEZE/F`/`THAW/T`、`LOCK/LO`/`UNLOCK/U`、`COLOR/C`（ACI 数字/七个标准色名/`aci:N`/`#RRGGBB`/`rgb:…`）、`LTYPE/L`、`LWEIGHT/LW`、`PLOT/P Y/N`；名单逗号分隔，含空格的值/名称可用双引号；整行原子、一条事务/一步撤销；`MAKE`/`SET` 与 `OFF`/`FREEZE` 复用当前层可见性规则；写形式经 Work-only `SetLayerProperty` 载体，Viewer 拒绝。逐选项交互重提示属全局范围。 |
| 466 | `LAYER` | 已实现 | Manages layers and layer properties. | chrome 本地选择子：打开图层面板 |
| 467 | `LAYERCLOSE` | 已实现 | Closes the Layer Properties Manager. | chrome 本地选择子：关闭图层面板（面板关闭按钮走同一命令） |
| 468 | `LAYERP` | 部分实现 | Undoes the last change or set of changes made to layer settings. | 会话内有界 16 步；不持久化 |
| 469 | `LAYERPALETTE` | 部分实现 | Opens the modeless Layer Properties Manager. | 具名拒绝（`layerpalette.unsupported`）：无独立图层调色板，请用 `LAYER`。 |
| 470 | `LAYERPMODE` | 部分实现 | Turns on and off the tracking of changes made to layer settings for use by the LAYERP command. | `ON\|OFF`；会话标志，关闭后 `LAYERP` 不记录快照；默认 ON；裸命令报告当前值 |
| 471 | `LAYERSTATE` | 部分实现 | Saves, restores, and manages sets of layer settings that are called layer states. | `LAYERSTATE SAVE\|RESTORE\|LIST\|DELETE <name>`；会话内最多 32 个命名快照（层属性+当前层+覆盖）；RESTORE 的图层属性写入一条事务/一步撤销；无 DWG 持久化 |
| 472 | `LAYERSTATESAVE` | 部分实现 | Displays the New Layer State to Save dialog box, where you can provide a name and a description for a new layer state. | 带 `LayerState` 载荷时转交既有 `LAYERSTATE` 会话存储；无载荷只报告缺少 New Layer State 对话框（`layerstatesave.needs_name`）。 |
| 473 | `LAYFRZ` | 部分实现 | Freezes the layer of selected objects. | 预选择输入（不弹 Select objects）；仅 Entity 选择，无 Settings/Viewports/Block 选项；v1 在全部视口冻结；当前层整命令拒绝；图层表写入一条事务/一步撤销 |
| 474 | `LAYISO` | 已实现 | Hides or locks all layers except those of the selected objects. | — |
| 475 | `LAYLCK` | 部分实现 | Locks the layer of a selected object. | 预选择输入（不弹 Select objects）；锁定选中对象所在的不同图层（含当前层）；图层表写入一条事务/一步撤销 |
| 476 | `-LAYMCH` | 部分实现 | Changes the layer of a selected object to match the layer of a selected object. | `LAYMCH` 的别名，同一实现 |
| 477 | `LAYMCH` | 部分实现 | Changes the layer of a selected object to match the destination layer. | 选择式：第一个选中对象的图层为目标，其余移入（一条事务/一步撤销）；空选择/<2 目标/非模型空间/块实例按名拒绝；无交互 Select objects |
| 478 | `LAYMCUR` | 部分实现 | Sets the current layer to that of a selected object. | 预选择输入（不弹 Select objects）；取第一个选中对象的图层并用既有的锁定/冻结/关闭当前层拒绝；仅会话状态 |
| 479 | `-LAYMRG` | 部分实现 | At the Command prompt, merges selected layers into a target layer. | `LAYMRG` 的命令行拼写；实体与层行同一事务/一步撤销。 |
| 480 | `LAYMRG` | 部分实现 | Merges selected layers into a target layer, removing them from the drawing. | 选择式：选中对象的层合并到当前层（重写实体 + 删除源层），同一事务/一步撤销；当前层/0 层不能作源；无交互 Settings |
| 481 | `LAYOFF` | 已实现 | Turns off the layer of a selected object. | — |
| 482 | `LAYON` | 已实现 | Turns on all layers in the drawing. | — |
| 483 | `LAYOUT` | 部分实现 | Creates and modifies drawing layouts. | `LIST`/`SET` 真实；`NEW/RENAME/DELETE` 真实写布局表（名称校验、重复名拒绝、删除级联 plot settings），一条事务/一步撤销；`COPY`/`TEMPLATE` 未实现 |
| 484 | `LAYOUTMERGE` | 已实现 | Combines specified layouts into a single layout. | 把源布局的图纸空间实体移入目标并删除源布局（plot settings 级联），同一事务/一步撤销；同一布局拒绝 |
| 485 | `LAYOUTWIZARD` | 部分实现 | Creates a new layout tab and specifies page and plot settings. | 真实创建布局并应用 Authored 纸张设置（一条事务/一步撤销）；无视口向导/打印设备 |
| 486 | `LAYTHW` | 部分实现 | Thaws all layers in the drawing. | 无选择输入（载荷即拒绝）；解冻全部冻结图层；不处理视口级冻结（本构建无视口模型）；图层表写入一条事务/一步撤销 |
| 487 | `LAYTRANS` | 部分实现 | Translates the layers in the current drawing to specified layer standards. | 具名拒绝（`laytrans.unsupported`）：图层转换标准未实现。 |
| 488 | `LAYULK` | 部分实现 | Unlocks the layer of a selected object. | 预选择输入（不弹 Select objects）；解锁选中对象所在的不同图层；图层表写入一条事务/一步撤销 |
| 489 | `LAYUNISO` | 已实现 | Restores all layers that were hidden or locked with the LAYISO command. | — |
| 490 | `LAYVPI` | 部分实现 | Freezes selected layers in all layout viewports except the current viewport. | 全视口冻结 + **指定视口索引形式**；AutoCAD 的“除当前视口”需要当前视口标记，本构建不存储 |
| 491 | `LAYWALK` | 部分实现 | Displays objects on selected layers and hides objects on all other layers. | 逗号分隔层名；只显示命名层+当前层（会话覆盖，同 LAYISO 原子路径）；未知/空拒绝 |
| 492 | `LEADER` | 部分实现 | Creates a line that connects annotation to a feature. | 同一共享引线计划（创建单个 MULTILEADER）；无引线样式对话框/块/公差注释 |
| 493 | `LENGTHEN` | 部分实现 | Changes the length of objects and the included angle of arcs. | `DE`/`PER`/`TOT` 从**靠近拾取点**的端点改长度；`DY` 拖一端；`LINE`/`ARC`/开放多段线/椭圆弧/开放样条；闭合对象拒绝；开放样条**延长**未实现（`lengthen.spline_extend_unsupported`），缩短精确 |
| 494 | `LIGHT` | 部分实现 | Creates a light. | 具名拒绝（`light.unsupported: 3D light objects are not modeled`）。 |
| 495 | `LIGHTLIST` | 部分实现 | Displays the Lights in Model palette that lists all lights in the model. | 具名拒绝（`lightlist.unsupported: 3D light objects are not modeled`）。 |
| 496 | `LIGHTLISTCLOSE` | 部分实现 | Closes the Lights in Model palette. | 具名拒绝（`lightlistclose.unsupported: 3D light objects are not modeled`）。 |
| 497 | `LIMITS` | 部分实现 | Sets an invisible rectangular boundary in the drawing area that can limit the grid display and limit clicking or entering point locations. | 具名拒绝（`limits.unsupported`）：无图形界限模型。 |
| 498 | `LINE` | 已实现 | Create a series of contiguous line segments. Each segment is a line object that can be edited separately. | — |
| 499 | `-LINETYPE` | 部分实现 | At the Command prompt, loads, sets, and modifies linetypes. | `LINETYPE` 的别名（同一 `CommandId`/路径）；`LIST` 只读、`SET` 复用 `SETENTITYPROPERTY` |
| 500 | `LINETYPE` | 部分实现 | Loads, sets, and modifies linetypes. | `LIST` 只读（Viewer 合法）；`SET <name>` 复用 `SETENTITYPROPERTY`、重新授权工作模式、一条事务/一步撤销；未知名 `property.unknown_linetype`；无线型表加载/新建/删除/改名 |
| 501 | `LIST` | 已实现 | Displays property data for selected objects. | 读取当前选择；报告模型/图纸/块空间、图层、draw order、颜色/线型/线宽/透明度/线型比例/可见性、文字样式/标注样式/块名与几何摘要；厚度/拉伸方向属未建模的全局范围 |
| 502 | `LIVESECTION` | 部分实现 | Turns on live sectioning for a selected section object. | 具名拒绝（`livesection.unsupported: this build has no section-object model`）。 |
| 503 | `LOAD` | 部分实现 | Makes the symbols in compiled shape (SHX) files available for use by the SHAPE command. | 具名拒绝（`load.unsupported: this build has no SHX shape-file loader`）。 |
| 504 | `LOFT` | 部分实现 | Creates a 3D solid or surface in the space between several cross sections. | 具名拒绝（`loft.unsupported: 3D loft is not implemented`）。 |
| 505 | `LOGFILEOFF` | 部分实现 | Closes the command history log file opened by LOGFILEON. | 会话令牌置关（`logfileoff.toggled`）；无日志文件。 |
| 506 | `LOGFILEON` | 部分实现 | Writes the contents of the command history to a file. | 会话令牌置开（`logfileon.toggled`）；无日志文件。 |
| 507 | `LSP` | 部分实现 | Displays a list of all available AutoLISP commands, functions, and variables. | 具名拒绝（`lsp.unsupported: this build has no AutoLISP runtime or compiler`）。 |
| 508 | `LSPSURF` | 部分实现 | Displays the contents of an AutoLISP file by individual functions. | 具名拒绝（`lspsurf.unsupported: this build has no AutoLISP runtime or compiler`）。 |
| 509 | `LTSCALE` | 部分实现 | Sets the global linetype scale factor. | 一个正有限小数；写内存 `$LTSCALE`（一次 revision，无撤销）；当前导出器不写该变量，SAVE→OPEN 不往返 |
| 510 | `-LWEIGHT` | 部分实现 | Sets the current lineweight, lineweight display options, and lineweight units. | 同一线宽显示报告的命令行拼写。 |
| 511 | `LWEIGHT` | 部分实现 | Sets the current lineweight, lineweight display options, and lineweight units. | 只读报告会话线宽显示令牌（默认 ON，`lweight.report`）；渲染器不消费该标志，LWEIGHT 对话框未实现。 |
| 512 | `MAKELISPAPP` | 部分实现 | Compiles one or more AutoLISP (LSP) source files into an application (VLX) file that can be distributed to users and protect your code. | 具名拒绝（`makelispapp.unsupported: this build has no AutoLISP runtime or compiler`）。 |
| 513 | `MARKUP` | 部分实现 | Opens the Markup Set Manager. | 具名拒绝（`markup.unsupported: this build has no collaboration markup/trace model`）。 |
| 514 | `-MARKUPASSIST` | 部分实现 | At the Command prompt, analyzes an imported markup and can help place text callouts and revision clouds faster and with less manual effort. | 具名拒绝（`-markupassist.unsupported: this build has no collaboration markup/trace model`）。 |
| 515 | `MARKUPASSIST` | 部分实现 | Analyzes an imported markup and can help place text callouts and revision clouds faster and with less manual effort. | 具名拒绝（`markupassist.unsupported: this build has no collaboration markup/trace model`）。 |
| 516 | `MARKUPCLOSE` | 部分实现 | Closes the Markup Set Manager. | 具名拒绝（`markupclose.unsupported: this build has no collaboration markup/trace model`）。 |
| 517 | `-MARKUPIMPORT` | 部分实现 | At the command prompt, imports a marked up drawing (image/pdf) in-place into your DWG as a new trace. | 具名拒绝（`-markupimport.unsupported: this build has no collaboration markup/trace model`）。 |
| 518 | `MARKUPIMPORT` | 部分实现 | Imports a marked up drawing (image/pdf) in-place into your DWG as a new trace. | 具名拒绝（`markupimport.unsupported: this build has no collaboration markup/trace model`）。 |
| 519 | `MASSPROP` | 部分实现 | Calculates the mass properties of selected 2D regions or 3D solids. | 闭合圆/整椭圆/闭合折线 + **闭合 SPLINE（采样）** + **HATCH 环嵌套净面积**；样条/导入填充边界为近似（打印弦差/样本数）；无惯性矩/主轴/体积/密度 |
| 520 | `MATBROWSERCLOSE` | 部分实现 | Closes the Materials Browser. | 具名拒绝（`matbrowserclose.unsupported: this build has no material/texture model`）。 |
| 521 | `MATBROWSEROPEN` | 部分实现 | Opens the Materials Browser. | 具名拒绝（`matbrowseropen.unsupported: this build has no material/texture model`）。 |
| 522 | `MATCHCELL` | 部分实现 | Applies the properties of a selected table cell to other table cells. | **无单元格样式复制面**（`matchcell.unsupported`）。
| 523 | `MATCHPROP` | 已实现 | Applies the properties of a selected object to other objects. | Basic 六项 + Settings 子集；Text 组复制文字样式与字体、Dimension 组复制标注样式、Hatch 组复制填充样式/图案/渐变；**不**复制字高、标注覆盖值与填充原点（与 AutoCAD 一致）；异类目标跳过该组并报告 |
| 524 | `MATEDITORCLOSE` | 部分实现 | Closes the Materials Editor. | 具名拒绝（`mateditorclose.unsupported: this build has no material/texture model`）。 |
| 525 | `MATEDITOROPEN` | 部分实现 | Opens the Materials Editor. | 具名拒绝（`mateditoropen.unsupported: this build has no material/texture model`）。 |
| 526 | `MATERIALASSIGN` | 部分实现 | Assigns the material defined in the CMATERIAL system variable to the objects you select. | 具名拒绝（`materialassign.unsupported: this build has no material/texture model`）。 |
| 527 | `MATERIALATTACH` | 部分实现 | Associates materials with layers. | 具名拒绝（`materialattach.unsupported: this build has no material/texture model`）。 |
| 528 | `MATERIALMAP` | 部分实现 | Adjusts how a texture is mapped to a face or an object. | 具名拒绝（`materialmap.unsupported: this build has no material/texture model`）。 |
| 529 | `MATERIALS` | 部分实现 | Opens the Materials Browser. | 具名拒绝（`materials.unsupported: this build has no material/texture model`）。 |
| 530 | `MATERIALSCLOSE` | 部分实现 | Closes the Materials Browser. | 具名拒绝（`materialsclose.unsupported: this build has no material/texture model`）。 |
| 531 | `MEASURE` | 未实现（同名不同义） | Creates point objects or blocks at measured intervals along the length or perimeter of an object. | 本项目 MEASURE 是测量入口（默认 DIST），不是沿对象放置点/块；**不计入覆盖** |
| 532 | `MEASUREGEOM` | 部分实现 | Measures the distance, radius, angle, area, and volume of selected objects, a sequence of points, or dynamically. | **v1（2026-11-07）选项路由**：`MEASUREGEOM DISTANCE\|ANGLE\|AREA\|POLYLINE` 复用既有 `DIST`/`MEASUREANGLE`/`AREA`/`MEASUREPOLYLINE` 工具（同一交互请求，无第二测量引擎）；`RADIUS` 只读报告所选单个 `ARC`/`CIRCLE` 的圆心/半径/直径/圆周长（圆弧另报弧长）。裸名 `measuregeom.needs_option`，未知选项按名拒绝。**未实现**：`VOLUME`（本构建无 3D 实体/体积模型，显式 `measuregeom.volume_unsupported`，不编造数值）；AutoCAD 的其它选项/动态测量未接线。 |
| 533 | `MENU` | 部分实现 | Obsolete. Loads a customization file. | 具名拒绝（`menu.unsupported`）：本构建无菜单文件模型，界面固定。 |
| 534 | `MESH` | 部分实现 | Creates a 3D mesh primitive object such as a box, cone, cylinder, pyramid, sphere, wedge, or torus. | 具名拒绝（`mesh.unsupported: free-form mesh creation is not implemented`）。 |
| 535 | `MESHCAP` | 部分实现 | Creates a mesh face that connects open edges. | 具名拒绝（`meshcap.unsupported: mesh capping is not implemented`）。 |
| 536 | `MESHCOLLAPSE` | 部分实现 | Merges the vertices of selected mesh faces or edges. | 具名拒绝（`meshcollapse.unsupported: mesh collapse is not implemented`）。 |
| 537 | `MESHCREASE` | 部分实现 | Sharpens the edges of selected mesh subobjects. | 具名拒绝（`meshcrease.unsupported: mesh creasing is not implemented`）。 |
| 538 | `MESHEXTRUDE` | 部分实现 | Extends a mesh face into 3D space. | 具名拒绝（`meshextrude.unsupported: mesh extrusion is not implemented`）。 |
| 539 | `MESHMERGE` | 部分实现 | Merges adjacent faces into a single face. | 具名拒绝（`meshmerge.unsupported: mesh merge is not implemented`）。 |
| 540 | `MESHOPTIONS` | 部分实现 | Displays the Mesh Tessellation Options dialog box, which controls default settings for converting existing objects to mesh objects. | 具名拒绝（`meshoptions.unsupported: no mesh tessellation options model`）。 |
| 541 | `MESHPRIMITIVEOPTIONS` | 部分实现 | Displays the Mesh Primitive Options dialog box, which sets the tessellation defaults for primitive mesh objects. | 具名拒绝（`meshprimitiveoptions.unsupported: no mesh primitive options model`）。 |
| 542 | `MESHREFINE` | 部分实现 | Multiplies the number of faces in selected mesh objects or faces. | 具名拒绝（`meshrefine.unsupported: mesh refinement is not implemented`）。 |
| 543 | `MESHSMOOTH` | 部分实现 | Converts 3D objects such as polygon meshes, surfaces, and solids to mesh objects. | 具名拒绝（`meshsmooth.unsupported: mesh smoothing conversion is not implemented`）。 |
| 544 | `MESHSMOOTHLESS` | 部分实现 | Decreases the level of smoothness for mesh objects by one level. | 具名拒绝（`meshsmoothless.unsupported: no mesh smoothness level model`）。 |
| 545 | `MESHSMOOTHMORE` | 部分实现 | Increases the level of smoothness for mesh objects by one level. | 具名拒绝（`meshsmoothmore.unsupported: no mesh smoothness level model`）。 |
| 546 | `MESHSPIN` | 部分实现 | Spins the adjoining edge of two triangular mesh faces. | 具名拒绝（`meshspin.unsupported: mesh edge spin is not implemented`）。 |
| 547 | `MESHSPLIT` | 部分实现 | Splits a mesh face into two faces. | 具名拒绝（`meshsplit.unsupported: mesh face split is not implemented`）。 |
| 548 | `MESHUNCREASE` | 部分实现 | Removes the crease from selected mesh faces, edges, or vertices. | 具名拒绝（`meshuncrease.unsupported: mesh uncreasing is not implemented`）。 |
| 549 | `MIGRATEMATERIALS` | 部分实现 | Finds any legacy materials in Tool Palettes and converts them to generic type. | 具名拒绝（`migratematerials.unsupported: this build has no material/texture model`）。 |
| 550 | `MINSERT` | 已实现 | Inserts multiple instances of a block in a rectangular array. | `MINSERT <block> <rows> <cols> <row_spacing> <col_spacing> <x> <y>`：矩形 INSERT 阵列，ARRAYRECT 单元规则 + INSERT `T(i)·T(-base)`，一条事务；独立副本（域无 MINSERT 实体，非关联） |
| 551 | `MIRROR` | 部分实现 | Creates a mirrored copy of selected objects. | `MIRRTEXT=0` 可读镜像（位置镜像、文字保持可读）、`MIRROR ERASE YES/NO`、标注反射后重合成；**`MIRRTEXT=1` 字形镜像未实现**（文字实体无镜像标志） |
| 552 | `MIRROR3D` | 部分实现 | Creates a mirrored copy of selected 3D objects across a mirroring plane. | 具名拒绝（`mirror3d.unsupported: no 3D mirror-plane model`）。 |
| 553 | `MKLTYPE` | 部分实现 | Creates a linetype definition based on selected objects and stores them in a specified linetype definition (LIN) file. | 具名拒绝（`mkltype.unsupported: this build has no .lin definition-file writer`）。 |
| 554 | `MKSHAPE` | 部分实现 | Creates a shape definition based on selected objects. | 具名拒绝（`mkshape.unsupported: this build has no SHX shape compiler`）。 |
| 555 | `MLEADER` | 部分实现 | Creates a multileader object. | 引线路径类型直线/样条（样式表决定）、注释文本；2 顶点样条不可弯折（报告）；无 dogleg/attachment/角度约束覆盖 |
| 556 | `MLEADERALIGN` | 部分实现 | Aligns and spaces selected multileader objects. | `LEFT/RIGHT/CENTER [spacing]` 对齐多条 MULTILEADER 的文字 X 或等距纵排；箭头锚定，其余顶点/根落点平移；一条事务；无 distribute/column 选项 |
| 557 | `MLEADERCOLLECT` | 部分实现 | Organizes selected multileaders that contain blocks into rows or columns, and displays the result with a single leader. | 合并 ≥2 MULTILEADER 为一条（根/线拼接、文本以 `\P` 连接、首样式/平面保留、根落点重定向），删除源；一条事务；要求样式/覆盖掩码/平面一致且无保留数据 |
| 558 | `MLEADEREDIT` | 部分实现 | Adds leader lines to, or removes leader lines from, a multileader object. | `ADD <x> <y>` 给根 0 增加一条引线（箭头到既有 dogleg 顶点）；`REMOVE <index>` 删除一条（不可清空根）；一条事务；无逐线覆盖/箭头编辑 |
| 559 | `MLEADERSTYLE` | 已实现 | Creates and modifies multileader styles. | `?`/`LIST`/`SET`/`NEW`/`RENAME`/`DELETE`/`MODIFY`；一条 mleader-style patch/一步撤销；`Standard` 不可删；新 MLEADER 解析当前样式；既有引线保留烘焙快照 |
| 560 | `-MLEDIT` | 部分实现 | Edits multiline intersections, breaks, and vertices. | `MLEDIT` 的命令行拼写，同一实现与边界 |
| 561 | `MLEDIT` | 部分实现 | Edits multiline intersections, breaks, and vertices. | 两独立 LINE 的可表示子集：`MERGE/CLOSED/CROSS/TEE` 清理（把各自最近端点 trim/extend 到交点）、`OPEN` 在交点断开、`BREAK` 移除两条平行线之间的带；一条事务；导入的 `AcDbMline` 复合体按名拒绝；无 join/cap/fill/顶点模型（`mledit.residual`） |
| 562 | `MLINE` | 部分实现 | Creates multiple parallel lines. | 恰好两条平行 `LINE`；无 join/cap/多段基线 |
| 563 | `MLSTYLE` | 部分实现 | Creates, modifies, and manages multiline styles. | 会话内最多 16 个命名样式，喂 MLINE；不持久化 |
| 564 | `MOCORO` | 部分实现 | Moves, copies, rotates, and scales selected objects with a single command. | `MOVE|COPY dx dy`、`ROTATE bx by degrees`、`SCALE bx by factor` 作用于当前选择；委托既有 MOVE/COPY/ROTATE/SCALE 路径；COPY 一条事务/一步撤销，其余原地变换；无交互 gizmo |
| 565 | `MODEL` | 已实现 | Switches from a named layout tab to the Model tab. | payload-free 选择子，复用 SPACE MODEL 的校验 + 自动取景；Model 标签页走同一命令 |
| 566 | `MOVE` | 已实现 | Moves objects a specified distance in a specified direction. | — |
| 567 | `MOVEBAK` | 部分实现 | Changes the destination folder for drawing backup (BAK) files. | 具名拒绝（`movebak.unsupported: the backup-folder location is owned by the host filesystem`）。 |
| 568 | `MPEDIT` | 部分实现 | Edits multiple polylines; also converts multiple line and arc objects to polyline objects. | 对每个选中 LWPOLYLINE 应用 `CLOSE\|OPEN\|REVERSE`；非多段线逐条跳过；一条事务 |
| 569 | `MREDO` | 部分实现 | Reverses the effects of several previous UNDO or U commands. | 重做 N 步（默认 1），每步一条历史移动；空栈拒绝；不足时 `mredo.partial` |
| 570 | `MSLIDE` | 部分实现 | Creates a slide file of the current model viewport or the current layout. | 具名拒绝（`mslide.unsupported: this build has no slide (SLD) writer`）。 |
| 571 | `MSPACE` | 部分实现 | In a layout, switches from paper space to model space in a layout viewport. | 复用 `MODEL`/`SPACE` 会话路径切到模型空间；仅会话状态，已在该空间为 `mspace.noop`，不写库。 |
| 572 | `MSTRETCH` | 部分实现 | Stretches objects with multiple crossing windows and crossing polygons. | 仅 payload 多窗口列表（无命令行语法）；复用 STRETCH planner 的窗口并集；一条事务/一步撤销；无多边形窗口 |
| 573 | `MTEDIT` | 部分实现 | Edits multiline text. | `TEXTEDIT` 的别名（同一 `CommandId`/路径）；编辑当前单选的 Text/MText/MULTILEADER 字符串；无第二编辑路径 |
| 574 | `-MTEXT` | 部分实现 | Creates a multiline text object. | `MTEXT` 创建工具的别名（同一两角 + 终端文本捕获）；无第二实现 |
| 575 | `MTEXT` | 部分实现 | Creates a multiline text object. | 两角宽度模型；无行距/分栏/背景填充 |
| 576 | `MULTIPLE` | 部分实现 | Repeats the next command until canceled. | 设置会话 `MultipleRepeat` 标志；本宿主无重复循环，报告 `multiple.repeat_not_implemented`，不伪造重复 |
| 577 | `MVIEW` | 部分实现 | Creates and controls layout viewports. | 矩形（2 点）+ **多边形（≥3 点）视口创建**；多边形 clip 数据真实但渲染受同一 3/4 点限制（`mview.polygon_not_renderable`）；无视口孔洞 |
| 578 | `MVSETUP` | 部分实现 | Sets up the specifications of a drawing. | `MVSETUP <rows> <cols> <scale>` 在活动图纸布局建等分视口网格并设比例；非 AutoCAD Standard 配置；无 Align/Create/Options/title-block 提示 |
| 579 | `NAVBAR` | 部分实现 | Provides access to viewing tools from a unified interface. | 真实设置写入 `NAVBAR [ON\|OFF]`（持久化导航栏可见性）；无 SteeringWheels/ShowMotion/3Dconnexion。 |
| 580 | `NAVSMOTION` | 部分实现 | Provides an on-screen display for creating and playing back cinematic camera animations for design review, presentation, and bookmark-style navigation. | 具名拒绝（`navsmotion.unsupported: this build has no 3D navigation/ViewCube model`）。 |
| 581 | `NAVSMOTIONCLOSE` | 部分实现 | Closes the ShowMotion interface where you can navigate within a drawing by selecting a named view. | 具名拒绝（`navsmotionclose.unsupported: this build has no 3D navigation/ViewCube model`）。 |
| 582 | `NAVSWHEEL` | 部分实现 | Provides access to enhanced navigation tools that are quickly accessible from the cursor. | 具名拒绝（`navswheel.unsupported: this build has no 3D navigation/ViewCube model`）。 |
| 583 | `NAVVCUBE` | 部分实现 | Indicates the current viewing direction. Dragging or clicking the ViewCube rotates the scene. | 真实设置写入 `NAVVCUBE [ON\|OFF]`（按当前 2D/3D 视图类别切显示位）；`PROPERTIES` 具名拒绝；无罗盘/边角目标/动画。 |
| 584 | `NCOPY` | 部分实现 | Copies objects that are contained in an xref, block, or DGN underlay. | 真实编辑：选中块参照成员按世界变换复制到模型空间；一条事务/一步撤销；无嵌套选择界面。 |
| 585 | `NETLOAD` | 部分实现 | Loads a .NET application. | 具名拒绝（`netload.unsupported: no .NET application loader model`）。 |
| 586 | `NEW` | 已实现 | Starts a new drawing. | — |
| 587 | `NEWSHEETSET` | 部分实现 | Creates a new sheet set data file that manages drawing layouts, file paths, and project data. | 具名拒绝（`newsheetset.unsupported: this build has no sheet-set model`）。 |
| 588 | `NEWSHOT` | 部分实现 | Creates a named view with motion that is played back when viewed with ShowMotion. | 具名拒绝（`newshot.unsupported: this build has no ShowMotion named-view/motion model`）。 |
| 589 | `NEWVIEW` | 部分实现 | Saves a new, named view from the display in the current viewport, or by defining a rectangular window. | 带 `View` 载荷时转交既有命名视图存储；无载荷只报告缺少窗口/名称提示（`newview.needs_name`）。 |
| 590 | `-OBJECTSCALE` | 部分实现 | From the Command prompt, adds or deletes supported scales for annotative objects. | 具名拒绝（`-objectscale.unsupported`）：同一拒绝的命令行拼写。 |
| 591 | `OBJECTSCALE` | 部分实现 | Adds or deletes supported scales for annotative objects. | 具名拒绝（`objectscale.unsupported`）：无注释性对象模型。 |
| 592 | `OFFSET` | 已实现 | Creates concentric circles, parallel lines, and parallel curves. | `Through`/`Erase`/`Layer`/`Multiple`(单步)/固定距离；椭圆与椭圆弧→**样条**；`OFFSETGAPTYPE` Sharp/Fillet/Chamfer（含闭合环全部 wrap 角）；闭合环自交拒绝；开放折线远端自交未检查（AutoCAD 未文档化） |
| 593 | `OFFSETEDGE` | 部分实现 | Creates a closed polyline or spline object that is offset at a specified distance from the edges of a selected planar face on a 3D solid or surface. | 具名拒绝（`offsetedge.unsupported: no 3D face/edge offset model`）。 |
| 594 | `OLECONVERT` | 部分实现 | Specifies a different source application for an embedded OLE object, and controls whether the OLE object is represented by an icon. | 具名拒绝（`oleconvert.unsupported: this build has no OLE object storage model`）。 |
| 595 | `OLELINKS` | 部分实现 | Updates, changes, and cancels a selected linked OLE object. | 具名拒绝（`olelinks.unsupported: this build has no OLE object storage model`）。 |
| 596 | `OLEOPEN` | 部分实现 | Opens the selected OLE object in its source application. | 具名拒绝（`oleopen.unsupported: this build has no OLE object storage model`）。 |
| 597 | `OLERESET` | 部分实现 | Restores the selected OLE object to its original size and shape. | 具名拒绝（`olereset.unsupported: this build has no OLE object storage model`）。 |
| 598 | `OLESCALE` | 部分实现 | Controls the size, scale, and other properties of a selected OLE object. | 具名拒绝（`olescale.unsupported: this build has no OLE object storage model`）。 |
| 599 | `OOPS` | 部分实现 | Restores erased objects. | v1 只恢复最近一次 ERASE（不覆盖 BLOCK/WBLOCK 删除）；恢复实体用新 id；撤销恢复副本但不恢复快照 |
| 600 | `OPEN` | 已实现 | Opens an existing drawing file. | — |
| 601 | `OPENDWFMARKUP` | 部分实现 | Opens a DWF or DWFx file that contains markups. | 具名拒绝（`opendwfmarkup.unsupported: this build has no DWF markup model`）。 |
| 602 | `OPENFROMWEBMOBILE` | 部分实现 | Opens a drawing file from your online Autodesk Account. | 具名拒绝（`openfromwebmobile.unsupported: no Autodesk Account (web/mobile) file-exchange model`）。 |
| 603 | `OPENSHEETSET` | 部分实现 | Opens a selected sheet set. | 具名拒绝（`opensheetset.unsupported: no sheet-set (DST) model`）。 |
| 604 | `OPTIONS` | 已实现 | Customizes the program settings. | chrome 本地选择子：打开选项对话框 |
| 605 | `ORTHO` | 已实现 | Constrains cursor movement to the horizontal or vertical direction. | — |
| 606 | `OSNAP` | 已实现 | Sets running object snap modes. | — |
| 607 | `-OVERKILL` | 部分实现 | Removes duplicate or overlapping lines, arcs, and polylines. Also, combines partially overlapping or contiguous ones. | `OVERKILL` 的别名，同一去重实现 |
| 608 | `OVERKILL` | 已实现 | Removes duplicate or overlapping lines, arcs, and polylines. Also, combines those that are partially overlapping or contiguous. | 容差（`0`=精确）、忽略 Color/Layer/Linetype/LTScale/Lineweight/Transparency、段优化与 `NOBREAK`、共线部分重叠/端到端合并、弧覆盖圆、重复段删除；保留最低 `(draw_order,id)`；一条事务/一步撤销 |
| 609 | `-PAGESETUP` | 部分实现 | Modifies the current page layout settings for the plotting device, paper size, plot scale, and several other settings in the command line. | 裸命令只读报告；typed 形式写入 Authored PLOTSETTINGS（纸张/比例/旋转），一条事务/一步撤销。 |
| 610 | `PAGESETUP` | 部分实现 | Controls the page layout, plotting device, paper size, and other settings for each new layout. | 裸命令只读报告每个布局的入库 `PLOTSETTINGS`；`PAGESETUP <layout> <paper> <width> <height> [<num>:<den>] [0|90|180|270]` 写入 Authored 记录，一条事务/一步撤销；无绘图仪/CTB/STB。 |
| 611 | `-PAN` | 部分实现 | Shifts the view by specifying up to two points to determine a distance and direction. | `PAN` 的别名；`dx dy` 或两点，走既有相机路径；只改视图（不写库/不 bump 修订/不入历史） |
| 612 | `PAN` | 已实现 | Shifts the view without changing the viewing direction or magnification. | — |
| 613 | `-PARAMETERS` | 部分实现 | Opens the Parameters Manager palette that includes all dimensional constraint parameters, reference parameters, and user variables in the current drawing. | 具名拒绝（`-parameters.unsupported: no dynamic-block parameter/lookup model; the Parameters Manager palette is not implemented`）。 |
| 614 | `PARAMETERS` | 部分实现 | Opens the Parameters Manager palette that includes all dimensional constraint parameters, reference parameters, and user variables in the current drawing. | 具名拒绝（`parameters.unsupported: no dynamic-block parameter/lookup model; the Parameters Manager palette is not implemented`）。 |
| 615 | `PARAMETERSCLOSE` | 部分实现 | Closes the Parameters Manager palette. | 具名拒绝（`parametersclose.unsupported: no dynamic-block parameter/lookup model; the Parameters Manager palette is not implemented`）。 |
| 616 | `-PARTIALOAD` | 部分实现 | Loads additional geometry into a partially opened drawing from the command line. | 具名拒绝（`-partiaload.unsupported: no partial layer-index open/load model`）。 |
| 617 | `PARTIALOAD` | 部分实现 | Loads additional geometry into a partially opened drawing. | 具名拒绝（`partiaload.unsupported: no partial layer-index open/load model`）。 |
| 618 | `PARTIALOPEN` | 部分实现 | Loads geometry and named objects from a selected view or layer into a drawing. | 具名拒绝（`partialopen.unsupported: no partial layer-index open/load model`）。 |
| 619 | `PASTEASHYPERLINK` | 部分实现 | Creates a hyperlink to a file, and associates it with a selected object. | 具名拒绝（`pasteashyperlink.unsupported: no hyperlink storage model`）。 |
| 620 | `PASTEBLOCK` | 部分实现 | Pastes objects from the Clipboard into the current drawing as a block. | 应用内剪贴板 → 自动命名块；不是 OS 剪贴板 |
| 621 | `PASTECLIP` | 部分实现 | Pastes objects from the Clipboard into the current drawing. | 应用内剪贴板；不是 OS 剪贴板，不跨应用 |
| 622 | `PASTEORIG` | 部分实现 | Pastes objects from the Clipboard into the current drawing using the original coordinates. | 应用内剪贴板；不是 OS 剪贴板，不跨应用 |
| 623 | `PASTESPEC` | 部分实现 | Pastes objects from the Clipboard into the current drawing and controls the format of the data. | 具名拒绝（`pastespec.unsupported: clipboard special-paste (OLE/format selection) is not modeled; use PASTECLIP or PASTEBLOCK`）。 |
| 624 | `PCEXTRACTCENTERLINE` | 部分实现 | Creates a line through the center axis of a cylindrical segment in a point cloud. | 具名拒绝（`pcextractcenterline.unsupported: no point-cloud extraction model`）。 |
| 625 | `PCEXTRACTCORNER` | 部分实现 | Creates a point object at the intersection of three planar segments in a point cloud. | 具名拒绝（`pcextractcorner.unsupported: no point-cloud extraction model`）。 |
| 626 | `PCEXTRACTEDGE` | 部分实现 | Infers the intersection between two adjacent planar segments, and creates a line along the edge. | 具名拒绝（`pcextractedge.unsupported: no point-cloud extraction model`）。 |
| 627 | `PCEXTRACTSECTION` | 部分实现 | Generates 2D geometry from a section through a point cloud. | 具名拒绝（`pcextractsection.unsupported: no point-cloud extraction model`）。 |
| 628 | `PDFADJUST` | 部分实现 | Adjust the fade, contrast, and monochrome settings of a PDF underlay. | 具名拒绝（`pdfadjust.unsupported: no PDF underlay adjustment model`）。 |
| 629 | `-PDFATTACH` | 部分实现 | Attach a PDF underlay from the command line. | 具名拒绝（`-pdfattach.unsupported: PDF underlays are not implemented`）。 |
| 630 | `PDFATTACH` | 部分实现 | Inserts a PDF file as an underlay into the current drawing. | 具名拒绝（`pdfattach.unsupported: PDF underlays are not implemented`）。 |
| 631 | `PDFCLIP` | 部分实现 | Crops the display of a selected PDF underlay to a specified boundary. | 具名拒绝（`pdfclip.unsupported: no PDF underlay clip model`）。 |
| 632 | `-PDFIMPORT` | 部分实现 | Imports the objects in a PDF file or an attached PDF underlay from the command line. | 具名拒绝（`-pdfimport.unsupported: PDF geometry import is not implemented`）。 |
| 633 | `PDFIMPORT` | 部分实现 | Imports the geometry, fills, raster images, and TrueType text objects from a specified PDF file. | 具名拒绝（`pdfimport.unsupported: PDF geometry import is not implemented`）。 |
| 634 | `PDFLAYERS` | 部分实现 | Controls the display of layers in a PDF underlay. | 具名拒绝（`pdflayers.unsupported: no PDF underlay layer model`）。 |
| 635 | `-PDFSHXTEXT` | 部分实现 | Converts the SHX geometry imported from PDF files into individual multiline text objects from the command line. | 具名拒绝（`-pdfshxtext.unsupported: PDF SHX text conversion is not implemented`）。 |
| 636 | `PDFSHXTEXT` | 部分实现 | Converts the SHX geometry imported from PDF files into individual multiline text objects. | 具名拒绝（`pdfshxtext.unsupported: PDF SHX text conversion is not implemented`）。 |
| 637 | `PEDIT` | 部分实现 | Edits polylines, objects to be joined to polylines, and related objects. | `Close`/`Open`/`Join`/`Fit`/`Spline`/`Decurve`/`Reverse`/`Undo` + LINE/ARC/SPLINE 转换；本轮新增 **Edit vertex 子菜单**（NEXT/PREVIOUS/INSERT/MOVE/STRAIGHTEN；TANGENT/WIDTH 拒绝）；`Width`/`Ltype gen` 未建模 |
| 638 | `PERFANALYZER` | 部分实现 | Opens the Performance Analyzer palette from which you can diagnose operations in AutoCAD that seem slow or unresponsive. | chrome 本地选择子：非模态性能面板，只读显示宿主 `cad_status.performance` 与文档/会话计数；缺失指标显示 N/A（不伪造 0），无命令录制/采样。 |
| 639 | `PERFANALYZERCLOSE` | 已实现 | Closes the Performance Analyzer palette. | chrome 本地选择子：关闭性能分析浮窗（幂等成功）。 |
| 640 | `PFACE` | 部分实现 | Creates a 3D polyface mesh, vertex by vertex. | 具名拒绝（`pface.unsupported: no 3DFACE/mesh entity model`）。 |
| 641 | `PLAN` | 部分实现 | Displays an orthographic view of the XY plane of a specified user coordinate system. | payload-free 选择子：Top/plan 2D 视图；无 UCS/World/Current 选项（本构建无 UCS 模型） |
| 642 | `PLANESURF` | 部分实现 | Creates a planar surface. | 具名拒绝（`planesurf.unsupported: no planar-surface entity model`）。 |
| 643 | `PLINE` | 已实现 | Creates a 2D polyline, a single object that is composed of line and arc segments. | `Geometry(Polyline)` 现在接受 **bulge 弧段**与 **Close**（含隐式闭合段），逐位保留顶点/凸度；`Width`/交互逐顶点 `Undo` 未建模（全局范围） |
| 644 | `-PLOT` | 部分实现 | Plots a drawing to a plotter, printer, or file in the command line. | 具名拒绝（`-plot.unsupported`）：请用 PLOT；本构建只有一条矢量出图路径；命令行拼写。 |
| 645 | `PLOT` | 部分实现 | Plots a drawing to a plotter, printer, or file. | 仅矢量 SVG/PDF；无 CTB/STB、无批量多布局 |
| 646 | `-PLOTSTAMP` | 部分实现 | Places a plot stamp on a specified corner of each drawing and logs it to a file. | 具名拒绝（`-plotstamp.unsupported`）：本构建无打印戳记模型；命令行拼写。 |
| 647 | `PLOTSTAMP` | 部分实现 | Places a plot stamp with information such as date, time, and scale on a specified corner of each drawing and logs it to a file. | 具名拒绝（`plotstamp.unsupported`）：本构建无打印戳记模型。 |
| 648 | `-PLOTSTYLE` | 部分实现 | Lists all available plotstyles in the current drawing and to set a plotstyle current. | 具名拒绝（`-plotstyle.unsupported`）：打印样式列表的命令行拼写。 |
| 649 | `PLOTSTYLE` | 部分实现 | Controls the named plot styles that are attached to the current layout and can be assigned to objects. | 具名拒绝（`plotstyle.unsupported`）：命名打印样式未实现（无 CTB/STB 表）。 |
| 650 | `PLOTTERMANAGER` | 部分实现 | Displays the Plotter Manager, where you can add or edit a plotter configuration. | 具名拒绝（`plottermanager.unsupported`）：本构建无绘图仪配置模型。 |
| 651 | `PLT2DWG` | 部分实现 | Imports legacy HPGL files into the current drawing, retaining all colors. | 具名拒绝（`plt2dwg.unsupported: HPGL (PLT) import is not implemented`）。 |
| 652 | `PMTOGGLE` | 已实现 | Controls whether the Performance Recorder is turned on or off. | chrome 本地选择子：切换与 HIDEPALETTES/SHOWPALETTES 相同的选项板可见性状态（幂等；关闭打开的选项板） |
| 653 | `PNGOUT` | 部分实现 | Saves selected objects to a file in a Portable Network Graphics format. | 具名拒绝（`pngout.unsupported: raster PNG export is not implemented; use PLOT for vector SVG/PDF`）。 |
| 654 | `POINT` | 已实现 | Creates a point object. | 单点 + **payload 多点 `Multiple`**（n 个 `AcDbPoint`，一条事务/一步撤销）；交互重复捕获（Exact(1)）未接线 |
| 655 | `-POINTCLOUDATTACH` | 部分实现 | Inserts a point cloud file into the current drawing from the Command prompt. | 具名拒绝（`-pointcloudattach.unsupported: no point-cloud attachment/crop/style model`）。 |
| 656 | `POINTCLOUDATTACH` | 部分实现 | Inserts a point cloud scan (RCS) or project file (RCP) into the current drawing. | 具名拒绝（`pointcloudattach.unsupported: no point-cloud attachment/crop/style model`）。 |
| 657 | `POINTCLOUDCOLORMAP` | 部分实现 | Displays the Point Cloud Color Map dialog box, used to define settings for Intensity, Elevation, and Classification point cloud stylizations. | 具名拒绝（`pointcloudcolormap.unsupported: no point-cloud attachment/crop/style model`）。 |
| 658 | `POINTCLOUDCROP` | 部分实现 | Crops a selected point cloud to a specified polygonal, rectangular, or circular boundary. | 具名拒绝（`pointcloudcrop.unsupported: no point-cloud attachment/crop/style model`）。 |
| 659 | `POINTCLOUDCROPSTATE` | 部分实现 | Saves, restores, and deletes point cloud crop states. | 具名拒绝（`pointcloudcropstate.unsupported: no point-cloud attachment/crop/style model`）。 |
| 660 | `-POINTCLOUDMANAGER` | 部分实现 | Allows use of automation to control display of regions and scans in point cloud objects. | 具名拒绝（`-pointcloudmanager.unsupported: no point-cloud attachment/crop/style model`）。 |
| 661 | `POINTCLOUDMANAGER` | 部分实现 | Displays the Point Cloud Manager palette, used to control display of point cloud projects, regions, and scans. | 具名拒绝（`pointcloudmanager.unsupported: no point-cloud attachment/crop/style model`）。 |
| 662 | `POINTCLOUDMANAGERCLOSE` | 部分实现 | Closes the Point Cloud Manager. | 具名拒绝（`pointcloudmanagerclose.unsupported: no point-cloud attachment/crop/style model`）。 |
| 663 | `POINTCLOUDSTYLIZE` | 部分实现 | Controls the coloration of point clouds. | 具名拒绝（`pointcloudstylize.unsupported: no point-cloud attachment/crop/style model`）。 |
| 664 | `POINTCLOUDUNCROP` | 部分实现 | Removes all cropped areas from selected point clouds. | 具名拒绝（`pointclouduncrop.unsupported: no point-cloud attachment/crop/style model`）。 |
| 665 | `POINTLIGHT` | 部分实现 | Creates a point light that radiates light in all directions from its location. | 具名拒绝（`pointlight.unsupported: no 3D light model`）。 |
| 666 | `POLYGON` | 已实现 | Creates an equilateral closed polyline. | 默认内接 + 外切多词选择子；`Edge` 法与数字半径无输入通道；边数取会话 `POLYSIDES` |
| 667 | `POLYSOLID` | 部分实现 | Creates 3D solid in the shape of a wall or series of walls. | 具名拒绝（`polysolid.unsupported: 3D solid primitives are not implemented`）。 |
| 668 | `PRESSPULL` | 部分实现 | Dynamically modifies objects by extrusion and offset. | 具名拒绝（`presspull.unsupported: no press/pull model`）。 |
| 669 | `PREVIEW` | 部分实现 | Displays the drawing as it will be plotted. | 具名拒绝（`preview.unsupported`）：本构建无打印预览窗口。 |
| 670 | `PROJECTGEOMETRY` | 部分实现 | Projects points, lines, or curves onto a 3D solid or surface from different directions. | 具名拒绝（`projectgeometry.unsupported: no 3D solid edge/geometry projection model`）。 |
| 671 | `PROPERTIES` | 已实现 | Controls properties of existing objects. | chrome 本地选择子：打开特性面板 |
| 672 | `PROPERTIESCLOSE` | 已实现 | Closes the Properties palette. | chrome 本地选择子：关闭特性面板（面板关闭按钮走同一命令） |
| 673 | `PROPULATE` | 部分实现 | Update, list, or clear Drawing Properties data. | 具名拒绝（`propulate.unsupported`）：本构建无特性传播模型。 |
| 674 | `PSBSCALE` | 部分实现 | Specifies or updates the scale of block objects relative to paper space. | 按活动视口 paper-per-model 因子缩放选定图纸空间块参照（插入点固定）；单方向/单视口是声明规则；无像素证据 |
| 675 | `-PSETUPIN` | 部分实现 | Imports a user-defined page setup into a new drawing layout using the command line. | 同一图纸内页面设置复制的命令行拼写；一条事务/一步撤销。 |
| 676 | `PSETUPIN` | 部分实现 | Imports a user-defined page setup into a new drawing layout. | 从同一图纸的源布局复制已存 `PLOTSETTINGS` 到目标布局，一条事务/一步撤销；跨文件/无源记录拒绝。 |
| 677 | `PSOUT` | 部分实现 | Creates a PostScript file from DWG file. | 具名拒绝（`psout.unsupported: PostScript output is not implemented; use PLOT for SVG/PDF`）。 |
| 678 | `PSPACE` | 部分实现 | In a layout, switches from model space in a layout viewport to paper space. | 复用 `SPACE` 会话路径切到第一个图纸布局；无图纸布局为 `pspace.no_layout`，不写库。 |
| 679 | `PSTSCALE` | 部分实现 | Specifies or updates the scale of text objects relative to paper space. | 按同一因子缩放图纸空间 TEXT/MTEXT 存储字高（及 MTEXT wrap 宽度）；单方向/单视口；不处理 MULTILEADER/TABLE 文本 |
| 680 | `PTYPE` | 已实现 | Specifies the display style and size of point objects. | `PTYPE <code> [size]` 委托既有 `PDMODE`/`PDSIZE` 头部写入（两次 header 写、无 header undo 补丁）；bare 为只读报告 |
| 681 | `-PUBLISH` | 部分实现 | Publishes drawings to DWF, DWFx, and PDF files, or to printers or plotters using the command line. | 具名拒绝（`-publish.unsupported`）：本构建无批量发布模型；命令行拼写。 |
| 682 | `PUBLISH` | 部分实现 | Publishes drawings to DWF, DWFx, and PDF files, or to printers or plotters. | 具名拒绝（`publish.unsupported`）：本构建无批量发布模型。 |
| 683 | `-PURGE` | 部分实现 | Removes unused named objects, such as block definitions and layers, from the current drawing using the command line. | `PURGE` 的别名，同一实现 |
| 684 | `PURGE` | 部分实现 | Removes unused items, such as block definitions and layers, from the drawing. | 只清未使用的**图层**（复用 `DELETELAYER` 的引用拒绝）；BLOCK/STYLE/LINETYPE 无删除 API，按名拒绝；PURGE 的层删除仍**有意不记历史**（其他图层写已可撤销） |
| 685 | `PURGEAECDATA` | 部分实现 | Removes the invisible AEC data (AutoCAD Architecture and AutoCAD Civil 3D custom objects) in the drawing at the Command prompt. | 只读能力报告：本构建不导入/建模 AEC 对象，无 AEC 数据可清理（`purgeaecdata.no_aec_model`）；不写库。 |
| 686 | `PUSHTODOCSCLOSE` | 部分实现 | Closes the Push to Autodesk Docs palette. | 具名拒绝（`pushtodocsclose.unsupported: no Autodesk Docs connector model`）。 |
| 687 | `PUSHTODOCSOPEN` | 部分实现 | Opens the Push to Autodesk Docs palette where you can select AutoCAD layouts to upload as PDFs to Autodesk Docs. | 具名拒绝（`pushtodocsopen.unsupported: no Autodesk Docs connector model`）。 |
| 688 | `PYRAMID` | 部分实现 | Creates a 3D solid pyramid. | 具名拒绝（`pyramid.unsupported: 3D solid primitives are not implemented`）。 |
| 689 | `QCCLOSE` | 已实现 | Closes the QuickCalc calculator. | 关闭 QUICKCALC 浮窗（幂等成功） |
| 690 | `QDIM` | 已实现 | Creates a series of dimensions quickly from selected objects. | `Continuous`/`Staggered`/`Baseline`/`Ordinate`/`Radius`/`Diameter` + `Datum Point`/`Edit`/`Settings`；Staggered 步长 = `DIMDLI`（声明）；Ordinate 基准默认 `(0,0,0)`；不支持项逐条 `qdim.skipped`；一条事务/一步撤销；角度不是 QDIM 模式 |
| 691 | `QLATTACH` | 部分实现 | Attaches a leader line to an mtext, tolerance, or block reference object. | 具名拒绝（`qlattach.unsupported`）：无 QLEADER 注释附着模型。 |
| 692 | `QLATTACHSET` | 部分实现 | Globally attaches leader lines to mtext, tolerance, or block reference objects. | 具名拒绝（`qlattachset.unsupported`）：无 QLEADER 注释附着模型。 |
| 693 | `QLDETACHSET` | 部分实现 | Detaches a leader line from an mtext, tolerance, or block reference object. | 具名拒绝（`qldetchset.unsupported`）：无 QLEADER 注释附着模型。 |
| 694 | `QLEADER` | 部分实现 | Creates a leader and leader annotation. | 与 LEADER/MLEADER 共享引线选项计划：直线/样条引线、注释类型 text、箭头开；箭头关/最大点数按名拒绝；无设置对话框/块/公差注释 |
| 695 | `QNEW` | 部分实现 | Starts a new drawing from a specified drawing template file. | 具名拒绝（`qnew.single_document`）：本构建只有一个文档；请用 NEW。 |
| 696 | `QQUIT` | 部分实现 | Closes all open drawings and exit. | 具名拒绝（`qquit.unsupported`）：应用退出由宿主负责。 |
| 697 | `QSAVE` | 部分实现 | Saves the current drawing using the specified default file format. | SAVE 别名：有损 DXF 另存；源为 .dwg 时可实验性导出 DWG（新文件、默认 <stem>-export.dwg、绝不覆盖源；仅合成 self-roundtrip 证据，vendor/AutoCAD NOT VERIFIED） |
| 698 | `QSELECT` | 部分实现 | Creates a selection set based on filtering criteria. | `Equals`/`Not Equal`/`>`/`<`/`>=`/`<=`/通配符/`Select All`；整图（当前空间）或当前选择；`Include`/`Exclude`/`Append`；嵌套块不参与；只读；**分类/自定义对象特性未建模**，属性集为基础子集 |
| 699 | `QTEXT` | 部分实现 | Controls the display and plotting of text and attribute objects. | 会话令牌 `qtext`；仍按几何绘制文字，不显示占位框。 |
| 700 | `QUICKCALC` | 已实现 | Opens the QuickCalc calculator. | chrome 非模态可拖动浮窗计算器：四则 + 括号/小数/正负号/清除/退格/M+/MR/MC/复制/键盘输入/Esc；科学模式含三角/反三角/对数/开方/幂/平方/倒数/π/e 与 DEG/RAD；无伪工程键，域错误不编造数值 |
| 701 | `QUICKCUI` | 部分实现 | Displays the Customize User Interface Editor in a collapsed state. | 具名拒绝（`quickcui.unsupported: no CUI (menu-customization) model`）。 |
| 702 | `QUICKPROPERTIES` | 部分实现 | Displays quick property data for selected objects. | chrome 本地选择子：打开现有特性面板；紧凑布局像 PROPERTIES 一样拒绝。 |
| 703 | `QUIT` | 部分实现 | Exits the program. | 具名拒绝（`quit.unsupported`）：应用退出由宿主负责。 |
| 704 | `QVDRAWING` | 已实现 | Displays open drawings and layouts in a drawing using preview images. | chrome 面板：Quick View Drawings，列出打开的图形（单文档）与预览卡；点击聚焦，Esc 关闭 |
| 705 | `QVDRAWINGCLOSE` | 已实现 | Closes preview images of open drawings and their layouts. | chrome 本地选择子：关闭 Quick View Drawings（幂等成功） |
| 706 | `QVLAYOUT` | 已实现 | Displays preview images of model space and layouts for the current drawing. | chrome 面板：列出真实布局，点击派发 `SPACE #<id>`/`MODEL` 并关闭 |
| 707 | `QVLAYOUTCLOSE` | 已实现 | Closes preview images of model space and layouts in the current drawing | chrome 本地选择子：关闭 Quick View Layouts（幂等成功） |
| 708 | `RAY` | 部分实现 | Creates a linear object that starts at a point and continues to infinity. | 两点创建，但落库为裁剪到模型包围盒的有限线段 |
| 709 | `RECOVER` | 部分实现 | Repairs and then opens a damaged drawing file. | 只读：对当前文档逐实体跑 `validate_entity`（`recover.summary`/`recover.invalid_entity`）；文件级 DWG 修复/重新打开由宿主负责（`recover.file_level_not_modeled`）；不写库/不升修订/不入历史。 |
| 710 | `RECOVERALL` | 部分实现 | Repairs a damaged drawing file along with all attached xrefs. | 只读：同一完整性扫描 + `recoverall.single_document`（本构建单文档，无多文档/外部参照批处理）；不写库。 |
| 711 | `RECTANG` | 已实现 | Creates a rectangular polyline. | 两角→闭合四顶点 LWPOLYLINE；`payload_length` 守卫；`Fillet`/`Chamfer`/`Rotation`/`Area` 无输入通道（payload 仅两角），矩形可由 `PLINE` 表达 |
| 712 | `REDEFINE` | 已实现 | Restores AutoCAD internal commands overridden by UNDEFINE. | `REDEFINE <name>` 重新启用被 `UNDEFINE` 禁用的内建命令；未知/从未定义按名拒绝；essential 命令双向拒绝；会话状态不写图 |
| 713 | `REDIR` | 部分实现 | Redefines hard-coded paths in xrefs, images, shapes, styles, and rtext. | 具名拒绝（`redir.unsupported: no xref/image path-redirection model`）。 |
| 714 | `REDIRMODE` | 部分实现 | Sets options for the REDIR Express Tool by specifying which object types are included. | 具名拒绝（`redirmode.unsupported: no xref/image path-redirection model`）。 |
| 715 | `REDO` | 已实现 | Reverses the effects of previous UNDO or U command. | — |
| 716 | `REDRAW` | 部分实现 | Refreshes the display in the current viewport. | 当前视口刷新（session generation）；不改相机、不写库、不进 `ZOOM PREVIOUS` |
| 717 | `REDRAWALL` | 部分实现 | Refreshes the display in all viewports. | 所有视口刷新；同 REDRAW 的只读边界 |
|  718 | `REFCLOSE` | 部分实现 | Saves back or discards changes made during in-place editing of a reference, either an xref or a block definition. | 真实：`REFCLOSE SAVE|DISCARD`（裸命令 `refclose.needs_option`）；SAVE 逆变换写回块定义并删除克隆（一条事务/一步撤销），DISCARD 删除克隆且块定义不变。 |
|  719 | `-REFEDIT` | 部分实现 | Edits an xref or a block definition directly within the current drawing. | 真实：`REFEDIT`/`-REFEDIT` 仅接受一个相似变换模型空间 INSERT，把可见块成员按 INSERT 变换克隆到模型空间（一条事务），源 INSERT 走会话隐藏；`REFSET ADD|REMOVE` 只改会话工作集；`REFCLOSE SAVE` 逆变换写回块定义（成员更新/新增/删除与克隆删除，一条事务/一步撤销），`DISCARD` 删除克隆、块定义不变。仅提取可见成员；无参照淡显/锁定（其他宿主对象仍可选，REFSET 是显式选择）；成员图层/绘图次序不回写；QSAVE 应用层拒绝（`refedit.save_blocked`）。 |
|  720 | `REFEDIT` | 部分实现 | Edits an xref or a block definition directly within the current drawing. | 真实：`REFEDIT`/`-REFEDIT` 仅接受一个相似变换模型空间 INSERT，把可见块成员按 INSERT 变换克隆到模型空间（一条事务），源 INSERT 走会话隐藏；`REFSET ADD|REMOVE` 只改会话工作集；`REFCLOSE SAVE` 逆变换写回块定义（成员更新/新增/删除与克隆删除，一条事务/一步撤销），`DISCARD` 删除克隆、块定义不变。仅提取可见成员；无参照淡显/锁定（其他宿主对象仍可选，REFSET 是显式选择）；成员图层/绘图次序不回写；QSAVE 应用层拒绝（`refedit.save_blocked`）。 |
|  721 | `REFSET` | 部分实现 | Adds or removes objects from a working set during in-place editing of a reference, either an xref or a block definition. | 真实：`REFSET ADD|REMOVE`（裸命令=ADD）把当前选择加入/移出 in-place 块编辑工作集；会话状态，不写图；无活动会话 `refedit.no_session`、空选择 `refedit.selection_empty`；自引用实例拒绝。 |
| 722 | `REGEN` | 部分实现 | Regenerates the drawing from within the current viewport. | 推进当前视口 regeneration stamp，重建派生显示缓存；不写库 |
| 723 | `REGEN3` | 部分实现 | Regenerates the views in a drawing to repair anomalies in the display of 3D solids and surfaces. | `REGEN` 的别名（同一 `CommandId`/路径），无第二套实现；不修复 3D 实体显示异常 |
| 724 | `REGENALL` | 部分实现 | Regenerates the entire drawing and refreshes all viewports. | 所有视口 regeneration；不写库 |
| 725 | `REGENAUTO` | 部分实现 | Obsolete. Controls automatic regeneration of a drawing. | 会话令牌 `regenauto`；本构建无自动重生成开关，需显式 REGEN。 |
| 726 | `REGION` | 部分实现 | Converts objects that enclose an area into a 2D region object. | 具名拒绝（`region.unsupported`）：无面域几何模型。 |
| 727 | `REINIT` | 部分实现 | Reinitializes the digitizer, digitizer input/output port, and program parameters file. | 具名拒绝（`reinit.unsupported`）：本构建无设备/数字化仪初始化模型。 |
| 728 | `-RENAME` | 部分实现 | Changes the names assigned to named objects from the Command prompt. | `RENAME` 的命令行拼写；图层重命名一条事务/一步撤销。 |
| 729 | `RENAME` | 部分实现 | Changes the names assigned to items such as layers and dimension styles. | `RENAME <kind> <old> <new>`：`LAYER`/`STYLE`/`DIMSTYLE`/`MLEADERSTYLE`/`BLOCK` 真实写；一条表 patch/一步撤销；`LINETYPE` 仍无 rename API（具名拒绝） |
| 730 | `-RENDER` | 部分实现 | Creates a photorealistic or realistically shaded image of a 3D solid or surface model from the Command prompt. | 具名拒绝（`-render.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| 731 | `RENDER` | 部分实现 | Creates a photorealistic or realistically shaded image of a 3D solid or surface model. | 具名拒绝（`render.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| 732 | `RENDERCROP` | 部分实现 | Renders a specified rectangular area, called a crop window, within a viewport. | 具名拒绝（`rendercrop.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| 733 | `RENDERENVIRONMENT` | 部分实现 | Controls the settings related to the rendering environment. | 具名拒绝（`renderenvironment.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| 734 | `RENDERENVIRONMENTCLOSE` | 部分实现 | Closes the Render Environment & Exposure palette. | 具名拒绝（`renderenvironmentclose.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| 735 | `RENDEREXPOSURE` | 部分实现 | Controls the settings related to the rendering environment. | 具名拒绝（`renderexposure.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| 736 | `RENDEREXPOSURECLOSE` | 部分实现 | Closes the Render Environment & Exposure palette. | 具名拒绝（`renderexposureclose.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| 737 | `RENDERONLINE` | 部分实现 | Uses the Autodesk online resources to create an image of a 3D solid or surface model. | 具名拒绝（`renderonline.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| 738 | `RENDERPRESETS` | 部分实现 | Specifies render presets, reusable rendering parameters, for rendering an image. | 具名拒绝（`renderpresets.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| 739 | `RENDERPRESETSCLOSE` | 部分实现 | Closes the Render Presets Manager palette. | 具名拒绝（`renderpresetsclose.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| 740 | `RENDERWIN` | 部分实现 | Obsolete. Displays the Render window without starting a rendering operation. | 具名拒绝（`renderwin.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| 741 | `RENDERWINDOW` | 部分实现 | Displays the Render window without starting a rendering operation. | 具名拒绝（`renderwindow.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| 742 | `RENDERWINDOWCLOSE` | 部分实现 | Closes the Render window. | 具名拒绝（`renderwindowclose.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| 743 | `REPURLS` | 部分实现 | Finds and replaces a specified text string in a URL used in hyperlinks that are attached to all selected objects. | 具名拒绝（`repurls.unsupported: no hyperlink storage model`）。 |
| 744 | `RESETBLOCK` | 部分实现 | Resets one or more dynamic block references to the default values of the block definition. | 具名拒绝（`resetblock.unsupported`）：本构建无动态块参数重置 API。 |
| 745 | `RESUME` | 部分实现 | Continues an interrupted script. | 具名拒绝（`resume.unsupported: no script-file engine`）。 |
| 746 | `REVCLOUD` | 部分实现 | Creates or modifies a revision cloud. | `Object`/`Rectangular`/`Polygonal`/`Freehand`/`Arc Length`/`Style`/`Modify`；弧长按 payload → 会话 `REVCLOUDPROPERTIES` token → 5% 替代顺序；`Calligraphy` 未建模 |
| 747 | `REVCLOUDPROPERTIES` | 已实现 | Controls the approximate chord length for the arcs in a selected revision cloud. | `REVCLOUDPROPERTIES <arc-length>`（有限 > 0）写会话 token，`REVCLOUD` 真实消费并改变弧长；bare 为只读报告 |
| 748 | `REVERSE` | 部分实现 | Reverses the vertices of selected lines, polylines, splines, and helixes, which is useful for linetypes with included text, or wide polylines with differing beginning and ending widths. | 仅 LINE/LWPOLYLINE/SPLINE，当前选择为全部输入；无交互拾取提示，无螺旋/3D 变体 |
| 749 | `REVERT` | 部分实现 | Closes and then reopens the current drawing. | 具名拒绝（`revert.unsupported`）：本构建无重新打开源文件的路径。 |
| 750 | `REVOLVE` | 部分实现 | Creates a 3D solid or surface by sweeping an object around an axis. | 具名拒绝（`revolve.unsupported: 3D revolution is not implemented`）。 |
| 751 | `REVSURF` | 部分实现 | Creates a mesh by revolving a profile about an axis. | 具名拒绝（`revsurf.unsupported: no 3D surface (NURBS) model`）。 |
| 752 | `RIBBON` | 已实现 | Displays the ribbon. | chrome 本地选择子：显示功能区（set，不 toggle）；不进入 `cad_command` |
| 753 | `RIBBONCLOSE` | 已实现 | Hides the ribbon. | chrome 本地选择子：隐藏功能区 |
| 754 | `ROTATE` | 已实现 | Rotates objects around a base point. | — |
| 755 | `ROTATE3D` | 部分实现 | Moves objects about a 3D axis. | 具名拒绝（`rotate3d.unsupported: no 3D rotate-axis model`）。 |
| 756 | `RPREF` | 部分实现 | Displays the Render Presets Manager palette used to configure rendering settings. | 具名拒绝（`rpref.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| 757 | `RPREFCLOSE` | 部分实现 | Closes the Render Settings Manager palette. | 具名拒绝（`rprefclose.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| 758 | `RSCRIPT` | 部分实现 | Repeats a script file. | 具名拒绝（`rscript.unsupported: no script-file engine`）。 |
| 759 | `RTEDIT` | 部分实现 | Edits existing remote text (rtext) objects. | 具名拒绝（`rtedit.unsupported`）：远程文字未实现（无 RTEXT 模型）。 |
| 760 | `RTEXT` | 部分实现 | Creates remote text (rtext) objects. | 具名拒绝（`rtext.unsupported`）：远程文字未实现（无 RTEXT 模型）。 |
| 761 | `RTUCS` | 部分实现 | Rotates the UCS dynamically with your pointing device. | 具名拒绝（`rtucs.unsupported: no dynamic UCS model`）。 |
| 762 | `RULESURF` | 部分实现 | Creates a mesh that represents the surface between two lines or curves. | 具名拒绝（`rulesurf.unsupported: no 3D surface (NURBS) model`）。 |
| 763 | `SAVE` | 部分实现 | Saves the current drawing under a different file name or location without changing which drawing file is the current one. | 有损 DXF 另存；无 DWG 回写 |
| 764 | `SAVEALL` | 部分实现 | Save all open drawings. | 具名拒绝（`saveall.single_document`）：本构建只有一个文档；请用 SAVE。 |
| 765 | `SAVEAS` | 部分实现 | Saves a copy of the current drawing under a new file name or location. | SAVE 别名：有损 DXF 另存；源为 .dwg 时可实验性导出 DWG（新文件、默认 <stem>-export.dwg、绝不覆盖源；仅合成 self-roundtrip 证据，vendor/AutoCAD NOT VERIFIED） |
| 766 | `SAVEIMG` | 部分实现 | Saves a rendered image to a file. | 具名拒绝（`saveimg.unsupported: raster image export is host-owned; this command layer has no file handle (use PLOT PNG)`）。 |
| 767 | `SAVETOWEBMOBILE` | 部分实现 | Saves a copy of the current drawing to your Autodesk Account. | 具名拒绝（`savetowebmobile.unsupported: no Autodesk Account (web/mobile) file-exchange model`）。 |
| 768 | `SCALE` | 已实现 | Enlarges or reduces selected objects, keeping the proportions of the object the same after scaling. | — |
| 769 | `-SCALELISTEDIT` | 部分实现 | Controls the list of scales available for layout viewports, page layouts, and plotting. | 只读报告真实 `ACAD_SCALELIST`（`scalelistedit.entry`）；创建/编辑/删除/排序比例未实现（`scalelistedit.write_unsupported`）。 |
| 770 | `SCALELISTEDIT` | 部分实现 | Controls the list of scales available for layout viewports, page layouts, and plotting. | 只读：第一个图纸布局的页面 + `scalelistedit.write_unsupported`；无比例列表存储。 |
| 771 | `SCALETEXT` | 已实现 | Enlarges or reduces selected text objects without changing their locations. | — |
| 772 | `SCRIPT` | 部分实现 | Executes a sequence of commands from a script file. | 具名拒绝（`script.unsupported: no script-file runner`）。 |
| 773 | `SCRIPTCALL` | 部分实现 | Executes a sequence of commands and nested scripts from a script file. | 具名拒绝（`scriptcall.unsupported: no script-file engine`）。 |
| 774 | `SECTION` | 部分实现 | Uses the intersection of a plane with 3D solids, surfaces, or meshes to create a 2D region object. | 具名拒绝（`section.unsupported: no 3D section-plane model`）。 |
| 775 | `SECTIONPLANE` | 部分实现 | Creates a section object that acts as a cutting plane through 3D objects, and point clouds. | 具名拒绝（`sectionplane.unsupported: no 3D section-plane model`）。 |
| 776 | `SECTIONPLANEJOG` | 部分实现 | Adds a jogged segment to a section object. | 具名拒绝（`sectionplanejog.unsupported: no 3D section-plane model`）。 |
| 777 | `SECTIONPLANESETTINGS` | 部分实现 | Sets display options for a selected section plane. | 具名拒绝（`sectionplanesettings.unsupported: no 3D section-plane model`）。 |
| 778 | `SECTIONPLANETOBLOCK` | 部分实现 | Saves selected section planes as 2D or 3D blocks. | 具名拒绝（`sectionplanetoblock.unsupported: no 3D section-plane model`）。 |
| 779 | `SECTIONSPINNERS` | 部分实现 | Displays the dialog box to set the increment value for the Section Object Offset and Slice Thickness controls in the Section Plane ribbon contextual tab. | 具名拒绝（`sectionspinners.unsupported: no 3D section-plane model`）。 |
| 780 | `SECURITYOPTIONS` | 部分实现 | Controls the security restrictions for running executable files in the product. | 具名拒绝（`securityoptions.unsupported: no security/executable-trust settings model`）。 |
| 781 | `SELECT` | 部分实现 | Places selected objects in the Previous selection set. | 以选择载荷设置会话选择；不是 AutoCAD 的交互式 SELECT |
| 782 | `SELECTCOUNT` | 部分实现 | Finds all objects within the current count that match the properties of the selected objects, and then adds them to the selection set. | 只读统计与第一个选中对象五特性相同的模型空间对象数；不改选择/无事务 |
| 783 | `SELECTSIMILAR` | 已实现 | Finds all objects within the current drawing that match the properties of selected objects, and then adds them to the selection set. | 对象类型强制；默认**图层+类型**；Color/Linetype/Lineweight/Name/Object style 独立开关；透明**不**参与；`Name` 按**根块参照名**匹配展开成员；模型/图纸空间各自匹配；只读 |
| 784 | `SETBYLAYER` | 已实现 | Changes the property overrides of selected objects to ByLayer. | 颜色/线型/线宽/透明度复位随层；Include blocks 递归块成员；锁定图层拒绝；材质/命名打印样式属未建模的全局范围（AutoCAD 的 SETBYLAYER 本就不含文字/标注/填充样式） |
| 785 | `SETVAR` | 部分实现 | Lists or changes the values of system variables. | 读/写闭集 `PDMODE/PDSIZE/SNAPUNIT/LTSCALE/GRID/SNAP/ORTHO/TEXTSIZE`；写复用既有命令路径；另可写 `NAVVCUBEDISPLAY/DISPLAYVIEWCUBEIN2D/DISPLAYVIEWCUBEIN3D`（主机外观设置，读形式为 `setvar.write_only_variable`）；未知/越界按名拒绝 |
| 786 | `-SHADEMODE` | 部分实现 | Provides simple shading for 3D solids and surfaces. | 具名拒绝（`-shademode.unsupported: no 3D shaded display mode; VSCURRENT owns the 2D display token`）。 |
| 787 | `SHADEMODE` | 部分实现 | Controls the display of 3D objects. | 具名拒绝（`shademode.unsupported: no 3D shaded display mode; VSCURRENT owns the 2D display token`）。 |
| 788 | `SHAPE` | 已实现 | Inserts a shape from a shape file (SHX file) that has been loaded using LOAD. | 真实 `SemanticGeometry::Shape` 字形：`SHAPE <name-or-code> <x> <y> [size] [rotation_deg]`，字体取显式 key 或 `Standard` 样式，缺省 size/rotation 取会话值；一条事务/一步撤销；按 name 建行但本构建 SHX 仅保留编号，表示层保持既有 `Partial` |
| 789 | `SHARE` | 部分实现 | Share a link to a copy of the current drawing to view or edit in AutoCAD on the web (web.autocad.com) or AutoCAD on mobile. The drawing copy includes all external references and images. | 具名拒绝（`share.unsupported: no Autodesk cloud sharing/view model`）。 |
| 790 | `SHAREDVIEWS` | 部分实现 | Opens the Shared Views palette. | 具名拒绝（`sharedviews.unsupported: no Autodesk cloud sharing/view model`）。 |
| 791 | `SHAREDVIEWSCLOSE` | 部分实现 | Closes the Shared Views palette. | 具名拒绝（`sharedviewsclose.unsupported: no Autodesk cloud sharing/view model`）。 |
| 792 | `-SHAREVIEW` | 部分实现 | Publishes a representation of the current space or the entire drawing for online viewing and sharing. | 具名拒绝（`-shareview.unsupported: no Autodesk cloud sharing/view model`）。 |
| 793 | `SHAREVIEW` | 部分实现 | Publishes a representation of the current space or the entire drawing for online viewing and sharing. | 具名拒绝（`shareview.unsupported: no Autodesk cloud sharing/view model`）。 |
| 794 | `SHEETSET` | 部分实现 | Opens the Sheet Set Manager. | 具名拒绝（`sheetset.unsupported: no sheet-set (DST) model`）。 |
| 795 | `SHEETSETHIDE` | 部分实现 | Closes the Sheet Set Manager. | 具名拒绝（`sheetsethide.unsupported: no sheet-set (DST) model`）。 |
| 796 | `SHELL` | 部分实现 | Accesses operating system commands. | 具名拒绝（`shell.unsupported: the host owns process spawning; this command layer has no OS shell`）。 |
| 797 | `SHOWPALETTES` | 已实现 | Restores the display of hidden palettes, the ribbon, and the drawing tabs. | chrome 本地选择子：恢复功能区+选项板+文件标签 |
| 798 | `SHOWRENDERGALLERY` | 部分实现 | Displays the images that were rendered and stored in your Autodesk account. | 具名拒绝（`showrendergallery.unsupported: no render engine; only the 2D viewport renderer exists`）。 |
| 799 | `SHOWURLS` | 部分实现 | Displays all attached URLs contained in a drawing, and allows them to edited. | 只读：本数据库不保留超链接/URL 数据（`showurls.not_retained`），不虚构 URL 行、不写库。 |
| 800 | `SHP2BLK` | 已实现 | Converts all instances of a selected shape object with an equivalent block. | 选中 SHAPE → 按 `(font, code)` 建块（字形轮廓多段线）+ 每个源位置一个 INSERT（`T·R·S`），删除源，一条事务；需要宿主安装字体引擎，否则 `shp2blk.font_unavailable` |
| 801 | `SIGVALIDATE` | 部分实现 | Displays information about the digital signature attached to a drawing file. | 具名拒绝（`sigvalidate.unsupported`）：无数字签名模型。 |
| 802 | `SKETCH` | 部分实现 | Creates a series of freehand line segments. | 仅 `CommandPayload::Points`：一条活动层开放 LWPOLYLINE；两点以上且有限；一条事务/一步撤销；交互式定时采样 NOT IMPLEMENTED |
| 803 | `SLICE` | 部分实现 | Creates new 3D solids and surfaces by slicing, or dividing, existing objects. | 具名拒绝（`slice.unsupported: no 3D slicing model`）。 |
| 804 | `SNAP` | 已实现 | Restricts cursor movement to specified intervals. | — |
| 805 | `SOLDRAW` | 部分实现 | Generates profiles and sections in layout viewports created with SOLVIEW. | 具名拒绝（`soldraw.unsupported: no 3D solid-to-2D drawing view model`）。 |
| 806 | `SOLID` | 部分实现 | Creates solid-filled triangles and quadrilaterals. | 3-4 个 `x,y` 点 → 两三角 `Mesh`（`AcDbSolid`）；DXF 导出写 polyface mesh，不是 SOLID 记录 |
| 807 | `SOLIDEDIT` | 部分实现 | Edits faces and edges of 3D solid objects. | 具名拒绝（`solidedit.unsupported: no 3D solid-history/brep model`）。 |
| 808 | `SOLPROF` | 部分实现 | Creates 2D profile images of 3D solids for display in a layout viewport. | 具名拒绝（`solprof.unsupported: no 3D solid-to-2D drawing view model`）。 |
| 809 | `SOLVIEW` | 部分实现 | Creates orthographic views, layers, and layout viewports automatically for 3D solids. | 具名拒绝（`solview.unsupported: no 3D solid-to-2D drawing view model`）。 |
| 810 | `SPACETRANS` | 已实现 | Calculates equivalent model space and paper space distances in a layout. | `SPACETRANS <distance>` 用活动视口 `model_to_paper` 因子做模型↔图纸换算并报告方向/因子；模型空间/无视口按名拒绝 |
| 811 | `SPELL` | 部分实现 | Checks spelling in a drawing. | 具名拒绝（`spell.unsupported`）：无拼写检查词典/模型。 |
| 812 | `SPHERE` | 部分实现 | Creates a 3D solid sphere. | 具名拒绝（`sphere.unsupported: 3D solid primitives are not implemented`）。 |
| 813 | `SPLINE` | 已实现 | Creates a smooth curve that passes through or near a set of fit points, or that is defined by the vertices in a control frame. | fit 点法 + **CV 法**（degree 2/3，显式或简写 knots/weights）；`Close`/`Tangency`/`Knots` 参数化/`Object` 未建模 |
| 814 | `SPLINEDIT` | 部分实现 | Modifies the parameters of a spline or converts a spline-fit polyline to a spline. | 具名拒绝（`splinedit.unsupported`）：无 NURBS 编辑面。 |
| 815 | `SPOTLIGHT` | 部分实现 | Creates a spotlight that emits a directional cone of light. | 具名拒绝（`spotlight.unsupported: no 3D light model`）。 |
| 816 | `SSX` | 部分实现 | Creates a selection set based on a selected object. | Express **筛选表构建器**（不是 SELECTSIMILAR）：`BLOCK`/`COLOR`/`ENTITY`/`LAYER`/`LTYPE`/`STYLE`/`PICK`/`CLEAR`，单值覆盖/移除；模板来自当前选择；仅当前空间；替换选择；只读；**`FLAG`/`THICKNESS`/`VECTOR` 未建模**（实体标志位/厚度/挤出）⇒ 具名拒绝 |
| 817 | `STANDARDS` | 部分实现 | Manages the association of standards files with drawings. | 具名拒绝（`standards.unsupported: no CAD standards (DWS) model`）。 |
| 818 | `STATUS` | 部分实现 | Displays drawing statistics, modes, and extents. | 图形/数据库/修订/实体/图层/块/样式/布局/范围/模式；本轮新增 **current_space/space_usage/layer_usage/modes/selection**；创建/编辑时间、内存、limits/base 明确 `status.not_tracked` |
| 819 | `STLOUT` | 部分实现 | Stores 3D solids and watertight meshes in a format that can be used with stereolithography apparatus. | 具名拒绝（`stlout.unsupported: STL export is not implemented`）。 |
| 820 | `STRETCH` | 部分实现 | Stretches objects crossed by a selection window or polygon. | 交叉矩形与多边形/lasso payload；**IMAGE/MASK/TABLE 新增移动型分类**；圆按圆心、块/文字按插入点；`MULTILEADER` 整命令拒绝；多边形/lasso 交互捕获未接线 |
| 821 | `-STYLE` | 已实现 | Creates, modifies, or specifies text styles. | `STYLE` 的命令行拼写，同一 grammar/写入路径与边界 |
| 822 | `STYLE` | 已实现 | Creates, modifies, or specifies text styles. | `NEW <name> [<base>]`/`RENAME <old> <new>`/`DELETE <name>`/`SET <name> <FONT/KEYS/ADDKEY/REMOVEKEY> <value>`/`?`；一条 style-table patch/一步撤销；域 `Style` 只有 name/resource_keys，故只可写这两类字段；无“当前文字样式”会话变量 |
| 823 | `STYLESMANAGER` | 部分实现 | Displays the Plot Style Manager, where you can revise plot style tables. | 具名拒绝（`stylesmanager.unsupported`）：本构建无 CTB/STB 打印样式模型。 |
| 824 | `SUBTRACT` | 部分实现 | Creates as a new object by subtracting one overlapping region or 3D solid from another. | 具名拒绝（`subtract.unsupported: no 3D boolean model`）。 |
| 825 | `SUNPROPERTIES` | 部分实现 | Displays the Sun Properties palette. | 具名拒绝（`sunproperties.unsupported: no sun/sky lighting model`）。 |
| 826 | `SUNPROPERTIESCLOSE` | 部分实现 | Closes the Sun Properties palette | 具名拒绝（`sunpropertiesclose.unsupported: no sun/sky lighting model`）。 |
| 827 | `SUPERHATCH` | 部分实现 | Hatches an area using a selected image, block, xref, or wipeout object. | 具名拒绝（`superhatch.unsupported`）：基于图像/块的超级填充未实现。 |
| 828 | `SURFBLEND` | 部分实现 | Creates a continuous blend surface between two existing surfaces. | 具名拒绝（`surfblend.unsupported: surface blending is not implemented`）。 |
| 829 | `SURFEXTEND` | 部分实现 | Lengthens a surface by a specified distance. | 具名拒绝（`surfextend.unsupported: surface extension is not implemented`）。 |
| 830 | `SURFEXTRACTCURVE` | 部分实现 | Creates curves on surfaces and 3D solids. | 具名拒绝（`surfextractcurve.unsupported: surface curve extraction is not implemented`）。 |
| 831 | `SURFFILLET` | 部分实现 | Creates a filleted surface between two other surfaces. | 具名拒绝（`surffillet.unsupported: surface filleting is not implemented`）。 |
| 832 | `SURFNETWORK` | 部分实现 | Creates a surface in the space between several curves in the U and V directions, including surface and solid edge subobjects. | 具名拒绝（`surfnetwork.unsupported: surface network creation is not implemented`）。 |
| 833 | `SURFOFFSET` | 部分实现 | Creates a parallel surface a specified distance from the original surface. | 具名拒绝（`surfoffset.unsupported: surface offset is not implemented`）。 |
| 834 | `SURFPATCH` | 部分实现 | Creates a new surface by fitting a cap over a surface edge that forms a closed loop. | 具名拒绝（`surfpatch.unsupported: surface patching is not implemented`）。 |
| 835 | `SURFSCULPT` | 部分实现 | Trims and combines a set of surfaces or meshes that completely enclose a volume to create a 3D solid. | 具名拒绝（`surfsculpt.unsupported: surface sculpting is not implemented`）。 |
| 836 | `SURFTRIM` | 部分实现 | Trims portions of a surface where it meets another surface or type of geometry. | 具名拒绝（`surftrim.unsupported: surface trimming is not implemented`）。 |
| 837 | `SURFUNTRIM` | 部分实现 | Replaces surface areas removed by the SURFTRIM command. | 具名拒绝（`surfuntrim.unsupported: surface untrimming is not implemented`）。 |
| 838 | `SWEEP` | 部分实现 | Creates a 3D solid or 3D surface by sweeping a 2D object or subobject along an open or closed path. | 具名拒绝（`sweep.unsupported: 3D sweep is not implemented`）。 |
| 839 | `SYSVARMONITOR` | 部分实现 | Monitors a list of system variables and sends notifications when there are changes to any one in the list. | `ADD/REMOVE <name>`、`LIST [<name>]`、bare=LIST；会话监视集 + 查询时快照变化检测；无后台通知、不持久化；仅 SETVAR 闭集变量 |
| 840 | `SYSVDLG` | 部分实现 | Allows you to view, edit, save, and restore system variable settings. | 只读报告所有已暴露的会话变量当前值 + `sysvdlg.not_tracked`；无编辑/保存/恢复对话框 |
| 841 | `SYSWINDOWS` | 部分实现 | Arranges windows and icons when the application window is shared with external applications. | 具名拒绝（`syswindows.unsupported`）：应用窗口布局由宿主负责。 |
| 842 | `-TABLE` | 部分实现 | Creates an empty table object. | `TABLE` 的命令行拼写；同一真实创建实现。
| 843 | `TABLE` | 部分实现 | Creates an empty table object. | **导入模型 + 真实创建**：`TABLE <rows> <cols> <col_width> <row_height> <x> <y>` 创建空表格（Standard、+X/+Z、一条事务/一步撤销）；导入模型可 MOVE/ROTATE/SCALE、拾取/显示、FIND 逐单元格、真实 DXF 导出；无对话框/创建时合并/单元格样式。
| 844 | `TABLEDIT` | 部分实现 | Edits text in a table cell. | 真实单元格文本编辑 `TABLEDIT <row> <col> [text]`（1-based、选中一个表格、剩余整行文本、空文本清除）；一条事务/一步撤销；无样式/合并/块内容编辑。
| 845 | `TABLEEXPORT` | 部分实现 | Exports data from a table object in CSV file format. | **无 CSV/文件写出**（`tableexport.unsupported`）；表格单元格数据已在模型里。
| 846 | `TABLESTYLE` | 部分实现 | Creates, modifies, or specifies table styles. | **无表格样式表**（`tablestyle.unsupported`）；导入时只按样式解析边框可见性/双线。
| 847 | `TABLET` | 部分实现 | Calibrates, configures, and turns on and off an attached digitizing tablet. | 具名拒绝（`tablet.unsupported`）：本构建无数字化仪模型。 |
| 848 | `TABSURF` | 部分实现 | Creates a mesh from a line or curve that is swept along a straight path. | 具名拒绝（`tabsurf.unsupported: no 3D surface (NURBS) model`）。 |
| 849 | `TARGETPOINT` | 部分实现 | Creates a target point light. | 具名拒绝（`targetpoint.unsupported: no 3D light model`）。 |
| 850 | `TASKBAR` | 部分实现 | Controls the Peek/Preview behavior of the application's button in the Windows taskbar when multiple drawings are open. | 具名拒绝（`taskbar.unsupported`）：应用窗口装饰由宿主负责。 |
| 851 | `TCASE` | 部分实现 | Changes the case of selected text, mtext, attributes, and dimension text. | 选择式；只处理 Text/MText（MULTILEADER 走 with_text）；裸命令默认 Title，键入 `TCASE UPPER/LOWER/TITLE/SENTENCE/TOGGLE` 或 `U/L/T/S/G`；MTEXT 格式转义逐字保留；属性/标注文字未建模（`tcase.not_reset`） |
| 852 | `TCIRCLE` | 部分实现 | Creates a circle, a slot, or a rectangle around each selected text or mtext object. | `TCIRCLE CIRCLE\|RECTANGLE`；用**估算**文字外框（宽 = `height × 字符数`、高 = `height`、忽略旋转）在文字所在图层建 `Circle` 或闭合 `Polyline`；保留文字；无关联性 |
| 853 | `TCOUNT` | 部分实现 | Adds sequential numbering to text and mtext objects as a prefix, suffix, or replacement text. | `TCOUNT <start> <step> [prefix] [suffix]`；按上→下/左→右顺序把每个选中 Text/MText 替换为 `prefix+编号+suffix`；`step` 必须 >0；AutoCAD 的逐项前缀/后缀对话框模式未实现 |
| 854 | `TEXT` | 已实现 | Creates a single-line text object. | — |
|  855 | `TEXTALIGN` | 部分实现 | Aligns multiple text objects vertically, horizontally, or obliquely. | 15 个对齐拼写（6 个单轴 + 9 个组合，`TEXTALIGN TOPLEFT` 等）按选择并集包围盒在两轴对齐；`DISTRIBUTE [X|Y]` 按阅读序等间隙分布（首尾固定、≥3 个目标，`textalign.distribute_needs_three`）；`SETSPACING`/`CURRENTVERTICAL`/`CURRENTHORIZONTAL` 解析后具名拒绝；只移动参照点，不改内容/字高/旋转/对齐；一条事务/一步撤销；宽高仍为估算值。 |
| 856 | `TEXTEDIT` | 部分实现 | Edits a selected multiline or single-line text object, or the text in a dimension object. | 整串替换；无光标级编辑、不解析 MTEXT 标记 |
| 857 | `TEXTFIT` | 部分实现 | Expands or compresses the width of text objects based on new starting and ending points. | TEXT/MTEXT 高度拟合到目标宽度，其余字段逐位保留；真实字形宽度拉伸未实现（域无 width factor） |
| 858 | `TEXTMASK` | 部分实现 | Creates a blank area behind selected text or mtext objects. | 偏移（默认 0.35）/Wipeout 遮罩/`*TEXTMASK-*` 会话组/重跑替换/组上限/有限边界守卫；`3dface`/`solid` 遮罩类型无实体模型，按名拒绝 |
| 859 | `TEXTSCR` | 部分实现 | Opens a text window that displays a history of prompts and command line entries for the current session. | 置会话 `text_screen`=true；无文本屏幕窗口、不显示提示历史。 |
| 860 | `TEXTTOFRONT` | 部分实现 | Brings text, leaders, and dimensions in front of all other objects in the drawing. | 以**当前选择**为输入（偏离 AutoCAD 的 Select objects 提示）；仅可寻址模型空间 Text/MultiLeader/Dimension 参与，其它选中对象逐条 `texttofront.skipped` 跳过；以所有非合格模型空间对象的最大 draw order 为基线，按相对顺序连续上移（一条事务/一步撤销）；已在前方时 `texttofront.unchanged` 不建事务；图纸空间/块内 draw order 不重排；AutoCAD 的“全图文字”形式与 Draw Order 对话框未实现 |
| 861 | `TEXTUNMASK` | 部分实现 | Removes the mask from selected text or mtext that were masked with TEXTMASK. | 组优先删除 + 估计外框启发；清理已擦除文字留下的陈旧组；未分组遮罩来源不可证明（声明）；无匹配为具名无操作成功 |
| 862 | `TFRAMES` | 部分实现 | Toggles the display of frames for all wipeout and image objects. | 会话令牌 `tframes`；不改变图像/外部参照边框可见性。 |
| 863 | `THICKEN` | 部分实现 | Converts a surface into a 3D solid with a specified thickness. | 具名拒绝（`thicken.unsupported: no surface-to-solid model`）。 |
| 864 | `TIFOUT` | 部分实现 | Saves selected objects to a file in TIFF file format. | 具名拒绝（`tifout.unsupported: raster TIFF export is not implemented; use PLOT for vector SVG/PDF`）。 |
| 865 | `TIME` | 部分实现 | Displays the date and time statistics of a drawing. | 真实 UTC/文档/修订/计数 + 4 个显式 `time.not_tracked`（创建、最后更新、命令耗时、计时器）；未跟踪值不杜撰 |
| 866 | `TINSERT` | 部分实现 | Inserts a block in a table cell. | **无块进入单元格的创建**（`tinsert.unsupported`）；导入的单元格块内容未建模。
| 867 | `TJUST` | 部分实现 | Changes the justification point of a text object without moving the text. Works with text, mtext, and attribute definition objects. | `JUSTIFYTEXT` 的别名，同一语法与实现 |
| 868 | `TOLERANCE` | 部分实现 | Creates geometric tolerances contained in a feature control frame. | `TOLERANCE <x> <y> <text>`（`|` 分格）→ 闭合边框 LWPOLYLINE + 分隔 LINE + 每格 TEXT，活动图层，一条事务；是独立实体而非单个 `AcDbFcf`；轴对齐、格宽/行高为声明估计、≤6 格 |
| 869 | `-TOOLBAR` | 已实现 | Displays, hides, and customizes toolbars from the Command prompt. | chrome 本地选择子：`-TOOLBAR [SHOW|HIDE]` 设置快速访问工具栏可见性 |
| 870 | `TOOLBAR` | 已实现 | Displays, hides, and customizes toolbars. | chrome 本地选择子：显示快速访问工具栏 |
| 871 | `TOOLPALETTES` | 部分实现 | Opens the Tool Palettes window. | chrome 本地选择子：非模态工具选项板浮窗，仅 Blocks 页列出真实块定义并可启动既有 INSERT；无工具内容页，不进入 `cad_command`。 |
| 872 | `TOOLPALETTESCLOSE` | 已实现 | Closes the Tool Palettes window. | chrome 本地选择子：关闭工具选项板浮窗（幂等成功）。 |
| 873 | `TORIENT` | 已实现 | Rotates text, mtext, attribute definitions, and blocks with attributes for readability. | 裸命令 Most Readable、`TORIENT <角度>` 绝对旋转、`TORIENT REVERSE|R` 再加 180°（**本项目扩展**，AutoCAD 2026 无此选项）；只改 TEXT/MTEXT 的 rotation，位置不变；ATTDEF/带属性块因无属性模型不适用（全局范围） |
| 874 | `TORUS` | 部分实现 | Creates a donut-shaped 3D solid. | 具名拒绝（`torus.unsupported: 3D solid primitives are not implemented`）。 |
| 875 | `TPNAVIGATE` | 部分实现 | Displays a specified tool palette or palette group. | chrome 本地选择子：`TPNAVIGATE Blocks` 打开/聚焦 Blocks 页；其它页名或裸词显式拒绝（`toolpalettes.refusal.*`），不转发 `cad_command`。 |
| 876 | `TRACE` | 未实现（同名不同义） | Opens and manages traces from the command prompt. | AutoCAD 2026 的 `TRACE` 是协作追踪；本构建同名命令是**旧式宽线带**（闭合 4 顶点 LWPOLYLINE），语义不同，**不计入覆盖** |
| 877 | `TRACEBACK` | 部分实现 | Changes the active trace to view mode so you can edit the parent drawing while the trace is still visible. | 具名拒绝（`traceback.unsupported: no collaboration trace model`）。 |
| 878 | `TRACEEDIT` | 部分实现 | Changes the active trace to edit mode so you can contribute to the trace. | 具名拒绝（`traceedit.unsupported: no collaboration trace model`）。 |
| 879 | `TRACEFRONT` | 部分实现 | Changes the active trace to edit mode so you can contribute to the trace. | 具名拒绝（`tracefront.unsupported: no collaboration trace model`）。 |
| 880 | `TRACEPALETTECLOSE` | 部分实现 | Closes the Trace palette where you view and manage traces in the current drawing. | 具名拒绝（`tracepaletteclose.unsupported: no collaboration trace model`）。 |
| 881 | `TRACEPALETTEOPEN` | 部分实现 | Opens the Trace palette where you can view and manage traces in the current drawing. | 具名拒绝（`tracepaletteopen.unsupported: no collaboration trace model`）。 |
| 882 | `TRACEVIEW` | 部分实现 | Changes the active trace to view mode so you can edit the parent drawing while the trace is still visible. | 具名拒绝（`traceview.unsupported: no collaboration trace model`）。 |
| 883 | `TRANSPARENCY` | 部分实现 | Controls whether background pixels in an image are transparent or opaque. | 具名拒绝（`transparency.unsupported`）：未建模图像透明度。 |
| 884 | `TRAYSETTINGS` | 部分实现 | Controls the display of icons and notifications in the status bar tray. | 会话令牌（默认 ON）；无状态栏托盘 UI。 |
| 885 | `TREESTAT` | 部分实现 | Displays information about the drawing's current spatial index. | 显式 `treestat.not_tracked`；不造假计数 |
| 886 | `TREX` | 部分实现 | Combines the TRIM and EXTEND commands. | `TREX TRIM\|EXTEND`；选择集为边界，复用 `plan_trim`/`plan_extend`；无边界提示 |
| 887 | `TRIM` | 部分实现 | Trims objects to meet the edges of other objects. | 目标 LINE/ARC/CIRCLE/SPLINE/**ELLIPSE（含椭圆弧）**/**LWPOLYLINE（开放/闭合，bulge 保留为精确子弧）**；拾取投影所在的段保留（本项目 TRIM 语义，见 `docs/drawing-edit.md` §3）；边界折线/样条采样；**INSERT/HATCH/MESH/OPAQUE/COMPOUND 目标与 EXTEND 的 POLYLINE 目标按名拒绝** |
| 888 | `TSCALE` | 部分实现 | Scales text, mtext, attributes, and attribute definitions. | `TSCALE <factor>`；复用 SCALETEXT 的唯一 `scaled_text` 规则（height + MText `wrap_width`）；factor=1 为 `tscale.unchanged` 且不开事务 |
| 889 | `TXT2MTXT` | 部分实现 | Converts or combines single-line or multiline text objects into one or more multiline text objects. | COMBINE/SEPARATE/ORDER/word-wrap；**每个源文字高度以 `\H` 保留**、保留首个已存 wrap 宽度；`UNIFORM` 行距无字段 |
| 890 | `TXTEXP` | 已实现 | Explodes text or mtext objects into polyline objects. | 字体轮廓→每个轮廓一条 `LWPOLYLINE`（闭合 sfnt 环闭合、SHX 笔画开放），删除源 TEXT/MTEXT，一条事务/一步撤销；字体引擎由宿主安装；MTEXT 宽度换行未应用（声明） |
| 891 | `U` | 已实现 | Reverses the most recent operation. | — |
| 892 | `UCS` | 部分实现 | Sets the origin and orientation of the current user coordinate system (UCS). | 具名拒绝（`ucs.unsupported`）：本构建无 UCS 模型。 |
| 893 | `UCSICON` | 部分实现 | Controls the visibility, placement, appearance, and selectability of the UCS icon. | 会话开关（默认关）：翻转 `ucsicon`；不渲染任何 UCS 图标。 |
| 894 | `UCSMAN` | 部分实现 | Manages UCS definitions. | 具名拒绝（`ucsman.unsupported`）：本构建无 UCS 模型。 |
| 895 | `ULAYERS` | 部分实现 | Controls the display of layers in a DWF, DWFx, PDF, or DGN underlay. | 具名拒绝（`ulayers.unsupported: no DWF/PDF/DGN underlay layer-display model`）。 |
| 896 | `UNDEFINE` | 已实现 | Allows an application-defined command to override an internal command. | `UNDEFINE <name>` 会话内禁用内建命令；被禁用名按 `command.undefined: <NAME>` 拒绝且不派发；静态/运行时别名指向禁用目标时同样禁用；不写图 |
| 897 | `UNDO` | 已实现 | Reverses the effect of commands. | — |
| 898 | `UNGROUP` | 部分实现 | Disassociates the objects from a group. | 仅移除一个组名并保留全部实体；未知组名拒绝；不写实体/数据库 |
| 899 | `UNION` | 部分实现 | Combines two or more 3D solids, surfaces, or 2D regions into a single, composite 3D solid, surface, or region. | 具名拒绝（`union.unsupported: no 3D boolean model`）。 |
| 900 | `UNISOLATEOBJECTS` | 部分实现 | Displays objects previously hidden with the ISOLATEOBJECTS or HIDEOBJECTS command. | 清空会话隐藏集；无数据库写/撤销 |
| 901 | `-UNITS` | 部分实现 | Controls the precision and display formats for coordinates, distances, and angles. | 只读报告固定单位显示规则（`-units.*` 诊断码）；Units 对话框/逐图纸单位表未实现。 |
| 902 | `UNITS` | 部分实现 | Controls the precision and display formats for coordinates, distances, and angles. | 只读报告（`units.report`/`units.dialog_not_implemented`）：图纸单位 4 位小数、角度用度；UNITS 对话框与逐图纸单位表未实现。 |
| 903 | `UPDATEFIELD` | 部分实现 | Updates fields in selected objects. | 具名拒绝（`updatefield.unsupported`）：未建模字段。 |
| 904 | `UPDATETHUMBSNOW` | 已实现 | Manually updates thumbnail previews for named views, drawings, and layouts. | 立即刷新 Quick View 预览卡（重算预览状态并显示瞬时“已更新”；不做文件写） |
| 905 | `VBAIDE` | 部分实现 | Displays the Visual Basic Editor. | 具名拒绝（`vbaide.unsupported: no VBA IDE model`）。 |
| 906 | `VBALOAD` | 部分实现 | Loads a global VBA project into the current work session. | 具名拒绝（`vbaload.unsupported: no VBA loader model`）。 |
| 907 | `VBAMAN` | 部分实现 | Manages VBA project operations using a dialog box. | 具名拒绝（`vbaman.unsupported: no VBA runtime/IDE`）。 |
| 908 | `VBAPREF` | 部分实现 | Provides access to some of the VBA environment settings. | 具名拒绝（`vbapref.unsupported: no VBA runtime/IDE`）。 |
| 909 | `VBARUN` | 部分实现 | Runs a VBA macro. | 具名拒绝（`vbarun.unsupported: no VBA runtime`）。 |
| 910 | `VBASTMT` | 部分实现 | Executes a VBA statement at the AutoCAD Command prompt. | 具名拒绝（`vbastmt.unsupported: no VBA runtime/IDE`）。 |
| 911 | `VBAUNLOAD` | 部分实现 | Unloads a global VBA project. | 具名拒绝（`vbaunload.unsupported: no VBA runtime`）。 |
| 912 | `-VIEW` | 部分实现 | Saves and restores named model space views, layout views, and preset views from the Command prompt. | `VIEW` 的别名，同一实现 |
| 913 | `VIEW` | 部分实现 | Saves and restores named model space views, layout views, and preset views. | `SAVE/RESTORE/LIST/DELETE <name>`；会话内最多 32 个命名视图（相机+视图模式+工作平面+活动空间）；不持久化 DWG |
| 914 | `VIEWBACK` | 部分实现 | Restores sequential views backward after changing the view. | 复用每视口 `ZOOM PREVIOUS` 栈；空栈为 `view.back.empty` no-op |
| 915 | `VIEWBASE` | 部分实现 | Creates a base view from model space or Autodesk Inventor models. | 具名拒绝（`viewbase.unsupported: no 3D model-to-drawing base/detail/section view model`）。 |
| 916 | `VIEWCOMPONENT` | 部分实现 | Selects components from a model documentation drawing view for editing. | 具名拒绝（`viewcomponent.unsupported: no 3D model-to-drawing base/detail/section view model`）。 |
| 917 | `VIEWDETAIL` | 部分实现 | Creates a detail view of a portion of a model documentation drawing view. | 具名拒绝（`viewdetail.unsupported: no 3D model-to-drawing base/detail/section view model`）。 |
| 918 | `VIEWDETAILSTYLE` | 部分实现 | Creates and modifies detail view styles. | 具名拒绝（`viewdetailstyle.unsupported: no 3D model-to-drawing base/detail/section view model`）。 |
| 919 | `VIEWEDIT` | 部分实现 | Edits an existing model documentation drawing view. | 具名拒绝（`viewedit.unsupported: no 3D model-to-drawing base/detail/section view model`）。 |
| 920 | `VIEWFORWARD` | 部分实现 | Restores sequential views forward after displaying a previous view with VIEWBACK or Zoom Previous. | 新的每视口前进栈（上限 16）；空栈 no-op；文档化偏离 |
| 921 | `VIEWGO` | 部分实现 | Restores a named view. | 同 `VIEW RESTORE`；未知名称按名拒绝 |
| 922 | `VIEWPLAY` | 部分实现 | Plays the animation associated to a named view. | 具名拒绝（`viewplay.unsupported: no named-view animation model`）。 |
| 923 | `VIEWPLOTDETAILS` | 部分实现 | Displays information about completed plot and publish jobs. | 具名拒绝（`viewplotdetails.unsupported`）：无打印详细信息模型。 |
| 924 | `VIEWPROJ` | 部分实现 | Creates one or more projected views from an existing model documentation drawing view. | 具名拒绝（`viewproj.unsupported: no 3D model-to-drawing base/detail/section view model`）。 |
| 925 | `VIEWRES` | 部分实现 | Sets the tessellation for objects in the current viewport when WHIPARC is turned off. | 整数 `1..=1000` 仅存会话 token；越界 `viewres.out_of_range`；渲染器消费/像素效果 NOT IMPLEMENTED / NOT VERIFIED |
| 926 | `VIEWSECTION` | 部分实现 | Creates a section view of a 3D model that was created in AutoCAD or in Autodesk Inventor. | 具名拒绝（`viewsection.unsupported: no 3D model-to-drawing base/detail/section view model`）。 |
| 927 | `VIEWSECTIONSTYLE` | 部分实现 | Creates and modifies section view styles. | 具名拒绝（`viewsectionstyle.unsupported: no 3D model-to-drawing base/detail/section view model`）。 |
| 928 | `VIEWSETPROJ` | 部分实现 | Specifies the active project file for drawings containing model documentation drawing views from Inventor models. | 具名拒绝（`viewsetproj.unsupported: no 3D model-to-drawing base/detail/section view model`）。 |
| 929 | `VIEWSKETCHCLOSE` | 部分实现 | Exits the symbol sketch mode. | 具名拒绝（`viewsketchclose.unsupported: no 3D model-to-drawing base/detail/section view model`）。 |
| 930 | `VIEWSTD` | 部分实现 | Defines the default settings for model documentation drawing views. | 具名拒绝（`viewstd.unsupported: no 3D model-to-drawing base/detail/section view model`）。 |
| 931 | `VIEWSYMBOLSKETCH` | 部分实现 | Opens an editing environment so you can constrain section lines or detail boundaries to drawing view geometry. | 具名拒绝（`viewsymbolsketch.unsupported: no 3D model-to-drawing base/detail/section view model`）。 |
| 932 | `VIEWUPDATE` | 部分实现 | Updates drawing views that have become out-of-date because the source model has changed. | 具名拒绝（`viewupdate.unsupported: no 3D model-to-drawing base/detail/section view model`）。 |
| 933 | `-VISUALSTYLES` | 部分实现 | Creates and modifies visual styles from the command line. | 具名拒绝（`-visualstyles.unsupported: no visual-style/3D display model; VSCURRENT owns the 2D display token`）。 |
| 934 | `VISUALSTYLES` | 部分实现 | Creates and modifies visual styles and applies a visual style to a viewport. | 具名拒绝（`visualstyles.unsupported: no visual-style/3D display model; VSCURRENT owns the 2D display token`）。 |
| 935 | `VISUALSTYLESCLOSE` | 部分实现 | Closes the Visual Styles Manager. | 具名拒绝（`visualstylesclose.unsupported: no visual-style/3D display model; VSCURRENT owns the 2D display token`）。 |
| 936 | `VLISP` | 部分实现 | Displays the AutoLISP development environment. | 具名拒绝（`vlisp.unsupported: no AutoLISP runtime/IDE`）。 |
| 937 | `VPCLIP` | 部分实现 | Redefines a layout viewport object while retaining its properties. | 矩形 + **多边形 clip 真实写入**（N 点、共面/自交校验）；representation 仅接受 3/4 点 clip ⇒ 多边形按 `*.polygon_not_renderable` 诚实报告，不伪称可渲染 |
| 938 | `VPLAYER` | 已实现 | Sets layer visibility within viewports. | `VPLAYER FREEZE/THAW <layer,...> [<index>]` 写单视口 `layer_overrides`（全局层表不动），bare 为只读报告；命令行 grammar 已接；一条事务/一步撤销 |
| 939 | `VPMAX` | 已实现 | Expands the current layout viewport for editing. | 真实最大化编辑态：保存相机+视图模式+工作平面+空间的 `ViewSnapshot`，`VPMIN` 位级恢复；一次一个视口；无宿主差异化渲染（无像素证据） |
| 940 | `VPMIN` | 部分实现 | Restores the current layout viewport. | 恢复会话标记；空状态 `vpmin.nothing_to_restore` |
| 941 | `VPOINT` | 部分实现 | Sets the viewing direction for a 3D visualization of the drawing. | 向量或 `TOP/BOTTOM/FRONT/BACK/LEFT/RIGHT`；零/非有限向量拒绝；不改模型；向量形式进入 3D 透视 |
| 942 | `-VPORTS` | 部分实现 | Creates multiple viewports in model space or paper space. | `VPORTS` 的别名，同一实现；同一边界 |
| 943 | `VPORTS` | 部分实现 | Creates multiple viewports in model space or in a layout (paper space). | 在活动图纸布局创建简单 rows×cols 视口网格（四角 clip、`model_per_paper=1`），一条事务/一步撤销；命名/标准视口配置未实现 |
| 944 | `VPSCALE` | 部分实现 | On a layout, displays the scale of the current viewport or of a selected layout viewport. | 设置活动布局**第一个**视口比例（存储对角线 `1/factor`，保留模型锚点），一条事务/一步撤销；无视口选择 |
| 945 | `VPSYNC` | 部分实现 | Aligns the views in one or more adjacent layout viewports with a master layout viewport. | 把 viewport 0 的 `model_to_paper` 复制到其余视口；主视口不可选择（无逐视口选择模型） |
| 946 | `VSCURRENT` | 部分实现 | Sets the visual style in the current viewport. | 仅 `WIREFRAME` 存会话 token；其余样式 `vscurrent.style_not_implemented`；无渲染器切换、NOT pixel-verified |
| 947 | `VSLIDE` | 部分实现 | Displays an image slide file in the current viewport. | 具名拒绝（`vslide.unsupported: no slide (.sld) format model`）。 |
| 948 | `VSSAVE` | 部分实现 | Saves the current visual style under a new name. | 具名拒绝（`vssave.unsupported: no slide (.sld) format model`）。 |
| 949 | `VTOPTIONS` | 部分实现 | Displays a change in view as a smooth transition. | 只读：视图过渡设置未建模（`vtoptions.not_tracked`），不杜撰平滑/时长数值。 |
| 950 | `WALKFLYSETTINGS` | 部分实现 | Controls the walk and fly navigation settings. | 具名拒绝（`walkflysettings.unsupported: no 3D walk/fly navigation model`）。 |
| 951 | `-WBLOCK` | 部分实现 | Saves selected objects or converts a block to a specified drawing file. | 具名拒绝（`-wblock.unsupported`）：将块定义写入独立图形文件未实现。 |
| 952 | `WBLOCK` | 部分实现 | Saves selected objects or converts a block to a specified drawing file. | 具名拒绝（`wblock.unsupported`）：将块定义写入独立图形文件未实现。 |
| 953 | `WEBLIGHT` | 部分实现 | Creates a precise 3D representation of the light intensity distribution of a light source. | 具名拒绝（`weblight.unsupported: no 3D light model`）。 |
| 954 | `WEBLOAD` | 部分实现 | Loads a JavaScript file from a URL, and then executes the JavaScript code contained in the file. | 具名拒绝（`webload.unsupported: no JavaScript/web-load runtime`）。 |
| 955 | `WEDGE` | 部分实现 | Creates a 3D solid wedge. | 具名拒绝（`wedge.unsupported: 3D solid primitives are not implemented`）。 |
| 956 | `WHOHAS` | 部分实现 | Displays information about an open drawing file. | 只读：单文档构建，无网络查询，不列其他用户或文件锁。 |
| 957 | `WIPEOUT` | 部分实现 | Creates a wipeout object, and controls whether wipeout frames are displayed in the drawing. | `Frames ON|OFF|DISPLAY` 真实：ON = 遮罩填充 + 边框（显示与矢量出图都画边界折线），OFF = 仅填充，DISPLAY = 显示边框但出图不画；边框在 mask fill 之后用 Always-depth 线管线绘制（不是被覆盖的普通 Lines）；`Polyline`/`Erase Polyline` + 自由点串；`FRAME` 覆盖仍无命令 |
| 958 | `WMFIN` | 部分实现 | Imports a Windows metafile. | 具名拒绝（`wmfin.unsupported: Windows metafile import is not implemented`）。 |
| 959 | `WMFOPTS` | 部分实现 | Sets options for WMFIN. | 具名拒绝（`wmfopts.unsupported: no Windows metafile import model (WMFIN is not implemented)`）。 |
| 960 | `WMFOUT` | 部分实现 | Saves objects to a Windows metafile. | 具名拒绝（`wmfout.unsupported: Windows metafile export is not implemented`）。 |
| 961 | `WORKSPACE` | 已实现 | Creates, modifies, and saves workspaces and makes a workspace current. | chrome 工作空间选择器：功能区/选项板/文件标签可见性与面板宽度预设（内置 Drafting & Annotation） |
| 962 | `WSSAVE` | 已实现 | Saves a workspace. | 把当前 UI 状态保存为命名工作空间（会话内存，不落盘） |
| 963 | `WSSETTINGS` | 已实现 | Sets options for workspaces. | chrome 工作空间设置面：列出/重命名/删除/设为当前 |
| 964 | `XATTACH` | 部分实现 | Attaches selected DWG files as external references (xrefs). | 具名拒绝（`xattach.unsupported`）：本构建不把外部图纸作为外部参照附着。 |
| 965 | `-XBIND` | 部分实现 | Binds one or more definitions of named objects in an xref to the current drawing. | 具名拒绝（`-xbind.unsupported`）：本构建无外部参照绑定/插入模型。 |
| 966 | `XBIND` | 部分实现 | Binds one or more definitions of named objects in an xref to the current drawing. | 具名拒绝（`xbind.unsupported`）：本构建无外部参照绑定/插入模型。 |
|  967 | `XCLIP` | 部分实现 | Crops the display of a selected external reference or block reference to a specified boundary. | 真实（display-only）：`XCLIP RECT|POLY|ON|OFF|DELETE|?`（一个直接可寻址的模型空间 INSERT；RECT/POLY 为块局部坐标）；clip 存于 INSERT 的 `EntityRenderAttributes`（一条事务/一步撤销），表示层在块展开时映射为世界轴对齐矩形并精确裁剪 LINE/ARC/圆/MESH/IMAGE（TEXT/MASK 保留并报告）；矩形仅 0/90/180/270 世界旋转可渲染，其它旋转 `xclip.rotation_not_renderable`；多边形存而报告 `xclip.polygon_not_renderable`，INVERT 拒绝；拾取/范围仍按未裁剪几何；无 DXF/DWG 持久化。 |
| 968 | `XCOMPARE` | 部分实现 | Compares an attached xref with the latest state of the referenced drawing file, highlighting the differences with color within revision clouds. | 具名拒绝（`xcompare.unsupported: no xref comparison/diff model`）。 |
| 969 | `XCOMPARECLOSE` | 部分实现 | Closes the Xref Compare toolbar and ends the comparison. | 具名拒绝（`xcompareclose.unsupported: no xref comparison/diff model`）。 |
| 970 | `XCOMPARERCNEXT` | 部分实现 | Zooms to the next change set of the xref comparison result. | 具名拒绝（`xcomparercnext.unsupported: no xref comparison/diff model`）。 |
| 971 | `XCOMPARERCPREV` | 部分实现 | Zooms to the previous change set of the xref comparison result. | 具名拒绝（`xcomparercprev.unsupported: no xref comparison/diff model`）。 |
| 972 | `XDATA` | 部分实现 | Attaches extended object data (xdata) to a selected object. | 具名拒绝（`xdata.unsupported`）：本数据库不保留实体 XDATA。 |
| 973 | `XDLIST` | 部分实现 | Lists the xdata attached to an object. | 只读；每个选中实体报告 `xdlist.not_retained`；不伪造 xdata |
| 974 | `XEDGES` | 部分实现 | Creates wireframe geometry from the edges of a 3D solid, surface, mesh, region, or subobject. | 具名拒绝（`xedges.unsupported: no 3D solid/surface wireframe extraction model`）。 |
| 975 | `XLINE` | 部分实现 | Creates a construction line of infinite length. | 两点创建，但落库为裁剪到模型包围盒的有限线段 |
| 976 | `XLIST` | 部分实现 | Lists the type, block name, layer name, color and linetype of a nested object in a block or an xref. | 只读：列出块定义及成员数；实体 XDATA 不保留，不列出。 |
| 977 | `XOPEN` | 部分实现 | Opens a selected drawing reference (xref) in a new window. | 具名拒绝（`xopen.unsupported`）：本构建无外部参照编辑会话。 |
| 978 | `XPLODE` | 部分实现 | Breaks a compound object into its component objects, with specified properties for the resulting objects. | 真实编辑：复用 EXPLODE 分解选中 INSERT；一条事务/一步撤销；属性/特性控制未实现。 |
| 979 | `-XREF` | 部分实现 | From the Command prompt, manages drawings inserted as external references (xrefs). | 只读：外部参照导入溯源报告（`-xref.entry`/`-xref.summary`）；附着/卸载/重载/绑定未实现。 |
| 980 | `XREF` | 部分实现 | Starts the EXTERNALREFERENCES command. | 只读：外部参照导入溯源报告（`xref.entry`/`xref.summary`）；附着/卸载/重载/绑定/编辑未实现；本构建无选项板。 |
| 981 | `ZOOM` | 部分实现 | Increases or decreases the magnification of the view in the current viewport. | OBJECT（当前选择包围盒，含块展开）/nX/图纸空间 nXP/WINDOW/PREVIOUS 已实现；仅 DYNAMIC（交互式指针跟随缩放）具名未实现（`zoom.dynamic_not_implemented`） |

## 项目词表审计（合并自 `docs/autocad-command-coverage.md`）

> 本节是原「AutoCAD 命令覆盖审计」的内容，已并入本文；**状态以本文件上方总表为准**，
> 逐条能力边界以 `docs/compatibility.md` 为权威。原文件的 115 选择子 / 179 拼写是旧快照，
> 下列数字已更新到当前源码表（复核 2026-11-07）。

### 1. 「total」是什么，不是什么

仓库里**没有**任何**权威、可枚举的「AutoCAD 命令全集」**：没有 `acad.pgp` 解析器，没有
命令清单数据文件，也没有从 Autodesk 文档导入的表。本文把「total」拆成两个**可辩护**的量：

1. **我们的词表**（可精确计数，源码是唯一权威）：选择子表、测量选择子表、缩放步进表、
   带参数表、别名表。这些数**可以**说「全部通过契约测试」。
2. **官方 AutoCAD 2026 命令索引**（981 条，见上方全量清单）：这是本文的覆盖基准；
   状态逐条写入，但**不是**行为等价或验收。

> **禁止的读法**：不得把 `229`、`309` 或本文任何数字读成「已实现 AutoCAD 的 X%」。
> AutoCAD 的命令总数取决于版本、产品（完整版/LT/工具集）、是否含扩展与第三方；任何
> `222/981` 形式的百分比都只是**本清单的逐条计数**，不是兼容率。多少条名字有命令路径
> 与这些路径是否与 AutoCAD 等价，是两个问题。

### 2. 词表总量（每项都有 source of truth）

| 表 | 数量 | 源码位置 | 断言 / 说明 |
|---|---:|---|---|
| `COMMAND_TABLE` | **229** 个选择子 | `crates/cad-ui-core/src/session/commands.rs` | 与 `CommandId` 变体一一对应；`EXPECTED_COMMAND_COUNT = 229` 钉在 `crates/cad-ui-core/src/session/tests/tests_commands/command_vocabulary.rs`（并断言每变体在表中**恰好一次**、表长相等） |
| `MEASUREMENT_SELECTORS` | **5** | 同文件 | `DIST`/`MEASUREPOLYLINE`/`MEASUREANGLE`/`AREA`/`ID`；交互专用，没有自己的 `CommandId` |
| `ZOOM_STEP_SELECTORS` | **2** | 同文件 | `ZOOMIN`/`ZOOMOUT`（固定步进因子 `1.25` 与倒数） |
| `ARGUMENT_SELECTORS` | **65**（含通用 `ARGUMENT_SELECTOR`） | 同文件 | 接受命令行参数的基选择子；`ARGUMENT_SELECTOR` 是通用分发入口，其余按批分组 |
| `COMMAND_ALIASES` | **73** 行 | 同文件 | 别名 + 全词同义词；只收录**目标选择子真实存在**的拼写 |
| `admissible_names()` | **309** 个可输入拼写 | 同文件 | 229 + 5 + 2 + 73 = 309（无重名）。这是**拼写**数，**不是**命令数 |

> 这些数字是**当前源码表的长度**（复核 **2026-11-07**）。命令的**行为**数上限是
> `CommandId` 变体数（= 229），其中还有若干是**本项目自己的**名字（不是 AutoCAD 命令）。
> 表一变，本文的 `总计` 与本节都要跟着改；**权威是源码表与 `check-command-docs.py` 的实跑输出**，
> 不是手抄值。

**为什么不把 309 当命令数**：别名只是把输入**归一化**到同一个选择子（`normalize_command`），
不改变行为；`ZOOMIN`/`ZOOMOUT` 与各测量选择子不携带 `CommandId`。

### 3. 本项目自己的名字 / 同名不同义

**本项目自己的名字**（AutoCAD 无同名，或同名不同义），不计入 AutoCAD 覆盖：
`RESETVIEW`、`SWITCHBACKEND`、`SWITCH2D3D`、`STANDARDVIEW`（AutoCAD 的 `VIEW` 是命名的视图管理）、
`MODE`（Viewer/Work）、`RESOURCES`、`DIAGNOSTICS`、`SPACE`（模型/图纸空间切换）、
`SELECTINBOX`、`SELECTSIMILAR`（v1 仅按几何类型）、`CONFIRM`/`CANCELMEASURE`、`GRIPSTRETCH`、
`HOVERPREVIEW`、`HATCHPOINT`（AutoCAD 的 `HATCH` 走内部点提示；本仓库把它拆成独立选择子）、
`TOGGLELAYER`/`SETLAYERS`/`RESTORELAYERS`/`SETACTIVELAYER`/`CREATELAYER`/`RENAMELAYER`/
`DELETELAYER`/`SETLAYERPROPERTY`（AutoCAD 的对应物是 `-LAYER`/`LAYER` 子选项，语义不同）、
`SETENTITYPROPERTY`、`MEASUREPOLYLINE`/`MEASUREANGLE`（AutoCAD 的 `PLINE` 现在是绘制命令，
`ANGLE` 不是 AutoCAD 命令）、`AREA OBJECT`（AutoCAD 的 `AREA`→`Object` 提示的**选择式**变体）、
`POLYGON CIRCUMSCRIBED`（`POLYGON` 的外切构造，独立多词选择子）。

**同名不同义**（不计覆盖）：
- `MEASURE`：AutoCAD 沿对象按间距放置点/块；本仓库是**测量工具入口**（默认距离），不产生任何实体。
- `TRACE`：AutoCAD 2026 是**协作追踪**（打开/管理 trace）；本仓库同名命令是**旧式宽线带**。

### 4. 命令边界审计（旧表迁移）

> 下表保留旧审计的逐条边界，状态已按当前主表更新；新增批次的完整边界见上方索引与
> `docs/compatibility.md`。

#### 4.1 绘制

| AutoCAD 名 | 状态 | 界 |
|---|---|---|
| `LINE` | 实现 | 两点；退化/共线/非有限拒绝 |
| `CIRCLE` | 实现 | 圆心 + 半径点；**`2P`/`3P`/`D` 已实现（2026-11-01，多词 canonical）**：`CIRCLE 2P` 直径两端点、`CIRCLE 3P` 三点外接圆（复用 `arc_parameters`）、`CIRCLE D` 圆心 + **直径距离**（半径 = 距离/2）；三者都产一个 `Circle`，活动图层/模型空间，一条事务/一步撤销 |
| `ARC` | 实现 | **三点弧**；无 起点-圆心-端点 / 起点-圆心-角 / 连续弧 |
| `PLINE` | 实现 | 只产**开放 LWPOLYLINE**、**每段 bulge=0**；无弧段/`Close`/`Width`/`Length`/`Undo` |
| `RECTANG` | 实现 | 两对角点 → 闭合 4 顶点 LWPOLYLINE；无 `Chamfer`/`Fillet`/`Width`/`Elevation`/`Rotation`/`Area` |
| `POLYGON` | 实现 | **内接于圆**、边数 3–1024；无 `Edge` 分支、**无外切模式** |
| `ELLIPSE` | 实现 | 轴端点法**整椭圆**；无 `Arc`/`Center` 法/`Rotation`/`Isocircle` |
| `POINT` | 实现 | **单次放置**；显示走 `$PDMODE`/`$PDSIZE`（基础 `0`–`4`，周边 `32` 圆/`64` 方/`96` 两者，线段近似）；无 `Multiple` 重复放置 |
| `SPLINE` | 实现 | fit 点法（≥4 点）→ 三次/clamped/chord 插值 NURBS；无 `CV`/`Knots`/`Degree`/`Close`/`Tangency`/`Object` |
| `POLYLINE` | **未实现** | 旧式 heavy 2D 折线实体（`AcDb2dPolyline` + 顶点子实体）。我们**只创建 LWPOLYLINE**；别名到 `PLINE` 会**凭空发明语义**，故不做；不在官方 981 索引，故不影响总计 |
| `XLINE`（含 `RAY`） | **实现（有界创建，2026-11-01；导入有有界降级）** | **命令**：两点（base/through）经 `clip_segment_to_xy_rect` 裁到模型包围盒（外扩 5%、至少 1.0）或默认 1000 单位盒，生成一条有限 `Line`、type-key `AcDbXline`/`AcDbRay`，一条事务/一步撤销；**不是无限几何**，导出写 DXF `LINE`（`Converted`）。库里**没有无限直线几何**：导入期把 XLINE/RAY 压成**有限 `SemanticGeometry::Line`**，裁到**运行中的模型包围盒**（盒每边外扩对角线 5%、至少 1.0）⇒ 裁多长**取决于导入顺序**（先出现什么几何，盒就是什么）；还没有任何几何时用**默认 1000 单位盒**。判定：方向退化 → `Opaque` + `Partial`；裁不到盒内 → `Opaque` + `Partial`；否则 `Line` + **`Partial`**（原因是具名常量 `support::XLINE_SEGMENT_REASON` / `XLINE_DEFAULT_BOX_REASON`，并额外记一次 `import.construct_line`）。**导出**只能写 DXF `LINE`，报 `Converted`（`export.converted.line`，点名「不再是无限线」），**不是**精确往返。没有创建命令、没有无限表示。**规模**：命令 **M**（建在既有有界切片上）；真正的无限几何 **L / ADR 级**（见 §5） |
| `HATCH` | 部分 | **两个入口（2026-11-01）**：`HATCH` **先选闭合边界实体**；`HATCHPOINT` **按内部点**（本项目选择子；在可见的**整实体闭合** `Circle` / 完整 `Ellipse` / 闭合 `Polyline` **以及可见 `LINE`/`ARC` 边链**中取**最小包含边界**，嵌套环作岛屿；`LINE`/`ARC` 用平面半边面行走、弧存**精确 bulge**；不用当前选择、只认 `+Z` 同标高、仅模型空间；种子存 `seeds`/组 98）。填充类型 `0` 实体 / `1` 用户自定义 / `2` 预定义 / **`3` 渐变（2026-10-27）**；预定义图案**恰好** `ANSI31`/`ANSI32`/`ANSI37`（状态栏**按名**选，线上传 1 基索引）。**渐变创建**：双色两站（`stop 0.0` = `hatchGradientColorA`、`stop 1.0` = B），kind 只有 `LINEAR`/`SPHERICAL`（**从不**写 `CYLINDER`），角度**通道给度、实体存弧度**（一次 `.to_radians()`），颜色是 **ACI 整数 `1..=255`**（`0` 是 ByBlock 占位、无 sRGB，按名拒绝），默认 1 红 / 5 蓝；实体写 `HatchGradient::TwoColor { reserved: 0, shift: 0.0 }` + `pattern_type = Predefined`（`76=1`，**未查证的 vendor 约定**）。**界**：`SPHERICAL` 在矩形/圆这类「边界顶点同半径」的形状上**烘焙成平色/椭圆感、不是径向**（按三角化顶点取值）；只有 ACI 调色板（无真彩、无名/色簿）；stop 颜色下游还会被实体/图层 tint 乘；**无单色/tint 变体、无逐站命名**。内部点求边界已实现（`HATCHPOINT` v1，见上）；无 `76=2`/`.pat`、无关联性、无实时预览 |
| `MLEADER` | 部分 | 单簇/单线/2 顶点/**仅直线**；无 `Options`（引线类型/内容类型/最多点数/角度约束）；狗腿/箭头/字高取 `Standard` 样式表（常量为债务） |
| `TEXT` | 实现 | **单行**、1024 字符上限、**拒绝换行**；不解释 MTEXT 控制码（逐字存储）；**旋转取会话参数 `textRotation`**（通道**度**、任意有限、**不 wrap**、默认 `0`，创建时**一次** `.to_radians()` 存弧度；载荷本身没有旋转字段） |
| `MTEXT` | 部分 | **两角宽度模型** + 终端文本；**旋转取 `textRotation`**（同上，度→弧度一次）、**附着取 `mtextAttachment`**（闭集 `1..=9` 的 3×3 网格：`1..=3` top / `4..=6` middle / `7..=9` bottom，每行左/中/右——`h = (code−1) % 3`、`row = (code−1)/3`，**永不产出 `TextAlignV::Baseline`**；默认 `1` 左上），越界 code 与非有限旋转**按名拒绝**、不钳制；**仍未实现**：行距、分栏、背景填充、绘制方向、绘制流程内 MTEXT 对话框 |
| `TEXTEDIT` | 部分 | 拾取一个文本 → **预填当前串** → 整串替换；无光标级编辑、不解析 MTEXT 标记（MLEADER 三条入口已接） |

| `REVCLOUD` | 部分 | **对象模式 v1（2026-11-07）**：恰好 1 条选中的**闭合、全直线段** LWPOLYLINE；每条边换成一段**向外**弧（bulge 幅度 `0.5`，sweep ≈ 106.26°；CCW 环取 `+0.5`、CW 取 `-0.5`，符号由 `cad_geometry` 的 bulge 约定决定），实体**原地重写**、一条事务/一步撤销。**按名拒绝**：开放折线、已有 bulge、非折线、`+Z` 外/非有限/零面积、多选或空选。**未实现**：矩形/多边形交互绘制、`Arc length` 参数、对象拾取模式、AutoCAD 的弧长/弦长样式选项 |

| `TRACE` | **同名不同义（不计入）** | AutoCAD 2026 的 `TRACE` 是**协作追踪**（打开/管理 trace）；本构建同名命令是**旧式宽线带**（闭合 4 顶点 LWPOLYLINE），语义不同，**不计入覆盖** |

#### 4.2 标注

| AutoCAD 名 | 状态 | 界 |
|---|---|---|
| `DIMLINEAR` / `DIMALIGNED` | 实现 | **三点流程**（两原点 + 尺寸线位置）；**非关联**；无文字覆盖；当前样式恒为 `Standard` |
| `DIMRADIUS` / `DIMDIAMETER` | 实现 | **实体拾取圆弧/圆** + 位置；组 40 写 `0.0`（引线延长未建模）；弧的引线点落在**整圆**上、不钳到扫掠范围 |
| `DIMANGULAR` | 部分 | **仅两线形式可交互**；三点形式只到命令/载荷层；拾取范围**仅 `Line`** |
| `DIMORDINATE` | 实现 | 特征点 + 引线端点；自动 X/Y 规则为**我们的规则**（`\|Δy\| > \|Δx\|` ⇒ X） |
| `DIMCENTER` | 实现 | 产出**普通 `LINE` 几何**，**不是** DIMENSION 实体；`DIMCEN = 0` ⇒ 0 实体 + 具名诊断 |
| `DIMBASELINE` / `DIMCONTINUE` | 实现 | **显式拾取基准线性标注**（零跨命令状态）、N 点 + Enter、**一条事务/一步撤销**、`DIMDLI` 行距；**仅线性基准**、**不写 `DIMCONTINUEMODE`**、非关联、无文字覆盖 |
| `DIMSTYLE` | **部分** | `LIST`/`CURRENT` 只读报告 `cad-db` 的 `dim_styles`；`SET <name>` 在名称校验后按 `dimstyle.set_unavailable` **具名拒绝**（无 post-import 样式表写 API）。实体存**已解析快照**，编辑样式**不追溯** |
| `DIMEDIT` / `DIMTEDIT` / `DIMOVERRIDE` / `DIMBREAK` / `QDIM` / `DIMSPACE` | **部分** | `DIMEDIT` 文字覆盖/复位、`DIMTEDIT` 文字移动/旋转/复位、`DIMOVERRIDE` 实体快照变量、`QDIM` 直线 v1、`DIMSPACE` 等距重排是真实写路径（一条事务/一步撤销）；`DIMBREAK` 无打断记录，按 `dimbreak.unsupported` 拒绝。逐条见上方索引 |
| `DIMJOGLINE` | **部分** | `ADD`/`REMOVE` 语法校验后按 `dimjogline.unsupported` 拒绝（域无 jog 字段/存储） |
| `DIMARC` / `DIMJOGGED` / `DIMREASSOCIATE` | **未实现** | 无弧长/折弯标注字段与关联模型 |
| 关联性（`DIMASSOC=2`） | **未实现** | 创建与迁移的标注都是非关联单实体；导出不写 `ACAD_DIMASSOC` |

#### 4.3 修改

| AutoCAD 名 | 状态 | 界 |
|---|---|---|
| `MOVE` | 实现 | 平移选择；不可变换几何（`Shape`/`Opaque`/`Compound`…）整条拒绝；`SPLINE` 可平移（NURBS 仿射不变） |
| `COPY` | 实现 | 按位移复制、新 id；样式覆盖无法复制时记 `drawing.copy.style` |
| `ROTATE` | 实现 | 绕基点旋转（相似变换）；选择含 `MultiLeader` 时**仍可旋转**（缩放则整命令失败，见 `SCALE`） |
| `SCALE` | 实现 | **均匀**缩放；非均匀拒绝；`MultiLeader`/`TEXT` 等按类拒绝 |
| `MIRROR` | 部分 | 镜像并**新增副本**（保留原件）；含 `Arc`/`Ellipse`（含椭圆弧）；**拒绝 `Text`**（平面文字无法表达真反射）与 `MultiLeader` |
| `TRIM` | 部分 | 目标 LINE/ARC/CIRCLE/SPLINE/**ELLIPSE（含椭圆弧）**/**LWPOLYLINE（开放/闭合，bulge 保留为精确子弧）**；拾取投影所在的段保留（本项目 TRIM 语义，见 `docs/drawing-edit.md` §3）；边界折线/样条采样；**INSERT/HATCH/MESH/OPAQUE/COMPOUND 目标与 EXTEND 的 POLYLINE 目标按名拒绝** |
| `EXTEND` | 部分 | 目标 **`LINE` 或 `ARC`**（弧延伸上限半圈）；**`SPLINE` 目标显式拒绝**（NURBS 端点移动会改参数化，无外推策略）；`CIRCLE` 目标拒绝 |
| `BREAK` | 部分 | 两点打断：**第一次拾取实体同时是第一断点**，第二点确定移除区间；`LINE`/`ARC`/`CIRCLE`/`SPLINE` 走同一 `TargetCurve` 精确子曲线；`CIRCLE` 变 `ARC`（补集），源实体保留为第一段、第二段新 id，一条事务/一步撤销。**按名拒绝**：`LWPOLYLINE`/其它种类、两点同参数（`break.zero_span`）、整条曲线（`break.whole_curve`）、拾取投影出界（`break.pick_outside_span`）、块成员/子元素、非模型空间。**无 `F` 第一点覆盖、无多段打断** |
| `DIVIDE` | 部分 | **选择式**：恰好 1 条 `LINE`/`ARC`/`CIRCLE`/**直线段** `LWPOLYLINE`，按**等弧长**放置 `POINT`；开放曲线 `N−1` 个、闭合曲线 `N` 个（含起始参数）；`divideSegments` 2–1024；源保留、一条事务/一步撤销。**按名拒绝**：`SPLINE`/椭圆/含 bulge 折线（`divide.bulged_segment`）/非有限或零长/选择数≠1 |
| `LENGTHEN` | 部分 | **选择式**：恰好 1 条 `LINE`/`ARC`；`lengthenMode` `0` Delta / `1` Percent / `2` Total + `lengthenValue`，改**末端**长度（`LINE` 的 end、`ARC` 的 sweep），实体 id/图层/空间/draw order/`type_key` 保留；一条事务/一步撤销。**按名拒绝**：`CIRCLE`/`LWPOLYLINE`/`SPLINE`/其它、非正新长度、ARC 超过一整圈、选择数≠1。**只改末端、无逐端拾取**（声明的 v1 偏离） |
| `SCALETEXT` | 实现 | **选择式**：一个或多个 `TEXT`/`MTEXT`；`scaleTextFactor` 有限 **> 0**，字高乘该值，MTEXT 的 `wrap_width: Some(w)` 同步乘；位置/旋转/对齐/样式逐位保留；混合/非文本整命令拒绝；`1.0` 为具名 no-op；一条事务/一步撤销 |
| `OFFSET` | 部分 | `LINE` / `CIRCLE` / `ARC` / `LWPOLYLINE`（直线段**与含弧段**，后者 2026-10-28）；`ARC` 与 `CIRCLE` 共用 `\|p−c\|` 半径模型。**弧段同心**（`R' = R − s·sign(sweep)`；`R' <= tol` ⇒ `offset.arc_collapsed`，不删除不混合）；接点 = miter（line/line）/ 最近解析交点（line-circle、circle-circle；0 或两个等距 ⇒ `offset.join_failed`），相切闭式；弧 start/sweep 由接点重导并过 `bulge_arc_geometry`（`offset.arc_mismatch`）；闭合环含 wrap 角、开口弧端不封口。**按名拒绝**：`arc_collapsed`/`join_failed`/`ambiguous_plane`/`arc_mismatch`/`bulge_segment`（只剩非有限或不可构造）/`collapsed`/`non_planar` 与 `ELLIPSE`；**不做**弧段删除与邻居混合；**无全局自交修剪**；弧增长上限 `\|new\| <= \|old\| + π` |
| `ARRAYRECT` / `ARRAYPOLAR` | 部分 | **独立副本**（非关联，有意偏离）；无 `Path`/`Levels`/`Total`/角度间隔输入/关联编辑；极坐标角度步长 `i·fill/items` 是**我们的规则**（与 AutoCAD 的 `fill/(items−1)` 不同，已测试） |
| `STRETCH` | 部分 | 恒**交叉窗口**；逐顶点（折线）/控制点（样条）；`TEXT` 与 `MultiLeader` **整命令拒绝**；`HATCH` 部分包围不动 + 诊断；块参照只**刚性平移插入点**、不逐成员拉伸 |
| `ALIGN` | 部分 | **两对点**（平移 + 旋转 + 可选均匀缩放）；无 3 对点 3D 对齐、无非均匀缩放、无「1 对点移动」 |
| `EXPLODE` | 部分 | **一次一层**；`LWPOLYLINE`→线段+`ARC`、`INSERT`→直接成员（嵌套保持引用）、`Hatch`→子项；**拒绝** `Dimension`/`MULTILEADER`/非均匀 `INSERT`；`TEXT` 按子项 `child_refusal`；`SPLINE`/`SHAPE`/`OPAQUE`/镜像 `Arc`·`Ellipse` **整命令失败**；无 `EXPLMODE` |
| `JOIN` | 部分 | **仅**共线首尾相接 `LINE` 或同心相续 `ARC`；折线/样条/圆、需桥接空隙、跨图层/空间 ⇒ 按名拒绝 |
| `ERASE` | 实现 | 空选择拒绝；撤销恢复渲染属性与动态可见性 |
| `CHAMFER` | 部分 | **两线形式**：两条独立直线、两距离按拾取顺序施加；无 `Angle` 方法/`Multiple`/`Trim` 开关。**Polyline 模式是独立选择子** `CHAMFER POLYLINE`（2026-10-28，选择式、恰好 1 条开放直线段 `+Z` LWPOLYLINE；`chamferDistanceA` 沿**入边**、`B` 沿**出边**——**我们的规则**） |
| `FILLET` | 部分 | **两线形式**：两条独立直线 + 半径（`R=0` 只修角点）；平行线**显式拒绝**（不自动调半径）；无 `Multiple`/`Trim` 开关。**Polyline 模式是独立选择子** `FILLET POLYLINE`（2026-10-28，选择式、同上形状；半径取 `filletRadius`、每个内顶点加圆弧；`R ≤ 0` 按名拒绝、**不做** AutoCAD 的自动缩半径） |
| `MATCHPROP` | 部分 | **选择式**（AutoCAD 特性刷的文档化变体：首个选中为源）；只转移 `layer`/`color`/`linetype`/`lineweight`/`transparency` 五项；**无 Settings 子集**；文字样式/标注样式/填充图案一律**不**转移 |

| `SELECTSIMILAR` | 部分 | **v1 仅按几何类型（2026-11-07）**：把可见模型空间中与当前选择**同 `SemanticGeometry` 变体**的拾取项**并入**现有选择（保留顺序，不替换）；复用 `SELECTALL` 的可见目录与预检。**未实现**：layer/颜色/线型/线宽/透明度等属性匹配、图纸空间/块内选择、`SELECTSIMILAR` 的 Settings 子集 |

#### 4.4 块 / 外部参照 / 发布

| AutoCAD 名 | 状态 | 界 |
|---|---|---|
| `INSERT`（**读取**侧） | 实现 | 块定义与实例分离、实例展开几何（嵌套有界，保留 instance path）、可拾取、插入点夹点可拖（仅 `native-below` 宿主） |
| `INSERT`（**创建**侧） | **实现（v1，2026-11-01）** | **点-后-名**：先拾取插入点，再输入**本图已有块**的名称（终端文本通道）；`T(插入点)·T(−块基点)`，type-key `AcDbBlockReference`，活动图层/模型空间，一条事务/一步撤销。**无块表写入**（只读块定义 + 名称/基点元数据）；**无**块选择器/palette、比例/旋转/属性值/动态块参数；未知名称/缺元数据/空名/换行/非有限点按名拒绝。`BLOCK`/`PASTEBLOCK` 已实现 v1；`WBLOCK` 是具名拒绝（未实现单文件写出） |
| `BLOCK` | **实现（v1，2026-11-01）** | **选择 → 基点 → 名称**（终端文本）：选中模型空间顶层实体写入新块定义并移入 `SpaceId::Block`，几何**逐位保留**、只改空间；元数据存基点；参照为 identity `T(基点)·T(−基点)`，后续 `INSERT` 仍按 `T(I)·T(−基点)`；**一条事务/一步撤销**，块级 undo 补丁；参照 draw_order = 成员最大 draw_order、参照在活动图层。**v1 上限**：无 ATTDEF/ATTRIB、动态块参数、retain/delete 选择、块选择器；重名（精确/ASCII 大小写不敏感）拒绝；实例路径/子元素/图纸空间/块成员拒绝；名称非法字符/空/换行拒绝 |
| `WBLOCK` | **部分实现（具名拒绝）** | `wblock.unsupported`：无「写块到文件」；不写库/不升修订/不入历史 |
| `XREF` | **部分实现（只读报告）；只读导入期合并 v1** | `XREF`/`-XREF`/`EXTERNALREFERENCES` 只读报告 `drawing.external_references()` 中的导入期溯源行（`xref.entry`/`xref.summary` 等），`EXTERNALREFERENCESCLOSE` 是具名 no-op（无选项板）。**仍无**附着/卸载/重载（RELOAD）/绑定（BIND）/编辑/xref 编写，也无 External References 选项板。打开图纸时按路径策略读取并合并被参照图纸的模型空间到宿主 xref 块，图层 `xref\|layer`、块 `xref$0$name` 命名空间，状态见 `docs/xref.md`。浏览器无文件系统 ⇒ 显式 `browser-unavailable` 提示，**不 fetch/不猜**；DXF 另存拒绝摊平 xref 内容（`export.xref_content_dropped`），无 xref 记录/路径往返 |
| `IMPORT` | **未实现** | 打开 DWG/DXF 是**打开管线**，不是 `IMPORT` 命令（无 `IMPORT` 选择子） |
| `PUBLISH` | **未实现** | 无多布局批量出图 |

#### 4.5 图层 / 显示 / 开关

| AutoCAD 名 | 状态 | 界 |
|---|---|---|
| `LAYISO` | 实现 | 会话级显隐覆盖；keep = **所选实体图层 ∪ 当前层**（**我们的规则**）；**不写 DWG、不升 revision、不进历史** |
| `LAYOFF` | 实现 | 关闭选择所在图层，当前层除外；同样只是会话覆盖 |
| `LAYON` | 实现 | `RESTORELAYERS` 的**别名**（清空全部会话覆盖），不新增 `CommandId` |
| `LAYERP` | 部分 | 会话内**有界 16 步**快照环（仅真实改动前压栈）；**不持久化**，无 `LAYRSTATE`/`LAYRDO` |
| `GRID` | 实现 | **仅显示栅格**开关（另需 `view.overlays.grid` 偏好为真才绘制）；不移动任何点 |
| `SNAP` | 实现 | 会话级**栅格捕捉**（F9，二维：X/Y 量化，`z` 原样）；默认间距 **10.0 图形单位**是**我们的选择** |
| `OSNAP` | 实现 | 对象捕捉开关；候选生成/最佳候选规则见 `crates/cad-host`（不在本文范围） |
| `ORTHO` | 实现 | 仅 XY 约束；约束顺序 **OSNAP → 栅格捕捉 → 正交**，与 AutoCAD 内部顺序不同（已记录的刻意偏差） |
| `PDMODE` | 部分 | `0`/`1`/`2`/`3`/`4` 已画；`32`/`64`/`96` 周边圆/方**已实现（2026-10-28）**：圆 = 16 段弦环、方 = 4 段，半径 = 标记半幅（比例是我们的选择，**未经 AutoCAD 核对**）；写图纸头、**不可撤销** |
| `PDSIZE` | 实现 | `>0` 绝对尺寸 / `0` 视口比例 / `<0` 视口高度百分比；非有限/超限回退 `0` 行为 + 诊断 |

| `MODEL` | 实现 | **payload-free（2026-11-07）**：等价于 `SPACE MODEL`，复用同一校验 + 自动取景；会话状态 only，无数据库写/历史/revision。Model 标签页现在直接派发 `MODEL`（图纸标签仍派发 `SPACE #<id>`） |
| `PLAN` | 部分 | **Top/plan 2D 视图（2026-11-07）**：payload-free，等价于 `StandardView::Top`；已在 2D 时是具名 no-op，不记 `ZOOM PREVIOUS`。**未实现**：UCS/World/Current 选项（本构建无 UCS 模型） |

#### 4.6 剪贴板 / 视图 / 文件

| AutoCAD 名 | 状态 | 界 |
|---|---|---|
| `COPYCLIP` / `CUTCLIP` / `COPYBASE` / `CUTBASE` / `PASTECLIP` / `PASTEORIG` / `PASTEBLOCK` | 实现（**集群已实现**，`PASTEBLOCK` v1 于 2026-11-01 增补；仍非 OS 剪贴板） | **应用内**剪贴板：**不是** OS 剪贴板、不跨应用、替换文档即清空。**两个捕获形状**：`COPYCLIP`/`CUTCLIP`（`Ctrl+C`/`Ctrl+X`，**不记基点**；`COPYCLIP` 只读、Viewer 可用；`CUTCLIP` work-only）与 `COPYBASE`/`CUTBASE`（`Ctrl+Shift+C`/`Ctrl+Shift+X`，在预选择之上**再抓一个基点**存进剪贴板元数据；`COPYBASE` 只读、Viewer 可用；`CUTBASE` work-only，**先删除成功再提交剪贴板（含基点）**，失败的剪切不毁旧剪贴板）。**两个粘贴形状**：`PASTECLIP`（`Ctrl+V`，**单点插入点工具**，平移 `插入点 − 基点`，`base = clipboard_base.unwrap_or(世界原点)`，复用 COPY 的同一个 `cad_db` 变换——**不再是原坐标粘贴**）与 `PASTEORIG`（原坐标，忽略基点，等价改动前的 `PASTECLIP`）。所有权重走 `commands/accelerators.ts` → `admitCommandText` 的同一准入。**`PASTEBLOCK` 已实现 v1**（2026-11-01，`Ctrl+Shift+V`）：应用内剪贴板 → 自动命名 `PasteBlockN` 块 + 插入点参照；沿用剪贴板基点/世界原点；一条事务/一步撤销；块成员/图纸空间条目按名拒绝。`WBLOCK` 是具名拒绝（未实现单文件写出）。 |
| `ZOOM` | 部分 | `ZOOM <factor>`（正十进制，锚定视口中心）、`ZOOM ALL`/`EXTENTS`（⇒ `FIT`）、`ZOOMIN`/`ZOOMOUT`、**`ZOOM WINDOW`（2026-10-28）**、**`ZOOM PREVIOUS`（2026-10-28）**；**`OBJECT`（2026-11-11）**：当前选择为输入；模型空间用 `drawing_pick_items` + `pick_world_bounds`（块成员/选中 INSERT 展开），图纸空间只接受可直接寻址的布局实体；空选择/无界/3D 视图/跨文档/不可寻址逐条具名拒绝（`zoom.object.*`），成功时用 FIT margin `1.05` 与同一条 rect→camera 规则，记一步 `ZOOM PREVIOUS`。**`nX`（2026-11-11）**：相对倍率，等价于同一 `ZoomAt`（视口中心）。**`nXP`（2026-11-11）**：图纸空间视口比例，`2XP` ⇒ `VPSCALE 1:2`、`1/50XP` ⇒ `VPSCALE 1:50`，实际写入走已有 Work-only `VPSCALE` 路径（一条事务/一步撤销），模型空间按 `zoom.xp.requires_paper` 拒绝。**`DYNAMIC` 仍具名未实现**（`zoom.dynamic_not_implemented`，不伪装）。**`WINDOW`**：两角框选经宿主 box-gesture 路径（逻辑像素、实心带、点击不选择），矩形退化/非有限**不派发**；**仅 2D 正交**（3D 按名拒绝 `zoom.window.requires_2d`），另有 `zoom.window.capture_busy`（其它捕获/模态 pan 占着指针）；拟合用 **margin `1.0`**（FIT 是 `1.05`，**我们的选择**）、同一条「较紧轴」规则，`view_mode` 保持 2D，**不进历史、不 bump `session.generation`**。**`PREVIOUS`**：**每视口**一条上限 **16** 的快照栈（最旧丢弃），在每个**离散**视图变化**之前**记录（FIT/`ALL`/`EXTENTS`（真的取景时）、`ZOOM <factor>`/`ZOOMIN`/`ZOOMOUT`/旧 `Points` 载荷缩放、`ZOOM WINDOW`、`STANDARDVIEW`（2D 下的 plan 视图是 no-op、不记）、`SWITCH2D3D`、`PROJECTION`、`RESETVIEW`）；**pan/滚轮缩放/orbit 拖动不记录**（**声明的偏离**）；空栈 ⇒ 非错误诊断 `zoom.previous.empty`。快照 = camera + view_mode + work_plane。语法只认**全词**（`ZOOM W`/`ZOOM PREV`/`ZOOM O` 拒绝），拒绝文案点名 「positive factor, ALL, EXTENTS, WINDOW, PREVIOUS, OBJECT, nX or nXP；DYNAMIC 未实现」 |
| `PAN` | 实现 | 视图平移（实时） |
| `VIEW` | 部分 | 命名视图 `SAVE/RESTORE/LIST/DELETE <name>`：会话内上限 32（相机 + 视图模式 + 工作平面 + 活动空间），**不持久化 DWG**；预设标准视图（top/bottom/front/back/left/right/isometric）经 IPC 恢复；无视图对话框/缩略图 |
| `ORBIT` | 部分 | `{yaw, pitch}` 载荷，**仅三维视图**；无连续/受约束 orbit 交互 |
| `PROJECTION` | 实现 | 透视/正交切换 |
| `OPEN` | 实现 | 打开 DWG/DXF（宿主提供能力时） |
| `NEW` | 实现 | 新建空图纸；宿主缺能力时显式 `command.unsupported` |
| `SAVE` | 部分 | **有损 DXF 另存**（逐条记录转换/丢弃）；源为 `.dwg` 时可**实验性导出 DWG**（上游 `acadrust::DwgWriter`，新文件、默认 `<stem>-export.dwg`、绝不覆盖源）；DWG 导出有损/不完整，仅合成 self-roundtrip 证据，vendor/AutoCAD **NOT VERIFIED** |
| `PLOT` | 部分 | `PLOT` 是**矢量 SVG/PDF 导出**；无 CTB/STB 打印样式、无批量、无多布局发布；实体能力表里另记的「光栅 PNG（host readback）」是**既有出图通道**，不在本命令的默认路径里 |

| `AUDIT` | 部分 | **只检测（2026-11-07）**：逐实体调用 `cad_db::validate_entity`，输出 `audit.clean` 或 `audit.summary` + 每个 `audit.invalid_entity`；只读，无事务/历史/revision。**未实现**：AutoCAD 的「corrects some errors」修复半部；不修复、不删除、不改写任何实体 |

### 测量选择子（5 个 + `MEASURE` 同名不同义）

| 选择子 | 状态 | 界 |
|---|---|---|
| `DIST` | 实现 | 2 点：距离 + 角度 + Δx/Δy/Δz；单点不足即拒绝。**没有** `DI` 别名，也不是 `MEASURE DISTANCE` 这种多词名字（那个名字不存在） |
| `AREA` | 实现 | **用户选点**闭合多边形面积；自交/非共面/少于 3 点拒绝；**无** `Add`/`Subtract`。**没有** `AA` 别名，也不是 `MEASURE AREA` |
| `ID` | 实现 | 单点拾取 → X/Y/Z + 空间 + 捕捉来源 + 精度；**故意跳过**栅格捕捉与 `ORTHO`（**我们的规则**）；不写库、无撤销步 |
| `MEASUREPOLYLINE` | 实现（**本项目选择器**） | N 点开放链长度，需 `CONFIRM` 收尾 |
| `MEASUREANGLE` | 实现（**本项目选择器**） | 3 点顶点角；纸空间 `ANGLE` 显式拒绝（`ANGLE` 只是**拒绝文案里提到的 AutoCAD 名**，不是本仓库的选择子，也不是别名） |
| `MEASURE` | **同名不同义** | 本仓库是**测量入口**（默认 `DIST`＝距离），不沿对象放置点/块；不构成 AutoCAD `MEASURE` 的覆盖。它是**独立选择子**（`CommandId::Measure`），与上表五个交互选择子不是别名关系 |

> **这张表没有 `（别名 …）` 单元是有意的**：`COMMAND_ALIASES` 里没有任何指向这五个交互选择子或
> `MEASURE` 的别名——`DI`/`AA`/`ANGLE` 都不存在。`scripts/check-command-docs.py` 的自检做两件事：① 本表第一列必须是准入名字；
> ② 全文里每一个被写成别名的反引号 token 都必须真在 `COMMAND_ALIASES` 里。两条都只防「再造名字」，
> 其余语义仍是**人工核对**。

---

### 5. 缺口清单与规模

规模口径（**粗估，不含验证成本**）：**S** ≈ 一个有界补丁（单模块、无新契约面）；
**M** ≈ 一个模块（新命令 + 载荷 + 测试 + i18n + 文档）；**L** ≈ 子系统 / 需要 ADR 决策。

| 缺口 | 规模 | 说明 |
|---|---|---|
| 协作 `TRACE`（trace/标记/评审模型） | **L** | 2026 的 `TRACE` 是协作追踪；本构建同名命令是旧式宽线带，**不计覆盖**，无 trace 模型 |
| `POLYLINE`（旧式 heavy 2D 折线实体）创建 | **M** | 需要写 `AcDb2dPolyline` + 顶点子实体；**不得**把 `POLYLINE` 别名到 `PLINE`（不在官方 981 索引） |
| `XLINE`/`RAY` 真正的无限几何 | **L** | 命令已实现（有界线段）；真正无限表示要新增几何变体，影响 validate/bounds/transform/渲染/选择/导出等约 14 处强制点 + 约 12 处兜底 |
| `WBLOCK` | **L** | `BLOCK`/`PASTEBLOCK` 已有运行时块写入口；缺单文件写出、块定义导出与 `SavePurpose::WBlock` |
| `XREF` 命令面 | **L** | 只读导入期合并 v1 + Wave 5 的 `XREF`/`-XREF`/`EXTERNALREFERENCES` 溯源报告已落地；仍缺附着/卸载/重载/绑定、xref 记录与路径往返、`XCLIP`/完整 `VISRETAIN` 与选项板 |
| `IMPORT` | **L** | 依赖 acadrust 格式能力与迁移策略；打开 DWG/DXF 走 `OPEN`，不是 `IMPORT` |
| `PUBLISH` | **L** | 多布局/多图幅批量出图 + 设备/图幅/打印样式；`PLOT` 只到矢量单页 |
| `DIMSTYLE` 样式表写入与追溯语义 | **M** | `LIST`/`CURRENT` 只读、`SET` 具名拒绝；缺 post-import 样式表写 API 与“改样式是否回写实体”的产品决策（ADR-0017 定为不追溯） |
| 标注 jog / break / 弧长 / 折弯 / 关联 | **M–L** | `DIMJOGLINE`/`DIMBREAK` 无域字段（具名拒绝）；`DIMARC`/`DIMJOGGED`/`DIMREASSOCIATE` 与 `DIMASSOC=2` 未实现 |
| HATCH 自定义 `76=2` / `.pat` 加载 | **M** | 需要 `.pat` 解析器 + 名称解析 + 缺失图案的显式拒绝；当前只有三项内置图案 |
| HATCH 关联性 | **L** | 需要边界→填充持久引用并定义边界编辑后的重生成/失效规则；与现有非关联边界冲突，需 ADR |
| OFFSET `ELLIPSE` | **L** | 等距偏移曲线不是椭圆，需要新偏移曲线类型或显式折线近似 |
| 布局/视口 post-import 写 API | **L** | `LAYOUT` 写形式、`VPORTS`/`VPCLIP`/`VPSCALE`/`LAYVPI` 目前是稳定具名拒绝；`VPMAX`/`VPMIN` 只是会话标记 |
| xdata 模型 | **L** | `XDLIST` 只读并报告 `xdlist.not_retained`；域不保留 xdata |
| DWG group 对象持久化 | **M** | `GROUP` 系列是会话内组表，不写 DWG group、不过撤销栈、不跨文档保留 |
| 命名视图持久化 | **M** | `VIEW` 会话内上限 32，不持久化 DWG |
| 系统剪贴板 | **L** | 现为应用内剪贴板；不跨应用、替换文档即清空 |
| DWG 精确往返 | **L** | `SAVE` 的实验性 DWG 导出有损/不完整（上游 `DwgWriter`），仅合成 self-roundtrip 证据 |
| `VIEWRES`/`VSCURRENT` 渲染器接线 | **S** | 目前只写会话 token；tessellation/visual-style 效果 NOT IMPLEMENTED / NOT pixel-verified |
| `SKETCH` 定时采样、`MSTRETCH` 多边形窗口、`MOCORO` 交互 gizmo | **S** | 当前只有 payload/窗口并集/单次变换路径 |

> 其它已知缺口：`MATCHPROP` Settings 子集、`ARRAY` 的 Path/关联编辑、`STRETCH` 的 HATCH
> 部分拉伸、非均匀缩放与镜像的文字/`MULTILEADER`（域限制，非命令层缺失）。
> 3D/实体/约束/表格/点云/渲染/发布类命令整体不在本清单的 2D 优先范围内。

### 6. 如何复跑这次审计

```bash
# 本项目词表 / 拼写 / 别名的真实数量（源码是唯一权威）
python3 - <<'PY'
import re
src = open('crates/cad-ui-core/src/session/commands.rs').read()
def rows(pat):
    m = re.search(pat, src, re.S)
    return re.findall(r'\("([^"]+)"', m.group(1))
table = rows(r'pub const COMMAND_TABLE: &\[\(&str, CommandId\)\] = &\[(.*?)\n\];')
meas  = rows(r'pub const MEASUREMENT_SELECTORS: &\[\(&str, MeasurementToolKind\)\] = &\[(.*?)\n\];')
zoom  = rows(r'pub const ZOOM_STEP_SELECTORS: &\[\(&str, f64\)\] =\s*&\[(.*?)\];')
args  = re.findall(r'^\s{4}([A-Z_]+_SELECTOR),$',
                   re.search(r'pub const ARGUMENT_SELECTORS: &\[&str\] = &\[(.*?)\n\];', src, re.S).group(1), re.M)
aliases = [a for a, _ in re.findall(r'\("([^"]+)",\s*"([^"]+)"\)',
           re.search(r'pub const COMMAND_ALIASES: &\[\(&str, &str\)\] = &\[(.*?)\n\];', src, re.S).group(1))]
names = list(dict.fromkeys(table + meas + zoom + aliases))
print(len(table), len(meas), len(zoom), len(args), len(aliases), "admissible:", len(names))
PY
# 期望：229 5 2 65 73 admissible: 309

# 钉住选择子数量的契约
cargo test -p cad-ui-core --locked command_normalization_covers_every_variant
cargo test -p cad-ui-core --locked admissible_names_cover_the_vocabulary_exactly_once

# 名字门禁：本文「测量选择子」一节 + docs/shortcuts.md §2/§2b
python3 scripts/check-command-docs.py        # 一致 → 0 / mismatch → 1 / 读不出结构 → 2
python3 scripts/test-check-command-docs.py   # 自测：注入伪造 canonical/别名必须退出 1
```

复跑时**必须**同时读 `docs/compatibility.md` 的「绘制/编辑命令」「创建命令」「编辑命令」
「DIMENSION 族」四节：本节的边界表只是索引，不重述全部拒绝码。

**守门范围**：`check-command-docs.py` 只守**名字**（`shortcuts.md` §2/§2b 与本文测量选择子
一节的准入名/别名提法）。本文边界表的语义（部分/实现的界）仍是**人工按源码核对**，没有机器守卫。

### 7. 本审计**不**主张的事

1. **不**主张任何「AutoCAD 兼容率」或百分比（见 §1）。
2. **不**主张同名命令行为等价：多处已标「我们的规则」「未查证」「有意偏离」
   （极坐标阵列角度步长、`MIRROR` 保留原件、`ARRAYPOLAR` 独立副本、`DIMORDINATE` 自动 X/Y、
   HATCH 预定义图案值为**我们的转录**）。
3. **不**主张真实图纸/真实窗口/真实 GPU 验证；除个别历史有界运行（见各 ADR 的实施补充）外，
   证据是**契约测试与源码结构**。
4. **不**把「未实现」读成「永远不做」：缺口规模只是工作量分类，不是排期承诺。
