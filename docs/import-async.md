# 异步可取消导入（F01 / §4.10 / §8.3）

导入 DWG 是查看器最长的单次操作，**不得**在 UI 线程同步执行。本轮把导入拆成
「进度事件 + 后台 worker + 过期结果丢弃」三条不变式，并保持核心可无宿主测试。

## 契约

`cad-import-acadrust`：

- `ImportPhase`：`Reading` / `Parsing` / `Tables` / `Entities` / `Resolving` /
  `Finishing`，是同步导入器**真实到达**的边界，不是定时器阶段。
- `ImportProgress { phase, entities_done, entities_total: Option<usize>, bytes:
  Option<u64>, message: Option<String> }`。计数一律真实：源总数未知时
  `entities_total = None`，绝不用猜测填充；`bytes` 仅在确实可测时给出。
- `ImportProgressSink`（对闭包有 blanket impl，可直接传 `&|p| { .. }`）与
  `NoopProgress`（不分配、丢弃 tick）。
- `Importer::import_with_progress(request, cancelled, &sink)`；`import()` 是其
  `NoopProgress` 默认封装，旧调用点不受影响。`cancelled: &dyn Fn() -> bool`
  在阶段边界轮询，命中返回 `CadError::Cancelled`。

`cad-app`（`crates/cad-app/src/tasks.rs`）：

- `ImportManager`（每文档一个）保证**唯一当前任务**：`start()` 先取消旧任务并推进
  generation；`current_stamp()` 是当前任务结果必须携带的 `TaskStamp`。
- `ImportJob`：`drain_progress()` 取回自上次轮询以来的事件；`try_take(current)`
  仅在完成任务的 `TaskStamp` 与当前一致时返回 `Some(ImportedDrawing)`，否则
  `StaleResult`；`cancel()` 翻转 `CancellationToken`（异步、幂等）。
- `HostController::begin_async_open / cancel_async_open / poll_async_open`：
  `AsyncOpenPoll::{Idle, Running{progress,stamp}, Opened{opened,progress},
  Cancelled{progress}, Failed{error,progress}}`。
- worker 用 `std::thread` + `std::sync::mpsc` + `cad_platform::CancellationToken`，
  **无 Tokio、无平台 API**，可在裸 Linux 主机测试。

## UI 可轮询快照与 Slint 面板（本轮新增）

`cad-app` 新增纯投影，供 UI 读取，**不重实现** manager 逻辑：

- `ImportProgressSnapshot { running, phase: Option<ImportPhase>, entities_done,
  entities_total: Option<usize>, bytes: Option<u64>, cancellable, terminal:
  Option<ImportTerminal> }`，`phase_key()` 给出与目录键对应的稳定机器键
  （`reading`/`parsing`/…），使 `cad-ui-slint` 无需依赖 `cad-import-acadrust`。
- `ImportTerminal::{Opened{entities}, Cancelled, Failed{error}}`。
- `ImportProgressSnapshot::from_poll(&AsyncOpenPoll, previous)` 是 manager 轮询 →
  UI 快照的**唯一**映射；`Running` 无新 tick 时保留上一次真实阶段/计数，终态被保留
  以便失败/取消持续可见。
- `HostController::async_open_snapshot() -> Option<ImportProgressSnapshot>` 是**纯
  getter**：`begin_async_open` 置为“无 tick 的运行态”（`phase=None`，绝不猜
  `Reading`），`cancel_async_open` 置 `cancellable=false`，`poll_async_open` 只做投影。
- `CommandId::CancelLoading` 在 `HostController::execute` 被拦截并转调
  `cancel_async_open`，因此 UI 的取消按钮走普通命令路径，应用层契约不变。

`cad-ui-slint` 新增 `ImportProgressUiState::from_snapshot(snapshot, messages)`：
- `percent` **仅**在 `entities_total` 为 `Some(>0)` 时给出，否则为空 → 外壳
  `import-indeterminate` 渲染不确定进度条，绝不显示假百分比；
- `bytes` 仅在真实可测时拼入 `import-progress-text`，未知时**不**显示「0 字节」；
- `Opened` → 面板隐藏；`Cancelled`/`Failed` → 显式终态文案并保持可见；
- `cancellable` 驱动取消按钮启用；`UiHandle::set_import_state` 一次写入外壳，
  `set_locale` 用保留的原始快照重排文案。外壳回调 `cancel-open-requested` 派发
  `CancelLoading`。

## 不变式（均有测试）

1. **唯一当前任务**：启动新导入会取消并取代旧任务；旧任务即便越过取消竞争，其
   `TaskStamp` 也不再匹配，结果被丢弃。
2. **取消/过期结果绝不发布**：`try_take` 以完整 `TaskStamp`（文档 + generation）
   比较，别的文档的任务不可能被接受。取消报 `Cancelled`，竞争落败报 `StaleResult`，
   两者都丢弃数据库。
3. **发布是原子的**：`ImportedDrawing` 在被发布前已整体构建，取消或失败的导入
   永远到不了发布点，绝不会部分改写会话（`publish_imported` 只在结果就绪后调用）。
4. **取消是异步且幂等的**：token 在阶段边界被轮询，线程最终仍返回终态错误。

## 精确 vs 显式未实现

- **精确**：真实阶段事件与单调计数；取消 → `Cancelled`；取代 → `StaleResult`；
  过期/取消结果不发布；损坏 DWG 仍报真实错误而非 `Cancelled`。
- **显式未实现（不冒充）**：`Parsing` 阶段内 acadrust 读取是不可中断的粗窗口
  （只能在其前后取消）；`bytes` 仅在读取阶段已知；指纹（fingerprint）计算尚未
  分段/异步。
- **宿主接线状态**：`apps/app-android` 已接入（本 workstream）；`apps/app-web`
  **仍未**调用 `begin_async_open`/`poll_async_open`/`set_import_state`。Android 的
  接线见下节。

## Android 宿主接线（已实现，未设备复测）

`apps/app-android` 在 `OpenDrawing` / `open_drawing` 上走真实异步路径：

- `apps/app-android/src/host.rs`：先 `resolve_open_leave` 完成未保存决策与宿主写入，
  再 `HostController::begin_async_open(bytes, path)`；文档在 `poll_async_open` 发布
  前**不被触碰**。
- `apps/app-android/src/poll.rs`：`poll_import_once` 在每次轮询调用
  `HostController::poll_async_open`，把控制器快照经共享漏斗
  `state_push::push_import_state` → `UiHandle::set_import_state` 推入外壳；发布由核心的
  `TaskStamp` 守卫完成，宿主只镜像结果（`apply_opened`），**不重复导入**。第二次轮询
  为 `Idle`（已发布一次）。
- **轮询定时器**：`ensure_polling` 在 UI 线程用 `slint::Timer`（100 ms，`Repeated`）
  驱动 `poll_tick`；终态即 `stop_polling`，不空转。Android 目标有 `std::thread`，因此
  这是真实 worker；无 worker 的目标（`wasm32`）保留同步 `open_bytes` 回退
  （`worker_available()` 判断，回退不重复执行 leave 流程、不双重导入）。
- **取消**：外壳 `cancel-open-requested` → `CancelLoading` → 适配器命令路径 →
  `HostSink::send` → `HostController::execute` → `cancel_async_open`；只翻转
  `CancellationToken`，**从不**丢弃当前文档或未保存批注。面板的 `cancellable` 立即变为
  `false`，终态为 `Cancelled`。
- **同步回退**：`open_through_leave_flow` 保留，仅在无 worker 目标时使用；`begin_async_open`
  失败（当前核心在 spawn 失败时 panic，无 `Result`）不在可恢复范围内，故回退覆盖的是
  “无后台 worker”的编译目标，而不是伪造一个失败分支。

**诚实边界**：以上仅在宿主本机做 Android 目标类型检查（
`cargo check --target aarch64-linux-android -p app-android --tests`）与纯逻辑单测，
**未在模拟器/真机上打开 DWG 或观察进度面板**（审计 U07 的 surface 尺寸/旋转亦未设备
复测）。模拟器上的 F01 仍为 **NOT RUN**（`docs/validation-android.md` §6）。

## 复现

```bash
export CARGO_TARGET_DIR=/home/zenglanmu/.cache/yacr-async-target
cargo test -p cad-app --locked               # 后台 worker / 过期丢弃 / 快照投影
cargo check -p cad-ui-slint --lib --tests --target wasm32-unknown-unknown --locked
python3 scripts/check-i18n.py                 # 目录键/占位符/硬编码一致
# Android 宿主的异步接线（编译门；宿主测试在本机因缺 fontconfig NOT RUN）
cargo check --target aarch64-linux-android -p app-android --tests --locked
```

`cad-ui-slint` 的原生测试在本机无法构建（宿主缺 fontconfig），其进度映射逻辑保持
纯函数并由 wasm 目标类型检查覆盖；`cargo check ... --target wasm32-unknown-unknown`
是该 crate 的编译门。

合成夹具 `fixtures/dwg/synthetic-four-lines.dwg`（acadrust `DwgWriter` 生成的四条
LINE）用于端到端契约，**不代表真实兼容性**。
