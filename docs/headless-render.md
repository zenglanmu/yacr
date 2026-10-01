# 无头渲染与软件适配器（headless wgpu）

规范 §5.2 / §6 / §8 / §11 / §19；审计 F11（离线 `render` 不空成功）与 F12（设备丢失分类）。

本文固定 `cad-render-wgpu` **原生无头路径**的契约：在没有 Slint、没有窗口、没有宿主设备
的 CI/agent 环境里，用软件 Vulkan 适配器（Mesa **lavapipe**）离屏渲染一帧、回读为紧凑
RGBA、编码 PNG。渲染后端契约与管线细节见 `docs/render-backends.md`、`docs/render-3d.md`；
CLI 接线见 `docs/cli.md`；实际执行证据由控制器汇总到 `docs/validation.md`，**本文不代填**。

> 状态：`cad-render-wgpu/src/headless.rs` 与 CLI `render` 由并行工作流实现。本文固定
> **接口契约与测试断言**，不把尚未实跑的路径写成“已验证”。在合并且真实跑通前，第 §6 节
> 是设计契约而非通过证明。

## 1. 目的与边界

### 1.1 用途

- 让 CI / agent 在**无显示、无 GPU** 的机器上得到真实的一帧图像证据，而不是只做静态
  校验（现有 `wgsl_validation` 只解析 WGSL，不创建管线、不执行 draw）。
- 支撑离线 `render` 操作：DWG → 场景 → 离屏 GPU 帧 → PNG + 结构化 JSON。
- 为“固定视口/配置的黄金图”留出唯一入口；黄金图本身是否入库见 §8。

### 1.2 设备/宿主所有权规则

- **正常宿主路径不变**：Slint（Android / Web / 桌面预留）是唯一 Surface/呈现协调者，通过
  `Renderer::initialize_with_device(device, queue)` 交出共享 Device/Queue；CAD 只创建派生
  资源（管线、buffer、离屏纹理），**不**创建第二套事件循环、不争抢 present、不做整幅回读
  （`docs/adr/0002-gpu-ui-composition.md`）。
- **无头是明确的例外**：CI/agent 环境根本没有宿主，因此由 `cad-render-wgpu` 自己创建
  `Instance` → `Adapter` → `Device`/`Queue`。这仍然只发生在渲染器 crate 内部；创建完成后
  走的是同一条 `initialize_with_device` 路径，派生资源仍归 `Renderer` 所有。
- `HeadlessGpu` 拥有该无头会话的 `Instance`/`Adapter`/`Device`/`Queue` 生命周期；这些
  wgpu 对象**不**以裸类型逃逸到 `cad-render-wgpu` 之外供其它 crate 直接依赖（见 §1.3）。
  宿主若要用无头设备，只能通过本模块暴露的 `HeadlessGpu` / `AdapterInfo` 名字空间。
- 回读是**离屏诊断路径**，不是生产合成路径。规范 §5.2 禁止把每帧 GPU→CPU→GPU 整幅回读
  当作正式 CAD 合成；无头路径只在“取一帧做证据”时回读一次。

### 1.3 为什么 wgpu 只能留在 `cad-render-wgpu`

`scripts/check-architecture.py` 强制：**只有** `cad-render-wgpu` 可以依赖 `wgpu`，其余
`cad-*` crate 一旦出现 `wgpu` 即失败。因此：

- 无头适配器枚举、设备创建、回读、PNG 编码都放在 `cad-render-wgpu` 内；
- CLI 通过 `cad-render-wgpu` 的公开接口拿到图像字节，**不**直接 `use wgpu`；
- `cad-cli-tools` 对 `cad-render-wgpu` 的依赖只在 `cfg(not(target_arch = "wasm32"))`
  下声明（浏览器/CLI wasm 保持显式 `unsupported`，见 `docs/cli.md` §7）。

## 2. 冻结的公共接口

全部位于 `crates/cad-render-wgpu/src/headless.rs`（`pub`）。

| 项 | 角色 |
|---|---|
| `AdapterInfo` | 适配器只读描述：backend / name / device_type / driver（见 §3.3） |
| `HeadlessGpu` | 拥有无头 `Instance`/`Adapter`/`Device`/`Queue` 的会话对象 |
| `enumerate_adapters(preference)` | 枚举适配器并返回 `AdapterInfo`（如实可为空） |
| `create_headless_gpu(preference)` | 按偏好创建可用无头设备；失败返回显式错误（§3.4） |
| `RgbaImage` | 紧凑 RGBA8 图像：`width`、`height`、`pixels`（`width*height*4` 字节） |
| `RgbaImage::pixel(x, y)` | 取单个像素 `[u8; 4]` |
| `RgbaImage::count_differing_from(...)` | 与另一图像逐像素比较，统计不同像素数 |
| `RgbaImage::distinct_colors()` | 统计/返回图像中的不同颜色数 |
| `encode_png(&RgbaImage)` | 编码为 PNG 字节 |
| `Renderer::read_target_rgba(&self)` | 回读当前离屏 target，返回 `Result<RgbaImage, RenderError>` |

`preference` 复用现有 `BackendPreference`（`Auto` / `WebGpu` / `WebGl2`）；原生无头下
`Auto` 选择实际可用的原生后端（本环境为 Vulkan/lavapipe）。若 `Auto` 在原生被映射为
`ActiveBackend::Native`，以 `headless.rs` 的实现为准（现有 `caps_for` 即如此，见
`cad-render-wgpu/src/lib.rs`）。

> 未确认：`pixel` 是否返回 `[u8; 4]` 还是 `Option<[u8; 4]>`；`count_differing_from` 是
> “图像 vs 图像”还是“图像 vs 背景色”；`distinct_colors` 返回集合还是计数。这些参数形态
> 以合入的 `headless.rs` 为准，本文只固定存在性与语义。

## 3. 无头设备路径

### 3.1 步骤

```
wgpu::Instance::new(...)                     // 限定要求的 Backends（原生：Vulkan）
  → instance.enumerate_adapters(backends)    // 只读枚举，构造 AdapterInfo 列表
  → instance.request_adapter(&RequestAdapterOptions { ... })
  → adapter.request_device(&DeviceDescriptor { ... })
  → HeadlessGpu { instance, adapter, device, queue, info }
```

`request_adapter` / `request_device` 是异步的。无头路径**不做**自建 runtime，使用
`futures-lite`（workspace 固定 `2.6`）的 `block_on` 同步等待：`block_on(instance.request_adapter(...))`、
`block_on(adapter.request_device(...))`。这与 UI 线程禁止阻塞等待异步浏览器操作的规则
（规范 §6）不冲突：无头路径没有 UI 线程，且只在测试/CLI 入口调用。

### 3.2 初始化接线

创建出 `Device`/`Queue` 后，交给 `Renderer::initialize_with_device(device, queue)`，
从而复用既有管线、帧预算、绘制顺序与设备丢失分类，不复制一份渲染器。`HeadlessGpu` 应
暴露一个方式取得已初始化的 `Renderer` 并保持无头 `Device`/`Queue` 存活（spec：CAD 不
自行创建第二套设备，这里是宿主缺席下的受控替代）。

### 3.3 适配器报告

`AdapterInfo` 至少包含（值来自 `wgpu::AdapterInfo` 与 backend）：

| 字段 | 含义 | lavapipe 预期 |
|---|---|---|
| backend | `Vulkan` / `Gl` / `Metal` / `Dx12` / `BrowserWebGpu` 等 | Vulkan |
| name | 适配器名 | 含 “llvmpipe” / “lavapipe” 字样 |
| device_type | `Cpu` / `IntegratedGpu` / `DiscreteGpu` / `VirtualGpu` / `Other` | `Cpu` |
| driver | 驱动描述 | Mesa lavapipe |

失败时的说明必须可结构化输出（CLI 走 `gpu_failure`，见 §5）。

### 3.4 无适配器 = 显式失败，绝不空成功

- `enumerate_adapters` 可以如实返回**空列表**；空列表本身就是“没有可用适配器”的真实信息。
- `create_headless_gpu` 在无可用适配器 / 设备请求失败时返回
  `Err(CadError::GpuFailure(...))`，**绝不**返回一个“成功但没设备”的 `HeadlessGpu`。
- CLI 层把该错误映射为退出码非零 + `error.code = "gpu_failure"`（`docs/cli.md` §5），
  与旧行为（`render` 恒 `unsupported`）一样不空成功。

## 4. 离屏渲染 → 回读 → PNG

### 4.1 目标格式

离屏 target 固定 `wgpu::TextureFormat::Rgba8UnormSrgb`（与生产合成路径一致，见
`docs/adr/0002-gpu-ui-composition.md`）。清屏色沿用渲染器常量（线性 `r=0.06, g=0.07,
b=0.10, a=1.0`），硬件写入 sRGB 目标时完成线性→sRGB 编码。

### 4.2 回读需 `COPY_SRC`

`Renderer::read_target_rgba` 对自持有的 target 纹理做 `copy_texture_to_buffer`
→ `map_async` → 读取映射范围。这要求 target 纹理带 `TextureUsages::COPY_SRC`：
当前 `ensure_target` 只声明 `RENDER_ATTACHMENT | TEXTURE_BINDING`，实现该接口时必须补上
`COPY_SRC`，否则 `copy_texture_to_buffer` 会因 usage 校验失败。**这是冻结接口之外的
必要代码改动**（在 `lib.rs` 的 `ensure_target`），已记入 §8 的未确认项。

### 4.3 256 字节行对齐

`copy_texture_to_buffer` 要求 `bytes_per_row` 是 `wgpu::COPY_BYTES_PER_ROW_ALIGNMENT`
（= **256**）的整数倍：

```
padded  = align_up(width * 4, 256)
row     = width * 4
```

映射后不能把两块 padding 当作像素。`read_target_rgba` 必须**逐行**把每行前 `width*4`
字节拷出，得到 `width*height*4` 的紧凑缓冲，再构造 `RgbaImage`（`pixels.len() ==
width*height*4`）。

### 4.4 直接写 PNG

`RgbaImage` 存的是 sRGB 编码后的 8 位字节；`encode_png` 只做无损 PNG 封装，**不**再做
一次线性↔sRGB 转换。这样 PNG 与 GPU target 字节一致，回读→PNG→回读的往返可做精确比较
（§6 的 round-trip 断言）。

## 5. 如何运行

```bash
export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json   # force Mesa lavapipe
cargo test -p cad-render-wgpu --offline
cargo run -p cad-cli-tools --offline -- render <input.dwg> --png /tmp/opencode/yacr-render.png --width 1280 --height 720
```

说明：

- `VK_ICD_FILENAMES` 指向 lavapipe 的 Vulkan ICD，用于**强制**软件适配器，避免意外用上
  真实 GPU。
- ICD 路径**因发行版而异**，常见 `lpv_icd.json` 或 `lpv_icd.x86_64.json`；请以本机
  `/usr/share/vulkan/icd.d/` 实际文件为准。
- lavapipe 是 Mesa 的 **CPU 实现**：结果确定但慢，不代表任何 GPU 性能或真实驱动行为。
- `--offline` 避免构建期访问网络。
- `--png` / `--width` / `--height` 是 CLI `render` 操作的目标参数（接线见
  `docs/cli.md`）。无 GPU/无适配器时退出非零并报 `gpu_failure`，不写半截 PNG。

## 6. 测试断言（像素级、确定性）

`cad-render-wgpu` 的无头测试在软件适配器上真实创建管线并执行 draw，断言：

| 断言 | 判据 |
|---|---|
| 非背景覆盖 > 0 | 与清屏背景比较，`count_differing_from(...) > 0`（不代表该有几何的视图不会全背景） |
| 网格三角形 > 0 | `render_3d` 的 `FrameStats.triangles > 0`（预算内真实提交了三角形） |
| 3D 帧非空 | `distinct_colors()` / 非背景像素表明帧不是纯色 |
| PNG 往返 | `encode_png` 后再解码，`count_differing_from` 为 0（逐像素相等） |
| 回读先于渲染 | 未渲染时 `read_target_rgba` 返回 `RenderError::NotInitialized`，不是 panic、不是空图 |

这些都是**确定性**判据：lavapipe 上无异步时序抖动；测试不依赖具体驱动像素，只依赖
“有没有几何 / 是不是纯背景 / 往返是否字节一致”。

## 7. 已验证 / 未验证

本节严格区分。**未实跑的东西不写成通过**。

### 契约层已固定（本文职责）

- 无头接口签名与语义（§2）、设备路径（§3）、格式/对齐/sRGB 规则（§4）、运行命令（§5）、
  测试判据（§6）已在本文冻结。
- 依赖边界由 `scripts/check-architecture.py` 强制（wgpu 仅限 `cad-render-wgpu`），
  `cad-cli-tools` 仅在非 wasm 目标依赖渲染器。
- 既有渲染器的**离线**行为已有测试：未初始化 `render` 返回 `NotInitialized`
  （`render_before_init_is_not_a_device_loss`）、设备丢失分类、WGSL 静态校验
  （`tests/wgsl_validation.rs`）。

### 未验证 / 尚未执行

- `headless.rs` 的实际设备创建、回读与 PNG 编码**未在本文件执行**；`render` CLI 的
  接线由并行工作流完成。本环境禁止运行 `cargo`，无法给出通过证据。
- `Renderer::read_target_rgba` 依赖 `ensure_target` 增加 `COPY_SRC` usage，该改动**尚未
  在本文确认合入**。
- §2 中参数的精确形态（`pixel` / `count_differing_from` / `distinct_colors`、
  `encode_png` 的返回类型、`HeadlessGpu` 取得 `Renderer` 的访问器）以合入实现为准。

## 8. 明确未证明

- **真实样本黄金图**：没有授权 DWG/字体的固定视口黄金图，`fixtures/manifest` 仍为空
  （规范 §11.1、§11.2）。当前无头测试只用合成/程序化场景，**不构成**对真实图纸渲染正确性
  或兼容性的验收。
- **GPU 与软件等价**：lavapipe 通过**不代表**真实 GPU、Android、各驱动或在 `wgpu`
  其他后端上的像素一致；抗锯齿、深度精度、镜像剔除、透明合成在真实设备上仍未验证。
- **浏览器 / Android 宿主**：无头路径只在原生 CPU 上验证；Wasm/WebGPU/WebGL2 与 Android
  的 GPU 运行仍未执行（`docs/render-backends.md`、`docs/adr/0002`、`docs/adr/0003`）。
- **性能**：lavapipe 是 CPU 软件渲染，其耗时**不能**用于任何性能预算或 FPS 结论
  （规范 §11.3 要求真实基准设备）。
- **黄金图/跨后端对照**：`fixtures/manifest` 为空，任何实体兼容性与跨后端语义一致声明
  都不成立。本轮的真实 DWG 行为证据见 `docs/validation.md`，但那是导入/表示层证据，
  **不是**渲染黄金图。
- **CLI `render` 的 PNG/JSON 契约闭环**：由 CLI 工作流与控制器在 `docs/cli.md`、
  `docs/validation.md` 补齐；本文不预先宣称其通过。

## 交叉引用

- `docs/render-backends.md` — 后端选择与仍缺失项。
- `docs/render-3d.md` — 网格管线、深度、镜像剔除、设备丢失分类。
- `docs/validation.md` — 实际执行过的测试/构建证据（无头执行证据应写入此处）。
- `docs/cli.md` — `render` 操作、`gpu_failure`、退出码与原子输出。
- `docs/adr/0002-gpu-ui-composition.md`、`docs/adr/0003-web-composition.md` — 宿主设备所有权。
- `CAD_IMPLEMENTATION_SPEC.md` §5.2/§6/§8/§11/§19。
