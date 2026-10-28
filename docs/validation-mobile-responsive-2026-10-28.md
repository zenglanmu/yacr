# 移动/响应式 chrome 与浏览器载体验证（2026-10-28）

> 范围：`apps/desktop-ui` 的**响应式单 React 树**（桌面/移动两种呈现）、`src/host/` 的宿主传输缝、
> 以及 `apps/app-web/dist-ui` 浏览器 bundle 的**布局/DOM 验收**。
> **证据上限**：headless Chromium + **SwiftShader 软件渲染**，断言的是布局、DOM 与「画布非纯色」。
> **不是** GPU/像素正确性/性能验收，也**不是**「移动端已验收」。NOT VERIFIED 清单见 §8。

## 1. 结论（有界）

| 项目 | 结果 |
|---|---|
| 响应式布局切换 | **成立**：`data-layout` 在 390/768/914/1024 为 `mobile`，在 1025/1280 为 `desktop`；**live resize 双向翻转无需 reload**（含 1 px 门槛 `1025 ↔ 1024`） |
| 移动端 chrome 组成 | **成立**：`.mobile-topbar` + `data-layout` 驱动的抽屉 + 全幅 `Workspace` + `.mobile-commandbar`；顶层无 `.ribbon`/无 `.statusbar` |
| CAD 分区占比 | **成立（断言级）**：移动端 `.native-input` 满宽且 ≥ **70 %** 视口面积；桌面端 band + status bar 回归 |
| canvas 出帧 | **成立（非纯色）**：以**截图**解码统计画布矩形颜色数 > 1；backing store = CSS × DPR（±1 px） |
| 浏览器套件 | **PASS 7/7 cases（141/141 checks），退出码 0** |

单源（不要在多处写阈值）：`apps/desktop-ui/src/layout/layout-mode.ts` 的
`MOBILE_MAX_WIDTH = 1024` 与 `MOBILE_MEDIA_QUERY = '(max-width: 1024px)'`（`max-width` 含端点，
故 1024 是最后一个 mobile 宽度）；`main.shell` 与 `<html>` 都带 `data-layout="mobile" | "desktop"`，
由 `matchMedia` 的 live 订阅驱动，**resize 即时翻转、不重载页面**。

## 2. 架构要点（本轮记录的对象）

- **一套组件、两种呈现**：移动端不是第二套 UI。差异只在呈现层：`.mobile-topbar`（最左菜单按钮
  44×44，带 `aria-expanded`/`aria-controls`）→ ribbon **drawer**（in-DOM 覆盖层；标签 chip +
  2 列触摸网格；≤ `68dvh`，body 可滚动并带渐隐/滚动条；同一原因拒绝整片条目时只给**一条面板级**
  拒绝说明；目标尺寸 44 px）+ 全幅 `Workspace` + `.mobile-commandbar`（唯一底部组件）。
  移动端**不渲染**标题栏、桌面 ribbon 带、文档标签、布局标签、状态栏、dock/导航栏。
- **命令行逻辑只抽一次**：`apps/desktop-ui/src/commands/command-line-model.ts` +
  `use-command-line.ts` + `components/CommandField.tsx`；桌面 `CommandLine.tsx` 与
  `MobileCommandBar.tsx` 都从它渲染，规则不重复。
- **宿主传输缝**（`apps/desktop-ui/src/host/`）：构建期运行时常量 `__YACR_RUNTIME__` ∈
  `tauri | web | android`（Vite `define` 注入并校验，默认 `tauri`）；`invoke.ts` 是**唯一** import
  `@tauri-apps/api` 的模块；`runtime.ts` 决定路由；`web-adapter.ts` 把 `cad_status` /
  `cad_commands` / `cad_command_aliases` / `cad_command` / `canvas_session_status` /
  `canvas_runtime_failed` 映射到 wasm 会话，**其余命令一律以 `unsupported-runtime: <command>` 拒绝**
  （绝不空成功）。
- **载体能力诚实**：wasm 会话的 `shell_commands` 返回它**真正执行**的三个相机选择器
  `["FIT","ZOOMIN","ZOOMOUT"]`，`shell_aliases` 只返回目标在其中的桌面别名行
  （`ZOOM EXTENTS → FIT`）；`OPEN` 由 chrome 侧传输缝实现（文件选择器 + `open_bytes`），
  因此 `cad_commands` = 会话列表 + `OPEN`，`cad_status.canOpen` 由适配器抬成 `true`。
  其余名字仍一律 `<…> is not executable` 拒绝（`apps/app-web/src/shell.rs`）。
  **2026-10-28 更早一轮**的记载（两份列表都为 `[]`、chrome 禁用每个命令入口）描述的是
  接线之前的载体；当时的实测输出保留在本文件下文，未被改写。接线后的证据见
  `docs/validation-web-commands-2026-10-28.md`。浏览器/移动载体仍是**看 + 导航**
  （open/pan/zoom/resize），不是可编辑的 CAD 产品。
- **Android 宿主内边距**：`--yacr-safe-bottom`（CSS px）由 Android 宿主发布，`.mobile-commandbar`
  唯一消费；实测与边界见 `docs/android-host.md`。

## 3. 环境与命令（本轮实际运行）

```bash
bash scripts/build-web-ui.sh                 # → apps/app-web/dist-ui（chrome bundle + canvas/）
python3 scripts/serve-web.py --directory apps/app-web/dist-ui      # http://127.0.0.1:8090/
NODE_PATH=<playwright 的 node_modules> \
  node scripts/web-ui-smoke.cjs --url http://127.0.0.1:8090/ --verbose \
       --evidence /tmp/opencode/yacr-mobile-web-<UTC>Z
```

- 脚本自身也会起停 `scripts/serve-web.py`（显式 `--url` 时用已起的服务器）；`playwright` **不是**
  仓库依赖，用 `NODE_PATH` 指向仓库外的安装。
- 运行时：headless **Chromium 156.0.8078.4**，`--use-angle=swiftshader` ⇒ **软件光栅化**（脚本在
  汇总行里打印 `rendering=software` 提示）。**布局/DOM 证据**，不是 GPU/像素/性能证据。
- `apps/app-web/dist` 是**另一份**产物（旧的独立 vanilla harness，含 Worker 隔离），本轮**未触动**；
  `dist-ui` 只由 `build-web-ui.sh` 产出。

## 4. 逐 case 结果

| 视口（CSS px） | 期望布局 | checks | 结果 |
|---|---|---|---|
| 390×844（手机竖屏） | mobile | 29/29 | PASS |
| 768×1024（旧阈值宽度） | mobile | 18/18 | PASS |
| 914×412（同一手机的横屏） | mobile | 18/18 | PASS |
| 1024×900（最后一个 mobile 宽度，4:3 平板竖屏） | mobile | 18/18 | PASS |
| 1025×900（门槛后第一个宽度） | desktop | 14/14 | PASS |
| 1280×800（桌面窗口） | desktop | 14/14 | PASS |
| live-resize `1280×800 ↔ 390×844` 与 `1025×900 ↔ 1024×900` | 双向翻转 | 30/30 | PASS |

汇总行：**PASS 7/7 cases（141/141 checks）**，退出码 0。

**CAD 分区占比**：断言是「满宽且 ≥ 70 % 视口面积」。本轮**只保留通过结论**，未把每 case 的实测
百分比抄进本记录（`--verbose` 会打印带实测值的 check 明细）；不在此编造数字。canvas backing 断言为
`CSS 尺寸 × devicePixelRatio`（±1 px），且画布矩形**不是纯色**（比较的是**截图**像素，不是
`readPixels`）。

## 5. 断言做了什么、不做什么

**断言（`scripts/web-ui-smoke.cjs` / `scripts/canvas-pixels.cjs`）**：

- 页面加载无未捕获错误、无失败请求；不存在可见的 `[role="alert"]` 错误横幅（**断言其不存在**，
  不是把它压掉）。
- `data-layout` 在**两个属主**（`html` 与 `main.shell`）上一致。
- 移动端只有紧凑 chrome、桌面端 band/status bar 回归；CAD 分区占比与整宽。
- 抽屉：从顶栏按钮打开（`aria-expanded` 翻转）、切 tab chip、Escape 关闭；键入文本进入**共享**命令草稿
  （输入框回显 + 补全面板按它过滤）。
- live resize：在 resize 前设置的 `window.__yacrProbe` 标记在两次跨界后**仍存在** ⇒ 无 reload。
- canvas：backing = CSS×DPR（±1 px）；**画布非纯色**（这条是 Android present 缺陷的回归守卫，
  见 `docs/validation-android-present-2026-10-28.md`）。

**不做**：不证明几何/相机/抗锯齿/颜色正确，不证明「画面对」；截图只是调试产物，不是视觉验收；
不触碰原生宿主（Tauri/React 桌面窗口、Wayland 子表面、Win32 child HWND 完全未参与）。

## 6. 本轮顺带修掉的缺陷：永久 `probe_input` 错误横幅

**症状（既有缺陷，非本轮引入）**：浏览器载体上一加载就出现永久错误横幅，内容为
`unsupported-runtime: probe_input`（浏览器没有 Tauri 桥，`probe_input` 无处可去）。

**根因**：`native-below` 的输入通道在**载体尚未确定**时就被创建，并在被 dispose 之后仍把一次在途
拒绝报给 UI。**修复**：`apps/desktop-ui/src/cad/input.ts` 与 `App.tsx` 的 `nativeInputLane` —— 通道
只在载体确定为 native-below 后创建，已 dispose 的通道不再上报在途拒绝。修复后横幅不存在，
套件以**断言横幅缺席**的方式守护（语义仍是「显式拒绝」，不是静默回退）。

## 7. 证据位置约定

- 产物在 `--evidence` 指定目录（约定 `/tmp/opencode/yacr-mobile-web-<UTC>Z/`），**不是**持久证据、
  **不上传**：`390.png`、`390-drawer.png`、各视口 `<width>.png`、`resize-390.png`、
  `resize-back-1280.png`，以及像素测量写下的 `canvas-<name>.png`。
- 目录丢失不影响可复现性：按 §3 的命令**重跑**即可重建；本记录只固定命令、环境与结论。

## 8. NOT VERIFIED（必须照写）

- **真实 GPU**：未验证。本轮是 `--use-angle=swiftshader` **软件渲染**，不得用作 GPU/性能结论。
- **像素/视觉正确性**：只证明「画布上出现过**非纯色**的一帧」；颜色空间、几何、相机、抗锯齿、
  DPR≠1 的像素级等价**均未测**。合成的四线夹具也没有 golden。
- **色彩空间**：线性 present 目标 + blit 内 sRGB 编码与 sRGB surface 路径**按构造等价**，但**没有**
  中灰像素对比证据（夹具是纯黑底 + 纯白线，对编码不变）。
- **其它浏览器 / 其它 Chromium 版本**：只跑过 Chromium 156.0.8078.4（SwiftShader）。
  **WebKitGTK 未跑该 7-case 套件**，Firefox/Safari、真实 Chrome 稳定版、不同版本矩阵**未验证**。
- **真实触摸/手势**：未验证。resize 是**程序化视口变更**，不是真机旋转；pinch/双指平移、
  长按、拖拽滚动、`touch-action` 行为**未测**。
- **软键盘**：未验证（无移动端软键盘；布局只按 `dvh`/安全区内边距设计，未在键盘弹出时测过）。
- **性能**：未测（启动、布局、帧率、内存均无数据；软件渲染下也不得出性能结论）。
- **CI**：**未接线**。`scripts/build-web-ui.sh` 与 `scripts/web-ui-smoke.cjs` 是**本地/手工**入口；
  CI 的 `web-smoke` 仍是旧的 `apps/app-web/dist` 载体/状态冒烟（见 `docs/ci.md`）。
- **原生宿主**：桌面 Tauri 宿主的窄窗口行为不在本轮范围（见 `docs/handoff.md` 的既有条目）。
- **Android 真机/真实 GPU**：模拟器证据见 `docs/android-host.md`；模拟器 ≠ 真机。
