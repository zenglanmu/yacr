# AutoCAD 2026 下一批 100 条命令实施计划（2026-11-07）

> **进度（2026-11-07）**：累计 **99/100 条新增命令已合并并验证**（文字 10、图层 10、选择 10、
> 视图 10、实用工具 10、绘制/修改 10、布局/视口 10、标注 10、组/注释 10、绘制/视图补充 9）。
> 第 100 条 `TRACE` 已落地但**同名不同义**（AutoCAD 2026 是协作追踪，本构建是旧式宽线带），
> **不计入覆盖**。官方清单当前 **222 已实现（50 完整 + 172 部分）/ 759 未实现**。

> **这是排期与范围声明，不是验收。** 每条命令落地时必须在
> `docs/autocad-2026-command-coverage.md` 与 `docs/compatibility.md` 写明真实边界；
> “已实现/部分实现”只表示命令路径存在且有契约测试，不表示与 AutoCAD 行为等价，
> 也没有 GUI/GPU/像素/真机/vendor 证据。

## 纪律

- 2D 优先；3D/实体/约束/表格/外部参照/点云/渲染/发布类命令不在本 100 条内。
- 每条命令：Rust 命令路径 + `COMMAND_TABLE` + dispatch + 能力门 + React 目录/gating/i18n
  + 契约测试；未支持选项一律具名拒绝。
- 每 wave 独立 worktree/分支，隔离 `CARGO_TARGET_DIR`；合并后由 orchestrator 统一更新
  `docs/autocad-2026-command-coverage.md`、`docs/compatibility.md`、`docs/handoff.md`。
- 共享单表（`CommandId`、`COMMAND_TABLE`、dispatch、UI catalog、i18n）冲突由 orchestrator
  按显式路径合并；合并后重跑 cad-app/cad-ui-core/前端全量/门禁。
- 本轮所有结论仅 Linux 本机编译/契约；Windows/macOS/Android/Web 运行时 NOT VERIFIED。

## Wave 1（已启动，30 条）

### Lane layer-batch（10）
`LAYMCH`、`-LAYMCH`、`LAYMRG`、`-LAYMRG`、`LAYDEL`、`-LAYDEL`、`LAYERSTATE`、
`LAYERPMODE`、`LTSCALE`、`LAYWALK`

### Lane select-batch（10）
`QSELECT`、`FILTER`、`GETSEL`、`SSX`、`SELECTCOUNT`、`-FIND`、`FIND`、
`HIDEOBJECTS`、`ISOLATEOBJECTS`、`UNISOLATEOBJECTS`

### Lane text-batch（10）
`JUSTIFYTEXT`、`TJUST`、`TSCALE`、`TEXTALIGN`、`TCOUNT`、`TCIRCLE`、`TEXTMASK`、
`TEXTUNMASK`、`MTEDIT`、`-MTEXT`

## Wave 2（标注，10）
`DIMARC`、`DIMJOGGED`、`DIMTEDIT`、`DIMEDIT`、`DIMOVERRIDE`、`DIMBREAK`、
`DIMREGEN`、`DIMDISASSOCIATE`、`DIMREASSOCIATE`、`QDIM`

## Wave 3（视图/空间，10）
`REDRAW`、`REDRAWALL`、`REGEN`、`REGENALL`、`VIEW`、`-VIEW`、`VIEWGO`、
`VIEWBACK`、`VIEWFORWARD`、`VPOINT`

## Wave 4（绘制/修改补充，10）
`DONUT`、`SOLID`、`HELIX`、`MLINE`、`MLSTYLE`、`TREX`、`EXTRIM`、`MPEDIT`、
`-OVERKILL`、`DRAWORDER`

## Wave 5（查询/命名对象，10）
`PURGE`、`-PURGE`、`RENAME`、`-RENAME`、`SETVAR`、`TIME`、`WHOHAS`、`DBLIST`、
`TREESTAT`、`MREDO`

## Wave 6（块/属性，10）
`-BLOCK`、`BMAKE`、`WBLOCK`、`-WBLOCK`、`-INSERT`、`MINSERT`、`ATTDEF`、`ATTEDIT`、
`-ATTEDIT`、`ATTSYNC`

## Wave 7（填充/引线/注释对象，10）
`-HATCH`、`-HATCHEDIT`、`HATCHSETBOUNDARY`、`HATCHSETORIGIN`、`QLEADER`、`LEADER`、
`TOLERANCE`、`FIELD`、`RTEXT`、`RTEDIT`

## Wave 8（标注样式/图纸，10）
`DIMSTYLE`、`-DIMSTYLE`、`DIMJOGLINE`、`DIMINSPECT`、`-DIMINSPECT`、`DIMEX`、
`DIMIM`、`SPACETRANS`、`MVIEW`、`MVSETUP`

## Wave 9（视口/布局，10）
`LAYVPI`、`LAYOUT`、`LAYOUTMERGE`、`LAYOUTWIZARD`、`VPORTS`、`-VPORTS`、`VPMAX`、
`VPMIN`、`VPCLIP`、`VPSCALE`

## Wave 10（会话/脚本，10）
`SAVEALL`、`QNEW`、`REVERT`、`SCRIPT`、`RESUME`、`RSCRIPT`、`DELAY`、`MULTIPLE`、
`LOGFILEON`、`LOGFILEOFF`

## 备选（若某条被证明不可诚实实现，用同域备选替换）

- 文字：`TEXTFIT`、`TXTEXP`（cad-app 无 `FontEngine`，实现前必须先解决字体访问）
- 选择：`GROUP`、`-GROUP`、`UNGROUP`、`GROUPEDIT`（需新建组表）
- 图层：`LINETYPE`、`-LINETYPE`（需 post-import 线型表写 API）
- 修改：`TXT2MTXT`（已实现）、`HATCHTOBACK`（已实现）
- 查询：`XDLIST`、`XDATA`（需 xdata 模型）
