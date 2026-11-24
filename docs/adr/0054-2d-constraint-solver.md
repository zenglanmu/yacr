# ADR 0054：二维几何/尺寸约束模型与有界求解器

- 状态：**Accepted（2026-11-24）**。本 ADR **只放开 ADR-0043 D14 的“约束求解器”禁令**，
  限定为二维草图/工程图的几何与尺寸约束；三维装配/配合求解、全局优先级表、特征历史 rollback 仍是
  非目标。实现尚未开始，命令状态保持 `部分实现`（具名拒绝）直到对应实现与契约测试落地。
- 关联：[ADR-0043](0043-parametric-dependency.md)（参数/表达式/依赖事务；本 ADR 修订其 D14）、
  [ADR-0037](0037-ymodel-schema-and-persistence.md)（`.ymodel` 版本与表）、
  [ADR-0031](0031-acadrust-vendored-patch-policy.md)（acadrust 补丁台账）、
  [ADR-0036](0036-3d-body-model-and-solid-service.md)（solidcore 内核 seam）、
  [ADR-0053](0053-application-domains.md)（应用域与参数化面板摆放）。
- 影响：`cad-domain`（约束图、残差/Jacobian、表达式）、`cad-db`（约束表、快照、事务/历史）、
  `cad-store-zip`（新表与 minor）、`cad-app`（约束命令、求解前校验、ChangeScope）、
  `cad-ui-core`/`apps/desktop-ui`（Parametric 面板、约束栏/尺寸编辑）、
  `cad-connector-acadrust`（DXF/DWG 约束节点映射）。

---

## 背景

ADR-0043 D14 当时写的是“不做约束求解器”。现在两个前提变化了：

1. **solidcore 提供了可用的数值原语**：`solidcore::geom::local_solve_nd` 是通用 N 维
   Newton-Raphson 求解器（`EvalNFunctions`、有限差分 Jacobian、步长裁剪、区间/边界处理、
   收敛原因），`geom::matrix`/`compute_jacobian` 可复用；`geom::global_solver` 是曲线/曲面求交与
   极值的 branch-and-bound，**不是**草图约束求解器。solidcore 没有约束图、约束类型、残差装配、
   欠/过约束判定或 UI。
2. **acadrust 0.6.8 已能往返约束对象**：`AcDbAssoc2dConstraintGroup` 与
   `AssocConstraintNodeData::{Angle, Parallel, Distance, RadiusDiameter, Geometrical, Composite}`
   在 `objects/associative.rs` 与 DXF/DWG 读写路径中存在，DXF/DWG 持久化不再是完全空白。

但 solidcore **不是**现成的约束求解器，且它只在 native 可用（`cad-kernel-solidcore` 是
`cfg(not(target_arch = "wasm32"))`）。所以本 ADR 放开的是“可以自己建一个**载体无关**的有界二维
约束求解器”，不是“直接用 solidcore 的约束求解器”。

## 决策

### D1 — 支持域：二维草图/工程图约束

支持：

- 几何约束：重合（coincident）、共线（collinear）、同心（concentric）、固定（fix）、平行
  （parallel）、垂直（perpendicular）、水平（horizontal）、竖直（vertical）、相切（tangent）、
  平滑（smooth/G1）、对称（symmetric）、相等（equal）。
- 尺寸约束：对齐距离、水平/竖直距离、角度、半径、直径；驱动值与参考值；驱动值可使用
  ADR-0043 已实现的封闭表达式语法。
- 约束对象是已有二维实体（`LINE`/`ARC`/`CIRCLE`/`ELLIPSE`/`LWPOLYLINE`/`SPLINE`/`POINT` 的
  已建模子元素），不引入新的“草图实体”类型。
- 自动约束（`AUTOCONSTRAIN`）只执行声明的规则子集（水平/竖直/重合/共线/同心/相等），不承诺
  AutoCAD 的完整推断器。

不支持（保持具名拒绝或全局范围）：

- 三维装配/配合（mates）、三维几何约束、装配自由度分析。
- 全局优先级表、跨文档约束、特征历史 rollback/freeze。
- 对未建模子元素（例如 `INSERT` 内部成员、未导出的曲线类型）的约束；必须按名拒绝。
- AutoCAD 的完整 `GEOMCONSTRAINT` 选项顺序/提示文案；交互提示属于 ADR-0043 的全局范围声明。

### D2 — 模型归 `cad-domain`，不归 solidcore

新建载体无关的约束模型（建议模块 `cad-domain/src/constraint/`）：

- `ConstraintGraph`：约束节点、被约束的实体/子元素引用、驱动/参考标记、工作平面、状态
  （under/over/conflict/solved）。
- `ConstraintKind`：D1 的封闭枚举；每个 kind 有稳定的 token 和残差函数。
- `ConstraintVariable`：二维点/标量参数；驱动尺寸引用 ADR-0043 的参数/表达式求值结果，不另造
  第二套表达式引擎。
- 残差/Jacobian 是纯函数：无 DB、无 GPU、无 IO、无时间；输入是变量向量和约束行，输出是
  `F(x)` 与 `J(x)`。

`cad-db` 继续是唯一权威：约束表存**约束关系**，几何仍存实体；求解结果是实体几何的新值，经现有
`apply_drawing_changes` 路径写回，不存“求解器内部状态”。

### D3 — 求解器：纯 Rust、载体无关，solidcore 只作可选数值后端

- 默认实现放在载体无关的 crate（`cad-domain` 或新的 `cad-constraint`），使用
  Gauss-Newton/Levenberg-Marquardt 最小二乘 + 秩/条件数诊断：
  - 欠约束：按最小范数步或“保持被拖动点”的策略求解，并报告剩余自由度；
  - 过约束/冲突：报告冲突约束集合，**不写入**任何几何；
  - 奇异/不收敛：稳定拒绝，不猜测。
- `solidcore::geom::local_solve_nd` 只作为 **native 后端** 的加速/对照实现，经
  `cad-kernel-solidcore` 的窄 trait 暴露；其类型不得泄漏到 `cad-domain`/`cad-app`，也不得成为
  Web/X11 `dom-canvas` 的唯一路径。Web/wasm 必须走纯 Rust 路径，否则按
  `unsupported-runtime: constraints` 显式拒绝（但这是最后手段，不是默认设计）。
- 求解前校验、求解后一次事务/一次 revision/一步撤销；失败/冲突时不写库、不升修订、不入历史、
  不发布 `ChangeSet`。

### D4 — 持久化：新约束表 + `FORMAT_MINOR` 16

- 在 `.ymodel` 新增 `tables/constraints.json`（约束行：id、kind、实体/子元素引用、工作平面、
  驱动/参考、表达式、状态），不把约束塞进 `params` 或 `bim` 表。
- 块属性（ADR-0055）取 minor 14；`Text.mirrored` 取 15；约束取 **16**。若实际落地顺序改变，取下一个可用号，
  但必须在 `cad-store-zip/src/minor.rs` 的分档链和 `AGENTS.md` 的 minor 链中显式登记。
- 旧版本读新档按 `ymodel.unsupported_minor` 拒绝；新版本读旧档 = 无约束表/空表。求解器派生值
  不落盘；只存约束与驱动表达式。

### D5 — DXF/DWG：acadrust 约束节点映射，不静默丢

- 导入：把 acadrust 的 `AcDbAssoc2dConstraintGroup`/`AssocConstraintNodeData` 映射到 D1 支持的
  约束；不支持的节点/约束按名报告（`import.constraint.*`），不猜测。
- 导出：只写 D1 支持的约束；每个未写出的约束按 kind 报告（`export.constraint.*`），不得用
  几何近似冒充约束。
- 需要 acadrust 补丁时，只允许 `acadrust`（ADR-0031），必须固定版本/rev 并登记
  `docs/acadrust-patches.md`；当前不预设补丁，先以 0.6.8 的现有节点能否往返为准。
- 约束栏/约束图标是显示层，不属于 DXF 数据；DXF 只读写约束对象本身。

### D6 — 命令与状态

- 本 ADR 放开的命令集合：`GEOMCONSTRAINT`、`DIMCONSTRAINT`、10 个 `DC*`（`DCALIGNED`/
  `DCANGULAR`/`DCCONVERT`/`DCDIAMETER`/`DCDISPLAY`/`DCFORM`/`DCHORIZONTAL`/`DCLINEAR`/
  `DCRADIUS`/`DCVERTICAL`）、`DELCONSTRAINT`、`AUTOCONSTRAIN`、`CONSTRAINTBAR`、
  `CONSTRAINTSETTINGS`，以及 `PARAMETERS`/`-PARAMETERS`/`PARAMETERSCLOSE` 的**约束参数**
  入口（参数管理器本身按 ADR-0043 P0 的用户参数表实现）。
- 只有实现 + 契约测试（含欠/过约束、冲突不写、一次事务/一步撤销）落地后，才把对应命令从
  `部分实现` 改为 `完整实现`。崩溃、空成功、静默丢约束一律禁止。
- `DCCONVERT` 只转换已建模的关联尺寸；无关联模型时保持具名拒绝，不算完成。

### D7 — UI：Parametric 面板与约束栏

- Parametric 三个面板（几何/标注/管理）只在命令真实可用后接线；之前保持 `pending`，
  不摆可点假按钮（ADR-0053 D5/D8）。
- 约束创建、编辑、删除、显示/隐藏走真实命令；约束栏/图标、按住拖动、尺寸编辑是显示/交互层，
  必须有稳定拒绝或明确的支持域，不得伪造。
- `cad-ui-core` 的 wire/read seam 继续是唯一通道；新增读取 kind 不得新增每载体文件。

### D8 — 证据边界

- 证据以契约测试为主：模型/残差/Jacobian、欠/过约束、冲突不写、事务/撤销、DXF 往返、
  Web/wasm 与 native 两条路径。
- 真实 DWG、真实 GPU/像素、Windows/macOS/Android/Web 运行时结论必须显式标注；
  未运行写 **NOT RUN**，不得把合成测试推广为 vendor 等价或产品验收。

## 非目标

- 三维装配/配合求解、三维几何约束、全局优先级表。
- 完整 AutoCAD 约束推断器/选项顺序/提示文案。
- 把求解器结果直接写 GPU 或绕过 `cad-db`/事务。
- 在没有真实文件/契约证据时声称 DXF/DWG 约束往返“完整”。

## 实施分期

1. **P0 模型/求解器**：`cad-domain` 约束图 + 残差/Jacobian + 纯 Rust 最小二乘；3–5 类约束
   + 尺寸；欠/过约束诊断。无 UI。
2. **P1 命令/事务**：`GEOMCONSTRAINT`/`DIMCONSTRAINT`/`DELCONSTRAINT`/`DC*` 的 typed 路径、
   参数表达式、一次事务/一步撤销；`CONSTRAINTBAR`/`CONSTRAINTSETTINGS` 会话路径。
3. **P2 持久化/DXF**：`tables/constraints.json`、minor 16、acadrust 节点映射、导入/导出拒绝码。
4. **P3 UI**：Parametric 面板、约束栏/图标、选择与编辑；`autocad-structure` 只移除真实完成的
   pending 面板。
5. **P4 自动约束**：声明的规则子集；未覆盖规则按名报告。

## 后果

- ADR-0043 D14 的“不做约束求解器”被本 ADR 收窄：二维约束求解器现在在范围内；三维装配/配合、
  全局优先级、特征历史仍不做。
- 预计可诚实晋升 12–17 个命令名，但都是 L–XL，且必须先有模型/求解器；不应作为“quick win”
  计入第一批 20–30 条里程碑。
- solidcore 的 `local_solve_nd` 是可选数值后端，不是约束模型；acadrust 的约束节点是持久化
  载体，不是求解器。
