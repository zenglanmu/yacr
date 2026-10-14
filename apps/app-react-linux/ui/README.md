# React 原生透明探针 UI

X11 原生 Rust/Vulkan + 透明 WebView 的最小 React 外壳，非完整 CAD 产品。前端不使用 WASM、canvas、GPU 像素回读或第二份数据库。透明中间区域交给宿主原生 Vulkan；React 只绘制顶部 ribbon、底部 OPEN/FIT 命令框、ribbon 内展开的诊断面板和独立定时动态标记。打开/适合按钮由真实宿主状态决定可用性；不返回假成功。当前原生 child 位于中间区域上层，**不支持跨 CAD 区 DOM 浮层遮挡**，诊断面板不跨入该区域。

```sh
npm ci
npm test
npm run build
npm run dev
```

开发地址为 `http://127.0.0.1:1420`，产物为已忽略的 `dist/`。生产原生宿主应加载该目录。普通 UI 文案直接导入现有双语 JSON，不添加新文案键；字段名、计数及错误为探针诊断数据。

IPC 使用 `@tauri-apps/api/core` 的 `invoke`：

- 当前 UI 优先调用 `cad_status`，严格校验 `{documentLabel:string|null,entityCount:number|null,loading:boolean,error:string|null,canOpen:boolean,canFit:boolean,backend:string|null,adapter:string|null,camera:{center:[number,number],scale:number}|null,importCount:number,frameCount:number}`。缺失/未知 JSON 显式报告 `CAD_STATUS_INVALID`，清除可用状态而非伪装为空图纸。每次完成 250ms 后再轮询，不重叠；失败退避 1/2/4 秒、上限 4 秒，恢复后重新 250ms。没有宿主时显式显示错误，不模拟数据。`probe_status` 仍为宿主保留的原生探针接口，当前前端不另行轮询。
- `cad_command` 参数 `{command:'OPEN'|'FIT'}`，仅接受返回 `{accepted:true}` 为排队成功，**不等于操作完成**。按钮/输入在 loading、未知状态、命令提交中禁用；FIT 还要求 `canFit`、非空文档标识与已知实体数。底栏输入只允许 OPEN/FIT（其他命令用既有 `command.unsupported` 文案报告），Enter 提交、仅 accepted 时清空，错误保留输入并用 `command.failed` 显示。不抢占 Tab/方向键/Escape/IME Enter，无完成文案。OPEN 调宿主原生选择器；无前端文件上传或导入数据库。
- `probe_input` 参数 `{input:{kind,...,sequence}}`。`kind` 为 `pointer_down/pointer_move/pointer_up/pointer_cancel/wheel/resize/blur`。`resize` 的 `x/y` 是 `.native-input.getBoundingClientRect()` 相对 WebView viewport 的 left/top，`width/height` 是实际 CAD 区域 CSS px，DPR 独立传递；pointer/wheel 的 `x/y` 是该区域内局部 CSS px（client 坐标减区域 left/top），wheel delta 归一化 CSS px。输入交给原生宿主处理，相机来自 `cad_status`，React 不实现相机或图纸渲染。旧探针/新宿主是否实际驱动 CAD 由宿主运行证据判断，前端合成测试不证明 CAD 行为。
- 输入只有一条串行异步 IPC 队列；move 在 RAF 内取最后值，排队连续输入只合并相邻同类，不跨 down/up/cancel 合并。相邻 wheel 累加 delta。队列上限 64（另有一个在途），48 个排队项后拒绝开始新手势；连续输入最多占用 56 项，饱和时丢弃额外连续采样，预留终止输入容量。极端超限离散事件显式报告 `IPC_QUEUE_CAPACITY`，不声称成功或无限积压。
- pointer capture、release、cancel、丢失 capture、窗口 blur 清理均处理。诊断面板切换时取消现有手势；面板只在 ribbon 内展开，其关闭按钮不向原生转发。命令框聚焦取消/暂停 CAD 区手势，失焦恢复。ResizeObserver 观察 CAD 区与 ribbon/底栏布局贡献者；窗口 resize、DPR media query 共同通过一个 RAF 调度并去重，初次绑定立即发真实区域 rect。pointer 输入前也同步检查 rect；布局变化先 flush 旧 move，再提交 resize 屏障，不跨屏障合并 move。resize 使用保留容量而不是饱和时静默丢弃的连续采样。

Node 测试是纯合成输入/队列契约，Vite build 仅证明前端可构建。真实 X11/WebView 透明合成、浮层遮挡、硬件 Vulkan 与 React 同时刷新仍必须由主协调者实际运行验收。

本轮前端证据（2026-10-14）：Node v24.21.0，Vite v8.3.4；`npm run build` exit 0，`npm test` 6/6 合成契约通过。依赖安装曾多次超时，最终 `npm install -D vite --prefer-offline --ignore-scripts --no-audit --no-fund --fetch-timeout=10000 --fetch-retries=0` exit 0，并生成 `package-lock.json`。未运行真实 WebView/GPU、未做视觉验收。

最终前端复验（同日）：不重复安装，React/React DOM 19.3.0、Tauri API 2.12.3、Vite 8.3.4、Node 24.21.0、npm 11.19.0。小修：宿主返回的 `status.error` 在浮层关闭时也显示于底栏；品牌从已有 `app.title` 提取。补充断开 IPC 每次失败显式报告且仍尝试 up/blur、blur 后无捕获残留并允许重新开始手势的合成测试。首轮因文案测试正则把 `split(...)` 误识别为 `t(...)` 而 6/7；修正词边界后末次 `npm run build` exit 0、`npm test` 7/7，未跳过。两个现有字典各 306 键、键集一致；本 UI 使用 15 个现有键，无新增普通硬编码文案（仅保留语言代码、技术字段、React 标记及诊断数据）。源码全窗透明与不透明面板布局已审查，但 CSS 不能证明 GTK/native 合成正确或 React 未被覆盖；真实画面需主协调者人工或截图验收。

DOM rect 跟进（2026-10-14）：末次 build exit 0、纯契约测试 9/9，无跳过；新增 ribbon offset、底栏动态高度、局部指针坐标、CSS 尺寸与 DPR 独立、RAF 去重及 resize 顺序屏障用例。最终 `dist/index.html` mtime：`2026-10-14T09:35:17.752994367Z`（本机显示 `17:35:17.752994367 +0800`）。本轮不运行或推断 GTK 分区对齐与 OS 焦点实际效果，运行验收仍由主协调者负责。

真实命令/状态 IPC 前端跟进（2026-10-14）：末次 `npm run build` 成功、`npm test` 16/16 纯合成契约通过，无跳过。新增严格状态校验、未知/空文档区分、命令 accepted 非完成、unsupported/queue full/断开失败、loading/capability 禁用、键盘不截获 Tab/IME、单在途轮询与有界退避测试。现有输入轨道未改变。React 动态标记单独更新，不驱动 CAD 区组件刷新；CAD 区不由 React 绘制。最终 `dist/index.html` mtime：`2026-10-14T10:46:29.316244462Z`。未运行真实后端打开/相机/FIT/GTK/键盘验收，交由主协调者。
