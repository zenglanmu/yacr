# ADR 0028：光标 crosshair/pickbox 作为 renderer transient overlay

- 状态：**Proposed（2026-11-05）**；实现未完成。本文只冻结架构决策；Phase 1（suffix commit
  快速止血）已独立落地，**不**由本文宣称已验收。
- 日期：2026-11-05。
- 相关 ADR：[ADR-0012](0012-picking-and-selection.md)（pickbox/悬停/拾取语义）、
  [ADR-0023](0023-dom-on-top-cross-host-composition.md)（跨宿主合成与输入漏斗）、
  [ADR-0024](0024-dom-canvas-wasm-cad-path.md)（`dom-canvas` 载体）、
  [ADR-0014](0014-drawing-tools.md)（预览与交互）。
- 影响：光标几何的所有权从 `OverlayInputs`/场景准备/提交链移到 renderer transient；**不**改变
  `cad-db` 权威、`.native-input` 单一输入漏斗、base/overlay 两层 GPU target、拾取谓词、绘制顺序
  （preview < cursor < snap）或任何宿主路由。

---

## 1. 问题

2026-11-05 的 base/overlay 两层设计（`docs/handoff.md`）只解决了“移动鼠标不重绘/重新分页底图”：
base colour/depth 与 overlay colour/depth 分离，overlay 每帧合成。但光标仍属于
`OverlayInputs.cursor`，因此每次 `pointermove` 都会：

1. 改变 `overlay_fingerprint`（`crates/cad-app/src/render_scene/overlay/fingerprint.rs:99`）；
2. 触发 `set_overlays` 的 `visual_changed` → `advance_input()`
   （`crates/cad-engine/src/render/session_impl.rs:206-217`）；
3. 生成新的场景准备 token，并发布一次新的 scene revision；
4. 执行一次 suffix commit（`crates/cad-engine/src/render/runtime/session.rs:196-203,302-312`）。

Phase 1 已把“run-free suffix commit”的 GC、pulled 重建、base draw-plan 重建去掉（实测 169 次
suffix commit 的 `commit_us` p50 从 19.95 ms 降到 2.16 ms，`frame_total` p50 从 62.8 ms 降到
10.5 ms）。但每次移动仍要付一次场景准备 + suffix commit + pick table 增量 + lineweight 扫描，
并且准备 token 在连续快速移动时会被不断作废（`crates/cad-engine/src/render/mod.rs:415`），
光标可能停在上一次已发布位置直到手停下来。`docs/handoff.md`（2026-10-25）已把
“cursor 作为不参与发布令牌的动态 overlay”记为后续 ADR 事项。

问题不是“底图重画”，而是：

> **光标是每帧都在变的 O(1) 屏幕反馈，为什么还要走 O(场景) 的场景准备/发布/提交？**

## 2. 决策

### D1 光标 crosshair + pickbox 是 renderer transient，不是场景内容

- `OverlayInputs` **删除** `cursor`、`crosshair_percent`、`pickbox_logical_pixels` 及其
  overlay 指纹段；`cad-app::render_scene::overlay::cursor_overlay` 的几何生成保留为纯函数，
  但不再 merge 进 `VisualOverlay`。
- `cad-render-wgpu` 持有**自己的** transient cursor 状态与 GPU 缓冲，**不**进入
  `self.batches`、不分配 pick key、不进入 `draw_plan`/`BatchFacts`、不进入 id/pick pass。
- 移动光标只做：host 更新 cursor 输入 → engine 标脏 → 下一帧 overlay pass 用新 instance
  数据重画。**不**推进 `content_revision`、**不**生成场景、**不** commit、**不**作废准备 token。

### D2 几何权威仍在 cad-app，数据类型在 cad-scene

- `cad-scene` 定义载体无关的数据类型
  `TransientCursor { segments: Vec<[Point3; 2]>, color: [f32; 4], draw_order: i64 }`（最多 6 段：
  2 条 crosshair + 4 条 pickbox），避免 `cad-render-wgpu → cad-app` 的反向依赖。
- `cad-app` 提供纯函数
  `cursor_geometry(cursor, viewport_bounds, view_scale, pickbox_logical_pixels, crosshair_percent)
  -> Option<TransientCursor>`，从现有 `cursor_overlay` 的几何逻辑抽出；3D/无 bounds/
  不可投影时返回 `None`（显式空，不伪造）。
- `cad-engine` 只做传输：`RenderSessionHandle::set_cursor_transient(Option<TransientCursor>)`
  更新 runtime 状态并标脏；不进入 `OverlayInputs`、不触发 preparation。
- `cad-host::sync_render` 计算世界坐标/plot 参数并喂给上述入口；`ClearCursor`/`pointer_leave`
  喂 `None`。状态栏的 `cursorWorld`/`crosshair` 改读 host 的 cursor 源，不再读
  `render_session.overlays().cursor`。

### D3 固定 6 instance 的动态缓冲，按 draw_order 合并

- renderer 保留一个固定 6 个 `LineInstance` 的 `VERTEX | COPY_DST` 缓冲（约 384 B），
  `queue.write_buffer` 更新；用现有 `line_instanced_pipeline` 在 overlay pass 中画，一个
  per-batch uniform slot，pick-key word 为 `0`。不新增 WGSL。
- transient 不能“最后画”：绘制顺序固定为 **preview (2_000_000) < cursor (2_500_000) < snap
  hints (3_000_000)**（`cursor.rs` 常量与 `snap_glyphs` 契约）。因此 transient 以
  `draw_order = CURSOR_DRAW_ORDER` 合并进 `plan_overlay` 的排序；snap hints 仍留在 committed
  overlay，继续压在 pickbox 之上。
- `copy_base_to_overlay` 与 overlay depth 每帧清空保持不变；光标每帧重新合成，`None`/移动
  不需要额外擦除。

### D4 失败与边界显式

- 不喂 transient 的宿主 = 没有 crosshair；`cursor_moves` 契约必须把它判为失败，不能静默。
- 设备丢失/重建：缓冲随 renderer 资源重建，CPU 参数保留；初始化失败走既有 device-lost 判定。
- resize/相机变化：host 在同一次 sync 重新喂 bounds/scale，避免光标尺寸漂移。
- id/pick pass 永不绘制 transient；点击穿过 crosshair 仍拾取背后的图元（契约钉住）。
- `FrameStats` 如实计入 transient draw，不把它算进 base/overlay committed batch。

## 3. 后果

- 光标移动从“一次场景发布 + suffix commit”降为“一次小缓冲写 + overlay 重画”，预期
  `frame_total` 从 Phase 1 的 ~10.5 ms 进一步降到 ~1–2 ms（P620/1080p 有界估计，待实测）。
- 连续快速移动不再作废准备 token；光标不再因为场景发布落后而停滞。
- 代价：cursor 几何不再参与 `OverlayInputs::is_empty`/overlay fingerprint/场景快照；
  状态投影必须改源（D2），否则 `cursorWorld`/`crosshair` 会永久为空。
- snap hints 本轮**仍**在 committed overlay；工具态下移动仍可能触发 suffix commit。若后续实测
  snap 抖动成为主因，用同一 transient 机制扩展（需单独契约）。

## 4. 验证要求（实现阶段）

- 纯契约：`cursor_geometry` 与旧 `cursor_overlay` 的段集合/颜色/draw_order 等价（2D、3D、
  退化 scale、跨视图百分比）；transient 不进入 overlay fingerprint。
- renderer 纯契约：transient 合并排序在 preview/snap 之间；id pass 不含 transient；
  `FrameStats` 计数；`None` 不残留。
- GPU 契约（真实 adapter，串行）：移动一次后 base 像素不变、overlay 光标像素移动、pick 仍命中
  背后 entity、设备重建后恢复；无 suffix commit / 无 GC / 无 pulled 重建计数增量。
- 宿主/状态契约：`cursorWorld`/`crosshair` 来源改变后状态栏仍更新；3D 不显示 2D crosshair。
- 真实 GUI：P620/Wayland 复跑 synthetic DOM sweep，报告 suffix commit 数应为 0（纯光标移动），
  并标注 synthetic/real-mouse、GPU/软件、有界环境。Windows/macOS/Android/Web 仅编译/契约，
  不得写成已验收。

## 5. 未决

- snap hints 是否同批 transient 化（先测量，再单独 ADR/契约）。
- 状态事件的 wire 节流（`status-changed` 语义化 + `cursor-changed` 轻量事件）是独立改动，
  不在本 ADR 的实现范围。
