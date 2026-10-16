# cad-engine 解耦阶段验证记录

> **历史执行证据（Slint 桌面已退役，2026-10-16）**：本文记录的是**迁移前** Slint 宿主/模拟器/浏览器
> 的运行与验收证据，**不代表**当前 React/Tauri 宿主的编译、测试或真实 GPU 结论；Slint→React 迁移后的
> 最终验证 **NOT RUN**（P6）。当前架构见 `docs/architecture.md` 与
> [ADR 0010](adr/0010-react-linux-host-replaces-slint.md)。下列事实仅作历史记录，不得据此宣称当前平台可用。

## 当前结论

Oracle P2/P3/P4 App/engine/Host/Web codegate 均 accept；P3/P4 指定硬件用例与最新差异门禁已通过。P4 最终 workspace/WASM/Android 编译/测试 sweep 及后续差异门禁证据按 shell 分轮记录；不等于 browser、Android 设备、vendor DWG 或视觉验收。event subscription 是 in-process bounded foundation，不是外部插件兼容验收。未知设备/浏览器能力不外推。

代码维持 renderer-only wgpu、单 session/dispatch/config authority 与 Slint 单 presenter。P2/P3/P4 指定 codegate 与证据已闭环；P4 的 bounded in-process events/API 不等于外部插件兼容。运行、视觉和跨平台结论严格限于各节明确的实际证据。

## Oracle 检查状态

- Attempt 1 的四项 blocker 已有源码/合同修补：idle bounds 证据边界、UI flush 覆盖 session、Web backend 失败不得空成功、图片 Missing 诊断完整性。相关合同通过。
- Attempt 2 的两项 residual 已有源码/合同修补：host 正交 3D mode 以 session mode 为准，以及 UI Missing completeness 聚合；相关合同通过。
- Attempt 3 **终态 reject**：`cad-host/src/input.rs` primary drag 在正交 3D 仍依 `perspective` 走 Pan（旧 view-mode 语义应为 Orbit）；UI summary 以原因列表 `items.len()` 充当对象数。两项随后已修补。
- Oracle attempt 4 接受上述修补的 **P2 架构/源码 codegate**；之后最终串行 compile/check/test sweep 全部 exit 0。不得将此限定 accept 写成真实场景/运行验收，也不得与 P1 Oracle gate accept 混为一谈。

## 已有编译与合成验证证据

以下为 Host bounds 修复前的 P2 协调器编译/合成验证记录；本文档 lane 未复跑 Cargo。此处 `host 33` 是旧轮计数，修后最新为下文 host 34。先前 shell 记录索引：
`sh_135dfcc52001sQnxyRUTHREUkh`、`sh_135e125f1001AzHQOuMZhlv72h`。现有上下文未提供两条 shell ID 到各命令的可靠一一映射，故不臆测映射关系。

最新串行 shell `sh_135e64476001QjMcVRgdJvRBJB` 已全部 exit 0：workspace all-targets、host/UI tests、WASM、local-Skia Android aarch64 UI/app check 与 Clippy。下表按其最终计数更新；此前两条 shell ID 保留为先前验证记录索引。

| 命令 | 记录结果 | 证据边界 |
|---|---|---|
| `cargo check --workspace --all-targets --locked` | 通过；清理约 206.8 GiB `target` 后冷编译 | 编译通过不证明 HostCore 场景运行或 P2 gate 通过 |
| `cargo test -p cad-engine --lib --locked` | 36 passed | 合成单测 |
| `cargo test -p cad-host --lib --locked` | 33 passed | 合成单测；不等于实际 host 场景运行 |
| `cargo test -p app-web --lib --locked` | 5 passed | 纯逻辑测试，不是浏览器 E2E |
| `cargo test -p cad-ui-slint --lib --locked` | 129 passed | UI 库合成测试 |
| `cargo test -p cad-ui-slint --tests --locked` | 36 integration tests passed，包含 `concept_offscreen` | 测试名不证明硬件 GPU；不是真实窗口或真实 DWG |
| `cargo check --workspace --lib --target wasm32-unknown-unknown --locked` | 通过 | WASM 编译，不是浏览器运行 |
| `cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings` | 通过；此前 single-character lint 首次失败后已修复 | Android/Web clippy 不在此命令覆盖范围 |
| `cargo fmt -p cad-host -p cad-engine -p cad-ui-slint -p app-linux -p app-android -p app-web --check` | 通过 | 仅所列 package |
| `git diff --check` | 通过 | whitespace/diff 检查 |
| `SKIA_BINARIES_URL=file:///tmp/opencode/yacr-skia-cache/skia-binaries-b7f043e0b1e2a850e702-aarch64-linux-android-ganesh-gl-jpegd-jpege-pdf-vulkan.tar.gz cargo check --target aarch64-linux-android -p cad-ui-slint -p app-android --locked` | 通过，18.46 s | Android aarch64 UI/app 编译检查；不代表设备安装/运行 |

Architecture、fixture manifest、workflow、i18n 四静态门禁此前均通过；后续源码补丁未修改依赖、catalog、fixture 或 workflow，但仍须由主协调器 final rescan。完整 `cargo fmt --all -- --check` 仍被陌生未跟踪 `crates/cad-import-acadrust/examples/debug_fixture.rs` 的基线格式问题阻塞；不得修改该文件。当前 package-scope fmt 和 diff check 已通过。

## 合同证据口径

- Host idle bounds 合同只约束 setter/input/overlay revision，不覆盖 scene preparation、publication 或 redraw 稳定性。
- UI canvas 合同只证明 flush 不覆盖 session；不证明 safe-insets、DPR、拾取/缩放等端到端行为。
- Web 的 5 项测试仅覆盖纯逻辑，不覆盖 Reflect/browser 行为或 browser E2E。
- `concept_offscreen` 是 integration test 名称，不据此推断实际 GPU 后端；没有真实窗口、真实 DWG 或 GPU 验收。
- locale 合同目前只断言 raw code 与 summary；未验证 Key 参数完整性或真实 locale-change callback。
- attempt 4 接受的 host primary-drag 正交 3D Orbit 与 UI summary 对象计数修补包含在后续串行验证 sweep；host/UI tests 通过。locale 合同证据仍仅覆盖 raw code 与 summary，不覆盖 Key 参数或实际 callback。

## Android、Skia 与 NOT RUN

- Skia tarball：`/tmp/opencode/yacr-skia-cache/skia-binaries-b7f043e0b1e2a850e702-aarch64-linux-android-ganesh-gl-jpegd-jpege-pdf-vulkan.tar.gz`，7,098,908 B；gzip、tar、key/tag、aarch64 校验通过；SHA-256 `0d917b608d03632fa8428e00ff715bbe5663f89618893a5e0265ffea3e6cf519`。仅位于 `/tmp`。未做签名验证；local-Skia Android aarch64 UI/app check 已通过，不代表设备运行。
- Android 原生构建先前两次分别在 120 s 与 600 s 超时；没有从那两次得到编译结论。随后 local-Skia Android aarch64 UI/app check 已在最终串行 shell 通过（18.46 s）；这只是交叉编译检查，不是模拟器/真机运行。Skia tarball 校验和/路径见下方。
- 本阶段运行过真实 GPU + winit 窗口的合成/开源 fixture 用例（见下方分轮证据）；未运行真实/vendor DWG、Android 模拟器/真机或 Web browser E2E。设备/显示硬件存在不代表已覆盖其它运行验收。
- 本文档 lane 未运行 Cargo；最终 compile/check/test 结果由串行 shell 记录，最终静态四门禁 rescan 仍由主协调器负责。

## 续做

P2/P3/P4 codegate 与 focused tests/clippy 不替代未覆盖的运行验收。P3 六个同步路径硬件用例及独立异步 `MoveCursor` 真实窗口用例均已通过；多 cursor/队列压力、浏览器 RAF runtime 与视觉/跨平台产品验收仍未完成。P4 App/engine/UI/Host/Android/Web 指定源码接线、workspace sweep 与末次差异门禁已通过。

## 2026-10-14 真实硬件 GPU 分轮证据

以下三轮必须作为独立证据理解，不拼成“同一脚本一次全绿”：

1. **首轮六用例，1 PASS / 5 FAIL。** 命令：
   `YACR_REAL_E2E_ROOT=/tmp/opencode/yacr-p2-real-gpu YACR_TEST_DWG=/home/zenglanmu/sources/rust/yacr/fixtures/plot/a4-layout.dwg bash scripts/run-real-gpu-e2e.sh`，shell `sh_135ee5c09001wSG3U75Y0dNZ6j`，脚本 exit 1。Case 5 MOVE/undo/redo PASS，发生在 Host bounds 修复前。Cases 1–4 的失败是输入不匹配：`a4-layout.dwg` 只有 paper-space line，却用于要求 model-space bounds/scene 的测试。Case 6 是真实 Host P2 回归：screen top-left/bottom-right world bounds 的 Y 倒置使水平网格族缺失；最初四边差分 `[781,852,571,0]`。完整失败记录与 artifacts 仍保存在 `/tmp/opencode/yacr-p2-real-gpu/20261013-154256/`。
2. **模型空间输入复跑 cases 1–4，4/4 PASS。** 使用已授权 synthetic `fixtures/dwg/synthetic-four-lines.dwg`，SHA-256 `68637e120e71e471eccb5cfe593a2da156444b8ef2e8669bc4f521ce32c38135`。逐用例独立进程，设 `YACR_TEST_DWG` 为该绝对路径并 unset `VK_ICD_FILENAMES`，运行 `cargo test -p app-linux --lib <CASE> --locked -- --ignored --test-threads=1 --nocapture`；四例 exit 0。此举证明前四例是 fixture/input mismatch，不是 host/UI/app 接线缺陷，也不是 vendor DWG 兼容性结论。中间一次 wrapper 漏 `export YACR_TEST_DWG` 的失败与 case4 fallback pass 属操作者错误，保留在 `/tmp/opencode/yacr-p2-real-gpu-model-20261013-160356/`，不得计入产品结果。四例复跑证据在 `/tmp/opencode/yacr-p2-real-gpu-model-20261013-160427/`。
3. **Host bounds 修复后只重跑 case 6，完整阶段 PASS。** `cad-host` 对 2D screen→world 四角结果逐轴取 min/max，3D bounds 为 `None`；新增场景网格正/负合同及 controller/render 尺寸对照。命令：`YACR_REAL_MAXIMIZED_OUTPUT=<fresh-dir> cargo test -p app-linux --lib real_gpu_maximized_grid_crosshair_navigation_acceptance --locked -- --ignored --test-threads=1 --nocapture`，exit 0 / 3.34s；证据在 `/tmp/opencode/yacr-p2-real-gpu-grid-fixed-20261013-161501/`。maximize、grid、crosshair、zoom-out、pan、restore/resize、re-maximize 全阶段通过；四边差分 `[781,2520,851,278]` 全部 > 0；controller/render canvas 尺寸相等；7 张 PNG 非空。

**分轮汇总边界：** case 5 的唯一首轮 PASS 来自 Host bounds 修复前；model 轮只重跑 case 1–4；修复后只重跑 case 6。不能称为同一次 `run-real-gpu-e2e.sh` 六用例全通过。先前将 case6 归为 “sampling fragile/断言敏感” 的解释已撤回，不再作为结论。

**硬件证据：** 真实 winit 窗口，case6 日志 `backend=Some("vulkan")`；测试进程 PID `1218709` 映射到 NVIDIA Quadro P620（UUID `GPU-40bcb3db-d898-76ff-f17e-ab0f735df81a`）；`VK_ICD_FILENAMES` 未设置，未使用 software ICD、lavapipe 或 SwiftShader。只说明所测进程使用该 GPU，不扩大成 vendor/平台兼容结论。

**同阶段编译/合同门禁：** Host bounds 修复后 `cargo check --workspace --all-targets --locked` 通过；`cargo test -p cad-host --lib --locked` 34 tests passed；`cargo test -p app-linux --lib grid_overlay_spans_both_axes_across_the_viewport --locked` 与 `cargo test -p app-linux --lib grid_contract_rejects_inverted_or_shortened_bounds --locked` 均通过（各 1 test）；`cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings` 与 `cargo fmt -p app-linux -p cad-host --check` 通过。此前 WASM lib 与 Android compile 通过发生在 Host bounds 修复前，修后未复跑，不作为覆盖末次 Host 改动的最终证据。以上是 P2 证据形成时的状态；P3 后续状态见本文新章节。

**未验收：** 无 visual golden/reference，不下视觉正确结论；未跑 vendor/真实 DWG、Android/Web 运行验收。硬件 GPU smoke 与网格像素/几何合同不替代产品图纸视觉验收。

## P3 queue codegate 与验证状态

**结论边界：** Oracle 最终接受 P3 codegate。此为契约/源码审查结论，不替代完整运行验收。最新串行 shell `sh_1365feb790017xIWEfljOrAD2G` 全部 exit 0。P3 六用例真实硬件脚本已 6/6 PASS，独立异步 `MoveCursor` 真实窗口用例也 1/1 PASS；输入为 manifest 已授权、含 model-space LINE 的 `fixtures/dwg/synthetic-four-lines.dwg`。P2 早期失败/分轮 GPU 证据保留，不与本轮混算。异步单例不等于多 cursor 队列压力验收。

**最新 sweep（协调器执行，本 lane 未运行 Cargo）：**

- `cargo check --workspace --all-targets --locked`：通过。
- Host、Web 合成测试：`cargo test -p cad-host --lib --locked` 69 passed；`cargo test -p app-web --lib --locked` 15 passed。
- Engine 50、Android 29、UI 131 + 37 integration、Linux 12 passed / 8 ignored 是此前仍有效的独立记录，不声称来自 shell `sh_1365feb790017xIWEfljOrAD2G`；均为编译/合成测试证据，不是设备或浏览器运行。
- `cargo check --workspace --lib --target wasm32-unknown-unknown --locked`：通过。
- `SKIA_BINARIES_URL=file:///tmp/opencode/yacr-skia-cache/skia-binaries-b7f043e0b1e2a850e702-aarch64-linux-android-ganesh-gl-jpegd-jpege-pdf-vulkan.tar.gz cargo check --target aarch64-linux-android -p cad-ui-slint -p app-android --locked`：通过（本地 Skia）；只是 Android 原生目标编译检查。
- `cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings`：通过。六个变更 package 的 scoped fmt 与 `git diff --check` 通过。完整 `cargo fmt --all -- --check` 仍被陌生的 `crates/cad-import-acadrust/examples/debug_fixture.rs` baseline 格式问题阻塞，未修改该文件。
- architecture/fixture/workflow 静态门禁此前通过；i18n 306 项在 test-cfg 修订后通过。除上述记录外不推断它们属于最新串行 shell；主协调器负责最终状态核对。

**P3 冻结契约与实现验收范围：** owner-thread bounded queue 上限 256 outstanding（normal 240 + terminal reserve 16），四类 request；coalesce 保留完整 context、只合并队尾并产生占 terminal quota 的 `Superseded` completion。Request ID 使用 checked allocation，失败显式原子报错。Pump 上限 64 requests / 4 ms，预算只在 request 之间检查，单个 dispatch 不可抢占。WASM 时钟由宿主注入真实 `performance.now()`，不使用伪时钟；document epoch、config/metrics/tool generation stale guards 保持。同步请求 guard、file/user-gesture 同步路径保留；UI 仅 `MoveCursor` 异步，其余同步。UI 先 pump 再 poll completion；pending/busy/old-cursor error 不得吞掉新 cancel，pump failure 与 execution error 按契约优先级处理。平台 waker 是单次/弱所有权；Web 采用 RAF 管理，不引入永久 10 ms timer；必要 I/O polling 保留。Web external-file open 走 Host pipeline，保留实际同步快照与 current-attempt 报告语义。

**Oracle reject / remediation 历史（保留，不抹除）：** 已闭环项包括 terminal reserve 被 normal request 侵占、公开 sync 路径绕过队列、config stale、epoch late completion、UI completion 漏取及吞新 cancel、Web probe/绕 epoch、RAF scheduling failure、Scroll 重复正规化、对真实 closure-drop 的错误证明、旧 async terminal 遮蔽新 picker。最终 accept 仅覆盖已修复源码契约/codegate；不代表这些路径已经通过浏览器/设备端运行验收。

**Web RAF 所有权口径更正：** 成功注册的 `Closure` 由 `LIVE_RAF_CALLBACK` 槽持有且不 `forget`；真实 RAF callback 入口先 take 当前句柄、在 `RefCell` borrow 外 drop，再 pump。注册失败则立即 drop。schedule 前清理 previous slot 只是防御性清理，不是当前 callback 的主释放保证。纯 helper 的计数只测调度逻辑，不是浏览器实际分配/释放 memory probe；实际浏览器 RAF/closure 行为尚未运行验证。

**真实硬件命令与 artifacts：** 报告记录的命令为 `export YACR_TEST_DWG=/home/zenglanmu/sources/rust/yacr/fixtures/dwg/synthetic-four-lines.dwg; export YACR_REAL_E2E_ROOT=/tmp/opencode/yacr-p3-real-gpu-20261013-174754; unset VK_ICD_FILENAMES; bash scripts/run-real-gpu-e2e.sh`（exit 0，`all tests passed`）。artifact 根目录按报告原文记录为 `/tmp/opencode/yacr-p3-real-gpu-20261013-174754/20261013-174754/`；保留该路径日期标记，不以当前 session 日期推断。报告记录的五个图形用例进程映射到 P620/Vulkan；`real_dwg_cpu_load_stages` 不创建 GPU context。Wayland/winit、逐用例 PID 与环境证据见 `.slim/deepwork/cad-engine-decoupling/p3-real-gpu.md`。

**覆盖边界：** 六例覆盖同步 pointer/select、wheel `Scroll`、pan、渲染与选择/历史路径。新增独立异步用例 `real_gpu_async_cursor_submit_wake_and_cancel_acceptance` 经真实 UI pointer-move callback 调用异步 submit；事件循环驱动 wake，验证 no-inline、preview 权威/渲染一致、坐标误差不超过半像素、input revision/frame 前进、Cancel 零 DB 写入及 idle 稳定。该用例**不**覆盖多 cursor、QueueFull/reserve/coalesce/backpressure、Busy 高负载或 confirm-repeat。

**异步硬件用例证据：** 首次编译因误用不存在的 `view.input_revision()` 失败；按公开 API 修为 `view.render_session().input_revision()` 后通过，未新增 API/假 counter、未削弱断言。真实窗口测试命令：`YACR_REAL_ASYNC_CURSOR_OUTPUT=/tmp/opencode/yacr-p3-async-cursor-20261013-180242/evidence cargo test -p app-linux --lib real_gpu_async_cursor_submit_wake_and_cancel_acceptance --locked -- --ignored --test-threads=1 --nocapture`，1 passed，1.99s。真实 Wayland/winit + Vulkan，PID `1397363` 映射至 P620 UUID `GPU-40bcb3db-d898-76ff-f17e-ab0f735df81a`；artifact 根目录 `/tmp/opencode/yacr-p3-async-cursor-20261013-180242/`。报告见 `.slim/deepwork/cad-engine-decoupling/p3-async-cursor-gpu.md`。编译首失败记录与修复后 PASS 证据均保留。

**仍 NOT RUN / 不得外推：** 浏览器 RAF/Reflect/GC/WebGPU、Android 设备、vendor DWG、视觉 golden 均未运行验收。真实桌面 P620 用例不代表全平台；queue pressure/lag/reserve/coalesce/32 subscribers 仅由合成契约覆盖，browser ring 不跨页面持久化。无插件 loader/React/双 surface 验收。

## P4 focused codegate 与分层验证状态

**App（仅 `cad-app`）：** 原子受控 Commit、候选数据库完整 batch 验证、selection validation、traced command 与共享 `Arc<ChangeSet>` 已实现。Oracle App codegate accept。旧有 owned changes / DB mutate 后旧 command history 失败问题已通过 candidate→history→publish 顺序修复；新合同覆盖 history 拒绝时数据库、allocator、redo 状态不变以及 capture 保留。App 343 lib tests 与 App clippy 通过（按实现报告）。

**Engine（仅 protocol/queue/events）：** Oracle engine codegate accept；bounded EventRing/permit 与批发布行为已实现。此前初始 phantom 16 reservation、reserve exhausted 误杀旧 permit、非原子 batch publish、最后一批 unread events 被 `Exhausted` 吞掉、last-batch drop 等拒绝项均已修复并有 contracts。最新两包 shell `sh_137a3f0b3001RSKc4gOjTAnA8x`：engine 70 tests 与两包 clippy 通过。既有 permit 可兑现、unread last batch 先交付；这些是代码/合成合同证据，不是平台运行验收。

**UI compatibility：** shell `sh_137adcff0001KP0ImwLEq3shNv` 的 UI 132 lib + 37 integration 与 clippy 通过。UI 仅作 `Arc<CommandOutcome>`/`RequestResult::Api` 机械兼容，API 误路由显式报 invariant；mock 明确返回 Busy/Unsupported。没有自动 event subscription、插件 UI、`.slint` 或 catalog 改动。这不等于 UI 已可消费事件产品流。

**分层验证边界：** P4 final shell 与后续 Host/Linux focused、真实硬件证据见下节。历史拒绝/失败记录保留；下列终态只针对明确列出的源码、合同、平台与门禁范围。

## P4 最终集成、硬件及差异门禁

**codegate 与完整 sweep：** P4 App/engine/Host/Web Oracle codegate 均 accept。最终 sweep `sh_137d0698a001J14BRMEOwCRSk6` exit 0：workspace all-targets/tests；App 343、engine 73、Host 83、Android 30、Web 23、UI 132 + 37 integration、Linux 12 passed / 9 ignored；workspace WASM lib；Android aarch64 UI/app 使用 local Skia URL；Clippy（既有 exclusions）、7 changed packages fmt、architecture/fixture/workflow/i18n306 与 diff check 均通过。此前 `sh_137a3f0b3001RSKc4gOjTAnA8x` engine focused 70 tests + 两包 clippy、`sh_137adcff0001KP0ImwLEq3shNv` UI132 +37 + clippy 均通过，见分层记录。

**后续末次差异门禁：** Host 新增 3 contracts 后 test-only Host86 + clippy 通过；覆盖真实 Confirm commit 后 render failure 仍成功且 Undo 可还原、external backend failure 的真实 ID error completion/无 EntitiesChanged、action reentry Busy。Linux P4 ignored test 的 Arc outcome 解构编译错误已机械修复，新测试 source 的 Linux clippy/fmt 通过。协调器最终差异 shell `sh_137df9c5b00120goh09biLHuUM` exit 0：workspace all-targets check、Linux lib 12 passed / 10 ignored（其中 8 个指定 ignored 用例另有下列 hardware PASS）、7 changed packages fmt、architecture/fixture/workflow/i18n306、diff check 全通过。全 workspace 测试、WASM/Android 编译与 workspace clippy 复用 `sh_137d0698...` 证据，因为其后生产源码未变；末次差异主要为测试改动。陌生未跟踪 `debug_fixture.rs`/`debug-block.dwg` 未触碰，整个 dirty workspace 不归属本轮。

**最终真实硬件证据：** `.slim/deepwork/cad-engine-decoupling/p4-real-gpu.md` 的 artifact 时间标记为 `20261014-004223`，证据根 `/tmp/opencode/yacr-p4-final-20261014-004223/`。真实 Wayland/winit + NVIDIA Quadro P620/Vulkan，授权 `synthetic-four-lines.dwg`：新 `real_gpu_controlled_api_events_and_history_acceptance` 1/1、旧六用例脚本 6/6、async cursor 1/1，合计 8 terminal PASS；其中 `real_dwg_cpu_load_stages` 不创建 GPU context。新用例实际覆盖 controlled Commit、Select/highlight、request ID/origin/transaction/sequence、`Arc::ptr_eq` ChangeSet、Undo/Redo 与 idle。首次新测试编译的 `RequestResult::Api(Arc<ApiOutcome>)` 解构错误仅在测试内修正为保留 Arc 后通过；历史首错保留。

**终态与限制：** P1–P4 本轮解耦/队列/受控 API-event foundation 的 codegate、指定编译/合同测试及硬件用例闭环；不是完整 CAD 产品交付。订阅为进程内 bounded foundation，不构成外部插件兼容结论。未验 browser runtime/GC/RAF/Reflect/WebGPU、Android device、vendor DWG、视觉 golden；无 loader/React/双 surface。P2 paper-only/grid 初轮失败、P4 Oracle reject 及修复/合同历史均保留。当前仅此 docs lane 与协调器确认的最终门禁收尾，无运行 writer 或 Cargo task。

## CI build-layers `web-host-contracts` 失败修复（Node 合成合同）

**失败范围：** CI run `38495981291` 的 build-layers job `38495981291`（`gh run view ... --json jobs` 仅此 job `failure`，其余 job success/skipped）。失败步为 `Browser host module contracts (no wasm/GPU)`：`node --test scripts/test-web-host.mjs scripts/test-web-touch.mjs scripts/test-web-a11y.mjs scripts/test-web-pixels.mjs scripts/test-web-responsive.mjs scripts/test-host-panel-parity.mjs scripts/test-web-icon.mjs`，60 tests / 52 pass / 8 fail，全部来自 `scripts/test-host-panel-parity.mjs`（`not ok 1–6/8/9`）。无其它 failed job。

**根因判定：旧源码契约未迁移，不是真实 panel 漏线。** 该 test 固定的是 P2 之前的结构：`crates/cad-host/src/state.rs` 内直接调 `set_layer_state`/`set_property_state`/`set_layout_state`/`set_diagnostics_state`/`set_selection_highlight`/`set_measurement_preview`，用 `set_overlay_visibility(handle.effective_config()...)` 做 overlay gate，且各平台 overlay 必须含 `core.push()`。P2/P4 后共享腿已迁移为：

- `cad-host/state.rs`：派生 typed `HostState`/`NavigationState`，render overlay 经 `OverlayInputs` + `OverlayVisibility::from(self.viewer_config...effective().view.overlays...)`（config gate 仍在，只是表达式/位置变了）；`take_updates`/`HostUpdates` 在 `cad-host/input.rs`。
- `cad-ui-slint`：`EngineAdapter::apply_updates` → `UiHandle::apply_host_state`/`apply_config_snapshot`/`apply_backend_state` + `view.set_snap_hints`/`set_view_state`/`set_status_message` 才是 shell setter 的唯一写入点。
- 平台 overlay：web `browser.rs`/`state_push.rs`、android `poll.rs`/`state_push.rs` 在 `binding.flush_updates()` **之后** 只重推 host-specific 的 `push_import_state`/`push_resource_sections`。

逐通道核对（layer/property/layout/diagnostics/import/resources、selection/measurement/overlay-visibility、units/backend/diagnostics/counts）均在上述真实 owner 存在，未发现真实漏线。

**修复（仅 `scripts/test-host-panel-parity.mjs`）：** 契约重指到真实 typed-updates 管线：`state.rs` 派生通道 + `input.rs` 的 `take_updates`/`HostUpdates` + `engine_adapter.rs` apply 通道 + `handle.rs` panel setters/unit label + render overlay 的 effective-config gate；web/android overlay 断言 `flush_updates` 先于 overlay、且 overlay 文件不得含共享 authority setters（不覆盖 authority）。新增 negative-fixture 合同（丢 channel / 换 order / overlay 抢 authority setter 必须被判失败）。**未** 删除/skip/放松断言、**未** 扩大为全仓库文本搜索、**未** 改 workflow gate、**未** 恢复旧 `core.push()`/重复派发。

**终态证据：** 上述 exact workflow Node 命令 → `# tests 60 / # pass 60 / # fail 0 / # cancelled 0 / # skipped 0`；`scripts/test-host-panel-parity.mjs` focused 11/11 pass；`git diff --check` 通过。本 lane 仅改 test/docs，无 Cargo、无 product source、无 rootlock/workflow 改动。browser RAF/GC/WebGPU、Android device、vendor DWG、视觉 golden 仍未验收。
