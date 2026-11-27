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

## 2. 汇总（2026-11-26）

| 状态 | 数量 | 占比 |
|---|---:|---:|
| `full` | 52 | **75.4%** |
| `partial` | 4 | 5.8% |
| `pending` | 13 | 18.8% |
| 合计 | 69 | 100% |

- **50% 线已满足**（52/69 = 75.4%）。
- `full` 的主体仍是二维制图/编辑/导航/文件命令；SketchUp 特有的网格核心已完成布尔/修复：
  `UNION`/`SUBTRACT`/`INTERSECT` 现在同时接受精确 `Body3d{Solid,Exact}` 与闭合网格
  （混合选择按 `boolean.operand_class_mismatch` 拒绝），`MESHREPAIR`/`MESHREBUILD` 修复闭合网格；
  结果仍是**非精确**网格。`PRESSPULL` 现在同时支持精确面与带 `face_sources` 的网格面；
  `MOVE` 面选择、`MESHREFINE` 细分、`MESHSMOOTH` 平滑、`MESHCAP` 边界环补洞（含共面嵌套环带）
  已接线。
  边身份使用几何派生 `mesh:edge:<bits>` key，`MOVE` 可移动边；wire-only 边/跨多个三角形边的
  stored polyline 仍不可编辑，因此本表的“75.4%”不是“SketchUp 已复刻七成半”的营销结论。
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
| Tape Measure | `DIST` | partial | 只有距离；没有 guide line/点 |
| Paint Bucket | `MATERIALS` | pending | 没有材质/纹理模型（`MATERIALS` 命令是具名拒绝） |
| Orbit | `ORBIT` | full | |
| Pan | `PAN` | full | |
| Zoom Extents | `FIT` | full | |

### Drawing（8；full 5 / pending 3）

| 工具 | token | 状态 | 边界 |
|---|---|---|---|
| Line | `LINE` | full | 同上 |
| Freehand | `FREEHAND` | pending | 没有自由手绘命令（`REVCLOUD` 的 Freehand 是另一语义） |
| Rectangle | `RECTANG` | full | |
| Rotated Rectangle | `RECTANGROTATED` | pending | 没有旋转矩形构造 |
| Circle | `CIRCLE` | full | |
| Polygon | `POLYGON` | full | |
| Arc | `ARC` | full | |
| Pie | `PIE` | pending | 没有扇形构造 |

### Edit（7；full 4 / partial 2 / pending 1）

| 工具 | token | 状态 | 边界 |
|---|---|---|---|
| Move | `MOVE` | full | |
| Push/Pull | `PRESSPULL` | full | 精确面或带 `face_sources` 的网格面；无面身份拒绝；边移动由 `MOVE` 处理 |
| Rotate | `ROTATE` | full | |
| Follow Me | `FOLLOWME` | pending | 没有沿路径扫掠 |
| Scale | `SCALE` | full | |
| Mirror/Flip | `MIRROR` | partial | CAD 镜像，不是 Flip |
| Offset | `OFFSET` | full | 二维 |

### Construction（6；full 3 / partial 1 / pending 2）

| 工具 | token | 状态 | 边界 |
|---|---|---|---|
| Tape Measure | `DIST` | partial | 距离 only，无 guide |
| Dimension | `DIMLINEAR` | full | 是 CAD 标注，不是 SketchUp LayOut 尺寸 |
| Protractor | `PROTRACTOR` | pending | 没有角度构造/guide |
| Text | `TEXT` | full | 二维文字 |
| Axes | `UCS` | full | 会话坐标轴 |
| 3D Text | `TEXT3D` | pending | 没有三维文字网格 |

### Camera（9；full 6 / pending 3）

| 工具 | token | 状态 | 边界 |
|---|---|---|---|
| Orbit | `ORBIT` | full | |
| Pan | `PAN` | full | |
| Zoom In | `ZOOMIN` | full | |
| Zoom Out | `ZOOMOUT` | full | |
| Zoom Extents | `FIT` | full | |
| Previous | `VIEWBACK` | full | |
| Position Camera | `CAMERA` | pending | 只有 `cad_set_view` 的 typed 视图，没有相机放置工具 |
| Walk | `WALK` | pending | `3DWALK` 是具名拒绝 |
| Look Around | `LOOKAROUND` | pending | 没有独立视线工具 |

### Views（7；full 7）

七个标准视图（top/bottom/front/back/left/right/isometric）由 `STANDARD_VIEWS` 与
`cad_set_view` 承担。

### Solid Tools（6；full 3 / pending 3）

| 工具 | token | 状态 | 边界 |
|---|---|---|---|
| Union | `UNION` | full | 精确 `Body3d{Solid,Exact}` 或闭合网格；混合选择拒绝；网格结果非精确 |
| Subtract | `SUBTRACT` | full | 同上 |
| Intersect | `INTERSECT` | full | 同上 |
| Outer Shell | `OUTERSHELL` | pending | 没有“只留外表面”的网格操作 |
| Trim | `TRIMSOLID` | pending | 没有保留刀具的裁剪 |
| Split | `SPLITSOLID` | pending | 没有双结果分割 |

### Standard（11；full 11）

New/Open/Save/Cut/Copy/Paste/Erase/Undo/Redo/Print/Toolbars 由既有文件与编辑命令承担；
它们不是 SketchUp 特有，但官方工具栏包含这些单元。

## 4. Default Tray 面板（不计入 69 单元）

| 面板 | 状态 | 边界 |
|---|---|---|
| Entity Info | full | 属性行来自 `PropertyRow`；网格行有限 |
| Components | partial | 列出块定义；创建/插入 Component 的完整语义未接线 |
| Styles | partial | 只有主题切换，不是 SketchUp Style |
| Tags | full | 走图层面板 |
| Instructor | full | 工具提示/步骤 |
| Materials | pending | 无材质/纹理模型 |
| Scenes | pending | 没有场景/页面持久化 |
| Shadows | pending | 无阴影设置 |
| Fog | pending | 无雾 |
| Match Photo | pending | 无照片匹配 |
| Soften Edges | pending | 无边软化 |
| Overlays | pending | 无 overlay 系统 |
| Outliner | pending | 无层级树编辑 |

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
- 其余 `pending` 工具（Freehand/Rotated Rectangle/Pie/Follow Me/Protractor/3D Text/
  Outer Shell/Trim/Split/Camera/Walk/Look Around/Materials/Scenes/...）保持显式 pending；
  任何一项实现都要同时改本表、改 UI（从 `pending` 改为命令）并补契约测试。
