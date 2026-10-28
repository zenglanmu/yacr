# 浏览器/Android `dom-canvas` 载体：前端换轨到单漏斗（命令 + 输入 + 帧 + 弹窗 + 面板）

日期：2026-10-28。范围：**前端（`apps/desktop-ui`）+ 一处 wasm 载体能力字段**。证据为本机
Chromium（Playwright）**SwiftShader 软件渲染**上的 DOM/像素差与真会话状态，**不是**真机、真实 GPU、
性能或视觉正确性验收。

## 1. 结论（有界）

`dom-canvas` 载体（浏览器、Android WebView、X11）上的 chrome 不再只执行四条命令：

- `web-adapter.ts` 是**薄转发层**：`cad_status`→`shell_status()`、`cad_commands`→
  `shell_commands()`、`cad_command_aliases`→`shell_aliases()`、`probe_input`→wasm 同名导出；
  其余每条命令（19 条 wire 命令**以及任何陌生名字**）一律 `invoke(name, JSON.stringify(args))`，
  拆 `{ok,value}` / 原样抛 `{ok:false,error}`。**适配器没有命令表**，拒绝永远来自会话。
- 唯一宿主专属行为是 `OPEN`（DOM 文件选择器 + `open_bytes`；wasm 无文件系统），
  `cad_status.canOpen` 因此被抬成 `true`（唯一被抬起的字段）。
- 指针输入走**同一条 lane**：`bindProbeInput` 的 `ProbeInput` 事件在 `dom-canvas` 上也存在
  （新建条件 `inputLaneReady = carrierKnown && (dom-canvas ⇒ canvasLive)`），相机不再由传输层合成。
- 帧由 `cad/frame-loop.ts` 的 rAF 泵驱动（输入/命令/状态变化 kick；`presented:false` 连续
  `FRAME_IDLE_STEPS=2` 停；单次泵上限 `FRAME_MAX_STEPS=600`），空闲不空转。
- 面板能力改为**读宿主上报字段**：`palettesAvailable`（wasm 现报 `true`），单一读点
  `palettesOffered`。选项对话框的 Apply 会真的写穿到会话。

## 2. 命令映射表（`apps/desktop-ui/src/host/web-adapter.ts`）

| chrome 发的名字 | 适配器做的事 |
|---|---|
| `cad_status` | `shell_status()` 解析 + `canOpen: true` |
| `cad_commands` | `shell_commands()` 解析 + 去重后并入 `OPEN` |
| `cad_command_aliases` | `shell_aliases()` 解析 |
| `probe_input` | `probe_input(JSON.stringify(args.input))`（会话导出收的是 `ProbeInput` 本体，不是包装） |
| `cad_command` | `OPEN` → 文件选择器 + `open_bytes`；其余 → `invoke('cad_command', …)` |
| 其余 18 条 wire 命令 | `invoke(name, JSON.stringify(args))` |
| 陌生名字 | 同样转发 → 会话回答 `unsupported-runtime: <name>`（适配器不再有自己的名字表） |
| `canvas_session_status` / `canvas_runtime_failed` | 仍为"只上报"（记住 + 打标记行；有 Tauri 桥时由 `invoke.ts` 送 Tauri） |

`{ok:true}` → 返回 `value`；`{ok:false,error}` → 抛 `error` **原文**；既非二者 → `web-adapter:`
传输错误（绝不空成功）。

## 3. 输入与帧（`App.tsx` / `cad/canvas-host.ts` / `cad/frame-loop.ts`）

- **一条 lane**：`createInputLane` / `invoke('probe_input')` / `createCanvasTransport(` /
  `createFrameLoop(` 在 `App.tsx` 各**恰好一处**；`dom-canvas` 只是让同一条 lane 更晚一点建
  （等会话 live，否则会吃到载体诚实的 "no live CAD session" 拒绝并在横幅上常驻）。
- **传输层不再持有相机**：`bindCanvasNavigation`、`wheelZoomFactor`、`wheelZoomNotches`、
  `panDelta`、`ZOOM_STEP` 全部删除；`createCanvasTransport` 只剩 `start`/`resize`/一次性失败闩；
  `CANVAS_EXPORTS = ['start','resize','frame']`（缺 `frame` 在加载期就拒绝）。
- **帧泵**：`pump()` 立即清 idle 计数并起/续一条 rAF 链；每步 `frame()`；`presented:true` → 继续；
  连续 2 步无 present → 停；抛错 → 停并记 `lastError`（不再走传输失败路径，避免同一次失败报两遍）。
  kick 点：① 每次 `probe_input` 成功；② 每次被接受的命令；③ 轮询到的状态**内容变化**（模型变了才画，
  这是所有调色板/设置/弹窗写入的统一触发表，不需要每个命令点各记一次）。
- `watchCanvasRect` 只做测量 → `resize(css_w, css_h, dpr)`（并在尺寸变化后 pump 一次）。

## 4. 能力与弹窗

- `palettesAvailable`：`App.tsx` 的 `palettesOffered(status)`（`cad/surface.ts` 唯一读点，
  缺省=可用）；`apps/app-web/src/shell.rs` 的 `palettesAvailable` 由 `false` 改为 `true`
  （**这是本轮唯一的 Rust 改动**：该字段是载体自己的能力声明，而这个载体就是真桌面会话，
  它自己服务 `cad_layers`/`cad_setting*`；旧值 `false` 是为"chrome 自行推导"写的过渡值）。
  `CadStatus` 类型与 `parseCadStatus` 增加了该可选字段（出现时必须是真布尔）。
- 弹窗：`cad_open_popup`/`cad_close_popup` 经漏斗直通会话状态机；About/Options/TextEdit 都是
  同一 React 树的 in-DOM 覆盖层；**模态门在会话侧**（`Live::probe` 对非 resize 输入回
  `accepted:false, reason:"popup.modal_open"`），chrome 侧只镜像成 lane 的第二 suspend 原因。
- **仍未翻面的能力字段**：`standardViews: []` 仍是 wasm 载体上报的旧值（七个标准视图按钮因此
  在 `dom-canvas` 上被前端禁用），而 `cad_standard_view` 现在已由漏斗转发并会被会话执行 ——
  这是**已知的陈旧能力声明**，本轮按范围要求**未改**（改它需要 Rust 侧列出真实视图键集并补契约测试）。

## 5. 实测证据（本机）

### 5.1 前端契约测试

```
cd apps/desktop-ui && npx tsc --noEmit && node --test test/*.test.ts
ℹ tests 811   ℹ pass 811   ℹ fail 0
```

新增/改写的契约：`test/frame-loop.test.ts`（泵的停止/上限/kick/dispose/抛错共 9 项）、
`test/canvas-host.test.ts`（删除相机与导航绑定，改测"传输层没有相机面"、"`frame` 在导出契约里"、
"一条 lane 服务两个载体"）、`test/host-adapter.test.ts` 第 3 节重写为转发语义（含"适配器不拒绝任何
命令，只有会话拒绝"）、`test/canvas-session.test.ts` 的调色板用例改为"读上报字段"。

### 5.2 wasm 载体（唯一 Rust 改动）

```
cargo test -p app-web --locked
test result: ok. 74 passed; 0 failed
cargo fmt --all -- --check   # ok
```

### 5.3 浏览器冒烟（SwiftShader，软件渲染）

```
bash scripts/build-web-ui.sh
NODE_PATH=/tmp/opencode/pw/node_modules node scripts/web-ui-smoke.cjs \
  --url http://127.0.0.1:8090/ --verbose \
  --evidence /tmp/opencode/yacr-parity-20261028T141507Z
PASS 8/8 cases (193/193 checks)
```

命令用例外的新增断言（原文行，节选）：

```
ok the session advertises the camera selectors and the drawing commands -> {"count":155,"sample":["OPEN","NEW","PLOT","SAVE","CANCEL","PAN"]}
ok the chrome shows the active tool it can confirm
ok the first canvas click is accepted as a pick -> {"points":1}
ok the second canvas click is accepted as a pick -> {"points":2}
ok the keyboard belongs to the drawing, not a text field -> {"hasFocus":true,"tag":"DIV","isTextField":false}
ok Enter finishes LINE and the session holds one more entity -> {"before":4,"after":5,...,"commandLogTail":[…,{"kind":"command","command":"CONFIRM","code":null}]}
ok the new line adds ink to the composited canvas (the frame loop repainted) -> {"nonModalPixelsBefore":4907,"nonModalPixelsAfter":8366}
ok the composited canvas differs before and after the gesture -> {"bytesBefore":2947,"bytesAfter":5578,"identical":false}
ok the picked file is imported into the session -> {"before":1}
ok the opened drawing is the fixture, with its four LINE entities -> {"expected":4,"entityCount":4,"afterLine":5}
ok the open installed another document (importCount grew) -> {"before":1,"after":2}
ok LAYER opens the docked layers palette on this carrier
ok the palette is filled from the session's own layer table -> {"rows":1}
ok the ribbon Options entry opens the in-DOM Options dialog
ok Apply writes the setting through the session (cad_set_setting) -> {"before":false,"after":true}
ok OK closes the Options dialog
ok the session cleared its modal gate after OK
ok the ribbon About entry opens the in-DOM overlay
ok the overlay is the About dialog -> about
ok the session refuses a CAD interaction while the dialog is open -> {"accepted":false,"kind":"pointer_down","ok":false,"reason":"popup.modal_open","sequence":1,"step":"input"}
ok a canvas click while the dialog is open changes nothing in the session
ok no panel-wide "none of these tools can run" note on the drawing tab -> []
ok the drawer enables the drawing tools the session executes -> {"enabled":32,"disabled":5,"notes":[]}
```

其余七个用例（390/768/914/1024/1025/1280 六档视口 + live resize）全部保持原断言通过：
`data-layout` 翻转、分区面积 ≥ 70 %、canvas backing = CSS×DPR、**非单色出图**、无错误横幅。

### 5.4 门禁

```
python3 scripts/check-i18n.py        # i18n OK: 895 keys
python3 scripts/check-file-size.py   # file-size OK: 1134 code files, limit 1000
python3 scripts/check-architecture.py # Architecture OK: 28 packages
```

## 6. 仍被拒绝的东西（都是载体的事实，不是缺失）

- `NEW` / `SAVE` / `PLOT`：`Command New is not supported yet.`（浏览器没有文件系统/绘图仪；
  `canNew/canSave/canPlot` 由会话如实报 `false`）。
- 七个标准视图：前端在 IPC 之前拒绝，原因是 `cad_status.standardViews === []`（陈旧字段，见 §4）。
- 需要选择的命令（`MATCHPROP`、`JOIN`、`CHAMFER POLYLINE` …）：`Needs … selected objects`（真前置条件）。
- 未打开会话时：`web-adapter: canvas module unavailable: …` / 会话的 `no live CAD session …`。

## 7. NOT VERIFIED（不得引用为本轮结论）

- **X11/XWayland 宿主的运行时**：本轮只跑浏览器 bundle；X11 走的是同一份 chrome + 同一份 wasm，
  但 `apps/app-linux` 的 `dom-canvas` 运行**未运行**。
- **Android 运行时**：未在模拟器/真机重跑；`apps/app-android/dist` 未重建。
- **真实 GPU**：整轮是 SwiftShader 软件光栅化 ⇒ 无 GPU、无性能、无像素正确性结论。
- **worker/offscreen 载体**：仍是旧 viewer 会话，没有 `invoke`/`probe_input`/`shell_*` 面。
- **视觉正确性**：像素断言只证明"有帧且在变"（非单色 + 墨量增长 + 字节不同），不证明几何/色彩/相机正确。
- **WebKitGTK**：本轮只在 Chromium 141 上跑；WebKit 的 rAF/输入时序未验证。
