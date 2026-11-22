# ADR 0047：MSAA 只作用于持久 base 通道，overlay 与 wipeout mask 保持 1×

- 状态：**Accepted（2026-11-22）**。实现与 lavapipe 离屏像素契约已执行；真实 GPU、
  真实浏览器、Windows/macOS/Android 运行时**未验证**，见 §5 与
  `docs/validation-msaa-2026-11-22.md`。
- 日期：2026-11-22。
- 相关：[ADR-0008](0008-sdf-conic-rendering.md)（SDF 解析抗锯齿）、
  [ADR-0009](0009-antialiased-line-strokes.md)（解析线条抗锯齿）、
  [ADR-0022](0022-wipeout-mask-rendering.md)（wipeout mask）、
  [ADR-0026](0026-gpu-memory-accounting-and-budget.md)（GPU 预算）、
  [ADR-0010](0010-react-linux-host.md)（React 宿主与选项）。
- 影响：`cad-render-wgpu`（离屏附件、管线多重采样、resolve、mask 所在 pass）、
  `cad-engine`（`ViewSnapshot.msaa_samples` → `RenderTarget::samples`）、
  `cad-app` / `cad-host` / `cad-ui-core`（`view.msaa.mode` 与 `msaaMode` 设置）、
  `apps/desktop-ui`（显示选项）。
  **不改变**：`cad-db` 唯一权威、`.ymodel` 布局与 `FORMAT_MINOR`、载体所有权、
  patch 禁令、pick 的精确 1× 路径。

---

## 1. 决策

1. **MSAA 只作用于持久 base 通道**。base 颜色/深度按请求的 `1/2/4` 采样；base pass
   每个页面结束时通过 `resolve_target` 直接解析进**单采样**的 `cad-overlay-target`。
2. **presented overlay 与 wipeout mask 永远是 1×**。overlay 是网格/坐标轴/光标/预览等
   UI 几何；mask 是“用背景色遮住已有几何”的约定（ADR-0022）。若 mask 进入 MSAA base，
   边界像素会解析成 `背景 × (1−coverage) + 被遮几何 × coverage`，即隐藏内容从边缘透出，
   这不是“更柔和的抗锯齿”，而是 mask 失效。
3. **1× 路径逐字节保持原行为**：`samples == 1` 时仍是 base 颜色 `copy_texture_to_texture`
   到 overlay，mask 仍在 base pass 末尾绘制；MSAA 开启时才切到“base resolve + mask 在
   overlay pass 先画”的顺序。
4. **运行时切换**：`RenderTarget.samples` 是唯一请求源。产品路径只在 `ProjectionKind::ThreeD`
   把设置的采样数传给 frame；2D 视图固定 `1×`，保持 ADR-0009 解析线条的既有像素。
   `ensure_target` 在分配前比较 `requested/effective`；变化时只重建“会写入 base 颜色”的管线并丢弃离屏附件，
   **不清空** `batches`/`base_end`/纹理/选择键/暂存区，并递增 `texture_revision`，让渐进
   page walk 重开而不是对重新分配的 base 做 `Load`。
5. **能力钳制而不是初始化失败**：设置层接受 `off|2x|4x`，越界值拒绝；渲染器按适配器
   报告的支持集下调到“不超过请求的最高可用采样数”，并以稳定诊断
   `render.msaa.unsupported_samples: requested=<n> effective=<m>` 记录一次。显示质量选项
   不应因设备不支持而阻止启动。
6. **WebGL2 明确报告 1×**：`CanvasPresenter` 对 `Backend::Gl` 返回 `[1]`（wgpu-hal 强制
   canvas `antialias:false`，离屏 MSAA resolve 无真实浏览器证据），渲染器随即钳制并记录
   诊断，不静默假装生效。

## 2. 附件与 pass 拓扑

| 附件 | samples | 用途 |
|---|---|---|
| `cad-base-target` | `n` | base 颜色；仅 `n==1` 带 `COPY_SRC` |
| `cad-base-depth` | `n` | base 深度 |
| `cad-overlay-target` | `1` | `n==1` 时接收 base copy；`n>1` 时是 base pass 的 resolve 目标；随后是 overlay/mask 目标与 presented 帧 |
| `cad-overlay-depth` | `1` | overlay/mask 深度，每帧清空 |

- `n==1`：base pass（含 mask）→ copy base→overlay → overlay pass。
- `n>1`：base pass（不含 mask，`resolve_target=overlay`）→ overlay pass（先画 mask，
  再画 overlay 几何）。mask 与 overlay 使用 1× 管线集合；base 使用 `n×` 集合。
- pick 始终使用独立 1× 目标与管线；blit/present 始终 1×。

## 3. 管线

- base 几何族（line / instanced line / SDF arc / mesh ×4 / merged ×2 / image / textured
  ×4 / PBR ×12 / pulled ×3）按 `n` 创建。
- `n>1` 时额外创建 1× 的 overlay 几何族（line / instanced / arc / mesh ×4 / merged ×2 /
  image）；`n==1` 不重复编译，直接复用 base 族。
- mask 管线**永远** 1×；pick 管线永远 1×。
- 共享 shader 源；只有 sample count 变化时重建，不在帧路径重建。

## 4. 设置与载体

- `view.msaa.mode`：`off|2x|4x`，默认 `4x`；wire/`cad_set_setting` 键为 `msaaMode`。
- 宿主把 token 解析为 `ViewSnapshot.msaa_samples`（`1/2/4`），引擎建成
  `RenderTarget::samples`。
- 原生 presenter（Linux Wayland/X11、Windows、macOS、Android）在 attach 前把
  `supported_msaa_samples()` 交给会话；Web 的 `CanvasPresenter` 返回 `[1]`。
- 默认 `4x` 是常规 CAD/三维质量档；`2x` 供低端/高分辨率折中，`off` 供显存/性能优先。

## 5. 证据边界

- 已执行：离屏 lavapipe 上的 4× 帧非空、presented 纹理 `sample_count == 1`、4×→1×
  运行时切换、4× wipeout mask 边界等于 clear color（无 ghost edge）；1× 全套 headless
  回归通过。命令与结果见 `docs/validation-msaa-2026-11-22.md`。
- **NOT VERIFIED**：真实 GPU 的视觉质量/性能/显存、真实浏览器 WebGL2（已显式拒绝为 1×）、
  Windows/macOS/Android 运行时、真实 DPI/窗口合成的边缘表现。
- 2× 不在 WebGPU 规范保证集（保证 4×）；原生适配器是否支持 2× 由 presenter 查询，
  查询不到时按 `1×` 或 `n` 内不超过请求的最高值钳制。

## 6. 后果

- 优点：3D 模型边缘获得硬件 MSAA；mask 契约与 1 px UI 精确性不被破坏；1× 行为可回归；
  显存按 `2n+2` texel 计（base 颜色/深度 × n + presented 颜色/深度 × 1）。
- 代价：`n>1` 时多一套 1× overlay 几何管线；4× 使 base 附件约 4 倍；GPU 预算不足时
  仍以显式 `GpuMemory` 拒绝帧，而不是静默降级。
