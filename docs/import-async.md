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
  分段/异步；宿主侧（Slint/Android/Web）的进度面板与「后台打开」按钮未接线，
  本轮只交付可测试的契约与默认 worker。

## 复现

```bash
export CARGO_TARGET_DIR=/home/zenglanmu/.cache/yacr-async-target
cargo test -p cad-import-acadrust --locked   # 进度/取消契约
cargo test -p cad-app --locked               # 后台 worker / 过期丢弃
```

合成夹具 `fixtures/dwg/synthetic-four-lines.dwg`（acadrust `DwgWriter` 生成的四条
LINE）用于端到端契约，**不代表真实兼容性**。
