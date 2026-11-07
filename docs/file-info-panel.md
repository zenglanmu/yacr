# 文件信息面板与诊断（性能）面板

两个独立面板，入口都在导航栏、功能区 Utilities / Palettes 与状态栏：

| 面板 | 用户可见名称 | 组件 | 内容 |
| --- | --- | --- | --- |
| 文件信息 | `fileinfo.title`（文件信息 / File info） | `apps/desktop-ui/src/components/FileInfoPalette.tsx` | 打开文件后的**静态**诊断信息：渲染器、图纸、宿主状态 |
| 诊断 | `diagnostics.title`（诊断 / Diagnostics） | `apps/desktop-ui/src/components/DiagnosticsPalette.tsx` | **动态**性能读数，供后续优化定位方向 |

内部标识沿用历史命名：文件信息窗口的 `windowKey="fileinfo"`、诊断窗口的
`windowKey="diagnostics"`；`diagnosticsOpen` 对应“诊断（性能）”面板，`fileInfoOpen`
对应“文件信息”面板。旧的“诊断抽屉”内容（渲染器/图纸/状态）现在就是文件信息面板，
字段与语义保持不变；原面板的 FPS 行动态变化，已移到诊断面板。

性能读数不是成绩、不是兼容性声明，也不是设备验收。

## 1. 文件信息（静态）

`FileInfoPalette` 的内容来自 `cad_status` 的静态字段：

- **渲染器**：`backend`、`adapter`（宿主实际初始化的 wgpu 后端与 adapter/设备名）。
- **图纸**：`documentLabel`、`entityCount`、`importCount`、`camera`。
- **状态**：`loading`、`canOpen`、`canFit`、`accepted`。
- 面板为空时显示 `fileinfo.empty`；宿主错误仍以 `role="alert"` 显示。

这些字段不随每帧变化，也不包含 FPS/CPU/显存等动态采样。

## 2. 诊断（动态性能）

`DiagnosticsPalette` 从 `cad_status.performance`（Rust `CadPerformance`）和前端 FPS
采样器读取：

```json
{
  "performance": {
    "cpuPercent": 12.5,
    "processRssBytes": 524288000,
    "processVirtualBytes": 0,
    "systemTotalBytes": 16777216000,
    "systemAvailableBytes": 8388608000,
    "gpuMemoryUsedBytes": 2147483648,
    "gpuMemoryPeakBytes": 2147483648,
    "gpuMemoryReservedBytes": 3221225472,
    "vramTotalBytes": 4294967296
  }
}
```

- 整个块缺省或为 `null`：宿主没有采样器（旧宿主、未接线的载体），前端逐项显示 `—`，
  并显示 `diagnostics.perf.unavailable`。块内每个字段也可为 `null`：该指标在当前
  后端/平台不可测；**绝不以 `0` 代替**。
- 协议校验见 `apps/desktop-ui/src/cad/cad-status.ts` 的 `parsePerformance`；块存在时每个
  键都必须存在，值必须是有限非负数或 `null`。

| 字段 | 含义 | 不能当作 |
| --- | --- | --- |
| `status.backend` | wgpu 实际上报的后端字符串（`vulkan` / `dx12` / `metal` / `webgl2`） | 驱动版本或厂商能力表 |
| `status.adapter` | 实际使用的 adapter/显示设备名（例如 `Quadro P620`） | 已验收的 GPU 型号清单 |
| FPS（前端） | `frameCount` 的 rolling 2 s 采样（warmup 1 s），停止/错误时显式不可用 | 基准成绩、P95 帧耗时、真机帧率验收 |
| `cpuPercent` | **本进程**最近一个约 1 s 采样窗口的 CPU，按**单核**百分比计（多线程可超过 100%） | 整机 CPU 使用率 |
| `processRssBytes` | 本进程常驻内存（Linux/Android `/proc/self/status` 的 `VmRSS`） | 峰值/整机内存 |
| `processVirtualBytes` | 本进程虚拟地址空间（`VmSize`） | 物理内存 |
| `systemTotalBytes` / `systemAvailableBytes` | 整机物理内存总量 / 当前可用（`/proc/meminfo`） | 本进程占用 |
| `gpuMemoryUsedBytes` | **本渲染器** `GpuMemoryLedger` 记账（buffer + texture） | 整卡显存占用；共享 `Device` 的宿主资源不计入 |
| `gpuMemoryPeakBytes` | 同一记账自渲染器启动以来的峰值（包含重建上传的瞬时） | 整卡峰值 |
| `gpuMemoryReservedBytes` | wgpu allocator `total_reserved_bytes`（整个 `Device`，仅支持子分配的 backend） | 实际已分配字节 |
| `vramTotalBytes` | Vulkan 下最大的 device-local heap（**探测到的容量代理**） | 空闲显存、当前占用、整卡容量承诺；集显上它是共享系统内存 |

`vramTotalBytes` 与 `gpuMemoryUsedBytes` 的分母/分子**不是同一口径**：前者是驱动广告的
device-local heap，后者是本渲染器自己的记账。面板文案分别写成“设备本地显存总量（探测）”
与“GPU 显存占用（本渲染器）”，不得合成一个“显存使用率”。

## 3. 接线与节流

- 宿主 owner tick 最多每 1 s 采样一次：`PerformanceState::refresh_system()` 读
  Linux/Android `/proc`；GPU 报告来自 `RenderSessionHandle::gpu_memory_report()`
  （`≤1 Hz`，绝不逐帧查询 allocator）。
- 采样结果写进 `Shared.performance`，在构建 `cad_status` 快照时挂到
  `status.performance`；前端只读这一份投影，不自己算 CPU/显存。
- **当前只有 Linux `app-linux` 的 tick 接线**（系统指标 + GPU 记账）。
  Windows/macOS/Android 的 native tick 尚未调用 `refresh_system()`，其 `performance`
  为 `null`；Web / `dom-canvas` 没有 native 采样器，同样为 `null`。这些是显式的
  “未提供”，不是 0，也不是已验收。
- 非 Linux/Android 目标的 `PerformanceState` 只保留 `None`，不会猜测 CPU/内存数字。

## 4. 验证边界

两个面板都是**数据与布局**改动。没有真实 GPU / 真实窗口 / 像素或性能基准结论；
`cpuPercent`、RSS、GPU 记账都只在运行中的宿主里才是实测值。日志与报告中的数值仍以实际
运行输出为准，本文不给出任何性能成绩。
