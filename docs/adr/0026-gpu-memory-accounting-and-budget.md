# ADR 0026：原生 wgpu 的 GPU 显存记账与预算（与 wasm 堆预算对称）

- 状态：**Accepted（2026-10-24）**，已实现并在 Vulkan/Quadro P620 上实测。
- 相关 ADR：[ADR 0025](0025-wasm-memory-budget-and-worker-isolation.md)（wasm 堆预算/分块/worker）、
  [ADR 0024](0024-dom-canvas-wasm-cad-path.md)（DOM-canvas 载体）。
- 需求：`CAD_IMPLEMENTATION_SPEC.md` §21 / §"内存预算分开记录"。
- 影响：渲染器资源生命周期、上传准入、错误分类、CLI 契约与退出码；**不改**合成模型、唯一数据源、
  renderer-only wgpu、无整幅回读。

---

## 1. 问题

wasm 堆预算（ADR 0025）只管 CPU 侧线性内存。原生 backend 走**真 GPU**，显存是另一份预算：一张
4.2 MB 的授权样本实测在 Vulkan/P620 上常驻 **2.11 GB**。没有记账时，超限的失败形态是驱动 OOM、
device loss 或系统级内存压力——都不可诊断，且可能让**别的应用**被牵连。

wgpu 30.0.1 的能力边界（读源码确认，不是推测）：

| 能力 | 事实 | 用途 |
|---|---|---|
| `AdapterInfo` / `Instance::generate_report()` | **没有** VRAM 总量 | 应用上限只能是**估算**，必须显式标注 |
| `Device::generate_allocator_report()` | 有 `total_allocated_bytes`/`reserved_bytes`/逐项分配；GL 返回 `None` | **交叉校验**，不能作为唯一来源 |
| `InstanceDescriptor.memory_budget_thresholds` | 后端原生预算的**百分比**；仅 D3D12（及可选 Vulkan） | 后端强制；**实测在 Vulkan 上有害**，见 D5 |

## 2. 决策

### D1 渲染器自己记账（测量，不是估算）

`GpuMemoryLedger` 在**每次创建 buffer/texture 时**按类目计费，通过 drop guard 在拥有者释放时精确
退费；维护 current / **peak** / 10+ 类目（vertex、index、uniform、instance、stream、mask、
color/depth target、image/mask texture、other）。报告是若干原子读，**不做逐帧扫描**。
peak 记录真实的"旧常驻 + 新 staging 同时驻留"瞬态。

### D2 显式预算与显式拒绝

- `GpuBudget { cap_bytes, refuse_at }`；**默认 3 GiB**（`DEFAULT_GPU_BUDGET_BYTES`），文档写明这是
  **应用估算**（wgpu 拿不到 VRAM 总量），并附实测：授权样本 2,107,964,952 B。
- 覆盖顺序：`--gpu-budget-mb` > `YACR_GPU_BUDGET_MB` > 默认；`0` = 显式取消上限（并如实报告）。
- 准入点在上传 staging 之前（`prepare_upload_batches` / `upload_images` / `upload_masks` /
  `ensure_target` / `ensure_pick_target`）。拒绝时**常驻场景保持可用**，返回
  `RenderError::GpuMemory(GpuMemoryRefusal)`，带 `{origin, category, used, requested, cap}`。

### D3 驱动 OOM 必须被分类，不能混进通用帧错误

`create_texture`/buffer 创建包在 error scope 里，`wgpu::Error::OutOfMemory` →
同一个 `RenderError::GpuMemory`，`origin: DeviceOutOfMemory`。调用方据此区分"预算拒绝"与
"坏帧/device loss"。

### D4 报告必须说明来源

`GpuMemoryReport { buffer/texture/total/peak, reserved, allocator_total, budget, cap, source,
category_bytes }`，`source ∈ {Accounted, AllocatorReport, Estimate}`。宿主/CLI 不得把
`total_bytes` 说成"显存总量"。

### D5 后端阈值只在 D3D12 启用（实测否决了 Vulkan）

按 wgpu 文档设置 `for_resource_creation=80%` / `for_device_loss=95%`。**实测**：在 NVIDIA
580.178.04 / P620 的 Vulkan 上，驱动报告的 heap 是**进程级**的，第一个 device 已 reserve
≈192 MiB；同进程第二个 device（测试/探针 + CAD 并存的正常形态）在第一次 **72 字节**
`queue.write_buffer` 就 OOM，导致本 crate 自己的 3 个 headless 测试稳定失败。因此该阈值
**限定 D3D12**，便携防线是 D1+D2。此结论写进代码注释与本文，不允许"顺手也给 Vulkan 打开"。

## 3. 否决的备选

- **只依赖驱动 OOM**：失败形态不可诊断、可能牵连其它进程、且无预算可报——拒绝。
- **用 `Limits::max_buffer_size` 当总预算**：那是单资源上限，不是总量——拒绝。
- **伪造 VRAM 总量**：wgpu 不提供，任何"显卡 4 GB 所以预算 3.2 GB"的硬编码都是跨机器假数据——拒绝。

## 4. 后果

- 真 GPU 首次有了**可复现的显存数字**：授权样本常驻 **2,107,964,952 B**，驱动 allocator 交叉校验
  **2,109,386,656 B（差 0.07%）**，retained blocks 2,617,245,696 B。
- 最大头是 **`stream` 1,695,124,632 B（1.70 GB）**——instanced line / merged mesh 流；这比同一文件
  的 wasm 堆（1.43 GB）还大。**这是渲染器内存优化目标，不是预算能解决的**，另立项。
- CLI 契约：`gpu-memory:` 行（stderr）、`--gpu-budget-mb`、`--gpu-memory-detail`、退出码 **7**
  （预算/OOM 拒绝，**不写 PNG**）。

## 5. 非目标

- 不改 GPU 侧数据结构去降低 `stream` 占用（另立项）。
- 不做跨进程/跨应用显存协调；不读其它进程的显存。
- 不在 wasm/WebGL 路径启用阈值（无此 API；那边由 ADR 0025 的堆预算负责）。

## 6. 证据

见 `docs/validation-gpu-memory-2026-10-24.md`：默认预算成功渲染并写出 PNG；`--gpu-budget-mb 8`
→ 退出码 7 + `gpu-memory-refused:` 行 + **无 PNG**；flag > env 优先级；驱动 OOM 分类仅代码/单测
覆盖（**未**现场触发）。
