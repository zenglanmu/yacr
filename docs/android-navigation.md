# Android 导航推送的有界优化

## 适用前提

`HostCore::execute` 仍先运行 metrics，再走唯一的 `HostController::execute` 命令路径。只有命令成功、类型是 `Pan` 或 `Zoom`、命令 viewport 与当前 viewport 一致、drawing/handle/view 均存在且没有文档加载时，才以轻量推送替代全量 `push`。失败命令、其他命令及状态不完整/加载中的情形仍使用原全量推送；命令错误优先于推送错误的规则不变。

轻量推送调用 `sync_view`，同步完整的视口/会话快照（含相机、空间和投影状态），并重新计算 `snap_hints_near_cursor` 后交给 view。Zoom 改变 `world_per_px`，因此不能沿用旧捕捉结果。它不重发 drawing、layers、selection、preview 或其他面板，也不额外无条件调用尾部 `request_redraw`；视图快照和 snap setter 自己按状态变化请求绘制。Android 平台资源重复推送不在本次范围。

这只减少已确认导航不会修改的面板状态、drawing/overlay 数据推送，不减少 `metrics`、controller 命令执行、完整 `sync_view`、捕捉候选计算或状态栏更新。轻量资格不表示真机性能收益：Android 设备上的耗时/帧率尚未测量，需由设备验证 lane 后续核对。

剩余工作包括 `overlay bounds` 遍历与 progressive reset；本次未改变这些渲染路径，也未据此宣称导航已无全量成本或达到性能目标。

## 合成测试证据边界

已有 cad-host 合成测试继续覆盖资格矩阵、controller 状态不变量和无效命令错误；不再保留之前必然失败的 hardware stub。真实 HostCore→Slint bridge 验证放在 `apps/app-linux/src/host/validation.rs` 的 ignored 测试 `real_gpu_host_navigation_push_updates_bridge_and_preserves_full_push_semantics`，以复用 LinuxApp 的真实 handle/view 和 Slint timer/event-loop 生命周期。

该测试实际调用 HostCore Pan/Zoom，验证完整相机快照、selection/measurement/history/drawing 不变量、生产 snap getter 的 threshold 候选变化、非导航 Select 全量面板更新及错误/status 传播。随后以 50ms Slint timer 等待 Native Vulkan renderer live 且至少一帧；90 秒 deadline 内没有 ready/frame 会显式 panic，成功则自动 `quit_event_loop`，不需要人工关窗。最后验证 NVIDIA device fd 并写入 `renderer.txt`。没有强制 software ICD 或 adapter fallback。

Linux 真窗口入口（串行、每次必须使用全新证据目录）：

```bash
OUT="/tmp/opencode/yacr-navigation-$(date +%Y%m%d-%H%M%S)"
YACR_NAVIGATION_OUTPUT="$OUT" cargo test -p app-linux --lib real_gpu_host_navigation_push_updates_bridge_and_preserves_full_push_semantics --locked -- --ignored --exact host::validation::tests::real_gpu_host_navigation_push_updates_bridge_and_preserves_full_push_semantics --test-threads=1 --nocapture
```

测试记录 `host-navigation.txt`、`renderer.txt`；本机 Linux 硬件结果不等于 Android 真机性能收益。此前两次 cad-host 运行的历史证据分别保留在 `/tmp/opencode/yacr-navigation-20261012-020956/`（窗口被 SIGTERM、渲染尾断言未完成）与 `/tmp/opencode/yacr-navigation-20261012-023659/`（临时 fail-fast stub，非 renderer 验收）；二者均不冒充通过。

本次完整真实窗口运行（2026-10-12）通过，证据目录 `/tmp/opencode/yacr-navigation-20261012-024826/`：Wayland `wayland-0`、XWayland `:0`；没有设置 ICD/driver/adapter software override。bridge 记录 Pan 前/后 snap 候选数 `13/13`、Zoom 后 `1`，selection full push 归零；最终 renderer 为 `Native`、API `vulkan`、`frames_rendered=1`，并在 live renderer 进程中确认 `/dev/nvidia0` fd。系统 GPU 记录为 Quadro P620。该结果覆盖本机 HostCore→bridge→硬件 Vulkan 集成路径，不替代 Android 真机或性能验收。
