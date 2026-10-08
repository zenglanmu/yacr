# 性能与资源预算（测量方法，非成绩）

本文只描述**如何测量**、**测量了什么**、**没有测量什么**。仓库当前没有任何
“手机预算”“FPS 成绩”或峰值内存数字，本文也不声称任何一项。

## 1. 结论边界（先读）

- **不是兼容性声明，也不是性能评级。** `benchmark` JSON 的 `claim` 字段固定为
  `"measurement, not compatibility"`；它报告一次运行的原始测量，不与其他设备比较。
- **没有通用手机预算。** `SceneBudget`/`MemoryBudget` 的默认值是工程默认上限，
  不是某台手机的实测预算，也不是 regress 门槛。
- **没有 FPS 数字。** 没有 P50/P95/P99 帧耗时，没有温升/降频数据。
- **软件 Vulkan 警告。** 无头 GPU 测试运行在 Mesa **lavapipe**（CPU 光栅化，
  `VK_ICD_FILENAMES=.../lvp_icd.json`）。它的上传/渲染耗时代表软件路径，**不代表**
  任何真实 GPU 或浏览器的性能。
- **未测即未测。** 某条路径没有测量的阶段在 JSON 中是 `null`，不是 `0`。数值字段
  要么是真实被测值，要么显式缺失。

## 2. 已接入的真实预算（含“不静默丢弃”保证）

`crates/cad-scene/src/budget.rs` 的 `SceneBudget` 各字段现在都有实际消费者；任一
类别越界都会返回结构化原因，而不是悄悄丢批次。

| 字段 | 消费者 | 越界结果 |
| --- | --- | --- |
| `cpu_bytes` | `SceneCache` 逐块字节记账 + `evict` | 逐出最旧块；`total_cpu_bytes()` 是真实账 |
| `queued_tasks` | `TaskQueue::submit` | `BudgetError{category:"queued_tasks"}` |
| `upload_bytes_per_frame` | `FrameBudget::charge_bytes`（CPU `plan_frame` 与 GPU `plan_from_gpu`） | `OverBudget{category:"bytes"}` |
| `max_vertices_per_frame` | `FrameBudget::charge` | `OverBudget{category:"vertices"}` |
| `max_triangles_per_frame` | `FrameBudget::charge` | `OverBudget{category:"triangles"}` |

字节数是**精确打包大小**（`RenderBatch::upload_size_bytes`＝位置/法线/边位置
12 B/顶点 + 三角索引 `[u32;3]` 12 B + 去重后的边索引 `u32` 4 B），不是估算；
网格法线即使缺失也会在上传时补齐，因此始终计入 `vertices.len()` 条法线。

GPU 侧另有 `Renderer::last_upload_ms()`：最近一次 `upload` 的真实墙钟毫秒，
`LoadTimings::upload_ms` 的来源。

### CLI 渲染内存保护（2026-10-08 新增）

- `cad-cli-tools render/plot` 的**累计场景**（`SceneDelta`）受 `--max-batches` /
  `--max-vertices` 硬上限保护（默认 `4_000_000` / `128_000_000`，`0` = 关闭），
  超限以 `invalid_input` **显式失败**，而不是耗尽内存被 OOM-kill。这是 CLI 层的
  防护，替代不了 `SceneBudget` 的逐帧预算。
- `render` 已切换到与桌面一致的**打包虚线 + `SceneCache::build_compact`**：块引用
  爆炸时把 20.8M 批次合并到 ~97k 批次。同一张 4.2 MB 样张实测峰值 RSS 从
  **~20 GB（OOM）降到 ~3.8 GB、24 s 出图**（软件 Vulkan/合成样张；不构成性能成绩）。

### 桌面空闲重绘（2026-10-08 修复）

Linux App 的 50 ms 定时器此前每 tick 无条件写 Slint 属性并 `request_redraw`，
空闲视图以 ~20fps 重绘、单核 CPU 占用 ~40%（与图纸大小无关）。现在
`CadView::apply_view_snapshot` 对相同快照跳过重绘，`Runtime::metrics` 只在
窗口尺寸/配置 revision 变化时刷新布局。空闲 CPU 应随之显著下降；本机前后对比
见 `docs/validation.md` 与 `docs/linux-app.md`。

## 3. CLI `benchmark`：可复现的测量 schema

```
cargo run -p cad-cli-tools -- benchmark <sample.dwg> --out bench.json
```

（也可通过 CLI 二进制调用 `benchmark` 操作。示例使用仓库约定的 sample 路径。）

输出（数值来自真实测量，未测为 `null`）：

```json
{
  "schema_version": 1,
  "operation": "benchmark",
  "claim": "measurement, not compatibility",
  "sample_hash": "<内容 SHA-256 十六进制，或 null>",
  "environment": { "release_build": true, "profile": "release" },
  "context": {
    "sample_hash": "<同 sample_hash，或 null>",
    "device": null,
    "browser": null,
    "release_build": true,
    "viewport": "1",
    "quality_configuration": "scene-budget-default"
  },
  "timings_ms": {
    "parse": 12.3,
    "build": 45.6,
    "upload": null,
    "first_usable": null,
    "complete": 45.6
  },
  "memory_bytes": {
    "file": 123456,
    "domain": null,
    "cpu_geometry": 7890,
    "gpu_estimated": 6789,
    "atlas": null,
    "attachment": null
  },
  "budgets": {
    "cpu_bytes": 134217728,
    "upload_bytes_per_frame": 4194304,
    "queued_tasks": 8,
    "max_vertices_per_frame": 8000000,
    "max_triangles_per_frame": 2000000
  },
  "over_budget": [],
  "scene": { "primitives": 42, "batches": 42, "vertices": 512, "triangles": 128 },
  "failures": []
}
```

- `parse` 来自导入器对该次打开的真实计时（`ImportReport::parse_ms`）；若该路径
  没有跑导入器则为 `null`。
- `build`、`complete` 在 benchmark 调用内真实计时（表示构建 + 场景分批）。
- `upload`、`first_usable` 在本 CLI 路径上**没有 GPU 设备**，因此为 `null`；
  它们由需要真实设备的 `render` 路径（`Renderer::last_upload_ms`）负责。
- `cpu_geometry` 是 `SceneCache::total_cpu_bytes()`（逐块精确记账）；
  `gpu_estimated` 是各批次 `upload_size_bytes()` 之和（精确打包大小）。
- `domain`/`atlas`/`attachment` 当前无字节精确来源，保持 `null`，不猜。
- 时间四舍五入到微秒（毫秒 3 位小数），同一输入在稳定机器上产出可比的文档。

## 4. 复现步骤（原生）

```bash
# 纯核心（宿主无 fontconfig 开发头，故排除 Slint/宿主 crate）
cargo test --workspace --exclude cad-ui-slint --exclude app-android \
  --exclude app-web --locked

# 真实预算/字节/队列测试
cargo test -p cad-scene --locked
cargo test -p cad-diagnostics --locked
cargo test -p cad-cli-tools --locked

# 真实软件 Vulkan 设备上的上传计时与字节预算
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json \
  cargo test -p cad-render-wgpu --locked
```

## 5. 测量了什么 / 没测量什么

**测量（真实值）**

- `parse_ms`（导入器）、benchmark 路径的 `build_ms`/`complete_ms`；
- `file_bytes`（磁盘）、`cpu_geometry_bytes`（缓存账）、`gpu_estimated_bytes`（精确打包）；
- GPU 路径的 `Renderer::last_upload_ms()`；
- 场景顶点/三角形计数；
- 每个预算类别的越界报告（`vertices`/`triangles`/`bytes`/`queued_tasks`/`cpu_bytes`）。

**明确未测量（保持 `null` 或缺失）**

- 手机设备预算、任何 FPS/P50/P95/P99、帧抖动、温升/降频；
- 浏览器 GPU 内存（只能标应用估算，未做）；
- `domain_bytes`/`atlas_bytes`/`attachment_bytes`；
- benchmark CLI 路径的 `upload_ms`/`first_usable_ms`（该路径无 GPU 设备）。

## 6. 规范关系

规范 §11.3 的 60/30 FPS 是**建议预算**，不是当前成绩。§11.2/§8.4 要求的
“有界且把限制写进诊断、不得静默截断后报告完整”由本文第 2 节的越界报告保证。
