# 2026-11-07 附加批次：SKETCH / TRACE / MOCORO / MSTRETCH / -PAN / VIEWRES / VSCURRENT / REGEN3 / LINETYPE

本轮只做 **契约级** 实现：`cad-app` 命令、`cad-ui-core` 词表/参数文法、React 命令目录/门禁与
双语 i18n。没有运行 GUI、没有 GPU/像素证据，没有真机结论。所有“通过”仅指下列命令在本机
（Linux，共享 `/tmp` 目标目录）真实运行。

## 载体与权限

| 命令 | CommandId | 权限 | 写数据？ |
|---|---|---|---|
| `SKETCH` | `Sketch`（新） | Work | 一条开放 LWPOLYLINE |
| `TRACE` | `Trace`（新） | Work | 一条闭合 4 顶点 LWPOLYLINE |
| `MOCORO` | `Mocoro`（新） | Work | 复用 MOVE/COPY/ROTATE/SCALE |
| `MSTRETCH` | `MStretch`（新） | Work | 复用 STRETCH 规划器 |
| `-PAN` | `Pan`（既有） | Viewer/Work | 否（仅相机） |
| `VIEWRES` | `ViewRes`（新） | Viewer/Work | 否（会话 token） |
| `VSCURRENT` | `VisualStyle`（新） | Viewer/Work | 否（会话 token） |
| `REGEN3` | `Regen`（既有，别名） | Viewer/Work | 否（刷新戳） |
| `LINETYPE` | `Linetype`（新） | VIEWER 仅 `LIST`；`SET` 走 Work 授权 | `SET` 写实体属性 |
| `-LINETYPE` | `Linetype`（既有别名） | 同上 | 同上 |

`-PAN`、`REGEN3`、`-LINETYPE` **没有**第二个 `CommandId`：它们只是 `COMMAND_ALIASES` 的拼写，
归一化到 `PAN`/`REGEN`/`LINETYPE`，因此只有一条相机/重生成/线型路径。

## 各命令的真实边界

### 1. `SKETCH`

- 输入：`CommandPayload::Points(Vec<Point3>)`（宿主采样后的点序列）。
- 存储：活动图层上一条 `SemanticGeometry::Polyline { closed: false, bulges: 全 0 }`；
  `type_key` 是 `AcDbPolyline`，**没有** `SKETCH` 实体类型。
- 一次事务/一步撤销；拒绝 <2 点或任一非有限点，均发生在事务打开前。
- **NOT IMPLEMENTED**：交互式自由手采样（定时器、指针采集、自动闭合）。宿主负责采样；
  本层不伪造采样器。

### 2. `TRACE`

- 输入：`CommandPayload::Trace { start, end, width }` 或
  `TRACE <x1> <y1> <x2> <y2> <width>`。
- 存储：XY 平面内闭合 4 顶点 LWPOLYLINE（半宽 `width/2` 的法向偏移）；所有顶点取 `start.z`。
  **没有** `TRACE` 实体类型，也没有宽度字段。
- 一次事务/一步撤销；拒绝非有限点、重合端点、`width <= 0` 或非有限宽度。
- **NOT IMPLEMENTED**：AutoCAD 的 TRACE 实体/填充语义。

### 3. `MOCORO`

- 输入：`CommandPayload::Mocoro(MocoroCommand::{Move, Copy, Rotate, Scale})`。
  命令行为 `MOCORO MOVE|COPY <dx> <dy>`、`MOCORO ROTATE <bx> <by> <degrees>`、
  `MOCORO SCALE <bx> <by> <factor>`。
- `MOVE`/`ROTATE`/`SCALE` 原地变换；`COPY` 保留源并插入平移副本。全部复用既有
  `MoveEntities`/`CopyEntities`/`RotateEntities`/`ScaleEntities` 写路径（各自的实体类型拒绝与
  “一次事务/一步撤销”不变）。
- 空选择拒绝（`mocoro.selection_required`），发生在事务前。
- **NOT IMPLEMENTED**：AutoCAD 的交互式组合 gizmo/选项循环；本层要求完整输入。

### 4. `MSTRETCH`

- 输入：`CommandPayload::MStretch { rects_logical, base, delta }`。`rects_logical` 是逻辑像素
  矩形列表，经当前视口正交平面映射；`base`/`delta` 是世界坐标。
- 复用 `Application::stretch_windows`（即 `STRETCH` 的 enclosure/顶点移动规划器）：
  完全落在**任一单个**窗口内的实体整体移动；被任一窗口命中的实体按部分拉伸；顶点/端点只要
  落在**任一**窗口内就移动。一次事务/一步撤销。
- 拒绝：空窗口列表、非有限角点/基点/位移、退化窗口、零位移、非模型空间、非正交平面。
- **NOT IMPLEMENTED**：命令行参数文法（矩形列表没有可定义的单一参数分隔符）；只有
  payload/host IPC 路径。

### 5. `-PAN`

- `-PAN <dx> <dy>` 或 `-PAN <x1> <y1> <x2> <y2>`（第二形式取差）。
- 归一化到既有 `PAN`，payload 为 `Points([delta])`，走同一相机平移（符号约定与 `PAN` 相同）。
- 仅视图状态：无数据库写入、无 revision、无撤销、无 `ZOOM PREVIOUS` 步。

### 6. `VIEWRES`

- `VIEWRES <integer 1..=1000>`；存入 `SessionState::view_extras.viewres_percent`。
- 默认 1000；0、1001、小数、多个 token 均被具名拒绝（`viewres.out_of_range` / 文法拒绝）。
- 仅会话状态；无数据库写入/撤销。**没有**渲染器消费该值的改动，像素效果 NOT VERIFIED。

### 7. `VSCURRENT`

- `VSCURRENT <style>`；只接受 `WIREFRAME`，存入
  `SessionState::view_extras.visual_style`（`VisualStyleToken::Wireframe`）。
- 其它样式（`SHADED`、`CONCEPTUAL`、3D wireframe 等）以
  `vscurrent.style_not_implemented` 具名拒绝，不做静默映射。
- 仅会话 token；本批 **不**改渲染器、不做像素/GPU 验证。宿主可通过
  `SessionState::view_extras.visual_style` 消费，但当前没有渲染路径读取它。

### 8. `REGEN3`

- `COMMAND_ALIASES` 中的 `REGEN3 -> REGEN`；同一 `CommandId::Regen`、同一条
  `regen(...)` 路径，**不是**第二套实现，也不产生第二位图刷新戳。

### 9. `LINETYPE` / `-LINETYPE`

- `LINETYPE LIST`：只读列出当前图纸线型表（诊断代码 `linetype.list`），Viewer 允许。
- `LINETYPE SET <name>`：对当前选择中的每个实体写入显式线型，复用
  `SETENTITYPROPERTY` / `property_value` 的既有文法与写路径（一次事务/一步撤销）。未知线型是
  既有 `property.unknown_linetype` 具名拒绝；空选择是 `property.selection_empty`。
  `SET` 在应用层重新授权 `CommandId::SetEntityProperty`，因此 Viewer 下 `LIST` 可读、`SET` 被
  `PermissionDenied` 拒绝。
- **NOT IMPLEMENTED**：线型表的创建/加载/删除/重命名（`LOAD`/`CREATE`/`DELETE`/`RENAME`）。
  本构建没有线型表写 API。

## 契约测试

新增/更新：

- `crates/cad-app/src/tests/extras.rs`（9 个用例）：SKETCH 点数/开放/撤销、TRACE 宽度/闭合/撤销、
  MOCORO 四模式/空选择、MSTRETCH 双窗口/单撤销、-PAN 仅视图、VIEWRES 范围、VSCURRENT token/
  具名拒绝、LINETYPE LIST/SET/未知线型/Viewer 授权。
- `crates/cad-ui-core/src/session/tests/tests_commands/extras_arguments.rs`（7 个用例）：词表行、
  `REGEN3`/`-PAN`/`-LINETYPE` 别名、各参数文法与具名范围拒绝。
- `command_vocabulary.rs`：命令总数 213 → 220，新增 7 个 selector 的穷尽映射。
- `snap_grid.rs`：`ARGUMENT_SELECTORS` 期望表新增 `PAN/TRACE/MOCORO/VIEWRES/VSCURRENT/LINETYPE`。

## 门禁（本机真实运行）

```text
cargo test -p cad-app --locked                # 1898 passed, 3 ignored, 0 failed
cargo test -p cad-ui-core --locked            # 451 passed, 1 ignored, 0 failed
cargo check -p cad-app -p cad-ui-core --locked
cargo fmt --all -- --check
cargo clippy -p cad-app -p cad-ui-core --all-targets --locked -- -D warnings
python3 scripts/check-file-size.py
python3 scripts/check-i18n.py
python3 scripts/check-command-docs.py
python3 scripts/test-check-command-docs.py
python3 scripts/check-architecture.py
```

目标目录：`CARGO_TARGET_DIR=/tmp/opencode/target-extras-batch`（`CARGO_INCREMENTAL=0`、
`CARGO_PROFILE_DEV_DEBUG=0`、`-j 1`）。

## NOT RUN / 未验证

- React 前端测试/构建：`apps/desktop-ui/node_modules` 不存在，`npm test` / `npm run build` **NOT RUN**。
- 任何 GUI 运行、真实 GPU、像素、性能证据：**NOT RUN**。
- Windows/macOS/Android/Web 运行时：**NOT VERIFIED**（仅共享层契约；本批未新增平台代码）。
- 真实 DWG 样本：**NOT RUN**（本批用例全部为合成数据库）。

## 未触碰

`acadrust` 源码、`Cargo` patch、`docs/compatibility.md`、`docs/handoff.md`、
`docs/autocad-2026-command-coverage.md` 均未修改。`docs/shortcuts.md` §2b 因新增三个别名按门禁
要求同步了别名表与计数。
