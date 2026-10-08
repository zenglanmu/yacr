# 无头渲染与软件适配器（headless wgpu）

规范 §5.2 / §6 / §8 / §11 / §19；审计 F11（离线 `render` 不空成功）与 F12（设备丢失分类）。

本文固定 `cad-render-wgpu` **原生无头路径**的契约：在没有 Slint、没有窗口、没有宿主设备
的 CI/agent 环境里，用软件 Vulkan 适配器（Mesa **lavapipe**）离屏渲染一帧、回读为紧凑
RGBA、编码 PNG。渲染后端契约与管线细节见 `docs/render-backends.md`、`docs/render-3d.md`；
CLI 接线见 `docs/cli.md`；实际执行证据由控制器汇总到 `docs/validation.md`，**本文不代填**。

> 状态（2026-10-01，已执行）：`cad-render-wgpu/src/headless.rs` 与 CLI `render` 已
> 合入并在 Mesa **lavapipe** 上真实跑通（`VK_ICD_FILENAMES=.../lvp_icd.json`）。
> `cad-render-wgpu` 的 `tests/headless_render.rs`（6）与 `tests/render_effects.rs`（6）
> 全部通过；CLI 对真实 DWG 出帧并写出 PNG。执行证据汇总见 `docs/validation.md`，
> 本文只描述契约与已执行范围。

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
- `HeadlessGpu` 拥有该无头会话的 `Device`/`Queue`（并把适配器描述为 `AdapterInfo`）；
  `Instance`/`Adapter` 在创建成功后即可释放。这些 wgpu 对象**不**以裸类型逃逸到
  `cad-render-wgpu` 之外供其它 crate 直接依赖（见 §1.3）。宿主若要用无头设备，
  只能通过本模块暴露的 `HeadlessGpu` / `AdapterInfo` 名字空间。
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
| `AdapterInfo` | 适配器只读描述：backend / name / device_type / driver / driver_info |
| `HeadlessGpu` | 无头会话：`device`、`queue` 与 `adapter: AdapterInfo` |
| `enumerate_adapters(preference)` | 枚举适配器并返回 `AdapterInfo`（如实可为空） |
| `create_headless_gpu(preference)` | 按偏好创建可用无头设备（等价 `create_headless_gpu_with(preference, Auto)`）；失败返回显式错误（§3.4） |
| `create_headless_gpu_with(preference, gpu)` | 按 `GpuSelection` 偏好设备类型：`auto`/`high` = 优先独显、`low` = 优先核显；该类型缺失时回退到所选后端的**首个适配器**，绝不跨后端 |
| `GpuSelection` | 稳定名 `auto` \| `high` \| `low`；`auto`/`high` 为离散优先（双显卡默认），`low` 为核显优先；`parse` / `as_str` 稳定往返（CLI `--gpu`、UI `--gpu`、Slint power preference 共用） |
| `RgbaImage` | 紧凑 RGBA8 图像：`width`、`height`、`pixels`（`width*height*4` 字节） |
| `RgbaImage::pixel(x, y)` | 取单个像素 `[u8; 4]`（越界返回透明黑，不 panic） |
| `RgbaImage::count_differing_from(background, tolerance)` | 统计与给定背景色任一通道差异超过容差的像素数 |
| `RgbaImage::distinct_colors()` | 统计图像中的不同 RGBA 颜色数 |
| `encode_png(&RgbaImage)` | 编码为 PNG 字节（`Result<Vec<u8>, String>`，缓冲长度不符时报错） |
| `Renderer::read_target_rgba(&self)` | 回读当前离屏 target，返回 `Result<RgbaImage, RenderError>` |

`preference` 复用现有 `BackendPreference`（`Auto` / `WebGpu` / `WebGl2`）；原生无头下
`Auto`/`WebGpu` 接受 Vulkan 与 GL（Vulkan 优先），`WebGl2` 限定 GL。适配器选择按
`backend_priority` 确定性进行，`GpuSelection` 只在所选后端**内部**调整偏好（不换后端）。
混合显卡机器（Intel 核显 + NVIDIA 独显）**默认（`auto`/`high`）即优先独显**；
`--gpu low`（CLI）或 `--gpu low`（Linux App）才切到核显。偏好只是尽力而为：无可出图
/可达的独显（如表面不兼容）时回退到所选后端的首个适配器。

## 3. 无头设备路径

### 3.1 步骤

```
wgpu::Instance::new(...)                     // 限定要求的 Backends（原生：Vulkan）
  → instance.enumerate_adapters(backends)    // 只读枚举，构造 AdapterInfo 列表
  → instance.request_adapter(&RequestAdapterOptions { ... })
  → adapter.request_device(&DeviceDescriptor { ... })
  → HeadlessGpu { device, queue, adapter: AdapterInfo }
```

`request_adapter` / `request_device` 是异步的。无头路径**不做**自建 runtime，使用
`futures-lite`（workspace 固定 `2.6`）的 `block_on` 同步等待：`block_on(instance.request_adapter(...))`、
`block_on(adapter.request_device(...))`。这与 UI 线程禁止阻塞等待异步浏览器操作的规则
（规范 §6）不冲突：无头路径没有 UI 线程，且只在测试/CLI 入口调用。

### 3.2 初始化接线

创建出 `Device`/`Queue` 后，交给 `Renderer::initialize_with_device(device, queue)`，
从而复用既有管线、帧预算、绘制顺序与设备丢失分类，不复制一份渲染器。`HeadlessGpu`
本身不持有 `Renderer`：调用方解构 `HeadlessGpu { device, queue, adapter }`，把
`device`/`queue` 交给自己的 `Renderer`，并用 `adapter` 做结构化报告（CLI 即如此）。

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
→ `map_async` → 读取映射范围。这要求 target 纹理带 `TextureUsages::COPY_SRC`；
`ensure_target` 已在 `RENDER_ATTACHMENT | TEXTURE_BINDING` 之外声明 `COPY_SRC`，否则
`copy_texture_to_buffer` 会因 usage 校验失败。生产合成路径不做整幅回读，只有无头
证据路径使用该接口。

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

### 4.5 提交等待与帧拟合

- `Renderer::render` 以有界 `poll(Wait{timeout})` 区分“完成 / 设备丢失”。默认 1 秒面向
  交互式宿主；软件适配器的大图纸（数万 draw call）会超过 1 秒，因此无头调用方显式
  `Renderer::set_poll_timeout(600s)`。否则慢的 CPU 帧会被误报为 `DeviceLost`（实测
  `canteen.dwg` 42868 draw call 在默认 1 秒下误报，提高等待后 8.8 秒成功）。
- CLI 相机按**实际绘制的批次**（`local_origin + vertex` 的世界坐标）拟合，而非
  `drawing.bounds()`；后者包含未绘制内容（文字、块定义几何），会使出图偏小偏心。
  无任何批次时以 `invalid_input` 失败，不伪造空帧。

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
| 适配器报告 | `AdapterInfo.backend == "vulkan"`、`device_type == "cpu"`、`driver == "llvmpipe"`（强制 lavapipe 时） |
| 线几何非空 | 矩形 `LineList` 渲染后与左上角背景比较，非背景像素 > 0 且内部仍为背景 |
| 网格三角形 > 0 | `FrameStats.triangles > 0` 且 `opaque_batches == 1`，非背景像素 > 0 |
| 3D 帧非空 | `render_3d` 后非背景像素 > 0，无 `RenderError` |
| PNG 往返 | `encode_png` 后校验 PNG 签名/ IHDR 宽高；`render_effects` 用 `png` 解码逐字节比对 |
| 回读先于渲染 | 未渲染时 `read_target_rgba` 返回 `RenderError::NotInitialized`，不是 panic、不是空图 |
| 透明合成 | `alpha=0.5` 的批次计入 `transparent_batches`，叠加像素与 `alpha=1.0` 不同；`alpha=0` 计入 `invisible_batches` |
| 帧预算 | 超预算时 `FrameStats.over_budget.is_some()` 且 `skipped_batches >= 1`，不静默丢批 |
| 设备丢失分类 | `note_device_lost(...)` 后 `render` 返回 `Err(DeviceLost)`，`is_device_lost()` 为真 |
| 确定性 | 同一场景两次渲染的回读字节完全相同 |
| 大坐标精度 | `local_origin` 很大而局部顶点很小时仍非背景（相对原点不丢精度） |

这些都是**确定性**判据：lavapipe 上无异步时序抖动；测试不依赖具体驱动像素，只依赖
“有没有几何 / 是不是纯背景 / 往返是否字节一致”。测试在无适配器时**显式跳过并打印**，
不会假装通过。

## 7. 已验证 / 未验证

本节严格区分。**未实跑的东西不写成通过**。

### 已执行（本轮，2026-10-01）

- 依赖边界由 `scripts/check-architecture.py` 强制（wgpu 仅限 `cad-render-wgpu`），
  `cad-cli-tools` 仅在非 wasm 目标依赖渲染器；架构检查通过。
- `cad-render-wgpu`：`tests/headless_render.rs` 6 通过、`tests/render_effects.rs` 6 通过、
  单元 31 通过、`wgsl_validation` 3 通过（`VK_ICD_FILENAMES=.../lvp_icd.json`）。
- 设备创建/回读/PNG 已在 lavapipe 上执行；`ensure_target` 的 `COPY_SRC` 已合入。
- CLI `render` 对真实 DWG 出帧：`patient-chairs`（11855 draw call，`non_background=22381`）、
  `lockers`、`baseline-sample`、`map-of-uae`、`canteen`（42868 draw call，2.76M 顶点，8.8s）
  均成功并写出 PNG；无可绘制几何（`anonymous-names`/`point_object_id`）显式
  `invalid_input`。逐项命令与数值见 `docs/validation.md`。
- `cad-cli-tools`：单元 6 + 契约 12 通过；wasm `--lib` 仍编译（`render` 保持 `unsupported`）。

### 未验证 / 尚未执行

- 无适配器分支（`gpu_failure`）在本机未能实际触发（强制 bogus ICD 仍回退到常驻 GL/llvmpipe
  适配器）；该分支是结构性的（`create_headless_gpu → GpuFailure → gpu_failure → 退出 1`），
  但**未在本机观测**。
- 真实样张的**黄金图基线**与跨 GPU/驱动/后端的像素矩阵尚未建立。
- 文字（Text/Instance/Image）不经本路径绘制（属各自子系统；CLI `render` 未接字体整形）。

## 8. 明确未证明

- **真实样本黄金图**：仍没有大型授权 DWG/字体与跨后端固定的视口黄金图矩阵（规范
  §11.1、§11.2）。仓库已提交开源 QCAD flange 样本及其上游参考图，但它是 `Partial`
  回归输入，**不构成**对真实图纸渲染正确性或兼容性的验收。
- **GPU 与软件等价**：lavapipe 通过**不代表**真实 GPU、Android、各驱动或在 `wgpu`
  其他后端上的像素一致；抗锯齿、深度精度、镜像剔除、透明合成在真实设备上仍未验证。
- **浏览器 / Android 宿主**：无头路径只在原生 CPU 上验证；Wasm/WebGPU/WebGL2 与 Android
  的 GPU 运行仍未执行（`docs/render-backends.md`、`docs/adr/0002`、`docs/adr/0003`）。
- **性能**：lavapipe 是 CPU 软件渲染，其耗时**不能**用于任何性能预算或 FPS 结论
  （规范 §11.3 要求真实基准设备）。
- **黄金图/跨后端对照**：`fixtures/manifest` 只有合成夹具与一个开源 QCAD `Partial`
  样本，任何实体兼容性与跨后端语义一致声明都不成立。真实 DWG/DXF 的渲染行为证据见
  `docs/validation.md`、`docs/validation-dxf-flange.md`，但都**不是**跨后端黄金图或
  兼容性验收。

## 交叉引用

- `docs/render-backends.md` — 后端选择与仍缺失项。
- `docs/render-3d.md` — 网格管线、深度、镜像剔除、设备丢失分类。
- `docs/validation.md` — 实际执行过的测试/构建证据（无头执行证据应写入此处）。
- `docs/cli.md` — `render` 操作、`gpu_failure`、退出码与原子输出。
- `docs/adr/0002-gpu-ui-composition.md`、`docs/adr/0003-web-composition.md` — 宿主设备所有权。
- `CAD_IMPLEMENTATION_SPEC.md` §5.2/§6/§8/§11/§19。
