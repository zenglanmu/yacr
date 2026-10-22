# DOM 位于原生 CAD 之上的 Linux 有界探针（2026-10-22）

> **2026-10-23 注记**：本文记录的是**已退役的 opt-in 探针阶段**。`YACR_PROBE_DOM_ON_TOP` 与
> `window.__YACR_PROBE_DOM_ON_TOP__` 已随生产迁移**删除**（见
> [ADR 0023](adr/0023-dom-on-top-cross-host-composition.md) §7）。文中的启动命令仅作历史复现
> 说明，**不是**现行操作指引；现行回归夹具与生产路径见
> [react-linux-app.md](react-linux-app.md)，本轮生产证据见
> [validation-dom-on-top-production-2026-10-23.md](validation-dom-on-top-production-2026-10-23.md)。

## 结论与范围

**仅显式 opt-in 探针，不是生产合成迁移或双后端替代验收。** 默认产品仍采用 CAD 在 WebView
之上的分区安排。本次结果说明该顺序不是 GTK/Linux 的必然要求，但不能由 Wayland 成功推出
X11 同样可行，也不能由本次 X11 失败推出 X11 不可能。

| 路径 | 本轮结论 |
| --- | --- |
| 原生 Wayland | Observer 对 initial/popup/after 三张合成帧判定：不透明按钮遮挡、左侧半透明面板与标记混合、弹窗跨 chrome/CAD 边界、采样标记运动 **PASS**；不是完整产品验收 |
| X11（Mutter 下 XWayland） | 原生 sibling 顺序已建立，但 **合成 FAIL**：透明区域显示后方另一 CAD 应用，不是本探针的 CAD 标记；最终独立 Observer 复核确认 FAIL，取代此前 LIMITED 判断 |

真实环境限于同 UID 的现有 GNOME/Mutter Wayland 会话、`scale=1`，CAD presenter 为 **Quadro P620 /
Vulkan / discrete_gpu**；另保存了 Intel i915 DRM fd 与 NVIDIA 进程证据。独立 Xorg、其它合成器、
DPI>1/分数缩放、其它 GPU/vendor：**NOT RUN**。无抗锯齿/平滑质量、性能或 CAD 精度结论。

## 实现与启动契约

- 仅 `YACR_PROBE_DOM_ON_TOP=1` 启用（其它值/未设置均不启用），且必须显式带 `--probe`；
  否则在 GTK 初始化前拒绝。`YACR_PROBE_ONLY` 不能替代该 CLI 要求。
- 宿主在页面脚本执行前注入 `window.__YACR_PROBE_DOM_ON_TOP__ = true`，前端只接受严格布尔值
  `true`；普通启动不启用探针。保持原 marker presenter、resize/输入准入，不伪造就绪。
- UI 只保留一个透明 `.native-input`，复用既有绑定；DOM 控件是其上方 sibling，header116px、
  footer140px、面板位于 CAD 高度50%、中央不透明按钮，局部弹窗 top82px。
- Wayland：仍使用与借用的 GDK 父表面不同的 app-owned CAD child；仅探针请求
  `wl_subsurface.place_below(parent)`，保留 desync、空 input region、原有释放顺序；父 commit
  通过 GTK invalidation/queue_draw 请求，不向 GDK 父表面提交 Vulkan buffer。
- X11：沿用重挂 GTK 层级，尝试提升 WebView 的有效 GDK native window，并用 XQueryTree 验证
  它与 CAD 是不同原生 sibling 且 WebView 在上。不能建立时显式记录 Unsupported；
  **建立 stacking 不等于合成成功**。日志保留 `composite=NOT_VERIFIED`。
- 探针通过 GTK draw 请求透明清理及清除 opaque region，并记录 visual/depth；这不是实际 alpha
  像素验证。Wayland 当前 opaque-region 协议状态不可查询；最终合成要看合成器图像。
- `window.__yacr_dom_on_top_probe` 提供 mounted/popup/lastIpcError、事件 counters 和 ipc 的
  submitted/resolved/accepted/refused/unrecognized/lastAck/lastRefusalReason；**transport resolved
  不是 admission accepted**。本轮未通过任意 eval 接口直接读取该对象。

本轮在前端更新后执行 `cargo build -p app-linux --bin yacr --locked`。**该 debug 构建没有启用
custom-protocol，实际页面是现有 Vite `http://localhost:1420/`，不是嵌入的 `ui/dist`**；已保存并核对
实际服务的 main/Probe/CSS，包含最新准入诊断、50% 面板和140px footer。`npm run build` 通过不能
被写成这些截图使用了嵌入资源。复测应在当前用户真实图形会话中，先在另一终端运行：

```bash
(cd apps/app-linux/ui && npm run dev)
cargo build -p app-linux --bin yacr --locked
GDK_BACKEND=x11 YACR_PROBE_DOM_ON_TOP=1 ./target/debug/yacr --probe
GDK_BACKEND=wayland YACR_PROBE_DOM_ON_TOP=1 ./target/debug/yacr --probe
```

两次启动应顺序进行，并使用该用户实际 DISPLAY/XAUTHORITY/WAYLAND_DISPLAY/XDG_RUNTIME_DIR；不要
照搬他人会话凭据。正式嵌入资源路线仍见 [宿主文档](react-linux-app.md) 的 custom-protocol 说明。

## 图像与来源核对

运行根目录（本机临时证据，不是可分发资源）：
`/tmp/opencode/yacr-dom-on-top-20261022T043353Z/`。总记录 `RESULTS.txt`，逐次启动与时间戳在
各目录 `launch.json`/`events.json`，主日志 `host.log`；X11 来源调查见
`x11-attempt2/FINDINGS.txt`、`source-correlation.json`、`pixel-correlation.json`。

**Wayland**：`wayland/{initial,popup,after-synthetic-input}.png` 为实际 Mutter ScreenCast 合成帧。
Observer 确认实际背景是浅蓝灰色（不是黑色）；橙色标记采样位置约 x549→919→225，左面板混合标记、
中央按钮遮挡以及跨边界弹窗均可见。协议日志仅报告 below-parent 请求；**图像才提供实际合成证据**。

**X11**：`x11-interactions/` 三图初看 DOM 控件在上，但其黑底静态白色线条和细黄色竖线不属于
marker 夹具，故不能算 DOM-over-native-CAD 通过。调查确认后方现存应用 PID **3186773**、
top XID **0xc00003**，命令为 `./target/release/yacr --open fixtures/dwg/synthetic-four-lines.dwg --fonts-dir fonts`。
新探针 PID **3424178**，实际 cmdline 为 `target/debug/yacr --probe`，top/CAD/WebView 分别为
**0x1800003 / 0x1800020 / 0x1800021**。attempt2 使用 `YACR_PROBE_NO_MAXIMIZE=1`，无生命周期驱动；
CAD 全局矩形为 `(513,391,960,344)`，depth32/TrueColor，原生 sibling 顺序验证成立。

`x11-attempt2/` 保存启动前、三张探针帧、仅最小化新探针后的背景、退出后的合成帧，以及辅助的
CAD/WebView/parent drawable 抓取和背景应用自身 drawable。之前三张 X11 图在同一全局区域各有
**50,000/50,000 像素**精确匹配背景；attempt2 清晰条带也包括黄色线条逐像素匹配；退出后原 CAD
区域 **330,240/330,240 像素**与启动前相同。由此确认是**另一应用透出**，不是错显示器或误启动
普通 CAD。遮挡中的 CAD child 的 XGetImage 为黑色，但其像素可未定义，**不能**据此诊断 renderer
clear 或 stale swapchain。具体 X11 clipping/重定向/缓冲机制尚未闭环。

最终独立 Observer 对 attempt2 的图像复核确认：屏幕坐标对齐的黄色线 x885 与白色线 x1367
延续到探针窗口外的背景，预期 marker/grid 缺失。**不透明控件对自身 native CAD 的遮挡、半透明
混合、弹窗对自身 native CAD 的合成均 FAIL**；DOM 自身可见不算通过。由于缺失预期标记，运动
结论为 **INDETERMINATE**（不是已证明标记静止）。该终判取代早先 LIMITED；Wayland 的有界 PASS
不变。此图像结论与像素来源核对一致，但不补全具体 clipping 根因。

本轮没有尝试离屏重集成、CPU 回读、纹理共享或其它“救援架构”。若继续 X11，需要新的有界设计/
证据步骤，不是只改 CSS 或翻转 sibling 顺序；当前实验失败可保留为显式失败。

## 输入、生命周期与安全边界

- 既有驱动的**合成** resize、pointer down/move/up、wheel 经真实 IPC，宿主日志记录 accepted；
  AT-SPI 的**合成 accessibility action**打开局部弹窗并让按钮计数0→1，不证明物理命中/阻断路由。
  真实鼠标键盘输入、直接 diagnostics 对象回读、完整 frontend counters 读取：**NOT RUN**。
- 默认最大化；resize API 返回 Ok，但未建立所请求1200×800/900×560的实际 client size 结论。
  X11 最小化后 frames 停在692，直到关闭仍未恢复。Wayland 在后续 forced unrealize/realize
  后新 generation 又有呈现，**不是普通 unminimize 成功证明**。`restored/resized/recreated`
  只是截图文件名，不是 PASS 标签；重复最小化后的截图可能只含桌面。
- 两个 full 运行通过自然 window.close 退出0；X11 补充 interaction 与 attempt2 是计划 SIGTERM
  清理（-15），不是崩溃或自然关闭证据。只清理本轮启动的进程组；背景应用未被操作。
- 合成截图是**整屏**，可能含其它窗口；**仅本地私密审阅，不上传**。未安装包、提权、创建虚拟显示、
  解锁或修改截图权限；现有同 UID 会话本来已解锁。ScreenCast 会话均关闭，本轮进程组已清理。

## 源码验证记录与未运行范围

本轮实现阶段已通过：改动 Rust 文件 fmt、`cargo test -p app-linux --lib --locked` **160 passed**、
`cargo check -p app-linux --locked`、上述宿主 build；前端探针契约 **6 passed**、`npm run build`、
i18n 检查。文档 diff 检查见本轮交接。测试是合成/契约证据，不扩大运行结论。
按日常有界策略，**全 workspace 测试/门禁、平台矩阵 NOT RUN**；attempt2 最终独立图像终判已补入
上文，最终集成验证由父会话负责。本报告不替代此前迁移/产品验收，也不更改默认合成策略。
