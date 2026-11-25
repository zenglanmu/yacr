# 原子 API 与 Measurements 路线（lane 2 / lane 3）

> 目标：继续复刻概念建模的高层交互，但把**语义留在 Rust 原子操作**，由 React 组装工具与界面；
> 第三方插件通过同一组 IPC 操作复用它。本文只记录计划与验收口径，完成后把结果写进
> `docs/handoff-archive/` 与相应契约文档。
>
> 基线：`@yacr/plugin-api` 的 `api.ui.register` 已发布（lane 1）。证据边界见
> `docs/application-domains.md`；真实窗口/GPU/像素/非 Linux 载体仍 **NOT VERIFIED**。

## 现状

| 层 | 已有 | 缺口 |
|---|---|---|
| Rust 原子写入 | `ApiRequest::{Commit,SelectByRefs,SelectByObjectIds}`；`EntityEdit::{Insert,Translate,Delete}`；`NewEntity::{Line,Circle,Arc}` | 只能建三种二维实体，不能建多段线/文字/网格/块参照；选择操作未通过插件 IPC 暴露 |
| Rust 原子读取 | 文档/revision/符号表/实体/当前选择 | 无图层逐项读取（前端走 `cad_layers`），无场景/材质/相机 |
| 命令漏斗 | `cad_command` 文本命令，React 与插件命令都过共享准入 | 插件不能执行宿主命令；`PRESSPULL`/`FOLLOWME` 等本身未实现 |
| 插件 UI | `api.ui.register` 声明式 menu/toolbar/tray | 无自定义托盘 body；无插件 locale 切换 |
| Measurements | `cad_status.tool.params` + React 输入控件 | 七个核心工具（LINE/RECTANG/CIRCLE/ROTATE/MOVE/SCALE/OFFSET）已接线；其余工具的数值输入仍是后续 |

## lane 2：扩大 Rust 原子写 API

原则：每个新增操作都必须是**单一、可验证的原子语义**，不把 SketchUp 工具流程塞进 Rust；
React/插件负责把多个原子操作组合成工具。

### P1 实体创建闭集扩展（先做）

- 新增 `NewEntity::Polyline { points: Vec<Point3>, closed: bool }`（至少 2 点；闭合需 ≥3 点）。
- 新增 `NewEntity::Text { origin: Point3, height: f64, rotation: f64, value: String }`
  （height > 0、value 非空；rotation 弧度）。
- `cad-app::api` 的 `validate_new_entity` / `new_geometry` 同步；`EntityEdit::Insert` 的 JSON
  解码（`crates/cad-ui-core/src/session/api_commit.rs`）接受 `"Polyline"` / `"Text"`。
- `packages/cad-plugin-api` 的 `SUPPORTED_NEW_ENTITY_TYPES` 与 `TransactionEdit` 联合
  增加 `Polyline` / `DBText`，文档与契约测试同步。
- 不新增描述符行：`transaction.commit` 的载荷闭集扩展，仍是同一操作。
- 验证：`cargo test -p cad-app api::`、`cargo test -p cad-ui-core session::`、
  插件包 `test/transaction.test.ts`、`test/contract.test.ts`。

### P2 选择原子操作

- 新增 `EditorInput.editor.setSelection`（mutate lane）：参数 `{ objectIds: string[] }`，
  宿主映射到 `ApiRequest::SelectByObjectIds`（`SelectionMode::Replace`）。
- 描述符 JSON 加一行 `supported`；`cad-ui-core` 的 submit 解码与 `cad-app` 执行分支同步。
- 用途：插件的“查找/过滤/批量编辑”工具可以把结果交回用户选择，而不是伪造 UI 高亮。
- 显式拒绝：空数组是合法“清空选择”；未知 id 由宿主以 `cad_api.unknown_object: <id>` 拒绝。

### P3 结构化命令执行（评估后决定）

- 候选操作 `EditorInput.editor.execute`：参数 `{ command: string }`，仅允许**非交互**命令；
  交互命令（需要拾取/文本步）以 `api.pending: EditorInput.editor.execute.interactive` 拒绝。
- 风险：命令完成语义、undo 分组、错误传播、与 `cad_command` 文本漏斗的重复。先做 spike，
  不直接接线；若无法给出“完成/失败”的诚实答案，则不做。

### P4 概念域内核（spike 已完成，seam 原子操作已加）

2026-11-25 在固定 pin `55ae49ef919b` 上做了真实内核 spike（`crates/cad-kernel-solidcore/tests/
presspull_spike.rs`，两个用例都通过；体积由内核积分）：

- **推拉**可表达：`prism_section(body, plane)` 给出精确截面区域，`extrude_planar_regions` 拉出
  1000 + 3×100 = 1300，`boolean(Subtract)` 压入 1000 − 3×100 = 700，两者都过内核有效性检查。
- **跟随**可表达：`sweep_stations(profile: &[sweep::Curve], stations, Ends::Capped)` 用四条直线
  轮廓扫出 16 的实体；`sweep_placed`（线/共轴圆弧路径）是命令层的候选，但它只认**已证明**的
  路径形状，不能冒充 SketchUp 的任意路径。

结论：`solidcore` **已经**有可用的面移动/拉伸/扫掠入口，此前“没有 extrude”的判断是错的；
真正的缺口在 seam 和载体层，不在内核。因此本轮加了**最小的原子操作**：

- `SolidOperation::OffsetFace { brep, selector: SolidFaceSelector, distance }`：
  `distance` 带符号（正 = 沿外法向拉出、负 = 压入），0 由 `validate` 拒绝；
  选择器是 `SolidFaceSelector::NearestToPoint`，按**裁剪面**（`Body::point_on_face`）解析，
  容差与倒角边选择一致（`max(1e-9, 1e-3 × 包围盒对角线)`）；
  实现为 `cad-kernel-solidcore::ops::offset_face` → `solidcore_ops::offset::offset_faces`，
  平面面与曲面面都可移动（曲面面实测半径 1 → 1.5）。
- 能力位 `ModelerCapabilities::offset_face`（solidcore 声明 `true` 并登记 `offset.face`）；
  拒绝码 `solid.face_not_found` / `solid.offset_face_failed`，未声明能力仍是
  `solid.unsupported`，曲面操作数仍受 `nurbs` 闸门约束。

**2026-11-25 命令接线完成**：`PRESSPULL` 已从具名拒绝变成真实的单面移动工具——
`CommandPayload::PressPullFace` + `DrawToolKind::PressPull` 一次面拾取 +
`ToolParameter::PressPullDistance`（带符号，Measurements 输入）+ 原地改写（同一 id、一次
revision、一步撤销）；宿主在工具启动时校验 modeler 与“恰好一个精确实体”，拾取必须是所选 body
的面（否则 `presspull.pick_not_selected_body`）。SketchUp 大工具集的推拉按钮已从 `pending`
改为真实命令，并加了专用 `pushPull` 图标。

**2026-11-25 EXTRUDE 接线**：`SolidOperation::ExtrudeProfile` + `EXTRUDE` 零端点预选择工具把
所选闭合平面轮廓（LINE 闭环 / 闭合直线 LWPOLYLINE / 单个 CIRCLE，平行于 XY 平面）拉成**新的**
精确 `AcDb3dSolid`，距离走 `extrudeDistance`（带符号）。轮廓保留原位，一条事务/一步撤销。
这补齐了「画形状 → 拉起」的主流程；`PRESSPULL` 继续负责已有实体的面移动。

**仍未接线**：拖拽驱动幅度（只有 typed 距离）、多面/连续推拉、面高亮/预览、挖孔/多环轮廓、
拔模、任意 3D 平面、部分圆弧壁（中性读回限制）、REVOLVE/LOFT/SWEEP。`FOLLOWME` 仍是具名拒绝：
`SPEC` 只证明内核 `sweep_stations`/`sweep_placed` 可用，还需要“面 + 3D 路径”的文档模型，
不能靠 2D `LINE`/`ARC` 冒充。

## lane 3：SketchUp Measurements 数值输入

原则：Rust 侧的 tool capture 只接受**已校验的会话参数**；React 的 Measurements 框是这些参数的
唯一输入面。没有参数的当前步骤只显示真实提示，不伪造输入框。

### P1 LINE 长度/角度（先做）

- `ToolParameter` 新增 `LineLength` / `LineAngle`（机器键 `lineLength` / `lineAngle`）；
  `LineLength > 0`，`LineAngle` 任意有限角度（度，内部转弧度）。
- `cad_status.tool.params` 增加 `lineLength` / `lineAngle`，仅在 LINE capture 活跃时投影。
- `cad-app` 的 LINE capture 在第二点解析时优先使用参数：
  - 只有 length：方向取当前光标方向（零长度方向则拒绝 `line.direction`）；
  - length + angle：终点 = 起点 + length·(cosθ, sinθ, 0)；
  - 只有 angle：仍按光标距离，方向替换为角度。
- React `tool-parameter-specs.ts` 增加两个数值控件；`StatusBar` 的 concept Measurements 自动
  渲染，无需第二套 UI。
- 验证：`cargo test -p cad-app draw_tool`、`cargo test -p cad-ui-core status`、
  `apps/desktop-ui` 的 tool-parameter 契约测试。

### P2 其余核心工具

按同一形状逐个落地：已完成 `RECTANG`（宽/高）、`CIRCLE`（半径）、`ROTATE`（角度）、
`MOVE`（距离）、`SCALE`（比例）、`OFFSET`（距离）。OFFSET 的“实体级偏移距离”原子操作
**本来就存在**（`CommandPayload::OffsetOptions` + `OffsetMode::Distance`，由 typed
`OFFSET <distance>` 命令写入 session 默认值），此前把它误判为缺口；缺的只是 Measurements
通道。现在 `offsetDistance > 0` 时侧点只选边、距离来自 payload；`0` 回退到 session 的
`OFFSET <distance>` 或 `Through`。每个参数都要有：范围校验、仅在该工具活跃时投影、
capture 解析、React spec、契约测试。不得一次性把未实现参数摆成可输入控件。

## 实施进度

- **2026-11-25 lane 1 完成**：`api.ui.register` 公共面（见 handoff）。
- **2026-11-25 lane 2 P1 完成**：`NewEntity::{Polyline, Text}`（wire typeName
  `Polyline`/`DBText`），`EntityEdit::Insert` 的校验与几何映射、wire 解码、
  `SUPPORTED_NEW_ENTITY_TYPES` 与测试全部同步。证据：`cad-app api::tests`、`cad-ui-core
  tests_api_submit`、插件包 `transaction.test.ts`。
- **2026-11-25 lane 2 P2 完成**：`EditorInput.editor.setSelection`（描述符第 18 行）走
  submit/poll lane，宿主 `HostController::plugin_select_ids` 先校验 drawing/active space 再
  应用；空数组清空，未知 id 为 `cad_api.unknown_object: <id>`，跨空间 id 为
  `api.invalid_argument: objectIds.space: <id>`，答案 `{ selected: <count> }`。证据：
  `cad-ui-core tests_api_submit`、包 `test/prompts.test.ts`、`plugin_api_status`。
- **2026-11-25 lane 3 P1 完成**：`ToolParameter::{LineLength, LineAngle, RectWidth,
  RectHeight, CircleRadius, RotateAngle, MoveDistance, ScaleFactor}`（`lineLength` /
  `lineAngle` / `rectWidth` / `rectHeight` / `circleRadius` / `rotateAngle` /
  `moveDistance` / `scaleFactor`），仅对应 capture 投影；session 在
  `draw_intent` 与 preview 里组装 typed geometry。`0` 是八者的 unset sentinel；`lineAngle`
  仅在 `lineLength > 0` 时生效，因此暂不能通过该通道强制 0° 方向（已在提示文案里声明）。
  RECTANG 的 typed extent 保留光标方向的符号；CIRCLE 与 MOVE 的 typed distance 只接管距离；ROTATE 的
  typed angle 接管方向、保留光标半径；SCALE 的 typed factor 缩放捕捉的参考向量。证据：
  `cad-app host::tests::typed_dimensions`、`cad-ui-core tests_typed_dimensions`、
  `tool-parameters.test.ts`。
- **2026-11-25 lane 3 P2 完成（OFFSET）**：`ToolParameter::OffsetDistance`（`offsetDistance`，
  slot 44）仅在 OFFSET capture 活跃时投影。`host::draw_intent_map` 让 typed 距离优先于
  session 的 `OFFSET <distance>`，`0` 回退到存储模式；`OffsetMode::Distance` 让侧点只选边。
  证据：`cad-app host::tests::typed_dimensions`、`cad-ui-core tests_typed_dimensions`、
  `tool-parameters.test.ts`。OFFSET 预览仍不画偏移结果（typed 命令路径同样如此），
  距离在 confirm 后生效。

## 实施进度（P4 追加）

- **2026-11-25 P4 spike + seam 原子操作完成**：真实内核 spike 证明推拉/跟随都可用；
  `SolidOperation::OffsetFace`（带符号单面移动）与 `ModelerCapabilities::offset_face` 接线，
  `solidcore` 声明 `offset.face`。证据：`cad-kernel-solidcore` 的 `offset_face`（6 通过）与
  `presspull_spike`（2 通过）、`cad-kernel-adapter` 的 `solid::tests`（16 通过，含校验/能力/nurbs
  闸门）。命令与 UI 未接线，`PRESSPULL`/`FOLLOWME` 仍是具名拒绝。

## 顺序与停点

1. lane 2 P1（多段线/文字原子创建）——先打通插件写入闭集。
2. lane 3 P1（LINE length/angle）——验证 Measurements 的 Rust→React 组装。
3. lane 2 P2（选择）与 lane 3 P2（其余工具）交替推进。
4. 任何需要新内核几何的操作（推拉/跟随）先出 spike，不直接改 UI。

每完成一项：对应契约测试 + `python3 scripts/check-i18n.py` + `check-file-size.py`；全量门禁与
真实载体验收只在用户明确要求或发版前运行。
