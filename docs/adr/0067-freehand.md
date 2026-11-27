# ADR-0067：自由手绘（Freehand）

- 状态：**已接受（决策，2026-11-27）**；**已实现（2026-11-27，拖拽采样 + `SKETCH` 落盘 + 一步撤销；无真实 GUI/GPU 证据）**
- 关联：ADR-0015（交互工具捕获）、ADR-0058/0059（线/多段线语义沿用）、ADR-0064/0065/0066（同属“托盘/工具不再 pending”）

## 背景

SketchUp 的 Freehand 是一个**拖拽**工具：按住并移动指针，松开后把轨迹变成一条手绘边/多段线。
本仓库已有：

- `SKETCH` 命令（`CommandPayload::Points`）把一组已采样点写成**一条开放 `LWPOLYLINE`**（≥2 点、
  一次事务/一步撤销、无闭合、无平滑）；
- 指针管线的“lasso”自由手绘拖拽（`STRETCH` 的 polygon 窗口）：按下开始采样、每次移动追加一个
  抽样点、松开把路径交给捕获；
- 交互工具的 `BeginDraw` 路由与 `DrawToolKind` 目录。

缺的只是把三者接起来：`SKETCH` 没有路由、`DrawToolKind::Freehand` 不存在、host 没有把拖拽
采样喂给捕获。

## 决策

### D1 拖拽 = 一次连续捕获，不是点击

- 新增 `DrawToolKind::Freehand`（`PointArity::AtLeast(2)`，无实体拾取，追加在 `ALL` 末尾以保持
  索引稳定）。
- `SKETCH` 的裸选择器路由到 `BeginDraw(Freehand)`：命令行/按钮启动工具，工具持有指针。
- host 复用 lasso 的 `LassoGesture`（同一 2px 抽样步长与 1024 上限）在 `freehand_gesture` 中
  采样：按下开始，移动追加，松开把路径交给捕获。
- 捕获以**原始**世界点填充（`capture_freehand_point`）：不套用吸附候选、不套用正交约束——手绘是
  手的路径，不是推断点；其它工具的捕获按名拒绝该入口。

### D2 松开即提交，一条开放多段线

- 松开 → `finish_freehand_gesture`：把逻辑像素路径经当前视口的 `screen_to_world` 投影（与框选/
  lasso 同一映射），逐点喂给捕获，然后走普通 `confirm_tool` → `DrawIntent::Freehand` →
  `CommandId::Sketch` + `Points` 一次提交。
- 结果是一条**开放** `LWPOLYLINE`（无闭合、无 bulge、无 spline 拟合），一次事务/一步撤销；
  `SKETCH` 命令自身的校验（≥2 点、有限点）仍是最终权威。
- 一次点击（路径任一轴 < 2 逻辑像素）不是笔画：`sketch.too_few_points` 具名拒绝、工具继续等待、
  不写文档。3D/透视视图没有唯一平面点：`sketch.requires_plan_view` 具名拒绝、不写文档。
- 取消/多指接管丢掉采样，不提交（tool 由既有的 cancel 语义结束）。

### D3 预览与文案

- 拖拽期间轨迹走既有 `BoxSelectionPreview` 瞬时 overlay（实线，WINDOW 色），因为这是正在画的
  几何而不是 crossing 窗口；不新增渲染通道。
- 工具状态行是 `freehand: drag to trace a stroke; release to finish`（采样数随之更新），不提示
  “按 Enter”。
- 概念域工具栏/菜单的 Freehand 从 `pending` 变成真实工具（命令 `SKETCH`），i18n 增加提示键。

## 不在本 ADR 范围

- 平滑/拟合（SketchUp 的 freehand 边是折线，本实现同样不拟合）、自动闭合、压感/抽稀曲线拟合；
- 3D 空间的手绘输入（只在 2D 平面视图投影）；
- 把笔画自动切成若干条边/面（SketchUp 的“边切分面”语义仍未建模）；
- 真实 GUI/GPU 交互证据与其它载体运行时。

## 后果

- 新增一个工具 kind（索引追加）、一条 selector 路由和 host 采样路径；**无新命令、无格式档位变化**。
- 工具目录长度 64 → 65，`ALL` 末尾追加，前端按索引的镜像不需要重排。
- 自由手绘的采样上限与 lasso 共用（1024），捕获本身也有同样的上限，二者一致。
