# ADR 0013：设备能力协商（feature / limits）

- 状态：**已接受（Accepted，2026-10-17）**——**政策已固化，且同日已落地实现**。下文「决策」文本未因实现而改写；
  实现点、限值裁剪理由与栅栏见本文件「实现状态（2026-10-17）」。政策与实现仍分开陈述：本 ADR 只约束
  “如何协商、如何报告”，不承诺任何设备上更快。
- 取代：ADR-0007「P3b」中的「**从不向宿主设备请求 feature**」一句。ADR-0007 其余结论（能力门控的加速档、
  像素等价 fence、WebGL2 降级）**保持有效**。
- 范围：Linux 宿主与 renderer-only wgpu 的**设备创建**与**能力报告**。不覆盖浏览器/Android 运行验收，
  不给出 vendor / 精度 / 性能结论。
- 相关：`docs/adr/0007-entity-as-data-renderer.md`、`docs/adr/0012-scene-spatial-index-and-interaction.md`、
  `docs/render-optimization-backlog.md`、`docs/validation-realhost-frame-cost-2026-10-17.md`、
  `docs/compatibility.md`。

## 背景（实测事实）

两条需求与旧规则正面冲突：

1. **测量需求**：GPU 帧时间**从未被测量**（逐帧日志 `gpu_ms=None`），`wait_ms` 只是等待代理；
   在不请求 `TIMESTAMP_QUERY` 的前提下这条路**不可闭合**。
2. **运行时可选需求**：indirect multi-draw（backlog N5）与**更高的设备 limits**——当前 256 MiB 的
   `max_buffer_size` 使加速组织在大图纸上被容量门直接跳过；抬高 limits 是**独立的设备契约决策**，
   与 feature 请求同轴。

支撑上述判断的**实测**（单机、单适配器）：

- **适配器支持 ≠ 设备启用**（`crates/cad-render-wgpu/tests/gpu_capabilities.rs` 探针）：app-choice 适配器
  （Vulkan / Quadro P620 / 驱动 580.178.04）**支持** `MULTI_DRAW_INDIRECT_COUNT`、`INDIRECT_FIRST_INSTANCE`、
  `DownlevelFlags::INDIRECT_EXECUTION`、`TIMESTAMP_QUERY`（含 inside encoders/passes）；而**已启用**的
  设备 feature 集为**空**，因为两处设备创建点都没有提出任何请求。
- **limits 是请求值，不是硬件上限**（实测 + 算术）：**adapter** `max_buffer_size` = **4,292,870,144 B**，
  **device** 得到 `Limits::default()` = **268,435,456 B（256 MiB）**。`complex-test.dwg` 的一次 pulled
  重建**实测需要 2,281,858,648 B**，因此被容量门跳过；在容量门存在之前，同一分配**实测崩溃**为
  `wgpu error: Out of Memory`（panic，exit 101）。这是**请求限制**造成的容量缺口，不是 4 GB 卡的硬件极限。
- **必须存在真实回退面**（实测）：同一张卡的 **GL 后端**适配器报告 `MULTI_DRAW_INDIRECT_COUNT = false`；
  WebGL2/GLES **没有** `INDIRECT_EXECUTION`。任何能力都必须有可用的通用路径。
- **能力 ≠ 性能**（实测）：llvmpipe（纯 CPU）也报告 `MULTI_DRAW_INDIRECT_COUNT = true`；同一套顶点/tri
  预算在两个真实图纸上产出 **6.7×** 不同的 `wait_ms`（19.382 ms vs 2.895 ms）。能力位不蕴含任何速度。

探针读数（**实测**，app-choice 适配器 = Vulkan / P620；"启用"一列为设备实际拿到的集合）：

| 能力 | 适配器支持 | 设备启用 |
|---|---|---|
| `MULTI_DRAW_INDIRECT_COUNT` | true | false（未请求） |
| `INDIRECT_FIRST_INSTANCE` | true | false（未请求） |
| `DownlevelFlags::INDIRECT_EXECUTION` | true | —（downlevel flag，不随请求变化） |
| `TIMESTAMP_QUERY` / inside encoders / inside passes | true | false（未请求） |
| `Features` 全集 | 见探针 | **空** |

## 决策

能力分**三**类，每类带强制义务。

### 1. measurement-only（例如 `TIMESTAMP_QUERY`）

- **允许**：仅在**显式、有文档的 opt-in** 之后请求（测量构建/测量运行），请求点必须可见。
- **必须**：输出标注为**测量**（真时间戳 vs 代理不得混口径）；未请求时 `gpu_ms` 保持 `None`，不得填估算值。
- **禁止**：成为运行时依赖、改变渲染或像素、进入产品默认配置。测量设备**不是**产品配置。

### 2. runtime-optional（例如 `MULTI_DRAW_INDIRECT_COUNT`）

- **请求集**：`requested = adapter.features() ∩ desirable`。`desirable` 必须是**显式白名单**，
  不得用 `Features::all()` 或"能用就要"。
- **基础路径**：**必须存在并保持 fenced**。能力缺失、请求失败或回退时走通用 per-run，
  像素与 native 路径**一致**（ADR-0007 P3b 的等价 fence 延伸到新路径）。
- **三态上报**：`Native` / `Emulated` / `Unsupported`，在 `docs/compatibility.md` **与**逐帧/probe
  诊断中同时出现。**`Emulated` 不得报告为 `Native`**。`Emulated` 的准确含义是"API 可用但被模拟"
  （例如 `MULTI_DRAW_INDIRECT_COUNT` 缺失时 wgpu 把 `multi_draw_*` 展开为一串 `draw_indirect`），
  此时**不得**声称省下 CPU 提交。

三态的判据（**设备探测**，不是 backend 类别）：

- **`Native`**：本设备已启用该 feature / 该路径由驱动原生执行。
- **`Emulated`**：API 存在但由 wgpu 或本仓上层模拟；语义可用，性能收益**不成立**。
- **`Unsupported`**：本设备不提供该能力（例如 WebGL2/GLES 无 `INDIRECT_EXECUTION`）。
- 三态描述的是**能力**，不是**实现进度**。路径尚未实现时，表中如实写"能力 = Native，路径未实现"，
  不得用 `Native` 冒充已完成。

### 3. limits（`max_buffer_size`、`max_storage_buffer_binding_size`、…）

- **请求策略**：取 adapter 能力，由**显式 policy 上限**裁剪（逐字段，不整体 `Limits::all()`）；
  policy 值与其理由必须写进实现与诊断。
- **降级**：`request_device` 失败则回退 `Limits::default()` 重试，并**显式报告**协商结果
  （requested / negotiated / 降级原因）。不得把降级吞成"请求成功"。
- **容量缺口**：一律**显式 skip / error 状态**（现有 `PulledState` 三态），**不得 panic**，
  **不得空成功**。

## 不变量（本次不改变）

- **pick / snap / selection 仍以 CPU 为主**（ADR-0012 §8、§9）；能力不改变判定与排序。
- **不新增第二套失效真相**（ADR-0012 §5）：capability 快照只描述设备，不参与场景失效。
- **不静默回退**（铁律 2）：每条降级路径都必须有可机读状态。
- **能力位不是性能结论**（铁律 6）：沿用的唯一合法用法是"是否可走某条路径"。
- **像素一致**：能力缺失或发生 skip 时，加速组织必须与通用路径**逐像素一致**（ADR-0007 P3b fence）。

## 落点（ADR 记录时的点名；实际实现见下节）

- 设备创建：`crates/cad-render-wgpu/src/headless.rs`（`create_headless_gpu*`，记录时为
  `DeviceDescriptor { ..Default::default() }`；实现后经 `request_device_with_negotiated_limits`
  走 `negotiated_device_limits`）与 `crates/cad-render-wgpu/src/native_presenter.rs`
  （宿主 CAD 设备，同形）。两者是**唯一**的请求点。
- 容量门：`crates/cad-render-wgpu/src/renderer_impl.rs`（`PulledState` 判定与 `pulled_capacity_fits` /
  `pulled_budget_for`）；状态机定义在 `crates/cad-render-wgpu/src/lib.rs`。
- 逐帧诊断：`apps/app-linux/src/host.rs` 的 `[probe] actual-render … pulled={}` 行——三态与协商后的
  limits/降级原因在此可见。
- 探针：`crates/cad-render-wgpu/tests/gpu_capabilities.rs`（只读能力查询，不请求非默认 feature）。

## 实现状态（2026-10-17）

**政策与实现分开记录**：下面只写**已落地**的东西与它的**实测**依据；「决策」文本未改。

- **limits 协商已落地**：`crates/cad-render-wgpu/src/headless.rs:232` 新增纯策略函数
  `negotiated_device_limits`——以 `Limits::default().or_worse_values_from(adapter)` 为底，**只**把
  `max_buffer_size` 抬到 adapter 值；两处设备创建点（`headless.rs` 的 `create_headless_gpu*` 与
  `native_presenter.rs` 的宿主 CAD 设备）都经同一 `request_device_with_negotiated_limits`
  （`headless.rs:273`）请求设备并拿到 `DeviceLimitsReport`。本机（P620/Vulkan）实测：
  `[probe] device-limits adapter_max_buffer_size=4292870144 requested_max_buffer_size=4292870144
  granted_max_buffer_size=4292870144 requested_max_storage_buffer_binding_size=134217728
  granted_max_storage_buffer_binding_size=134217728 degraded=false reason=-`。
- **`max_storage_buffer_binding_size` 刻意封顶在默认 134,217,728 B**（adapter 报 2,147,483,644 B）：
  **实测**抬高后 pulled 分段器产出约 2 GiB 的单次分配，一次 pulled 重建的 `create_buffer` 失败，
  wgpu 30.0.1 返回**无效 handle**，其下一次使用表现为 **validation** 错误
  （`Buffer with 'cad-pulled-line-storage' label is invalid`），而 `ErrorFilter::OutOfMemory` 作用域
  **捕获不到**它 ⇒ 默认处理器 abort（exit 101）。锁回默认值后同一重建成功（分段更多、draw 更多），
  **像素不变**。这是「policy 上限」决策的实测理由，不是临时规避。
- **降级路径已落地**：`request_device` 以协商值失败 ⇒ 用 `Limits::default()` 重试，报告 `degraded=true
  reason=…`；两次都失败 ⇒ 显式错误（不 panic、不空成功）。
- **OOM → 显式判定已落地（同轮，被本次放宽 unmask 的预存缺陷）**：路径是「驱动拒绝 `create_buffer`
  → 无效 handle → 后续 `write_buffer` 报 validation → 超出 OOM-only 作用域 → abort」。
  修复在 `crates/cad-render-wgpu/src/renderer_impl.rs`：pulled 构建置于嵌套 `Validation` +
  `OutOfMemory` 作用域（所有路径 LIFO push/pop，`ErrorScopeGuard::drop` 兜底），每次分配单独守护、
  **先验证创建**再做上传/绑定，首次失败即中止并丢弃半成品，本帧回退通用 per-run 并给出显式状态：
  设备报 OOM ⇒ `pulled=skipped-out-of-memory(required_bytes=…,budget_bytes=…,message=…)`；其它错误 ⇒
  新增 **`pulled=skipped-allocation-failed(required_bytes=…,budget_bytes=…,message=…)`**——**不**冒充
  显存耗尽证据。容量门、通用路径与像素不变。
- **栅栏落地（对应上文「合入时必须新增的栅栏」）**：`headless.rs` 增策略/降级单测（含确定性
  `refused_allocation_is_reported_without_panicking`，请求 `max_buffer_size + 1` 触发真实的
  invalid-handle/validation 机制、不耗显存）；`crates/cad-render-wgpu/src/lib.rs` 的 `PulledState`
  新增 `skipped-allocation-failed` 状态与文本。**未做**：runtime-optional feature（`MULTI_DRAW_*`）
  与 `TIMESTAMP_QUERY` 的请求——feature 集合仍为空，三态中「未启用」一列不变。
- **测试（实测）**：`cargo test -p cad-render-wgpu --lib -- --test-threads=1` → 155 passed / 0 failed /
  3 ignored；`--test accelerated_tier` 23、`--test pulled_prefix_equivalence` 9、`--test headless_render` 19。
- **真实宿主（实测）**：`fixtures/complex-test.dwg` 由
  `pulled=skipped-insufficient-vram(required_bytes=2281858648,budget_bytes=268435456)` 变为
  `pulled=active`，`entityCount=44591`、settled 帧 `complete=true`、自然 exit 0；
  `fixtures/debug-default-fonts.dwg` `pulled=active`、`entityCount=10656`；无 panic、无 `is invalid`。
- **未测**：真实驱动/显存 OOM 在本机**不可模拟**（新测试触发的是无效 handle/validation，不是 OOM）；
  其它 GPU/vendor/平台未验证；`gpu_ms` 仍 `None`。

## 合入时必须新增的栅栏

- **协商可复现**：给定 adapter 的 `features()` / `limits()`，请求集是确定性函数（纯函数 + 单测）。
- **降级可机读**：`request_device` 失败后的 `Limits::default()` 重试路径有测试，协商结果与降级原因
  出现在诊断里。
- **三态不撒谎**：模拟路径断言报 `Emulated`，原生路径断言报 `Native`（互不串档）。
- **像素不变**：能力缺失 / 跳过 / 模拟三条路径与通用路径逐像素一致（沿用 ADR-0007 P3b 的 fence 形式）。
- **容量不 panic**：超限仍走显式 skip 状态，OOM 捕获范围保留。

## 否决方案

- **硬请求 feature，缺失即失败**：WebGL2/GL 上直接不可用，违反"能力门控 + 显式回退"。
- **依赖 `caps_for` 做能力判定**：它按 **backend 类别**（WebGl2 vs 其他）报告，不是**设备探测**；
  会把"平台理论上支持"当成"本设备支持"。
- **按 vendor 特判**：per-vendor 分支无法用本仓证据支撑，且把未验证行为固化。
- **把测量设备当产品配置**：会让 `TIMESTAMP_QUERY` 变成运行时依赖，违反第 1 类义务。

## 后果与风险

- 抬高 limits 会改变 pulled 的**分段结果**（更大的 `max_storage_buffer_binding_size` ⇒ 更少的
  `pulled.segments`，单段更大），内存压力从"段数"转向"单次分配"，须重新测量。
- 更高的 limits 会让**此前被跳过**的 pulled 重建真正去分配：`ErrorFilter::OutOfMemory` 的捕获范围与
  显式 skip 状态必须保留，作为最后一道安全网（不是性能优化）。
- 逐 vendor 行为仍**未验证**；本 ADR 只约束"如何协商、如何报告"，不承诺任何设备上更快。

## 证据边界

- 单机、单适配器（Quadro P620 / Vulkan / 驱动 580.178.04），Linux 宿主。**未**做 vendor 交叉验证，
  **未**跑浏览器/WebGL2，**未**做任何 GPU 时间测量（`gpu_ms` 至今仍为 `None`）。
- 文中 gpu_capabilities 的原始输出为本轮记录，非长期基线；换机后必须重跑探针再引用。
