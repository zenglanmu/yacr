# 概念域建模：Follow Me、旋转矩形、扇形（2026-11-27）

本轮把三个概念域工具从 pending 接到真实命令：

* **Follow Me**：同时选中一条闭合轮廓与一条开放直线路径，按 `FOLLOWME`，按回车把轮廓沿路径
  扫掠成**一个闭合网格体**。
* **Rotated Rectangle**：三次点击（角点、第一边端点、宽度点）生成任意平面上的闭合四顶点
  `LWPOLYLINE`。
* **Pie**：三次点击（圆心、圆周点、终点角）生成闭合扇形 `LWPOLYLINE`（弧按 24 段/圈采样）。

## 行为

- **零点工具**：`DrawToolKind::FollowMe`（`Exact(0)`，`requires_selection`），`DrawIntent::FollowMe`
  由 `host/draw_intent_map.rs` 映射为 `ExtendedCommandId::FollowMe` +
  `CommandPayload::FollowMeProfile(selection)`；没有参数、没有拾取点。授权（Work-only）与
  「先有选择」都在工具起点完成。
- **分区**（`cad-app::app_follow_me`）：一个闭合直线 `LWPOLYLINE` 或一组 `LINE` 闭环是轮廓；
  一个开放直线 `LWPOLYLINE` 是路径（轮廓是 `LINE` 环时，路径必须是开放 polyline）。两条
  `LINE` 链无法区分角色，按 `followme.profile_missing` 明确拒绝而不是猜。
- **轮廓**：闭合直线 `LWPOLYLINE`、`LINE` 闭环，或 `CIRCLE`（按 `CIRCLE_SEGMENTS = 48` 采样成
  多边形环；投影圆是椭圆，网格面片本来就分段，SketchUp 的圆也是多边形）。
- **几何**：轮廓平面用 Newell 法向；轮廓环先检查自交（主导轴投影）与共面；每段路径用
  「轮廓法向 → 段切向」的最小旋转确定装载姿态并沿路径平行输运；**拐角站是轮廓沿入段方向
  投影到角平分面的剪切断面**（真正的斜接），两端用同一姿态封盖。直轮廓在拐角只被剪切、不变形。
- **网格**：每个墙面四边形带自己的 `face_sources` key（`followme.wall.<segment>.<edge>`），
  两个端盖各一个（`followme.cap.start` / `.end`），因此 `PRESSPULL`/`MOVE` 仍可按面工作；
  顶点按面复制、法线为真实面法线（硬边）。整体符号由有符号体积决定，保证外向。
- **存储**：一个新的 `AcDbSubDMesh` 行；轮廓与路径**保持原位**、不删除、不改写；一次事务、
  一步撤销、一次 revision。`ELEV` 不平移该网格（显式放置的三维几何，和 `Body3d` 同一规则）。
- **拒绝**（全部在写入前、稳定标记）：`followme.requires_payload` / `selection_required` /
  `not_addressable` / `not_model_space` / `unsupported_entity` / `profile_missing` /
  `profile_multiple` / `profile_unsupported`（弧、带 bulge 的段）/ `profile_degenerate` /
  `profile_not_planar` / `profile_self_intersecting` / `path_missing` / `path_multiple` /
  `path_unsupported` / `path_degenerate` / `path_closed` / `path_reversal` / `ambiguous` /
  `mesh_invalid`。

## 旋转矩形与扇形

- **`RECTANGROTATED`**：`Points([corner, edge_end, width_point])`。前两点是第一条边（非零长度），
  三点确定平面（共线按 `rectangrotated.degenerate` 拒绝）；宽度方向是平面法向×边方向，宽度为
  第三点在该方向的投影（为 0 也按名拒绝）。四个角点按平面法向逆时针排列，直接可作为闭合轮廓
  继续 `PRESSPULL`/`FOLLOWME`。
- **`PIE`**：`Points([center, start, end])`。半径是 `|start-center|`（0 拒绝），终点只定角度
  （与起点半径共线按 `pie.degenerate` 拒绝）；弧按 `PIE_SEGMENTS_PER_TURN = 24` 采样，环是
  「圆心 + 弧点」，整圈时去掉与起点重合的末点。三点捕获只能确定 ≤180° 的转向，这是该切面的
  显式边界（更大的扇形用 `ARC`+`LINE`）。
- 两者都是一次事务/一步撤销，层与颜色由共享插入漏斗决定，没有内核参与。

## 为什么是网格而不是精确 B-Rep（已测的否定证据）

先按「`SolidOperation::SweepProfile` → `solidcore::loft`」实现并跑了真实内核探针，结果是
**不能用**：

- 斜接断面（把轮廓投影到角平分面）作为中间截面交给 `loft`，90° 弯得到
  `solid.kernel_failed: ... inside out`（壳方向反了），三段路径得到
  `pointers check fails`；
- 只用刚体旋转的断面（不剪切）时壳合法但**截面被夹窄**：1×1 截面走两段 4 长的 90° 路径，
  体积是 **6** 而不是 8；
- `sweep_stations`（`Ends::Capped`）对同一组放置返回 `NotSolidBecause(VolumeNegative)` /
  `PointersInvalid`，或体积 2.67。

因此精确斜接扫掠在当前 pin 的内核 API 下不可表达；把 `SweepProfile` 接缝回滚，改为
`cad-app` 内**确定性网格生成**（与 SketchUp 的面片结果一致，且不需要内核）。伪代码与小样本
测量见本轮提交说明；这是**行为选择**，不是性能/内核结论。

## 命令与结果

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked follow_me` | PASS（**18 passed**：直路体积 4、90° 斜接体积 7、L 形凹轮廓耳切封盖体积 6、CIRCLE 按 48 边形扫掠（体积 12.5305）、LINE 环 + 开放 polyline、闭合/一致朝向断言、空选择/双轮廓/缺路径/闭合路径/反向/弧/bulge/非共面/自交/歧义 全部具名拒绝、`ELEV` 不平移、轮廓/路径保留） |
| `cargo test -p cad-app --lib --locked concept_shapes` | PASS（**6 passed**：旋转矩形四角精确、任意平面上定宽、扇形 8 点与采样面积、退化/点数拒绝、两条命令各一次事务一步撤销） |
| `cargo test -p cad-app --lib --locked` | PASS（**3461 passed, 3 ignored**） |
| `cargo test -p cad-ui-core --locked` | PASS（**945 passed, 1 ignored**；含 FOLLOWME/RECTANGROTATED/PIE 的 extended 表一行/Work-only 期望） |
| `cargo test -p cad-host --locked` | PASS（**487 passed**） |
| `cargo clippy -p cad-app -p cad-ui-core --all-targets --locked -- -D warnings` | PASS（exit 0） |
| `cargo fmt --all -- --check` | PASS（exit 0） |
| `python3 scripts/check-architecture.py` | PASS（40 packages，无环） |
| `python3 scripts/check-file-size.py` | PASS（2911 文件，≤1000 行） |
| `python3 scripts/check-i18n.py` | PASS（3639 键，两目录一致） |
| `python3 scripts/check-command-docs.py` | PASS（0 mismatches） |
| `(cd apps/desktop-ui && npm test)` | PASS（**2110 passed**：`tsc --noEmit` + 全部前端契约，含概念域 pending 集与命令目录/闸门） |

## 证据边界

- **没有真实 GUI/GPU/像素证据**：没有在 Wayland/X11 窗口里实际点选轮廓与路径、按 P、
  观察网格；上面的体积/封闭性/朝向都是命令层合成契约。
- **没有真实 SKP 语料**：轮廓/路径是本测试构造的 `LWPOLYLINE`/`LINE`。
- **没有内核参与**：这是有意的（网格生成在命令层），因此也没有内核/性能结论；精确 B-Rep
  斜接扫掠仍是 **NOT IMPLEMENTED**，上面的探针是它的否定证据。
- **载体**：`cad-app` 是共享层，命令不依赖内核，因此所有载体都有同一路径；但
  Windows/macOS/Android/Web 的运行时运行 **NOT VERIFIED**。
- **不支持的子集**：圆/弧轮廓（投影成椭圆）、闭合路径、三维自交路径、按路径面点选驱动、
  多轮廓同时扫掠都按名拒绝，不静默近似。
