# `dom-canvas` 生产翻面（wasm 侧）：真 `CadSession` 接管 page canvas

日期：2026-10-28（本 lane）。范围：**只有 wasm/page 载体**。桌面宿主、`apps/desktop-ui`、
worker/offscreen 载体**未改**（见 §7）。

## 1. 结论（有界）

`apps/app-web` 的 **page 载体**（`start`/`open_bytes` 打开的 `<canvas>`）现在由
`cad_ui_core::session::CadSession` —— 与原生宿主**同一个**会话实现 —— 拥有：真命令路由器、
真 `cad-db` 权威、真输入缝、真弹窗状态机、真状态投影。旧的 viewer 会话保留给
**worker/offscreen 载体**（未改）。

- `start` / `open_bytes` 建/替换真会话，ADR 0025 两道预算在**解析/安装之前**执行；
- `frame()` 是**帧步骤**（tick → `apply_ready_updates` → `render` → `present`），每 rAF 一次，
  wasm 内**不起定时器**；
- `pan`/`zoom_at` 用**合成 `ProbeInput`**（中键拖拽 / 一次滚轮）走 `handle_input`，前端换轨前
  现有 UI 仍可用；
- `invoke(name, args_json)` 成为**生产入口**（无 feature gate），服务全部 20 条 wire 命令；
- `shell_status/commands/aliases/command` 线形不变，改为真会话投影 + **完整词汇**；
- `probe_input(args_json)` 是生产导出，`cad_open_popup`/`cad_close_popup` 走真状态机；
- `debug-session` spike（第二个会话）**删除**，feature 名保留为空以免旧证据命令报错。

## 2. 宿主验收（无 GPU、无 DOM）

`cargo test -p app-web`（74 tests）+ `cargo test -p cad-ui-core`（310 tests）。关键用例外：

| 用例 | 断言 |
|---|---|
| `live::tests::the_production_session_holds_the_fixture_and_reports_its_own_facts` | `entityCount = 4`、`importCount = 1`、`canFit = true`、`canNew/canOpen/canSave/canPlot = false`、`db_retained = true` |
| `live::tests::open_bytes_replaces_the_document_in_place_and_keeps_the_session` | 第二次 open 后 `documentLabel` 变、`importCount = 2`、`entityCount` 不变（同一份 bytes） |
| `live::tests::a_refused_replacement_leaves_the_open_document_and_its_status_intact` | 坏字节 → `step: "import"`，旧文档/计数不变 |
| `live::tests::the_input_byte_budget_refuses_before_anything_is_parsed` | 64 MiB + 1 → `step: "budget"`、`MAX_INPUT_BYTES`、`measured = 67108865`、消息含 `not parsed`/`unchanged`，文档未动 |
| `live::tests::the_heap_budget_refuses_before_another_document_is_installed` | 堆 > 3 GiB → `MAX_WASM_HEAP_BYTES` 拒绝；首次 install 同样拒绝 |
| `live::tests::a_post_install_measurement_over_the_cap_is_reported_not_ignored` | 安装后测到超限 → `over_budget` 块（不否认、不抬高上限） |
| `live::tests::open_popup_answers_a_real_acknowledgement_and_gates_the_input` | `about` → `accepted/kind/popup_label=popup-1/existing=false` + About payload；模态门拒绝指针（`popup.modal_open`）；close 幂等 |
| `live::tests::the_text_edit_popup_derives_its_payload_from_the_selection_or_refuses` | 无选区 → `popup.no_text_selection`；未知 kind → `popup.unknown_kind` |
| `live::tests::probe_input_draws_a_line_through_the_real_input_seam` | `LINE` + 两次点击 + `CONFIRM` → `entityCount 4 → 5`、`tool = null`、`error = null` |
| `live::tests::the_synthesized_pan_is_the_middle_drag_the_chrome_sends` / `..._zoom_is_one_wheel_event...` | 4×50 px pan → 相机中心移动 200×`world_per_px`（误差 < 1e-6）、尺度不变；1.25 倍滚轮 → 尺度 /1.25，0.8 回到原位 |
| `live::tests::a_zoom_factor_the_wheel_would_clamp_is_refused_not_approximated` | `0.1/10/0/-2/NaN` 全部 `step: "navigation"` |
| `shell::tests::*` | 冻结键集（21 键）+ `surfaceMode/palettesAvailable/standardViews/canvasSession`；`entityCount/tool/selection/commandLog` 来自真会话；词汇 155、别名 51；`OPEN` 在词汇内但执行时被 `unsupported-filesystem` 拒绝 |
| `session::bytes::tests::*`（cad-ui-core） | 就地替换 + `importCount 1→2`；空白 label / 坏字节不改旧文档 |

## 3. 浏览器证据（headless Chromium + SwiftShader）

脚本：`scripts/web-flip-evidence.cjs`（只驱动**生产**导出；harness 页面由脚本生成，直接调
`start(...)`，**不走** `apps/app-web/web/index.html` 的 worker 优先探测，因此测的是本 lane 的
page 载体）。构建：`bash scripts/build-web.sh`。

运行命令：

```
NODE_PATH=/tmp/opencode/pw/node_modules node scripts/web-flip-evidence.cjs \
  --bundle apps/app-web/dist \
  --evidence /tmp/opencode/yacr-flip-20261028T-final2 \
  --open /run/user/1000/doc/51c79ae3/complex-test.dwg --verbose
```

**结果：PASS 36/36 checks，退出码 0。** 渲染器
`ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero)), SwiftShader driver)` ⇒
**软件渲染**，本 run 只是「有浏览器运行 + 有像素进 canvas」，不是 GPU/性能/像素正确性结论。

关键数字（`/tmp/opencode/yacr-flip-20261028T-final2/summary.json`）：

| 项 | 值 |
|---|---|
| `start` | `ok`、`label=synthetic-four-lines.dwg`、`entityCount=4`、`backend=gl` |
| canvas backing | `1280×800`（CSS 1280×800 × dpr 1） |
| fixture 的首帧 | `start` 内部有限次尝试后 `presented = 1`；`import_ms = 59.2`、`heap_after_open = 2 752 512 B`（≈2.6 MiB） |
| `frame()` ×120 | presented 120、waited 0 |
| `LINE` + 2 次 `probe_input` 点击 + `CONFIRM` | `entityCount 4 → 5`、`tool=null`、`error=null` |
| 像素差（改动前后 canvas box，1280×800） | **5442 / 1024000 px 变化（0.53 %）** |
| `pan(60, 0)` | 相机 centre 1.5 → 1.35234375（= 60 × `world_per_px` 0.0024609375），scale 不变 |
| `zoom_at(1.25, 640, 400)` | scale 0.0024609375 → 0.00196874998798（= /1.25，f32 滚轮 delta 的舍入） |
| `cad_open_popup{about}` | `{accepted:true, kind:"about", popup_label:"popup-1", existing:false, payload:{version:"0.1.2", build:"release"}}` |
| 弹窗打开时 `cad_command{LINE}` | `{ok:false, error:"popup.modal_open"}`；`shell_status().popup.kind == "about"` |
| `shell_commands()` / `shell_aliases()` | **155** 个名字 / **51** 行别名 |
| `shell_command("ZOOMIN")` / `("FLOOPY")` | `{accepted:true}` / `unsupported-runtime: cad_command: unsupported CAD command: FLOOPY` |
| 未捕获页面错误 | 0 |

截图（本地，未上传）：

- `/tmp/opencode/yacr-flip-20261028T-final2/canvas-01-before-line.png` —— fixture 四条线（软件渲染，
  线宽 1 px、13 种量化颜色）；
- `/tmp/opencode/yacr-flip-20261028T-final2/canvas-02-after-line.png` —— **多了一条从
  (320,260) 到 (900,520) 的斜线**（正是两次 `probe_input` 点击的位置），并有会话自己画的
  十字光标（x≈900 / y≈520 贯穿全幅）。实测观察：几何位置与点击坐标一致；
- `/tmp/opencode/yacr-flip-20261028T-final2/page-03-after-popup.png` —— 整页（本 harness 无 chrome，
  所以**弹窗没有像素证据**，只有 §2/上表的 ack 数据）；
- `/tmp/opencode/yacr-flip-20261028T-final2/canvas-04-external-drawing.png` —— 外部
  `complex-test.dwg`（建筑平面，彩色填充多楼层 + 表格）**由真会话画进同一个 canvas**。
  实测观察：图纸、填充色、表格都完整；十字光标在最后一次指针位置；本图是 pan/zoom 之后、
  `open_bytes` 就地替换后拍的（替换走的是桌面 open 的 `fit()`）。

## 4. 保留数据库的代价（主要风险，实测）

`--open` 用同一入口就地替换成外部样本（`complex-test.dwg`，4 228 788 B）：

| 项 | 实测 |
|---|---|
| `entityCount` | 44 591 |
| `batches`（prepared scene 常驻） | 145 776 |
| `import_ms`（构建/导入，不含上传） | 1.8 s（三次运行的 1 777 / 1 816 / 1 793 ms） |
| `heap_after_open`（安装后立即） | 615 251 968 B ≈ **587 MiB** |
| `wasm_heap_bytes`（场景上传完成后，240 帧后） | 2 873 491 456 B ≈ **2.68 GiB** |
| `presented` / `waited`（240 帧） | 103 / 137（progressive 上传真实存在，等待被计为 waited） |
| 会话诊断 | `unsupported-resources: fonts: 12255 requested face(s), text_shaping=true; …`（**非致命**：图纸打开、文字不整形，`ok` 仍为 true，诊断照报） |
| 像素差 vs fixture | 48 091 / 1024000（4.7 %) |

对照文档里 viewer 的口径（ADR 0025：`complex-test.dwg` 峰值 1.91 GB，worker 隔离后 1.43 GB）：
**真会话说 "常驻 2.68 GiB"**，因为它保留数据库（viewer 释放）。这是保留换来的真实代价，
本轮**没有**抬高 3 GiB 上限，也**没有**在超限时静默放行。

**已知不够的地方（必须记录）**：ADR 0025 的堆检查在本 lane 只能做「安装**之前**」的守卫
（`deliver_external_open` 是宿主原子替换，装完再拒绝无法回滚）。实测 2.68 GiB < 3 GiB 上限，
所以「再开一张同量级图纸」**不会**被拒绝 —— 峰值可能接近 4 GiB abort 线。把它做对需要
**峰值感知的预算**（为下一次 open 预留）或流式导入，属 ADR 0025 后续，不在本轮。

## 5. wasm 体积

| 构建 | 字节 |
|---|---|
| 旧默认（viewer only，Cargo.toml 记录） | 9.66 MB |
| 旧 spike（`--features debug-session`，记录值） | 13.41 MB |
| **本轮生产默认**（`scripts/build-web.sh`，无 feature） | **13 566 622 B** wasm + 139 022 B js |

即翻面把真会话（+ 真命令层/输入/弹窗/状态投影）装进**默认**产物：比旧默认大约 **+40 %**，
与旧 spike 体量相当（spike 是同一批代码藏在 feature 后）。spike 的第二个会话代码已删除，
所以体积增量来自能力本身，不是重复实现。

## 6. 导出映射（生产 vs 仍拒绝）

| 导出 | page 载体（本 lane） | worker 载体（未改） |
|---|---|---|
| `start`/`open_bytes` | 真会话（预算 → 建/就地替换 → 有限次首帧） | viewer（原行为） |
| `frame()` | 一步（tick/upload/render/present） | 拒绝（`state`） |
| `resize`/`pan`/`zoom_at`/`fit_view` | 真会话（合成输入 + `set_canvas_metrics`） | viewer（原行为） |
| `status`/`document`/`last_import_ms` | 真会话事实 | viewer 事实 |
| `probe_input` | 真 `handle_input` + 模态门 | 拒绝（无输入面） |
| `invoke`（20 条） | 真会话 + `DirectSink`/`DirectReads`/真 `PopupTarget` | 仅 `cad_commands`/`cad_command_aliases` + `NO_SESSION` |
| `shell_status` | 真 `CadStatus` + 4 个载体键 | 诚实空状态（无 page 会话） |
| `shell_commands`/`shell_aliases` | `admissible_names()` / `alias_pairs()` | 同（纯函数） |
| `shell_command` | 共享门 + 真路由器 | `unsupported-runtime: cad_command: …` |
| `start_offscreen`/`open_bytes_offscreen`/`worker_capability` | 未改（worker/viewer） | 同左 |

**仍然拒绝的东西（不是漏洞）**：`canNew/canOpen/canSave/canPlot` 全为 false（本载体无文件系统、
无保存对话框、无出图导出）；`OPEN` 在词汇内，但执行时被 `unsupported-filesystem` 拒绝
（页面自己的 picker 在 `web-adapter.ts` 里拦截，是传输能力）；字体/图像资源不可用 →
`unsupported-resources` 诊断（非致命）；标准视图 `standardViews: []`（无 `cad_standard_view`
传输）；`palettesAvailable: false` 且 chrome 由 `surfaceMode` 自行推导（前端换轨才启用）。

## 7. NOT VERIFIED / 未做

- **worker/offscreen 载体未迁移**：仍是 viewer（无 `invoke`/`probe_input`/`shell_status` 事实），
  真会话在 worker 内运行、内存口径都未验证；`web-smoke` 在 Chromium 上走的正是这条
  （`target=offscreen-canvas isolation=worker batches=4 presented=1 draws=1`，`scripts/web-smoke.cjs` 本地跑通）。
- **前端未换轨**：`apps/desktop-ui` 未改（本 lane 禁止）；`web-adapter.ts` 仍只路由四条命令，
  所以 X11/Android 的 chrome 现在能用真会话执行 `FIT/ZOOMIN/ZOOMOUT/OPEN`，其余命令**要等前端
  单漏斗 `invoke`**；`frame()` 也还没有人调用 → 现有页面靠 `resize`/`pan`/`zoom_at` 各自 present
  一帧维持重绘（已实测 `start` 用有限次首帧尝试把画面先画出来）。
- **X11 / WebKitGTK / 真机 / 真 GPU / 其它浏览器**：本轮只有 Chromium 141（SwiftShader）。
  WebKitGTK 无 worker WebGL2，会走 page 载体（即本 lane），但**未运行**。
- **视觉/像素正确性**：像素差只证明「有东西进了 canvas」，不证明颜色/几何/抗锯齿正确；
  截图是诊断产物。
- **性能**：`import_ms`/帧数/等待数是**测量值**，不是性能结论（软件渲染）。
- **worker 与 page 的构建产物共用**：`apps/app-web/dist`（`scripts/build-web.sh`）与
  `apps/app-web/dist-ui`（`build-web-ui.sh`）是两份不同运行时常量的产物，本轮只重建了前者的
  `pkg/`（`apps/app-web/web/pkg`）；`apps/desktop-ui/public/canvas` 未重建（属桌面宿主 lane）。
