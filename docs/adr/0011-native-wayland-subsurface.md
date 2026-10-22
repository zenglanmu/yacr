# ADR 0011：原生 Wayland 后端的 app-owned 子表面桥

- 状态：**已接受（Accepted，2026-10-16）**，范围**有界**：源码 + scoped 编译/契约 + 有界原生运行
  （`--probe`、CAD、合成输入、生命周期、几何修复、有界截图视觉 PASS、X11 运行回归）已执行；
  DPR>1/分数缩放、toplevel 替换、设备丢失、vendor/精度/性能仍 **NOT RUN**；**最终 Oracle（W3 attempt3）
  终判 ACCEPT-WITH-LIMITATIONS（无 blocker，有界本地验收：GNOME/Mutter + P620 + `scale=1`；非完整产品）**。
- 相关：`CAD_IMPLEMENTATION_SPEC.md` §21；`docs/react-linux-app.md`；`docs/architecture.md`；
  `docs/validation-native-wayland-2026-10-16.md`；`docs/handoff.md`；
  `.slim/deepwork/native-wayland/`（含 `design-inputs.md`、`gate-w1.md`、`gate-w2-attempt*.md`、
  `w2a-remediation.md`、`w2b-remediation.md`、`w2-native-probe.md`）。
- 前身：ADR 0010（React/Tauri 宿主取代 Slint）。ADR 0006/0010 描述的是**当时**的 X11/XWayland
  范围，作为历史保留，**不**被追溯改写为原生 Wayland 结论。

## 背景

`apps/app-linux`（React chrome + GTK 中央原生 CAD 分区 + renderer-only wgpu）此前**强制
`GDK_BACKEND=x11`**，把原生 Wayland 会话经 XWayland 处理。用户要求**真实原生 Wayland**（非
XWayland），同时**保留 X11 支持**、不做 UI 重设计、不走 CPU 帧拷贝的迂回方案。

难点：CAD 分区需要一个**独立的原生表面**，而 GDK toplevel 的 `wl_surface` 由 GDK 拥有并提交，
**不能**在上面挂 Vulkan swapchain（会覆盖 chrome，且与 GDK 双写）。把 `GtkDrawingArea` 提升为
native（`gdk_window_ensure_native`）在 Wayland 上会创建错误的独立窗口/表面，**禁止**。GDK 也不
公开 `wl_subcompositor` 的 getter。

## 决策

**在原生 Wayland 上，CAD 分区使用 app-owned 子表面：宿主自建一个 child `wl_surface` +
`wl_subsurface`，父级是 GDK toplevel 的 `wl_surface`；渲染器只接收该 child 的 live Wayland
raw handles。X11 走既有的原生 X child 窗口路径，二者行为互不改写。**

关键点：

1. **私有小 C protocol shim**：`apps/app-linux/native/wayland_shim.{c,h}` 用系统
   `wayland-client-protocol.h` 包装（libwayland 的协议 wrapper 是头文件内联函数，无动态导出）。
   shim 借用 GDK 的 `wl_display`，**不**新建连接、**不** `dispatch`/`roundtrip`/`flush`；
   **GTK 是唯一的事件分发者**（Default Queue）。
2. **异步 registry + discovery 屏障**：shim 在 registry 请求后追加 `wl_display_sync`，其 `done`
   表示初始 global 已全部送达。回调代理在自身 `done` 内销毁（并在 `free` 兜底），**先于** userdata 释放。
   global 缺失/版本过老（`wl_compositor < v3`）是**显式失败**，**不会**永远 Pending。
3. **只绑定 owned `wl_compositor`/`wl_subcompositor`**（版本校验）；借用的父 `wl_surface` 只作
   subsurface 父级，**不**被提交/缩放/改 input region/销毁。child 与父 **必须不同**（C 硬检查 +
   Rust debug assert）。
4. **child 配置**：`set_desync`；**空 input region**（使输入穿透到 DOM；`NULL` 表示全表面输入，禁止）；
   **整数 `buffer_scale`**；logical `set_position`。
5. **父级 commit 语义**：`wl_subsurface.set_position` 是**父级**相对的双缓冲状态，在下一次**父级**
   commit 时生效；宿主只请求 GTK toplevel `queue_draw`，**从不**自行提交 GDK surface。child 的
   buffer/scale 由 Vulkan present 提交。
6. **生命周期不变量**：释放顺序为 **presenter → subsurface → child surface**，且必须**早于** GTK
   parent unrealize；`Ready` 时每帧/每次 resize 前做**原生健康检查**（shim/display 错误 + 当前 GDK
   父 `wl_surface` 身份），不健康则**有序释放**、绝不提交陈旧帧；父表面被替换时非破坏性检测，释放后
   重建并递增 generation。
7. **后端策略**：`apps/app-linux/src/backend.rs` 显式建模 `GDK_BACKEND`——`Auto` 优先原生 Wayland
   （需 `WAYLAND_DISPLAY` + `XDG_RUNTIME_DIR`），否则 X11/XWayland；显式 `x11`/`wayland` 保留；
   非法/歧义值硬错；**无静默回退**。选择意图与**实际**后端/协议证据分开记录。
8. **边界不变**：`cad-host` 仍不依赖任何 UI 工具包；`cad-ui-core` 仍 UI/GPU 无关；renderer-only
   wgpu、importer-only acadrust 不变；C shim 与 GDK Wayland 访问只存在于 `app-linux`，**不**进入
   `cad-*`。
9. **构造期 panic 边界（有界、诚实）**：wgpu-hal 30 在 `vkCreateWaylandSurfaceKHR` 失败时是
   `expect`（panic 而非 `Err`），故对**整段** presenter 构造做有界 `catch_unwind`，诊断如实写
   `presenter construction panicked: <有界 payload>`，终态不重试；仅在默认 unwind（非
   `panic = "abort"`）下有效。

## 备选方案

- **GDK-owned surface + Vulkan**：在 GDK toplevel/child `wl_surface` 上直接建 swapchain——与 GDK 双写、
  覆盖 chrome，**否决**。
- **`gdk_window_ensure_native`**：Wayland 上会生成错误的独立窗口/表面，**否决**（仅 X11 使用）。
- **CMake/meson 或引入 wayland-rs 生成绑定**：需要额外对象/队列模型与依赖审计；本轮采用“私有 C shim”
  以满足最小、可控、无新依赖的要求（`cc`/`pkg-config` 已锁定）。
- **在 GTK 默认队列自行 `dispatch`**：与 GTK 争用队列，**否决**；改由 GTK 独占分发。

## 后果与权衡

- 优点：真实原生 Wayland；chrome 与 CAD 分区 co-visible；**无 CPU 帧拷贝**；X11 路径保持；边界不反转。
- 代价/限制：需要 `libwayland-dev`（`cc` + `pkg-config wayland-client`）；本 shim **仅在 Linux 宿主
  编译**（从非 Linux 宿主交叉构建到 Linux：原生 Wayland shim **不支持**，`build.rs` 会显式警告）；
  GTK3 只有整数缩放，**DPR>1/分数缩放未验证**；真实合成器行为已有**有界**运行时证据（子表面可见性由
  同 UID Mutter `ScreenCast` 截图佐证、输入经合成输入验证、discovery/几何已修），但**源码批准不等于验收**，
  且证据范围有界（见下）。

## 2026-10-23 增补：合成方向反转为 DOM-on-top（生产语义）

保留以上历史决策与验证事实。以下变更由 [ADR 0023](0023-dom-on-top-cross-host-composition.md) D1 决定，
**取代**本 ADR 的“默认排列”语义（子表面机制的其余契约不变）：

- **决策 4 的空 input region 保留且仍是防御纵深**（输入已全部走 DOM 单一漏斗；空 region 保证 CAD 不会
  吞事件后无人转发）。
- **`place_below` 无条件化**：`yacr_wl_shim_create_child` 现在**总是**执行
  `wl_subsurface_place_below(subsurface, parent)`（DOM/WebView 在上、CAD 子表面在下）。旧的
  above-parent 排列与开关**已删除**，**无双模式**、无运行时切换。
- **决策 5 的父 commit 语义继续成立**，并成为生产不变量：`place_below` 与 `set_position` 一样是
  父级相对、double-buffered 的子表面状态，随**父表面 commit** 生效；宿主只 `queue_draw`，从不自行
  提交 GDK surface。
- **generation 重放**：`create_child` 在每个 surface generation 重建时都会重放 `place_below`，因此
  unrealize/realize、minimize/restore、父表面替换后的重建仍成立。注意边界：**N>1 次 generation 的
  运行期重放本轮仅代码路径验证（runtime N>1 NOT RUN）**，不得视为运行时已验证。
- **顶层 alpha 是生产必需**：GTK toplevel 必须 `set_app_paintable(true)`、在 draw 中
  `set_opaque_region(None)` 并用 `Operator::Source` 透明清屏，否则 `place_below` 的 CAD 永远不可见
  （`gtk_bridge/owner.rs::install_parent_alpha_clear`，无条件执行）。
- **决策 7 的后端策略被收紧**：原生 Wayland 仍是生产路径；**X11/XWayland 改为显式 Unsupported**
  （GTK/Tauri 初始化前拒绝，稳定标记 `yacr: unsupported-backend: x11` + 退出码 4，**无静默回退**）。
  本 ADR 中“X11 走既有的原生 X child 窗口路径”的描述**仅作历史记录**；该路径编译/测试保留但运行时
  不可达（注释 `unreachable_since_dom_on_top`）。
- **证据**：opt-in 探针（2026-10-22）见 `docs/validation-dom-on-top-linux-2026-10-22.md`；生产迁移的
  有界证据见 `docs/validation-dom-on-top-production-2026-10-23.md`（GNOME/Mutter + P620 + `scale=1`）。

## 验证状态（诚实，截至 2026-10-16）

- **已执行**：scoped `cargo check -p app-linux --all-targets`、`cargo test --lib`（`app-linux` **58/58**、
  `cad-ui-core` **61/61**）、clippy（0 warning）、wasm32 cfg 隔离、`check-architecture.py`、`check-i18n.py`、
  scoped `fmt`；**有界真实原生 Wayland `--probe`**；原生 **CAD `--open`**（4 实体）+ **9 个合成 DOM 输入**
  （全 `accepted`，相机实际改变）+ resize；强制 **overlay** 生命周期释放/重建/恢复（**非** toplevel 替换）；
  **几何修复**后子缓冲与 DOM CAD 矩形一致（`926x266`/`1166x513`，margins `34/84`，最终二进制 sha256
  `7618d0d2…`）；**Observer 终态：有界截图视觉 PASS**（同 UID Mutter `ScreenCast` 捕获，无安全策略变更/无安装/
  无 sudo）；**X11 运行回归 PASS**（真实 XID，自然退出 0）。退出码按 case：原生 baseline 捕获后 SIGTERM **143**，
  resized 与 X11 **自然 0**。证据见 `docs/validation-native-wayland-2026-10-16.md`。
- **未执行 / 未支持**：DPR>1/分数缩放、GTK **toplevel** 替换、GPU 设备丢失、
  vendor/adapter 广度、几何/像素精度、性能/60fps、跨平台、打包、vendor/golden。**最终 Oracle W3 attempt3：
  ACCEPT-WITH-LIMITATIONS（无 blocker，有界本地验收 GNOME/Mutter + P620 + `scale=1`，非完整产品）；X11 视觉
  未由 observer 判读。** **不得**由本 ADR 推断“完整原生 Wayland CAD 支持”。
