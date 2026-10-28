# 浏览器/Android 载体的命令接线（2026-10-28，有界本地运行）

## 结论

浏览器/Android 载体（`apps/app-web` 的 wasm `dom-canvas` 会话 + `apps/desktop-ui` 传输缝）现在
**真的执行**四个命令，并且由**真实 UI**（命令行的 Enter 漏斗 → `queueCadCommand` → 浏览器适配器 →
wasm 会话）跑通：

| 命令 | 谁执行 | 路径 |
|---|---|---|
| `FIT` | wasm 会话 | `shell_command("FIT")` → `Session::fit_view` → `view::fit_camera`（**与开图同一条规则**，用开图保留的 `(min,max)`）→ `render_and_present` → `{"accepted":true}` |
| `ZOOMIN` | wasm 会话 | `shell_command` → `view::zoom_step(ZOOM_IN_FACTOR)`（视口中心锚点）→ `{"accepted":true}` |
| `ZOOMOUT` | wasm 会话 | 同上，`ZOOM_OUT_FACTOR` |
| `OPEN` | chrome 传输缝 | `web-adapter` 建隐藏 `<input type=file accept=".dwg,.dxf">` 并 `click()` → 读 `arrayBuffer` → 会话 `open_bytes(#cad-canvas, name, cssW, cssH, dpr, bytes)` → `{"accepted":true}`（用户取消也算已执行，不算失败） |

**词汇（两侧，逐字）**

- wasm 会话 `shell_commands()`：`["FIT","ZOOMIN","ZOOMOUT"]`
- wasm 会话 `shell_aliases()`：`[{"alias":"ZOOM EXTENTS","command":"FIT"}]`（桌面别名表过滤后的唯一一行；
  `shell_command` 也解析该别名）
- 适配器 `cad_commands`：`["FIT","ZOOMIN","ZOOMOUT","OPEN"]`（会话列表 + 宿主命令，去重、稳定顺序）
- 适配器 `cad_status`：`canOpen: true`（传输入口；会话自身仍报 `false`），`canFit` 原样来自会话
  （有活会话且保留了范围才为 `true`）

**仍在生效的拒绝**（未改动）：`PAN`、`ZOOM`、`SAVE`/`SAVEAS`/`QSAVE`、`PLOT`/`EXPORT`/`PRINT`、
`NEW`、全部图层/设置/测量/绘制/弹窗命令、`probe_input`、`cad_layers*` 等 —— 会话侧
`unsupported-runtime: cad_command: "<name>" is not executable — …`；适配器对未映射 IPC 命令
`unsupported-runtime: <command>`。

浏览器内直接调用会话导出逐字取回（同一次 Chromium/SwiftShader 会话，页面未接 UI）：

```
shell_commands() -> ["FIT","ZOOMIN","ZOOMOUT"]
shell_aliases()  -> [{"alias":"ZOOM EXTENTS","command":"FIT"}]
shell_command("PAN")    -> unsupported-runtime: cad_command: "PAN" is not executable — this carrier
                           executes ["FIT", "ZOOMIN", "ZOOMOUT"] only (open is the page's file
                           picker, pan/scroll are the pointer transport; there is no command router)
shell_command("ZOOM")   -> (同上，名字为 "ZOOM")
shell_command("SAVE")   -> (同上，名字为 "SAVE")
shell_command("probe_input") -> (同上，名字为 "probe_input")
shell_command("zoom extents") -> {"accepted":true}   ← 已发布的别名被解析成 FIT
```

`shell_status()` 的同一份浏览器实测：`canFit: true`、`canOpen: false`（会话自身）、
`entityCount: null`、`documentLabel: "synthetic-four-lines.dwg"`、`importCount: 1`。

## 代码改动

- `apps/app-web/src/view.rs`（新，**平台无关**）：`retain_bounds`、`fit_camera`（唯一的拟合规则）、
  `zoom_step`（中心锚点）。
- `apps/app-web/src/session/import.rs`：开图时保留 `(min,max)`（`Loaded.bounds`），仍 `drop(database)`，
  `db_retained: false` 不变。
- `apps/app-web/src/session/{session_state,session,upload}.rs`：`Session.bounds` + `fit_view` +
  `zoom_about_centre`；`status()`/`document()` 增 `bounds_retained`。
- `apps/app-web/src/shell.rs`：`EXECUTABLE_COMMANDS`、`Execution`、`Runner`/`dispatch_json`（宿主可测的
  接受路径）、`resolve_command`（含别名）、`aliases_json`（对象形态，与桌面 Tauri 命令一致）、
  `canFit` 来自 `has_bounds`；native 侧解析后报 `NOT_IMPLEMENTED`（不伪造 `accepted`）。
- `apps/app-web/src/lib.rs`：导出 `fit_view()`（+ native 孪生）。
- `apps/app-web/Cargo.toml`：`cad-domain`/`cad-db`/`cad-app` 提到**全 target** 依赖，`view` 因此
  可在宿主单测（其余仍 wasm-only）。
- `apps/desktop-ui/src/host/web-adapter.ts`：`OPEN` 宿主流程、`cad_commands` 合并、`canOpen` 抬升、
  可注入 `document`。
- `apps/desktop-ui/src/commands/commands.ts`：`FIT` 门禁改为 `canFit && documentLabel !== null`
  （去掉 `entityCount !== null`）——载体诚实上报 `entityCount: null`（数据库已释放），`canFit` 才是
  "能重拟合"的答案。桌面宿主上两者等价（有文档即有计数），故语义不变。

## 已验证（本轮实际运行）

```
cargo test -p app-web --locked                     -> 34 passed
cargo check -p app-web --target wasm32-unknown-unknown --locked   -> ok
cargo clippy -p app-web --all-targets --locked -- -D warnings     -> ok
cargo fmt --all -- --check                         -> ok
(cd apps/desktop-ui && npx tsc --noEmit && node --test test/*.test.ts) -> 778 pass, 0 fail
python3 scripts/check-architecture.py              -> OK
python3 scripts/check-i18n.py                      -> OK
python3 scripts/check-file-size.py                 -> OK
bash scripts/build-web-ui.sh                       -> ok
```

冒烟（Chromium + **SwiftShader 软件渲染**；因此只是布局/DOM 证据，不是 GPU/像素/性能验收）：

```
NODE_PATH=/tmp/opencode/pw/node_modules node scripts/web-ui-smoke.cjs \
  --url http://127.0.0.1:8090/ --verbose --evidence /tmp/opencode/yacr-mobile-cmds-20261028T071045Z
```

（两次独立运行结果一致：`PASS 8/8 cases (158/158 checks)`，退出码 0；前一次证据目录
`/tmp/opencode/yacr-mobile-cmds-20261028T070652Z`。）

新用例逐字摘要（`PASS commands ZOOMIN / FIT / ZOOMOUT / OPEN (17/17 checks)`）：

```
ok the session advertises the three camera selectors it executes -> ["FIT","ZOOMIN","ZOOMOUT"]
ok the live session reports canFit and a document (the gates FIT needs)
   -> {"canFit":true,"documentLabel":"synthetic-four-lines.dwg","entityCount":null}
ok the command field echoes the typed selector -> {"typed":"FIT","echoed":"FIT"}
ok FIT leaves the session with a usable camera -> {"scale":0.0025280898876404493}
ok ZOOMIN changes the live camera scale by exactly one step
   -> {"before":0.0025280898876404493,"after":0.0020224719101123593,
       "expected":0.0020224719101123593}
ok no visible error banner after ZOOMIN -> []
ok FIT returns the camera to the framing the zoom moved away from
   -> {"zoomed":0.0020224719101123593,"refitted":0.0025280898876404493,
       "fitted":0.0025280898876404493}
ok no visible error banner after FIT -> []
ok ZOOMOUT applies the reciprocal step
   -> {"before":0.0025280898876404493,"after":0.0031601123595505613,
       "expected":0.0031601123595505617}
ok no visible error banner after ZOOMOUT -> []
ok OPEN opens the host file picker -> /home/…/fixtures/dwg/synthetic-four-lines.dwg
ok the session reports the opened file as its document label
   -> {"expected":"synthetic-four-lines.dwg","label":"synthetic-four-lines.dwg"}
ok the open installed another document (importCount grew) -> {"before":1,"after":2}
ok no visible error banner after OPEN -> []
ok no uncaught page error during the commands -> []
ok the CAD canvas still shows a frame after the commands -> {"width":1246,"height":541,…}
ok screenshot commands-open.png written -> …/commands-open.png
```

总计：`PASS 8/8 cases (158/158 checks)`，退出码 0。

补充事实：

- **相机尺度**：`FIT` 后 `0.0025280898876404493` → `ZOOMIN` 后 `0.0020224719101123593`
  （= 前者 / 1.25）→ `FIT` 后回到 `0.0025280898876404493`（相对差 0，字符串级相同）→ `ZOOMOUT`
  后 `0.0031601123595505613`（= 前者 × 1.25，末位 1 ulp 差，校验容差 1e-9 相对）。
- **打开的文件**：`/home/zenglanmu/sources/rust/yacr/fixtures/dwg/synthetic-four-lines.dwg`
  （20 755 B，4 条 LINE，仓库内唯一的 DWG）。它与 `start` 编译内置的合成夹具同字节，所以**证据是
  `importCount` 1 → 2 与 `documentLabel`**，不是"标签从 A 变成 B"——打开确实重装了文档，耗时在本次
  运行中未单独计时（`documentLabel` 首次轮询即更新）。
- `FIT` 会在用例开始先跑一次：`start` 之后起始面板收起会触发一次 **resize**（只重配置表面、**不**重
  拟合），因此"开图时的尺度"与"当前视口的拟合尺度"本来就可能不同；先 `FIT` 再比较，才是同类比较。

## NOT VERIFIED / 不在本轮证据内

- **真 Android**：本轮只在桌面 Chromium（SwiftShader）跑；模拟器/真机、其它 WebView 版本、Android
  文件选择器（SAF `content://` URI 而非 `File`）**未验证**——Android WebView 的
  `<input type=file>` 是否给出带 `name` 的 `File` 需要真机复核，**未做**。
- **GPU/像素/性能**：全部数字来自软件光栅化；不得引用为 GPU、像素正确性或性能结论。
- **其它浏览器引擎**：本轮只有 Chromium（Playwright 自带 1.64.x 版本）一次运行；WebKitGTK、Firefox、
  Safari、Android WebView 上的 `<input type=file>` 手势/`cancel` 事件行为**未验证**（`OPEN` 的
  `click()` 发生在 Enter 手势的微任务里，Chromium 允许，其它引擎未实测）。
- **X11 宿主**（`dom-canvas`，同一 wasm 路径）：本轮**未**跑 `apps/app-linux`，故 X11 上的新命令
  词汇**未验证**。
- **真 X11 服务器/其它合成器/DPR≠1/多显示器**：未验证（沿用既有边界）。
- `OPEN` 的**取消**分支只有单元测试（假 DOM 触发 `cancel`）；浏览器里由用户真的取消一次的真实事件
  **未在本轮复现**。
- `fit_view()` 作为**公开导出**（`apps/app-web` 直接调用者）只有 native NOT IMPLEMENTED 单测 +
  会话内路径证据；没有页面直接调 `fit_view()` 的用例（命令走 `shell_command`）。

## 顺带发现（未修，非本轮范围）

- `apps/app-web/src/shell.rs` 的别名表现在是对象形态（`{alias, command}`，与桌面 Tauri 命令一致）；
  旧的 `[alias, canonical]` 数组形态在 `apps/desktop-ui/src/commands/commands.ts` 的
  `setCommandAliases` 里会被丢成"没有别名"，因此对象形态是能真正生效的那一种。
- 该载体的别名 `ZOOM EXTENTS → FIT` 在 chrome 里仍显示为**不可点**（`isSupportedCommand('ZOOM
  EXTENTS')` 为假：它不是 `cad_commands` 里的名字）。`shell_command` 能执行它，但 chrome 不会发出
  它。这是既有别名/词汇语义的边角，本轮未改。
