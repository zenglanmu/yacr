# 测量专用 GPU 帧时间（P0，measurement-only）验证（2026-10-17）

**目的**：记录 **backlog §P0 —— 测量前置（GPU 帧时间）** 落地后的**本机真实宿主实测**与**测试矩阵**。
政策依据是 **ADR 0013 第 1 类（measurement-only）**：仅在**显式、有文档的 opt-in** 之后请求
`TIMESTAMP_QUERY`，输出标注为**测量**，未请求时 `gpu_ms` 保持 `None`，且测量设备**不是**产品配置。

代码面：新纯模块 `crates/cad-render-wgpu/src/gpu_time.rs`（`GpuTimeConfig` / `GpuTimeState` /
`GpuTimeReport` / `negotiate_gpu_time` / `ticks_to_ms` / `GPU_TIME_FEATURES`，纯策略与纯换算）+
`Renderer::set_gpu_time_measurement` 的**一次性**资源创建 + 帧内**两查询 bracket** + 帧尾读回 +
`FrameStats::gpu_ms` / `gpu_time` 与 `[probe] actual-render … gpu_time={}` 的**追加式**上报。

**这不是**性能验收、**不是** vendor/发布版结论、**不是**视觉判定。**测量设备不是产品配置**（见下
「扰动」）。单机、单适配器、单会话、同 UID。口径与 `docs/validation-frame-pacing-2026-10-17.md`、
`docs/validation-chunk-culling-2026-10-17.md`、`docs/validation-realhost-frame-cost-2026-10-17.md`
**不同**，各组数字**不得混算**。

## 环境与授权边界

| 项 | 值（**实测**） |
|---|---|
| 机器/会话 | 本机桌面会话，**原生 Wayland**，同一 UID；**无** root/`sudo`、**无**屏录、**无**虚拟/无头显示 |
| 适配器 / 后端 | **Vulkan**，`Quadro P620`，驱动 **580.178.04** |
| 缩放 | `scale=1` |
| 宿主进程 | `target/release/yacr`（本轮**未记录**二进制哈希，故不声称具体 commit 对应关系） |
| 退出/卫生 | 四次运行（每 fixture 各 on/off）均自然 **exit 0**、**无** panic、**无**残留进程 |
| 计时口径 | 宿主逐帧 `[probe] actual-render …`；`gpu_ms` **仅在 opt-in 打开时**为真时间戳（见下） |
| 证据落盘 | 仅本地（`${YACR_EVIDENCE_ROOT:-/tmp/opencode}` 下本轮输出目录），**未上传** |

## 政策依据与分层规则（ADR 0013 class 1）

- **唯一读取点**：环境变量 **`YACR_PROBE_GPU_TIME`** 只在 `apps/app-linux/src/host.rs:1327` 读取
  （`std::env::var_os("YACR_PROBE_GPU_TIME").is_some()`）。**渲染器/引擎从不读环境**——宿主把结果装进
  显式 `cad_render_wgpu::gpu_time::GpuTimeConfig`，经 `gtk_bridge::install(&window, …, gpu_time)`
  → 呈现器设备创建（`native_presenter.rs` 的 `build_with_surface(…, gpu_time_config)`）
  → `session.attach_device_with_gpu_time(device, queue, self.gpu_time)`
  （`crates/cad-engine/src/render/mod.rs:1105`；调用点 `gtk_bridge.rs:497`、`:894`）
  → `Renderer::set_gpu_time_measurement`（`crates/cad-engine/src/render/runtime.rs:427`）逐层显式传参。
- **关闭时（产品默认）**：设备**不请求任何 feature**，`gpu_ms` 保持 `None`，并打印一行
  `[probe] gpu-time …` 报告 `state=disabled`。**不得**用估算值填充。
- **打开时**：请求 `TIMESTAMP_QUERY | TIMESTAMP_QUERY_INSIDE_ENCODERS`，与 adapter 支持集**取交**；
  本机两者**均被授予**（`state=enabled`）。`TIMESTAMP_QUERY_INSIDE_PASSES` **刻意不请求**——两次写入
  都在**编码器**上（不在 render pass 内）。
- **缺 feature**：显式报告 `gpu_time=unsupported(<FEATURE>)`（命名缺失的第一个 feature），
  **绝不**静默回退。

`gpu_time.rs` 的纯策略函数即 `negotiate_gpu_time(adapter_features, config) -> GpuTimeReport`；
`GpuTimeState::as_token()` 产出稳定无空白 token：`enabled` / `disabled` / `unsupported(<FEATURE>)`。

## 请求/授予的 feature（**实测**）

| 项 | 值 |
|---|---|
| 请求集（opt-in 打开） | `TIMESTAMP_QUERY \| TIMESTAMP_QUERY_INSIDE_ENCODERS`（`GPU_TIME_FEATURES`，`gpu_time.rs:33`） |
| 授予（本机 P620 / Vulkan） | 两者**均授予** ⇒ `state=enabled` |
| **刻意未请求** | `TIMESTAMP_QUERY_INSIDE_PASSES`（写入在编码器上，不在 pass 内） |
| 请求集（opt-in 关闭） | `Features(0x0)`（空）⇒ `state=disabled` |

> **字段说明（不伪造 verbatim 行）**：`[probe] gpu-time` 行的格式串为
> `requested={:?} granted={:?} state={}`（`native_presenter.rs:530`），逐帧行尾 token 为
> `gpu_time={}`（`apps/app-linux/src/host.rs:1993` 的 `actual-render` 格式串末尾）。本轮**未保留**
> 可逐字引用的捕获行，故只描述字段布局与取值，不编造某次运行的原文。

## 读回策略（帧内 bracket，无额外提交）

- 在帧**既有**的单个 `CommandEncoder` 上，帧首 `write_timestamp(query_set, 0)`、帧尾
  `write_timestamp(query_set, 1)`；随后 `resolve_query_set(query_set, 0..2, &resolve, 0)` 与
  `copy_buffer_to_buffer(&resolve, 0, &readback, 0, 16)`——**全部在同一编码器/同一次提交上**
  （`renderer_impl.rs:5538-5543`），**不新增** submission。
- **复用资源**（`set_gpu_time_measurement` 一次性创建，`renderer_impl.rs:693-717`）：
  2 个 timestamp 查询的 `QuerySet`；`resolve` 为 `QUERY_RESOLVE_BUFFER_ALIGNMENT`（**256 B**，
  `QUERY_RESOLVE | COPY_SRC`）；`readback` 为 **16 B** 的 `MAP_READ | COPY_DST` 缓冲。
- **读回经由帧已执行的 poll**：`map_async` 在 `submit` 之后、阻塞 poll **之前**排队
  （`renderer_impl.rs:5547-5551`），poll 完成该映射后 `read_gpu_time_ms` 读 ticks，
  **不做第二次阻塞等待**；ticks 用 `queue.get_timestamp_period()` 经 `ticks_to_ms` 换算。
- 资源创建计数器 `gpu_time_resource_creations` 必须恒为 **1**（逐帧复用，不逐帧重分配）。

## 扰动（诚实边界）

读回给帧**增加**了一次 GPU `resolve_query_set` + 缓冲拷贝与一次 CPU map。on/off 两次运行的帧总量
只差**几个百分点**（见下表），但**被测量的设备不是产品配置**（多了测量开销），故本轮数字**不得**
当作产品配置的性能结论。

## 逐帧测量（**实测**；on/off 各一次，本机原生 Wayland / Vulkan / P620）

| fixture | opt-in | frames | `gpu_ms` 中位 / p90 / max | `wait_ms` 中位 | `cpu_ms` 中位 | gpu/frame | gpu/wait |
|---|---|---:|---|---:|---:|---:|---:|
| `complex-test.dwg` | off | 697 | —（`None`） | 4.203 | 4.463 | — | — |
| `complex-test.dwg` | on | 698 | **3.948 / 7.479 / 24.416** | 4.242 | 4.111 | **43.8%** | **93.0%** |
| `debug-default-fonts.dwg` | off | 452 | —（`None`） | 2.234 | 2.055 | — | — |
| `debug-default-fonts.dwg` | on | 462 | **2.643 / 6.205 / 6.600** | 2.909 | 1.439 | **55.0%** | **93.9%** |

- **关闭腿的 `gpu_ms` 为 `None` 且**本机确认：**0/697** 与 **0/452** 帧带值（**实测**）——`None`
  不是缺测，而是**未请求** feature 的诚实结果。
- `gpu/wait ≈ 93%` ⇒ **`wait_ms` 基本就是真实 GPU 执行**，呈现/队列只占约 **6–7%**（**实测**）。

### 稳定下来的 `complete=true` 页（paced，**实测**）

| fixture | frames | `gpu_ms` 中位 | `cpu_ms` 中位 | `wait_ms` 中位 | frame 中位 | gpu/frame | cpu/frame |
|---|---:|---:|---:|---:|---:|---:|---:|
| `complex-test.dwg` | 50 | 0.025 | 22.158 | 0.683 | 22.930 | **0.1%** | **97.0%** |
| `debug-default-fonts.dwg` | 376 | 2.551 | 1.294 | 2.806 | 6.726 | **50.3%** | **44.3%** |

⇒ complex 的稳定页**几乎全是 CPU**（一张 1-draw 的 settled 页仍耗 ~22 ms CPU）；debug 的稳定页
GPU/CPU 大致对半。

### 每帧 CPU 相位（中位，占 `cpu_ms` 的份额，**实测**）

| fixture / 腿 | `plan` | `uniform` | `encode` | `submit` |
|---|---:|---:|---:|---:|
| `complex-test.dwg`（all-paced） | **53.2%** | 22.8% | 5.9% | 17.2% |
| `complex-test.dwg`（complete 页） | **48.0%** | 26.9% | 23.9% | 0.9% |
| `debug-default-fonts.dwg`（all） | **55.2%** | 7.1% | 20.4% | 18.1% |
| `debug-default-fonts.dwg`（complete 页） | **56.3%** | 7.7% | 19.9% | 17.5% |

⇒ **`plan_ms` 主导 CPU（48–56%）且大体与页大小无关**；`encode`（6–24%）与 `submit`（1–18%）都不主导。

## 证据驱动的重排优先级（本轮主结论）

N2 重开规则的**三个条件**据此判定（**实测**）：

- **条件 (a) 为假**：`exec ≈ wait_ms`（比值 **0.93**）⇒ `wait_ms` 就是真实 GPU 执行，present/queue 只占
  其约 **6–7%**。⇒ **「延后阻塞 poll 近乎免费」不成立**。
- **条件 (b) 为真**：paced 帧**超目标**，且 `cpu_ms` ≈ 帧时间的 **41–52%**。
- **条件 (c) 成立**：120 Hz（**8.33 ms**）下，complex 的 `cpu_ms` p90（≈**8.5 ms**）**单独**即超预算。

⇒ 工作顺序变为：**(1) 每帧 CPU plan 成本**（`plan_ms` 占 CPU 48–56%，且**大体与页大小无关**——一张
1-draw 的 settled 页仍耗 ~22 ms CPU）；**(2) N2 流水**（因 `cpu ≈ gpu ≈ 4 ms`，重叠大致可把帧时间减半，
但它**不减少** CPU 工作量本身）；**(3) N5 indirect multi-draw 最后**——它只打击 `encode_ms`（6–24%）与
`submit_ms`（1–18%），两者都不主导，且**碰不到** `plan_ms` 与 GPU 执行。

**N5 前提已被测量且不是瓶颈**（CPU 提交成本非主导）⇒ **N5 降级而非取消**；重促条件见
`docs/render-optimization-backlog.md` §N5。

## 测试矩阵（**实测**）

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-render-wgpu --lib` | **185 passed / 3 ignored** |
| `cargo test -p cad-render-wgpu --test headless_render` | **35**（含 **3** 个新 `gpu_time_contracts` fence） |
| `cargo test -p cad-render-wgpu --test accelerated_tier` | **23** |
| `cargo test -p cad-render-wgpu --test pulled_prefix_equivalence` | **9** |
| `cargo test -p cad-engine` | **100 passed / 8 ignored** |

门禁（**实测**）：`fmt` / `check --all-targets` / `clippy -D warnings` /
`check --lib --target wasm32-unknown-unknown` 全 clean；release 构建
（`--features tauri/custom-protocol`）OK。

### 栅栏（名称即契约，`crates/cad-render-wgpu/tests/headless_render.rs` 的 `gpu_time_contracts`）

| fence | 断言 |
|---|---|
| `disabled_requests_no_feature_and_reports_none` | 关闭 ⇒ 设备**无**该 feature、`gpu_ms=None`、**0** 个测量资源 |
| `unsupported_state_names_the_missing_feature` | opt-in 开但缺 feature ⇒ 状态**命名**缺失 feature（`unsupported(TIMESTAMP_QUERY)` / `…INSIDE_ENCODERS`），**0** 资源 |
| `enabled_measures_without_changing_pixels_and_does_not_grow` | 打开 ⇒ `gpu_ms` 为真、输出与关闭腿**逐字节一致**、资源创建计数器恒为 **1** |

- **tick 换算**为纯测试（`gpu_time.rs` 的 `ticks_to_ms` 三例：period 换算、零 period ⇒ 0、ticks 单调）。
- **关闭腿**的断言已由本机宿主**确认**：0/697 与 0/452 帧带值（**实测**）。
- **wasm**：库编译通过，`set_gpu_time_measurement` 在 `wasm32` 恒报 `Disabled`（源码分支
  `renderer_impl.rs:675-680`；**未**跑真实浏览器运行时）。

## 未测 / 边界（不得省略）

- **GPU 内部归因**：只测了**每帧总量**，**未**区分顶点/三角形吞吐 vs draw 数 vs 带宽。
- **N1d / N2 / N5 是否真能降低 `gpu_ms`**：**未测**（本轮只给出测量与结构推理）。
- **真实 adapter 的 `unsupported(<feature>)`**：本机 P620 两个 feature 均授予，`Unsupported` 分支只在
  `unsupported_state_names_the_missing_feature` 里经**能力注入钩子**
  （`set_gpu_time_feature_override`）**模拟**断言——**没有**真正缺该 feature 的 adapter 可用，**如实说明**。
- **wasm 运行时**：只有库编译；真实浏览器/浏览器 GPU **未测**。
- **真实宿主像素捕获**：字节一致是在 **headless harness** 里证明的；本机宿主两次运行只产出**日志证据**
  （**无**像素读回）⇒ 不得据此宣称宿主像素不变。
- **vendor / golden / 性能结论**：单机、单适配器，**不得**外推。
- **扰动**：测量设备**不是**产品配置（多了 resolve/copy + map 开销）。

## 交叉引用

- 政策：`docs/adr/0013-device-capability-negotiation.md`（第 1 类 measurement-only）。
- 决策与优先级：`docs/render-optimization-backlog.md` §P0 / §N1d / §N2 / §N5。
- 3D 预算/分页口径：`docs/render-3d.md` 的「分页预算与每帧活动预算」节。
- 能力表：`docs/compatibility.md`（`TIMESTAMP_QUERY` 行）。
