# React 原生 Linux 宿主（实验，bounded prototype）

状态：**实验**（用户授权 React 替代 Slint 的桌面路线）。当前为 **bounded prototype**，
**不是完整 CAD 产品**，也不代表跨平台验收。契约见
[`docs/adr/0006-react-native-linux-experiment.md`](adr/0006-react-native-linux-experiment.md)
与 `CAD_IMPLEMENTATION_SPEC.md` §21。

入口：`scripts/run-react-linux.sh`（在仓库任意目录调用；脚本用 git 定位根、路径加引号、
`set -euo pipefail`）。

## 构建依赖（系统包需预先安装，脚本不自动安装）

- **WebKitGTK 4.1 开发包**（`libwebkit2gtk-4.1-dev`）与 **GTK3 开发包**（`libgtk-3-dev`）；
  `pkg-config --exists webkit2gtk-4.1 gtk+-3.0` 应通过。
- **Node + npm**（用于 Vite 构建 `apps/app-react-linux/ui`）。
- Rust 工具链按仓库 `rust-toolchain.toml`。
- 默认 `debug` 构建即可快速运行；**release 构建为可选**（更慢，仅在需要时使用）。
- **不**自动安装系统依赖；缺失时脚本报错退出。

## 启动

当前入口 `scripts/run-react-linux.sh` 默认 **`--release`**（用户自改并保留，本文档不改动该脚本）；
`debug` 快速运行可用直接 cargo 命令（可选）。脚本会：检查 `DISPLAY` 并**始终强制
`GDK_BACKEND=x11`**（宿主拒绝其它后端，无静默回退）；`node_modules` 不存在时先 `npm ci`，
再 `npm run build`；最后以 `custom-protocol` 运行宿主（嵌入 `ui/dist`）。脚本**不解锁会话、不截图**。

```bash
bash scripts/run-react-linux.sh --open "$(pwd)/fixtures/dwg/synthetic-four-lines.dwg" --fonts-dir "$(pwd)/fonts"
```

参数（透传给宿主）：`--open <path>`、`--fonts-dir <path>`、`--images-dir <path>`、`--probe`。

## 显示与窗口默认

- UI chrome 中性灰 **`#dedede`**；CAD 区域由真实 GPU **clear 为纯黑 `#000000`**（不透明，模型/纸
  空间默认一致），**不存在**可覆盖的旧背景偏好。
- 默认窗口**最大化**：在 helper 介入前即验证 `_NET_WM_STATE = MAXIMIZED_HORZ, MAXIMIZED_VERT`
  （`fallback_maximize_used=false`），尺寸 **1853×1011**。
- 可见 FPS 为**真实计数**：以 `delta frameCount / performanceNow` 计算、rolling **2s**、warmup
  **1s**；`reset`/`stopped`/`error` 时显示**不可用**。**不是**占位 React 动画。

## 导航合同（窄 2D fastpath）

- 导航复用**同一权威底图**：GPU-resident、**同一 DB revision**（fonts/images/layers/space 一致），
  **Ready fastpath 取最新 camera**，**不展示旧 doc**。
- 拖拽期间 overlay 后缀**临时省略**，以显式 `FrameCompleteness::BaseOnlyPendingOverlays` 标记
  （**非静默**）；`PresentedFrame` 元数据带 `view_revision`（= 实际 camera）；`CurrentScene` 表示
  **GPU 已提交的当前场景**，**不是**渐进式全像素。当前 drawing 严格标记 `presented=false` + `baseOnly`。
- 严格模式：CPU ready 但 **GPU 未提交**时返回 `None`，**不返回旧底图**；restore 后同一底图 overlay
  提交（静态 base prefix 保留）；**device change 阻塞直到新上传**；**3D 严格路径不变**。
- **用户可见取舍**：拖拽期间 transient grid/axes/highlights/snaps/previews 可能**暂时隐藏**，直至
  prepared 恢复（<30ms 窗口测量，**非精确延迟**声明）。

## 性能与测量（2026-10-14，P620/Vulkan）

- 基线 `/tmp/opencode/yacr-react-baseline-perf-20261014T125447Z`（source hash locked）与 postfix
  `/tmp/opencode/yacr-react-postfix-perf-20261014T135021Z`：同 4s、494 events、~120Hz，
  fourline/flange 的 drag waiting **65.3%/67.8% → 0.0%/0.0%**；presented/s **21.0/19.5 → 60.7/59.9**。
- `view_current == view_rendered` 全部 **709/701 帧 0 mismatch**；post-release waiting **0**、无
  history、最终位置稳定（148/146 帧）；post-restore overlay 在 **T+30ms 采样窗口**内回到
  `current_scene`（**无精确延迟**声明）。
- `input_age_us`（Rust admission→drain）p95 **15.722ms / 15.022ms**（**非 input→visible**）；
  scope pop p50/max **1/3µs**；frame p95 **0.873ms / 3.132ms**；UI FPS **60/61**。
- **input→visible p95 < 50ms UNVERIFIED**：portal 仅 ≤3 张静态截图，**非真实视频捕获**；
  不宣称无性能回归，也不宣称 full visual golden（vendor 仍 `Partial`）。

## 支持范围

- **OPEN / FIT 已支持**：`--open` 经真实文件打开漏斗；OPEN 也可走原生 portal picker。
- **LINE 等其它绘图/编辑命令不支持**；未知命令走既有 **not supported** 路径，不空成功。
- **只读**：不写回 DWG；其它 UI 能力未接。用户可见文案来自现有
  `crates/cad-ui-slint/i18n/{zh-CN,en}.json`，不在 UI 硬编码中文。

## 坐标与度量契约

布局以**实际 DOM bounds** 为准（widget margins 取实际 DOM，而非固定常量）；
坐标区分 **CSS / canvas-local / world / physical** 四类，配合 **DPR** 与 **metrics revision**，
度量变化统一更新映射。该契约在 release 与**跨平台未来实现**中沿用，仍引用 RapidRAW 固定
commit `74597d066685283588502edbecd747bfe80404cf` 作为 Win/macOS 透明 webview 叠层的参照
（**参照不构成本项目实现或验收**）。

## 原生文件选择器（portal helper）与授权

`OPEN` 使用桌面 **native portal picker**。该路径需要一个**已授权的 portal helper**，使用
**transient**（一次性）工具，并要求**用户可观察**的交互确认；它**不修改默认权限**——
脚本与文档**不**对 `PermissionStore`/portal 做常驻授权变更。运行需要真实显示与桌面会话；
**无显示的 SSH/无头环境不受支持**。

## 限制（NOT RUN / 不夸大）

- 视觉：synthetic four-line 为 basic smoke；flange 视觉 **PARTIAL**（主几何可见、无明显翻转，
  但布局与参考有差异：缺 paper frame / title block 右侧细节、大量文字），**不构成
  full visual golden / vendor DWG** 结论。
- **实际 CAD presented/s 实测 ~60**（见性能节），但**不据此宣称产品级 60fps 或全场景无回归**；
  `input→visible` 未验（见上）。
- popup 跨 CAD 覆盖 **Unsupported**；**无 Android/Web 迁移**。
- 生命周期：minimize→restore 仅经**真实 WM 激活**路径通过；Tauri `unminimize()` API 请求在
  该 XWayland 上**未被采纳**（限制）。
- **API/字段不承诺**：本文档不为未支持平台（Win/macOS 仍 future）承诺字段；跨平台沿用同一契约，
  仍引用 RapidRAW 固定版本，**Linux 无 readback**。
- 不承诺跨语言稳定 IPC（仅 bounded 实验 adapter）；不扩张系统权限。
- 设备 fastpath 测试**不等于**平台/全产品验收。

## 隐私 / 锁状态

截图/证据**仅本地保存，未上传**。脚本不改变会话锁状态；证据轮次保留原有
unknown/desktop 锁状态（最近一次启动为 unlocked 则保持 unlocked，**不得声称 always locked**）。
