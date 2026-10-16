# 真实硬件桌面集成验收（2026-10-11）

> **历史执行证据（Slint 桌面已退役，2026-10-16）**：本文记录的是**迁移前** Slint 宿主/模拟器/浏览器
> 的运行与验收证据，**不代表**当前 React/Tauri 宿主的编译、测试或真实 GPU 结论；Slint→React 迁移后的
> 最终验证 **NOT RUN**（P6）。当前架构见 `docs/architecture.md` 与
> [ADR 0010](adr/0010-react-linux-host-replaces-slint.md)。下列事实仅作历史记录，不得据此宣称当前平台可用。

## 环境与命令

- Revision：`0013140d57b9276a6de261881c13e207efc2be76`；本地工作树含本轮并行改动。
- DWG：`fixtures/complex-test.dwg`，SHA-256 `0d77832ee8eeb4d4303133b62b84f5b9769c4171f56dfc373c19fad0ec3633eb`，AutoCAD 2004/2005/2006 格式。该文件被 gitignore；本仓记录的原始来源/许可未知，本次只作本机样本，不作为可再分发或 vendor 兼容证据。
- 桌面：`DISPLAY=:0`、`WAYLAND_DISPLAY=wayland-0`。`nvidia-smi` 报告 Quadro P620，驱动 `580.178.04`，UUID `GPU-40bcb3db-d898-76ff-f17e-ab0f735df81a`。
- 构建：`cargo build -p app-linux --bin yacr-linux --locked`（通过）。
- 五用例入口：`WGPU_ADAPTER_NAME='Quadro P620' YACR_TEST_DWG=/home/zenglanmu/sources/rust/yacr/fixtures/complex-test.dwg YACR_REAL_E2E_ROOT=/tmp/opencode/yacr-final-integrated-20261011T115112Z-3818711/evidence bash scripts/run-real-gpu-e2e.sh`；启动前显式 `unset VK_ICD_FILENAMES`，runner 也自行清除。未用 lavapipe、SwiftShader 或 offscreen。
- 适配器证据：在各测试运行期间以 `nvidia-smi --query-compute-apps=pid,gpu_uuid,used_memory` 采样；测试 PID `3823806`、`3825826`、`3828745`、`3834204` 均由 NVIDIA 报告使用上述 Quadro P620 UUID，峰值分别 857、857、409、37 MiB。未将“未设置 ICD”本身当作硬件证明。原始采样见 `/tmp/opencode/yacr-final-integrated-20261011T115112Z-3818711/nvidia-smi-samples.tsv`，摘要 `gpu-process-summary.txt`。

## 五用例结果

证据根目录：`/tmp/opencode/yacr-final-integrated-20261011T115112Z-3818711/evidence/20261011-115337/`。脚本串行启动独立进程，未降低断言阈值。

| 用例 | 结果与观察 |
|---|---|
| `desktop_dwg_event_loop_and_navigation_remain_responsive` | **FAIL**；45s。DWG 完整呈现 `(101544, 101544)` batches，约 36.47s ready；指针选择交互耗时 **2.949s**，超过 2s 断言。日志 `desktop_dwg_event_loop_and_navigation_remain_responsive.log`。 |
| `desktop_dwg_open_after_start_and_interaction_remain_responsive` | **FAIL**；42s。启动后约 0.46s 请求打开，完整呈现 `(101544, 101544)` batches，约 35.49s ready；选择交互耗时 **2.953s**，超过 2s 断言。日志 `desktop_dwg_open_after_start_and_interaction_remain_responsive.log`。 |
| `real_dwg_cpu_load_stages` | **PASS**；20s。导入 3.399s、fit 0.550s、scene 15.529s；44,591 entities、97,791 batches、50,883,794 vertices。仅为真实 DWG 的 CPU 导入/场景构建证据，不代表 GPU 视觉保真。日志 `real_dwg_cpu_load_stages.log`。 |
| `real_gpu_grid_zoom_axes_acceptance` | **FAIL**；120.69s。新就绪条件要求非空 DWG、帧已呈现且进度 `complete == total > 0`；测试至 120s 未满足，在采集网格/缩放前超时。故本轮**没有**网格切换、wheel-up 或轴线验收结论，也没有本用例 PNG。日志 `real_gpu_grid_zoom_axes_acceptance.log`。 |
| `real_gpu_select_move_undo_redo_acceptance` | **FAIL**；0.95s。该用例使用提交的 QCAD flange DXF；初始窗口截图成功，但真实窗口单击没有选中预期 LINE，在 MOVE/Undo/Redo 前失败。日志 `real_gpu_select_move_undo_redo_acceptance.log`。 |

汇总 **1 PASS / 4 FAIL**；完整 runner 输出 `…/e2e-runner.log`。本次无用例被跳过。两项复杂 DWG 窗口测试达到完整呈现后均在选择交互处超过 2s；网格测试则在其新的全量完成就绪门限前超时，属于另一条未闭环证据，不能用前两项 ready 代替。

## 截图、打开状态与限制

- 完整窗口测试快照：`/tmp/opencode/yacr-final-integrated-20261011T115112Z-3818711/evidence/20261011-115337/desktop-event-loop/initial.png`、`…/desktop-open-after-start/initial.png`（均 1280×800）。
- 功能测试初始快照：`/tmp/opencode/yacr-final-integrated-20261011T115112Z-3818711/evidence/20261011-115337/real-gpu-select-move-history/01-initial.png`（1280×800）；失败点之后未生成选择、移动、撤销或重做截图。
- runner 结束后，以 `target/debug/yacr-linux --open /home/zenglanmu/sources/rust/yacr/fixtures/complex-test.dwg --gpu high` 启动更新后的应用并保持运行：最终 PID `3845860`。`nvidia-smi` 将该 PID 映射到 Quadro P620 同一 UUID，稳定采样 857 MiB；采样为 `/tmp/opencode/yacr-final-integrated-20261011T115112Z-3818711/live-app-gpu.tsv`，进程日志为 `…/live-app.log`。此前对 GNOME Shell Screenshot D-Bus 的调用返回 `AccessDenied: Screenshot is not allowed`，未尝试绕过权限；没有本轮 live-app 全桌面截图。此前残留 PID `3756747` 经核验完整命令行后只对该 PID 发 SIGTERM，退出且其旧日志/证据未删除。
- 没有参考黄金图，故**未做视觉验收**；当前样本来源不明，不代表厂商 DWG 兼容性。当前硬件轮未覆盖测量、图层编辑、导出、New 的真实窗口操作；上述测试也不证明这些功能可用。轴线仅在本轮功能失败前没有新的视觉结论。

## 构建门禁

- `cargo fmt --all -- --check`：**PASS**（并行 adapter import 的最小格式更正后复跑通过）。
- `cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings`：**PASS**。
- `cargo check --workspace --exclude app-android --exclude app-web --all-targets --locked`：**PASS**。
- `cargo check --workspace --lib --target wasm32-unknown-unknown --locked`：**PASS**。
- 父级已运行的四项 Python 静态门禁本轮未重复；不据此补充本轮独立执行结论。

## 带选取剖析的真实 GPU 复跑（2026-10-11 12:29 UTC）

本轮只做诊断执行，没有源码改动。启动时旧 live-app PID `3845860` 已不存在，故未向其他进程发信号；GPU compute-app 查询为空。先执行 `cargo build -p app-linux --bin yacr-linux --locked`（通过），然后以 `YACR_PROFILE_PICK=1 WGPU_ADAPTER_NAME='Quadro P620' YACR_TEST_DWG=/home/zenglanmu/sources/rust/yacr/fixtures/complex-test.dwg` 运行完整五用例脚本，显式 unset `VK_ICD_FILENAMES`，fresh root 为 `/tmp/opencode/yacr-profile-pick-20261011T122850Z-3891449/evidence/20261011-122949/`。未使用软件 Vulkan/offscreen。最终更新应用以 `target/debug/yacr-linux --open /home/zenglanmu/sources/rust/yacr/fixtures/complex-test.dwg --gpu high` 重开并保持运行；PID `3901294`，nvidia-smi 见 P620 UUID `GPU-40bcb3db-d898-76ff-f17e-ab0f735df81a`，857 MiB。采样在 `/tmp/opencode/yacr-profile-pick-20261011T122850Z-3891449/live-app-gpu.tsv`。

硬件进程证据：测试进程 PID `3893240`、`3895296`、`3898240`、`3900158` 均由 `nvidia-smi` 映射到同一 Quadro P620 UUID，显存峰值依次 857、857、857、53 MiB；原始 TSV 是 `/tmp/opencode/yacr-profile-pick-20261011T122850Z-3891449/nvidia-smi-samples.tsv`。网格与功能用例日志另明确报告 `backend=Some("vulkan")`。

五用例 **3 PASS / 2 FAIL**（runner 用例墙钟 43/42/21/41/2s）：

- 两项 complex-test 窗口交互仍 FAIL：event-loop ready 36.402s / open-after-start presentation 36.041s；选择时间分别 **2.961280s / 2.909866s**，仍超过 2s。
- `YACR_PROFILE_PICK` 将耗时定位到 CPU 拾取：各次 `pick_at_screen` 对 `723587` 个展开 pick item；event-loop `borrowed_pick_items` 208.526ms、`pick_closest_borrowed` **2677.087ms**、drop 33.003ms、合计 pick 2918.658ms；open-after-start 分别 181.920ms、**2660.420ms**、32.926ms、总计 2875.304ms。选择前 `select_metrics/context` 合计不足 0.01ms；随后 controller 约 0.002ms、push 约 1.7ms、`select_command` 约 1.93ms。故可见瓶颈是 72 万级候选的 `pick_closest_borrowed` CPU 扫描，不是 UI push 或设备提交；本轮不改 picker/性能实现。
- CPU import case PASS：import 3.457s、fit 0.531s、scene 15.525s，44,591 entities、97,791 batches、50,883,794 vertices。
- grid/zoom/axes case PASS：在完整呈现后完成，40.45s；Vulkan、窗口截图 1280×800，CAD rect `[240,180,1040,500]`；canvas content changed 24,622 pixels，网格开关 changed 6,854 pixels，wheel-up `world_per_px 1231.320362715225 → 1043.4918328095127`。轴线仍须人工看 `01-axes-initial.png`。
- flange DXF selection/MOVE/undo/redo case PASS：选中 `EntityId(306)`，数据库和 UI history 状态往返验证通过，2.01s，Vulkan。flange 有 223 个 pick items；profiling 的 `pick_closest_borrowed` 约 0.05–0.09ms、完整 pick 约 0.07–0.13ms，显著低于大 DWG。

本轮完整日志：`/tmp/opencode/yacr-profile-pick-20261011T122850Z-3891449/evidence/20261011-122949/`；runner 汇总 `…/e2e-runner.log`。PNG 全路径（均 1280×800，待父级视觉核对）：

- `/tmp/opencode/yacr-profile-pick-20261011T122850Z-3891449/evidence/20261011-122949/desktop-event-loop/initial.png`
- `/tmp/opencode/yacr-profile-pick-20261011T122850Z-3891449/evidence/20261011-122949/desktop-open-after-start/initial.png`
- `/tmp/opencode/yacr-profile-pick-20261011T122850Z-3891449/evidence/20261011-122949/real-gpu-grid-zoom-axes/01-axes-initial.png`
- `/tmp/opencode/yacr-profile-pick-20261011T122850Z-3891449/evidence/20261011-122949/real-gpu-grid-zoom-axes/02-grid-on.png`
- `/tmp/opencode/yacr-profile-pick-20261011T122850Z-3891449/evidence/20261011-122949/real-gpu-grid-zoom-axes/03-wheel-up.png`
- `/tmp/opencode/yacr-profile-pick-20261011T122850Z-3891449/evidence/20261011-122949/real-gpu-select-move-history/01-initial.png`
- `/tmp/opencode/yacr-profile-pick-20261011T122850Z-3891449/evidence/20261011-122949/real-gpu-select-move-history/02-selected.png`
- `/tmp/opencode/yacr-profile-pick-20261011T122850Z-3891449/evidence/20261011-122949/real-gpu-select-move-history/03-moved.png`
- `/tmp/opencode/yacr-profile-pick-20261011T122850Z-3891449/evidence/20261011-122949/real-gpu-select-move-history/04-undone.png`
- `/tmp/opencode/yacr-profile-pick-20261011T122850Z-3891449/evidence/20261011-122949/real-gpu-select-move-history/05-redone.png`

样本来源仍未知、无黄金参考图；不据此作 vendor DWG 兼容或全功能结论。测量、图层操作、导出、New 的真实窗口使用仍未覆盖。

## 最终六用例与门禁（2026-10-11 13:19 UTC）

最终集成轮目录：`/tmp/opencode/yacr-final-sixcase-20261011T131731Z-3986781/`；runner/证据目录：`/tmp/opencode/yacr-final-sixcase-20261011T131731Z-3986781/evidence/20261011-131918/`。fixture 为绝对路径 `/home/zenglanmu/sources/rust/yacr/fixtures/complex-test.dwg`。运行前对仍存活且命令行完全匹配的旧 live app PID `3901294` 发 SIGTERM 并确认退出；其后无 NVIDIA compute app。设置 `YACR_PROFILE_PICK=1`、`WGPU_ADAPTER_NAME=Quadro P620`，显式 unset `VK_ICD_FILENAMES`；无软件 ICD、offscreen 或 SwiftShader。脚本六用例逐个 cargo test 独立进程串行执行。

硬件证明：`nvidia-smi` 为 Quadro P620 / driver 580.178.04 / UUID `GPU-40bcb3db-d898-76ff-f17e-ab0f735df81a`。测试 PID `3990888`、`3992967`、`3996105`、`3998110`、`3998298` 均被采样到该 UUID，峰值依次 863、863、863、91、91 MiB；采样原文 `/tmp/opencode/yacr-final-sixcase-20261011T131731Z-3986781/nvidia-smi-samples.tsv`。涉及 GPU 的测试日志报告 `backend=Some("vulkan")`。

| 用例 | 结果 | 实际观察 |
|---|---|---|
| `desktop_dwg_event_loop_and_navigation_remain_responsive` | FAIL，44s | DWG ready 37.540s；select 2.787088s（2s 阈值）。723,587 items；展开 204.934ms、`pick_closest_borrowed` 2509.193ms、drop 37.057ms、pick 总计 2751.218ms。 |
| `desktop_dwg_open_after_start_and_interaction_remain_responsive` | FAIL，46s | open 请求 454.671ms、present 38.911s；select 2.819018s。723,587 items；展开 215.752ms、`pick_closest_borrowed` 2526.384ms、drop 39.033ms、pick 总计 2781.204ms。 |
| `real_dwg_cpu_load_stages` | PASS，20.50s | import 3.447s、fit 0.572s、scene 15.889s；44,591 entities、97,791 batches、50,883,794 vertices。 |
| `real_gpu_grid_zoom_axes_acceptance` | PASS，42.24s | Vulkan；窗口 1853×1013，CAD rect `[240,180,1613,713]`；content 38,651 changed pixels，grid 8,170；wheel-up world/px `839.2856283978215 → 711.2590071167979`。 |
| `real_gpu_select_move_undo_redo_acceptance` | PASS，2.50s | Vulkan；flange entity `306` 被选中，MOVE/undo/redo DB/UI history 均验证；223 pick items，单次 pick 总计约 0.076–0.095ms。 |
| `real_gpu_maximized_grid_crosshair_navigation_acceptance` | FAIL，21.39s | startup maximize 截图和全边 grid 阶段完成；phase 2 crosshair 等待渲染失败。诊断原文为 `loading=false`，entities `419`，`frames=5, presented=true, progress=Some((236,236)), loading_phase=None, last_error=None, scene_error=None, backend=Some("vulkan")`，停在 `phase 2 did not render`。`frame_baseline` 在 pointer move 前取值；可见 frame 数仍为 5。属于该用例当前的 readiness predicate/指针 overlay invalidation 需要核查，不能据此声称 crosshair/后续 zoom-pan-restore-resize 已验收；本轮未改 predicate。 |

汇总 **3 PASS / 3 FAIL**，无跳过，未放宽断言。最终日志包括各 case 的完整失败诊断与剖析行，均位于上述 evidence 目录。选择瓶颈仍明确在大 DWG `pick_closest_borrowed` 的 CPU 候选扫描，非命令推送；本轮未改性能逻辑。

可用截图（全路径前缀 `/tmp/opencode/yacr-final-sixcase-20261011T131731Z-3986781/evidence/20261011-131918/`）：`desktop-event-loop/initial.png`、`desktop-open-after-start/initial.png`、`real-gpu-grid-zoom-axes/{01-axes-initial.png,02-grid-on.png,03-wheel-up.png}`、`real-gpu-select-move-history/{01-initial.png,02-selected.png,03-moved.png,04-undone.png,05-redone.png}`、`real-gpu-maximized-grid-crosshair/{01-maximized-initial.png,02-maximized-grid.png}`。PNG 均等待父级最终视觉复核；无 golden，因此不作视觉保真或 vendor 兼容结论。

最终门禁（格式化修正后复跑）：`cargo fmt --all -- --check` PASS；clippy FAIL（`crates/cad-spatial/src/pick.rs:579 needless_option_as_deref`）；native workspace check PASS；full workspace wasm lib check PASS；`check-architecture.py` PASS；`check-fixture-manifest.py` PASS；`check-workflows.py` PASS；`check-i18n.py` FAIL（scanner 报 `apps/app-linux/src/host/validation.rs: more than one #[cfg(test)] near EOF`）。`cargo build -p app-linux --bin yacr-linux --locked` PASS。为满足最终 fmt gate，本轮只运行了 `cargo fmt --all` 机械格式化；未修复 clippy/i18n 或任何测试行为。

最终更新应用仍运行：PID `4006015`，命令 `target/debug/yacr-linux --open /home/zenglanmu/sources/rust/yacr/fixtures/complex-test.dwg --gpu high`；nvidia-smi 映射至上述 P620 UUID，863 MiB。采样 `/tmp/opencode/yacr-final-sixcase-20261011T131731Z-3986781/live-app-gpu.tsv`，日志 `…/live-app.log`。本轮未覆盖测量、图层、导出、New 的真实窗口功能。图纸来源/许可未知；无 golden/reference，不据此作兼容性或 fidelity 结论。

## compound-cache / hover screenshot 等待后的复跑（2026-10-11 13:45 UTC）

此前最终应用 PID `4006015` 已不在运行，故未向其他 PID 发信号。fresh root `/tmp/opencode/yacr-final-sixcase-rerun-20261011T134407Z-4039013/`；六用例 evidence `/tmp/opencode/yacr-final-sixcase-rerun-20261011T134407Z-4039013/evidence/20261011-134532/`。`YACR_PROFILE_PICK=1`、`WGPU_ADAPTER_NAME=Quadro P620`、完整 DWG 路径；显式 unset `VK_ICD_FILENAMES`，runner 自身亦清除；串行一用例一进程，无 lavapipe/SwiftShader/offscreen。

NVIDIA 实际 PID 采样：`4042670`、`4044880`、`4047924`、`4049874`、`4050088` 均映射到 Quadro P620 UUID `GPU-40bcb3db-d898-76ff-f17e-ab0f735df81a`，峰值分别 863、863、863、91、91 MiB。原始采样 `/tmp/opencode/yacr-final-sixcase-rerun-20261011T134407Z-4039013/nvidia-smi-samples.tsv`；运行前后 GPU 信息在 `gpu-before.csv` / `gpu-after.csv`。涉及渲染的 case log 报告 Vulkan。

六用例 **3 PASS / 3 FAIL**，均实际执行，无跳过：

- 两个 complex DWG 选择响应 case FAIL：ready 39.907s、open-after-start present 38.002s；交互分别 **3.729284s** 与 **3.623915s**，未达到 2s 门限。均为 723,587 候选；`borrowed_pick_items` 187.625 / 202.896ms，`pick_closest_borrowed` **3464.131 / 3346.505ms**，drop 40.274 / 38.409ms，pick 总时长 3692.067 / 3587.862ms。compound cache 本轮未达到 2s；没有 `next heartbeat` 或最终像素判据成功的证据，不能据此报成功。
- `real_dwg_cpu_load_stages` PASS 19.89s：import 3.356s、fit 0.506s、scene 15.469s；44,591 entities、97,791 batches、50,883,794 vertices。
- `real_gpu_grid_zoom_axes_acceptance` PASS 41.20s：1853×1013、CAD rect `[240,180,1613,713]`；content 像素变化 38,611，grid 8,170；wheel-up world/px `839.2856283978215 → 711.2590071167979`。
- `real_gpu_select_move_undo_redo_acceptance` PASS 2.50s：Vulkan，EntityId(306)，MOVE/undo/redo DB/UI history 验证通过；223 pick items，pick 总计约 0.091–0.106ms。
- `real_gpu_maximized_grid_crosshair_navigation_acceptance` FAIL 17.06s，但较前次推进：initial maximize、四边 grid 检查、off-center crosshair 覆盖检查、zoom-out 与 pan 阶段均运行并产生截图；失败于恢复窗口 resize。具体为请求 `PhysicalSize { width: 1773, height: 953 }`，窗口实际仍为 `PhysicalSize { width: 1280, height: 800 }`，在 assertion `restored-window resize did not settle` 超时。没有 restored-resize 或 re-maximized 最终截图，不能声称 resize/re-maximize 验收通过。此前 phase-2 frame-count 等待问题本轮已越过；当前失败是另一个可见尺寸收敛失败。

本轮日志与剖析全文在 evidence 目录。关键截图前缀 `/tmp/opencode/yacr-final-sixcase-rerun-20261011T134407Z-4039013/evidence/20261011-134532/`：`desktop-event-loop/initial.png`、`desktop-open-after-start/initial.png`、`real-gpu-grid-zoom-axes/{01-axes-initial.png,02-grid-on.png,03-wheel-up.png}`、`real-gpu-select-move-history/{01-initial.png,02-selected.png,03-moved.png,04-undone.png,05-redone.png}`、`real-gpu-maximized-grid-crosshair/{01-maximized-initial.png,02-maximized-grid.png,03-maximized-crosshair-off-center.png,04-maximized-zoom-out.png,05-maximized-pan.png}`。等待父级视觉复核；无 golden/reference，样本来源/许可未知，不作 vendor/fidelity 声明。

本轮最终门禁：fmt PASS；workspace clippy **FAIL**，`crates/cad-spatial/src/pick.rs:1447` 测试代码触发 `clippy::redundant_pattern_matching`（建议 `is_none()`）；native workspace check PASS；全 workspace wasm lib check PASS；architecture、fixture manifest、workflows、i18n 四门禁均 PASS。最终 `cargo build -p app-linux --bin yacr-linux --locked` PASS。没有为消除 lint 或 E2E 失败改源码/断言。

复跑后应用 PID `4052352` 仍运行，命令 `target/debug/yacr-linux --open /home/zenglanmu/sources/rust/yacr/fixtures/complex-test.dwg --gpu high`；`nvidia-smi` 映射到上述 P620 UUID，863 MiB。采样 `/tmp/opencode/yacr-final-sixcase-rerun-20261011T134407Z-4039013/live-app-gpu.tsv`，日志 `…/live-app.log`。按现有 LinuxApp 启动实现，非 headless 窗口默认调用 `set_maximized(true)`；不代表跨窗口管理器/平台保证。

## compound-only local-bounds cache 与 Wayland configure 稳定等待后复跑（2026-10-11 14:06 UTC）

启动时 PID `4052352` 经核验命令行为 `target/debug/yacr-linux --open /home/zenglanmu/sources/rust/yacr/fixtures/complex-test.dwg --gpu high` 后发 SIGTERM 并确认退出。新 evidence root：`/tmp/opencode/yacr-compound-localbounds-20261011T140511Z-4080521/evidence/20261011-140615/`。环境为 `YACR_PROFILE_PICK=1`、`WGPU_ADAPTER_NAME=Quadro P620`、完整 DWG 路径；强制 unset `VK_ICD_FILENAMES`，runner 也清除，六 case 串行。无软件 ICD/offscreen/SwiftShader。

NVIDIA PID 证据：测试 PID `4083039`、`4085051`、`4087945`、`4089828`、`4090006` 均映射到 P620 UUID `GPU-40bcb3db-d898-76ff-f17e-ab0f735df81a`，峰值显存依次 863、863、863、91、91 MiB。原始采样 `/tmp/opencode/yacr-compound-localbounds-20261011T140511Z-4080521/nvidia-smi-samples.tsv`。相关渲染日志为 Vulkan。

本轮五 case pass、一 case 测试 harness FAIL；没有删改失败证据：

- 两项 complex DWG 选择响应测试 **PASS**，交互分别 **1.383612s / 1.397451s**（低于既有 2s 门限）。每次 723,587 pick items；展开 195.255 / 205.209ms，`pick_closest_borrowed` 1117.759 / 1124.500ms，drop 35.088 / 33.112ms，总 pick 1348.138 / 1362.850ms；命令后 `execute_push` 1.752 / 1.645ms。profiling family visits/culls/narrow 相同：line `218571/218571/0`，polyline `88200/68065/20135`，circle `18360/18360/0`，arc `210322/210322/0`，ellipse `1840/1840/0`，spline `133784/0/133784`，point `14673/14673/0`，mesh `13965/0/13965`，opaque `658/0/658`，compound `17008/12501/4507`，text `14073/0/14073`，mask `1305/1305/0`。这是 sample_every=64 的剖析计数，不把 culls 误称为 cache hit。两测试的 event loop heartbeat：均 30 ticks；最大事件间隔 1.447s / 1.461s，CAD frames 24 / 26，changed CAD pixels 均 42,884。故不仅报告延迟，也有后续 heartbeat 与最终 CAD 像素变化证据。
- CPU load **PASS** 19.97s：import 3.401s、fit 0.508s、scene 15.469s；44,591 entities、97,791 batches、50,883,794 vertices。
- grid/zoom/axes **PASS** 39.90s：1920×1013、CAD rect `[240,180,1680,713]`；content changes 36,459 pixels、grid 8,371；world/px `839.2856283978215 → 711.2590071167979`。
- selection/MOVE/undo/redo **PASS** 2.48s：EntityId(306)，history/DB 状态通过；flange 223 items，pick 总计约 0.072–0.084ms。
- maximized/grid/crosshair/navigation/restore/resize/re-maximize 全部实际推进并保存 7 张 PNG；Wayland configure 稳定等待后实际 restored size=`1088×680`、maximized size=`1920×1013`，top/bottom/left/right grid edge deltas=`707/2450/284/844`，crosshair off-center coverage 检查、zoom-out 和 pan 均通过。最后仍 **FAIL**：测试结束后外部 completion assert `apps/app-linux/src/host/validation.rs:1038` 报 `left: 0, right: 8`，尽管前一行已打印完整成功态诊断/全部 7 张产物，说明测试闭环计数未反映 closure 内完成状态；本轮不修测试行为，也不把 case 标成 PASS。

六 case 结果 **5 PASS / 1 FAIL**。PNG 全路径前缀 `/tmp/opencode/yacr-compound-localbounds-20261011T140511Z-4080521/evidence/20261011-140615/`：`desktop-event-loop/initial.png`、`desktop-open-after-start/initial.png`、`real-gpu-grid-zoom-axes/{01-axes-initial.png,02-grid-on.png,03-wheel-up.png}`、`real-gpu-select-move-history/{01-initial.png,02-selected.png,03-moved.png,04-undone.png,05-redone.png}`、`real-gpu-maximized-grid-crosshair/{01-maximized-initial.png,02-maximized-grid.png,03-maximized-crosshair-off-center.png,04-maximized-zoom-out.png,05-maximized-pan.png,06-restored-resized.png,07-maximized-after-resize.png}`。待父级视觉复核，无 golden/来源许可证据，不声称 vendor compatibility/fidelity。

本轮最终门禁：fmt、clippy、native check、全 workspace wasm lib check、i18n **PASS**；architecture/fixture manifest/workflows 沿用本轮前最近一次有效通过证据（本轮仅 spatial picker 与 validation 更新，按要求未重复这三项）。`cargo build -p app-linux --bin yacr-linux --locked` **PASS**。

最新应用保持打开：PID `4092870`，命令 `target/debug/yacr-linux --open /home/zenglanmu/sources/rust/yacr/fixtures/complex-test.dwg --gpu high`；`nvidia-smi` 映射到同一 Quadro P620 UUID、863 MiB。采样 `/tmp/opencode/yacr-compound-localbounds-20261011T140511Z-4080521/live-app-gpu.tsv`，日志 `…/live-app.log`。measure/layers/export/New 真实窗口操作仍未覆盖；无黄金图，图纸来源/许可未知。

## completion-phase 共享状态闭环修复后的定向验收

父级将 maximized 测试的完成阶段改为共享 `Rc<Cell<u8>>` 并仅在 phase 7 所有断言后写入 8；无生产逻辑或阈值修改。没有重跑此前有效的五项 pass。旧 live app PID `4092870` 以完整命令核验后发 SIGTERM 并退出。先前定向命令未设置测试要求的 `YACR_REAL_MAXIMIZED_OUTPUT`，因此仅报“runner must provide fresh evidence path”，没有创建截图/执行用例；已保留该日志 `targeted-test.log`。随后使用正确变量做有效的新鲜定向硬件测试：

```text
WGPU_ADAPTER_NAME=Quadro P620
YACR_REAL_MAXIMIZED_OUTPUT=/tmp/opencode/yacr-maximized-targeted-20261011T142110Z-4111611/evidence/real-gpu-maximized-grid-crosshair-rerun
VK_ICD_FILENAMES=<unset>
cargo test -p app-linux --lib real_gpu_maximized_grid_crosshair_navigation_acceptance --locked -- --ignored --test-threads=1 --nocapture
```

定向测试 **PASS**（1 passed，3.29s，backend Vulkan）。全部状态：startup maximize；四边 grid edge deltas `710/2383/851/849`；off-center crosshair 覆盖检查；zoom-out、pan；restore 尺寸请求/实测均 `1088×680`；re-maximize 实测 `1853×1013`；最终 phase 完成断言通过。截图全路径前缀 `/tmp/opencode/yacr-maximized-targeted-20261011T142110Z-4111611/evidence/real-gpu-maximized-grid-crosshair-rerun/`，文件 `01-maximized-initial.png` 至 `07-maximized-after-resize.png`（包含 `02-maximized-grid.png`、`03-maximized-crosshair-off-center.png`、`04-maximized-zoom-out.png`、`05-maximized-pan.png`、`06-restored-resized.png`）。此前同一实现轮五项通过 + 本次定向第六项通过，故六项 acceptance 证据均为 PASS；明确地，未再运行完整六 case runner。

实际 GPU 进程采样：定向测试 PID `4133213` 由 nvidia-smi 映射到 Quadro P620 UUID `GPU-40bcb3db-d898-76ff-f17e-ab0f735df81a`，峰值 91 MiB；采样 `/tmp/opencode/yacr-maximized-targeted-20261011T142110Z-4111611/nvidia-smi-targeted-samples.tsv`。格式门禁初次报告 `assert_eq!` 排版差异后运行 `cargo fmt --all`（仅机械格式化）并复查通过。定向代码检查 `cargo clippy -p app-linux --lib --all-targets --locked -- -D warnings`、`cargo check -p app-linux --lib --locked`、`python3 scripts/check-i18n.py` 均 PASS；此前完整 workspace gates 在 compound/cache 实现轮 fmt/clippy/native/wasm/i18n 已 PASS，本次源变更仅测试 closure bookkeeping。二次启动最终 app PID `4134293`，命令 `target/debug/yacr-linux --open /home/zenglanmu/sources/rust/yacr/fixtures/complex-test.dwg --gpu high`，nvidia-smi 映射至同一 P620 UUID、863 MiB；采样 `…/live-app-gpu.tsv`。所有 PNG 等待父级最终视觉核验，无 golden/vendor fidelity 声明。
