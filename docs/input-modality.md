# 输入模态与指针手势（mouse / touch / pen）

> **2026-11-01**：本页记录按**设备**仲裁的指针手势、双指 `pinch` 与
> `interaction.leftDragPans` 偏好。它是 `docs/react-linux-app.md` 手势小节的展开；
> 历史决策见 [ADR 0014](adr/0014-object-snap-and-box-selection.md) 决策 B（本页不改写它）。
>
> **证据上限**：只有源码与单元/契约测试（`cad-ui-core`/`cad-host`/`cad-app`/前端）。
> **真实触摸硬件、触摸屏笔记本、笔、Android 设备/模拟器手势、触控板 pinch 全部 NOT RUN**；
> 不得把本文读成真实设备行为或性能结论。平台边界见 `docs/compatibility.md`。

## 1. 设备检测（逐事件）

- React 输入层读取**每个** `PointerEvent.pointerType`，在 `pointer_down`/`pointer_move`/
  `pointer_up`/`pointer_cancel` 上附带可选 `device: "mouse" | "touch" | "pen"`。
- 字段**缺失或未知**一律按 **mouse**：旧宿主、旧前端与未上报 `pointerType` 的环境行为不变。
- 判定是**逐事件**的，不是按设备一次性分类：触摸屏笔记本上插入鼠标后，鼠标事件仍走鼠标语义。

## 2. 默认手势矩阵（所有载体一致）

仲裁在共享 `cad-host`；原生与 wasm 载体只有传输层不同，语义相同。

| 设备 | 主键 / 单指拖拽 | 平移 | 缩放 |
|---|---|---|---|
| mouse | 2D = 框选（≥4 逻辑像素；左→右 WINDOW / 右→左 CROSSING，`Shift` 加选，否则替换） | 中键拖拽（2D/3D） | 滚轮 |
| touch | 单指拖拽 = 平移；tap（阈值以下）= 选择/取点，**从不框选** | 单指拖拽 | 双指 pinch（中点平移 + 以中点为锚缩放） |
| pen | 同 mouse | 中键拖拽 | 滚轮 |

- 框选橡皮筋走 **renderer overlay**（`BOX_SELECT_DRAW_ORDER`；WINDOW 蓝实线、CROSSING 绿虚线）。
- 3D 主键拖拽仍是**轨道旋转**；3D 框选显式 NOT IMPLEMENTED。
- 工具、捕获、夹点拖拽、`ZOOM WINDOW`、模态 pan 的优先级**不变**——它们先于手势仲裁。

## 3. 双指 `pinch`

- **进入**：第二个触点按下时，前端发**一次** `pointer_cancel`（结束当前单指手势/工具）；不为第二根
  手指发 `pointer_down`。
- **数据**：连续 `pinch` 输入 `{x, y, dx, dy, scale}`：
  - `x,y` = 双指中点（element-local CSS px）；
  - `dx,dy` = 相对上一条 `pinch` 的中点位移（逻辑 px）；
  - `scale` = 距离比增量（`1.0` = 不缩放，`>1` = 放大）。
- **期间**：不逐指针转发 `move`/`up`。
- **退出**：触点数少于两个时再发**一次** `pointer_cancel`；pinch 没有终止数据事件。
- **宿主语义**：取消活动工具 → 按 `dx,dy` 平移 → 以中点为锚按 `scale` 缩放。
  **从不框选、不提交捕获。**
- 触控板 pinch 仍是 **Ctrl+滚轮**（原本已转发，未改）。

## 4. `interaction.leftDragPans` 偏好

- 持久化叶子 `interaction.leftDragPans: bool`，默认 `false`。
- 入口：Options → Drafting 的布尔项 “Left-drag pans” / 「左键拖动平移」。
- 开启后 **mouse/pen** 的 2D 主键拖拽改为**平移**（不再框选）；touch 本来就是平移。
- 宿主**每次派发**都从有效 viewer config 读取（`HostCore::left_drag_pans()`），
  因此原生与 wasm 载体行为一致。

## 5. 诚实上限

- **Android**：弹窗宿主已接（`AndroidPopupHost`，in-DOM 覆盖层，Options / About / text-edit 可用），
  配置/偏好持久化在应用私有 `filesDir`（`config.json` / `preference.json`，路径来自
  `nativeAssetsReady` 的 `fontsDir` 父目录）。因此该选项可经 Options 修改并在下次启动读回；
  **设备侧点击/落盘/重启读回 NOT RUN**（源码与开发机契约测试只证明接线与文件语义）。
  触摸经共享 DOM → `probe_input` 漏斗，pinch 契约同上游（见 `docs/android-host.md` §5）。
- **Web**：Options 可用，但只在**会话内**持久。
- **证据**：仅源码/契约测试；真实触摸硬件、手势与设备运行 **NOT RUN / NOT VERIFIED**。
