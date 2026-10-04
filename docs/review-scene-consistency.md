# cad-scene 状态一致性审查（2026-10-04）

本轮只修改 `crates/cad-scene` 与本文档，依据规范 v2.0 §8.1 的有界预算、§8.4 的缓存预算以及仓库禁止失败变成空成功的约束。不修改权威数据库、其他 crate 或共享交接文档。

## 已修正缺陷

### 1. 线批次错误消耗三角形预算

`RenderBatch::triangle_count` 原先对所有无索引批次返回顶点数除以三；六个线顶点因此被计为两个三角形，可能让纯线图纸触发三角形预算拒绝。现在 `Lines` 与 `MeshEdges` 始终返回零，只有 `Mesh` 使用索引数量或无索引三角形列表回退。

新增契约 `line_topologies_do_not_consume_the_triangle_budget`：六个线顶点在零三角形预算下成功计费，覆盖两种线拓扑，并保留无索引 mesh 的两三角形计数。

### 2. 不可能的缓存预留返回成功并破坏已有状态

`SceneCache::evict(required_bytes)` 原先在预留量超过整个 CPU 预算时淘汰全部缓存后返回成功；实际仍无法满足请求。其 `used_bytes + required_bytes` 还可能发生整数溢出，导致调试构建 panic 或发布构建误判。

现在先拒绝超过总预算的请求，返回含预留量和预算的 `CadError::InvalidInput`，且不修改已有缓存。合法请求通过 `used_bytes > cpu_bytes - required_bytes` 判断，避免求和溢出；正常淘汰顺序不变。

新增契约：

- `impossible_cache_reservation_preserves_live_chunks_and_accounting`：普通超限与极大超限都返回错误，保留批次与字节计数；可满足的精确边界正常工作。
- `cache_reservation_near_usize_max_does_not_overflow`：极大合法预算下，预留精确剩余空间不淘汰，预留全部空间淘汰已有批次，避免加法溢出。

### 3. 预算计数饱和后仍接受新用量

`FrameBudget` 的顶点、三角形、字节计数以及 `TaskQueue::submit` 原先使用饱和加法。当计数和限制都是 `usize::MAX` 时，新增用量被接受但计数不增长，破坏计费及队列槽位一致性。

现在使用检查加法；不可表示的总量显式返回原有预算错误类型，拒绝后所有计数保持不变。错误的 `requested` 在溢出时仍报告 `usize::MAX`，已在字段文档明确其饱和值含义。`max_bytes = usize::MAX` 仍不设置更小的字节限制，但不再允许计数本身溢出。

新增契约：

- `frame_budget_rejects_counter_overflow_without_changing_usage`：分别覆盖三类计数溢出、错误类别、拒绝原子性及最大计数下零计费仍成功。
- `budget::tests::task_queue_rejects_counter_overflow_without_consuming_a_slot`：直接构造边界队列状态，避免实际提交海量任务；拒绝溢出后释放一个槽位再提交仍正常。

## 验证与限制

**NOT RUN（子代理）**：未执行编译、测试、检查器或格式化器，未提交或推送。
主控统一格式化、提交、合并后完成 debug 全目标编译、严格 clippy 与 wasm 检查；
Rust 合成契约仅编译，未执行。最终快速门禁与中止记录见 `docs/handoff.md`。
这些检查不涉及真实 DWG、GPU、窗口或平台兼容性结论。

不扩展数据库身份映射、ChangeSet 顺序跟踪、缓存 chunk 标识分配或 mesh 拓扑诊断；这些需要独立契约设计。本轮也不改变 `publish` 已有的超预算淘汰行为。缓存不可能预留的错误使用现有 `CadError::InvalidInput`，不引入跨 crate 的新错误变体。
