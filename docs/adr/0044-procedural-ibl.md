# ADR 0044：PBR 通道程序化环境光照（IBL）与 IBL 会话开关

- 状态：**Accepted（2026-11-21）**。决策已定；契约测试与运行证据**尚未执行**，逐项状态见 §4 与
  `docs/validation-ibl-2026-11-21.md`（当前全部 **NOT RUN / 待补**）。能力汇总见
  `docs/compatibility.md` 的「网格 PBR 通道」一节。
- 日期：2026-11-21。
- 相关：[ADR-0038](0038-mesh-base-color-texture.md)（UV0 + 基色纹理）、
  [ADR-0039](0039-mesh-pbr-channels.md)（`mesh_pbr` 通道，本 ADR 修订其 §3 的 IBL 行、D6 的着色
  环境项与不变量 1）、[ADR-0037](0037-ymodel-schema-and-persistence.md)（`.ymodel` 与
  `FORMAT_MINOR`，本 ADR **不**改动）、[ADR-0026](0026-gpu-memory-accounting-and-budget.md)
  （GPU 预算）、[ADR-0010](0010-react-linux-host.md)（React 宿主与 `cad_status` 上报）。
- 影响：`cad-render-wgpu`（`mesh_pbr.wgsl` 的间接光分支、`Camera` 的 `pbr_eye.w` 开关位）、
  `cad-scene`（PBR 网格的边叠加抑制）、`cad-app`（`CommandId::Ibl` / `CommandPayload::Ibl` /
  会话态 `ViewExtras.pbr_environment`）、`cad-ui-core`（wire 与菜单态）、`cad-host` /
  `cad-desktop-shell`（`CadStatus.iblEnabled` 透传）、`apps/desktop-ui`（菜单/状态项）。
  **不改变**：`cad-db` 是唯一权威、`.ymodel` 布局与 `FORMAT_MINOR`、载体所有权裁决、
  patch 禁令（仍只有 `acadrust`/`rvt`）、`pbr: None` 网格的 `mesh_textured` / `mesh` 路径。

---

## 0. 摘要

在本 ADR 之前，`mesh_pbr.wgsl`（ADR-0039 D6）只有**一盏单方向光** `camera.light` 与一个常数环境项
`AMBIENT = 0.25`，且环境项写作 `base * (1 - metallic) * AMBIENT`。因此 `metallic = 1` 的材质
**既没有间接漫反射、也没有任何可反射的环境**：它在只有方向光的场景里只剩一个高频镜面瓣，
其余方向全黑。导入的真实资产正好落在这个洞里——Khronos `DamagedHelmet`
（`metallicFactor = 1`，五个通道贴图齐全）在黑背景上只看见白色三角边叠加，看起来像"贴图全丢"，
实际是**通道齐全但没有环境光**。

本 ADR 给 `mesh_pbr` 通道增加一个**程序化、解析计算的环境**（split-sum 预滤波近似 + 解析天空），
由会话开关 `IBL` 控制，**默认开**。不开任何新资源、不引入 cubemap、不改 uniform 布局、不升格式
版本：环境是**派生的显示概念**，不是文档数据。

---

## 1. 决策

### D1 — 环境用解析天空在 WGSL 内联计算

环境由常量块 + 顶点法线/视线**当场算出**：没有环境贴图资产，没有启动期预滤波 pass。

理由逐条：

1. **wasm 体积**：预滤波路线要么内嵌一张 HDR（数百 KB 到数 MB，进所有载体产物），要么在
   `dom-canvas` 的 wasm 会话里跑 PMREM 生成。前者违反"不新增资产"的最小闭环，后者把
   `wasm-bindgen` 产物和启动内存一起推高（ADR-0025 的内存预算已经紧张）。
2. **headless CLI（软件 Vulkan）启动成本**：`cad-cli-tools render` 与 Linux `linux-host-smoke`
   在 lavapipe/SwiftShader 上跑。预滤波 cubemap 是一条多 mip、多次采样的 GPU pass，
   在软件光栅器上是**秒级**开销；解析天空是几十条浮点指令，与分辨率线性相关。
3. **后端多样性**：项目要求同一 wgpu 模块覆盖原生 Vulkan/Metal/DX12、`dom-canvas` 的
   WebGL2 与（将来）WebGPU。WebGL2 **没有 compute shader、没有可靠的浮点渲染目标、不能逐 mip
   写**，预滤波必须为 WebGL2 写第二套降级实现——那正是"静默回退"的温床。
4. **跨后端确定性**：解析天空是纯算术，同一输入在 Vulkan/Metal/DX12/WebGL2 上结果在浮点误差内
   一致，因此可以写**数值契约测试**（见 §4）；预滤波的结果依赖驱动过滤与浮点精度，
   只能做宽松阈值比较，测不出回归。
5. **"缺失必须显式拒绝而非静默回退"**：资产式环境一旦资产缺失，只剩两条路——画黑（异常静默）
   或隐式换成另一个环境（观感静默改变）。解析天空没有"缺失"状态，因此没有需要拒绝的分支。
6. **扩展点**：环境以**着色器常量 + 一个 uniform 开关位**参与计算，未来若接入资产式 HDR，
   只需替换"环境值从哪来"，**不需要改 uniform 契约**或格式版本。

**明确否决**：启动期预滤波 cubemap、内嵌 HDR 资产、任何环境贴图文件。它们都不是本 ADR 的实现
路线，不得作为"以后顺手补上"的隐含承诺。

### D2 — 着色：split-sum 解析近似

间接光分三块计算，全部只依赖法线 `n`、视线 `v`、粗糙度 `roughness`、金属度 `metallic`、
基色与既有单方向光 `camera.light`：

```
sky_radiance(dir, sun)      // 解析天空的辐亮度（与方向相关，含太阳盘与地平线渐变）
sky_irradiance(n)           // 同一解析天空的半球积分近似（与法线相关，低频）
env_brdf_approx(roughness, n_dot_v) -> vec2(A, B)   // Karis 解析 BRDF LUT 近似，无查找表
```

- **预滤波近似**：
  `prefiltered = mix(sky_radiance(reflect(-v, n), sun), sky_irradiance(n), roughness * roughness)`。
  粗糙表面收敛到半球辐照度，光滑表面收敛到单方向辐亮度——这是 split-sum 用解析天空代替
  mip 链的等价形式。
- **菲涅耳/能量**：令 `f0 = mix(vec3(0.04), base, metallic)`（与 ADR-0039 D6 相同），
  `fss_ess = f0 * A + B`（单次散射菲涅耳能量）；多散射能量补偿的标量是
  `ess = A + B`，`energy = 1 + f0 * max(1 - ess, 0) / max(ess, 1e-4)`。
  间接镜面 = `prefiltered * fss_ess * energy * occlusion`。
- **间接漫反射** = `base * (1 - metallic) * sky_irradiance(n) * occlusion`。
- **IBL 开着时不再叠加 `AMBIENT`**：环境项替代常数环境光，不是在其之上再加一层。
- **AO 同时作用于间接漫反射与间接镜面**：这是对 ADR-0039 D6 的**修订**（那里 AO 只乘漫反射与
  常数环境项，镜面项不受遮蔽）。理由是环境遮蔽在物理上描述环境可见度，与"直接光镜面"无关，
  与"间接镜面"有关。
- **直接光项不变**：Cook-Torrance GGX + 单方向光 `camera.light`，`n_dot_l` 衰减，
  `f0 = mix(0.04, base, metallic)`，`diffuse = base * (1 - metallic)`。
- **不做 tonemap / exposure**：输出沿用 `mesh.wgsl` 的显示约定（线性 RGB 写入 sRGB 目标）。
  **例外一处**：IBL 开着时**不做 ink-flip**——环境反射是"被照亮的环境"，不是近白墨线，
  浅色背景下把它翻成黑会毁掉金属。`Legacy`（OFF）分支保持既有的 ink-flip 规则逐字不变。

### D3 — 开关：`IBL`，会话态，不持久化

- 命令：`CommandId::Ibl`，载荷 `CommandPayload::Ibl(bool)`。裸 `IBL` 为**翻转**，
  `IBL ON|OFF|1|0` 为**显式设置**；会话 setter 同时接受 `IMAGEBASED`/`LEGACY` 两个同义 token
  （大小写不敏感），其余值一律命名拒绝 `ibl.invalid_value`，不做"猜一个"。
- 会话态：`ViewExtras.pbr_environment: PbrEnvironment { Legacy, ImageBased }`。
  这是**会话状态**：不进 `ChangeSet`、不进撤销栈、不写 `.ymodel`、**不升 `FORMAT_MINOR`**。
- 上报：`CadStatus.iblEnabled` 供 React 宿主渲染菜单勾选态。宿主不自己保存这个布尔值，
  只读 `cad_status`（与其它会话令牌同一模式）。
- **OFF 语义**：OFF 就是**本 ADR 修改前**的路径——单方向光 + `AMBIENT = 0.25` +
  环境项 `base * (1 - metallic) * AMBIENT`，**逐字相同**。这不是"改完顺便一样"的巧合，而是
  一条被显式保留、由 fixture 锁定的回退路径（fixture 与运行状态见 §4 与
  `docs/validation-ibl-2026-11-21.md`）。
- **默认值**：**会话默认 `ImageBased`**（打开 PBR 资产就能看见环境）；
  **渲染器自身默认 `Legacy`**——`cad-cli-tools`、直接构造 `Renderer` 的调用者与既有测试
  在未显式开启时输出不变。
- **载体无关**：`native-below`（Wayland/Windows/macOS/Android）与 `dom-canvas`（X11/XWayland、
  Web）走**同一个** `CadSession` 与**同一个** wgpu 模块，因此不存在每载体文件、也不存在
  第二套着色实现。未来若某个载体没有 `mesh_pbr` 通道，必须以**命名原因**拒绝，
  不得静默画成无环境或退回 `mesh`。

### D4 — uniform：复用 `pbr_eye.w` 开关位

- `Camera::pbr_eye.w`（第 39 号 32-bit word）作为开关位：`0 = Legacy`、`1 = ImageBased`
  （着色侧按 `> 0.5` 判定）。该字段原为 unused，写入不改变任何既有读取（`pbr_eye.xyz`
  仍是视线原点）。
- `pbr_emissive.w`（第 35 号 word）继续保留为 `0`，不复用。
- `Camera` 仍为 **160 B**，uniform 槽步长仍为 **256 B**；**布局、绑定、槽位数一概不变**，
  因此不触发格式版本或管线布局变化。
- 开关位是**每帧 uniform**，不是 push constant / specialization constant：WebGL2 没有后者，
  用 uniform 才能保证四个后端同一条路径。

### D5 — PBR 网格不再生成三角边叠加

`cad-scene` 对 `pbr.is_some()` 的网格**不派生三角边叠加**：`RenderBatch.edges` 留空，
批次形状与 `DisplayPrimitive::MeshWithoutEdgeOverlay` 一致（同一 overlay 规则），
vertex/triangle/UV/tangent/PBR 流与图元种类保持不变——PBR 材料可能来自
`DisplayPrimitive::Mesh`，因此不需要改写入库图元，只在派生批次这一层关掉叠加。
`RenderBatch::is_text_glyph` 的判定同时加 `pbr.is_none()` 守卫，否则一个没有边叠加的
PBR 网格会被导航 LOD 误当成文字隐藏。

- 原因：叠加层是为**未贴图的 CAD 网格**做可读性用的（平面法线 + 无纹理时需要边线区分面）；
  一个带完整 PBR 材质、法线贴图与环境光的导入资产再压一层白色三角边，是纯粹的错误观感。
  这正是 `DamagedHelmet` 看起来"只剩白色三角线"的第二个来源。
- **显式建模**：这是"导入 PBR 资产不携带三角线框"的声明，不是静默抑制——`Body3d` 的绘制
  边（`BodyDisplay.edges`，经 `MeshWithStoredEdges`）与未贴图 CAD 网格**不受影响**，
  仍走 `Mesh` + 叠加。
- 该选择在 `cad-scene` 内按 `pbr.is_some()` 决定，**不看** IBL 开关：OFF 的 PBR 网格同样不叠边
  （OFF 只关环境光，不改变"这是 PBR 资产"的事实）。

### D6 — 声明边界（本 ADR 不做）

| 项 | 原因 |
|---|---|
| 资产式 HDR 环境 / 预滤波 cubemap | D1 的六条理由；环境是解析函数，不是文件 |
| tonemap / exposure | 会改变既有资产观感，需要新的显示管线约定；本 ADR 只补间接光 |
| 阴影（shadow map / SSAO / ray query） | 需要额外 pass 与预算模型，与"解析环境"是两件事 |
| 多光源 | 仍是单方向光 `camera.light` |
| `KHR_materials_*` 扩展族 | 与 ADR-0039 §3 相同，不在本轮 |
| 只有 `baseColor` 的 glTF（`pbr: None`） | 仍走 `mesh_textured`，**没有 IBL**——没有 PBR 就没有 PBR 通道，不硬塞 |
| 环境的世界锚定 | 环境是**相机相对**的：`camera.light` 是头灯，太阳随视角移动。这是**已声明限制**，不是可修的缺陷；三维场景光照仍只需"看得清形状" |
| 打开文档即 3D 视图 | `Viewport::new` 仍是 2D 顶视，需要 `SWITCH2D3D` 才进 3D；本 ADR 不改视图初始化 |

---

## 2. 被否方案

| 方案 | 否决理由 |
|---|---|
| 预滤波 cubemap（启动期生成） | 软件 Vulkan 秒级启动开销；WebGL2 无 compute/浮点 RT/mip 写，必须二套实现（D1） |
| 内嵌 HDR 资产 | wasm 与所有载体产物体积；资产缺失时的回退只能是黑或静默换图（D1） |
| 只用常数环境光但调大数值 | `metallic = 1` 仍无间接镜面，金属依旧黑；本质是掩盖不是修复 |
| 把 `AMBIENT` 直接乘进镜面项 | 不是环境反射，粗糙度/菲涅耳全丢，金属会变成"灰塑料" |
| IBL 恒开、不给开关 | 直接改变所有 PBR 资产与既有 PBR 契约测试的像素；且无法回退对照 |
| 开关做成文档属性 | 环境是显示派生概念，写进文档会污染 `cad-db` 权威与 `.ymodel` 版本 |
| 给每个载体单独的 IBL 实现 | 违反"同一 `CadSession`/同一 wgpu 模块"；会造成能力分叉与结论不可迁移 |

---

## 3. 边界与不变量

1. **ADR-0039 的"逐像素一致"收窄**：原文不变量 1（`pbr: None` 与 ADR-0038 输出逐字节/逐像素
   一致）**只适用于 `pbr: None`**。`pbr: Some` 的批次在 IBL 默认开时像素改变是**预期结果**，
   不是回归；`pbr: None` 的网格仍必须逐像素不变。
2. **OFF 是显式回退路径**：`IBL OFF` 的输出必须与 ADR-0039 D6 的原式逐字一致（单方向光 +
   `AMBIENT = 0.25`）。该性质由 fixture 锁定，不得因后续着色改动而漂移。
3. **`cad-db` 仍是唯一权威**：环境是**派生的显示概念**——不写入文档、不进 `.ymodel`、
   不进 ChangeSet、不进撤销栈、不参与导出。导出器写出的 glTF 不携带任何环境信息。
4. **不变量守恒**：`Camera` 160 B / 槽步长 256 B 不变；`pbr_emissive.w` 保持 0；
   `FORMAT_MINOR` 不变；`.ymodel` 读写路径零改动。
5. **无载体分叉**：`native-below` 与 `dom-canvas` 共用同一份着色与开关接线；没有
   `*_ibl_*.rs` 之类的每载体文件。缺 `mesh_pbr` 通道的载体必须命名拒绝。
6. **开关位只有两个取值**：写入侧只写 `0`（Legacy）或 `1`（ImageBased），着色侧按该位二选一；
   没有第三种模式，也没有运行期"自动挑选"。非 0/1 的值只能来自我们自己的 bug，因此不额外定义语义，
   只由契约测试固定写入侧的常量。

---

## 4. 验证

> 本节列出**契约测试面**；截至 2026-11-21 的记录全部是 **NOT RUN / 待补**，逐条命令与状态见
> `docs/validation-ibl-2026-11-21.md`。不得因为本节列出用例就宣称已通过。

| 面 | 断言 |
|---|---|
| `cad-render-wgpu --test wgsl_validation` | `mesh_pbr` 仍通过离线 WGSL 校验；`Camera` 布局断言（160 B / 256 B 步长）；`pbr_eye.w` = word 39、`pbr_emissive.w` = word 35 |
| `cad-render-wgpu --lib` | `metal_is_not_black_with_ibl_on`：`metallic = 1` 在 IBL 开时非黑（这是本 ADR 的**核心**回归）；非金属材质有可观测的间接光但不改变直接光结论；**非 PBR 批次不回归**（`mesh`/`mesh_textured`/`mesh_merged`/`mesh_pulled` 输出不变）；`IBL OFF` 与旧式逐字一致（fixture） |
| `cad-scene` | `pbr.is_some()` → `RenderBatch.edges` 为空（与 `MeshWithoutEdgeOverlay` 同形）；`is_text_glyph` 因 `pbr.is_none()` 守卫不误判；`pbr: None` 网格仍带叠加；`Body3d` 的 `MeshWithStoredEdges` 边不受影响 |
| `cad-app` | `CommandId::Ibl` / `CommandPayload::Ibl(bool)` 的翻转与显式设置；会话默认 `ImageBased`；`IBL OFF` 不产生事务/撤销/修订 |
| `cad-ui-core` | 裸 `IBL` 与 `IBL ON/OFF/1/0`（含 `IMAGEBASED`/`LEGACY` 同义 token）的 wire 解析；其余值按名拒绝 `ibl.invalid_value`；`CadStatus.iblEnabled` 反映会话态 |
| `cad-host` | 会话开关不写文档、不进 ChangeSet、不触发保存脏标记；`cad_status` 透传 |
| `apps/desktop-ui` | 菜单项勾选态来自 `cad_status.iblEnabled`，不本地缓存布尔值；`npm test`（`tsc --noEmit` + `node --test`） |

渲染侧证据（若补）只能是：软件 Vulkan / headless 的**数值 readback**（证明公式与开关位接线）
或真实 GPU 的定点用例；两者都**不是**像素、golden、性能或产品验收。

---

## 5. 后果

- **改动面**：`cad-render-wgpu`（shader + uniform 写入 + 管线不变）、`cad-scene`（边叠加抑制）、
  `cad-app`（命令 + 会话态）、`cad-ui-core`（wire + 状态）、`cad-host`/`cad-desktop-shell`
  （`iblEnabled` 透传）、`apps/desktop-ui`（菜单态）。**单 crate 级别的着色改动，无每载体分叉。**
- **格式**：零变化。`.ymodel` 不升 `FORMAT_MINOR`，旧构建仍读同一归档；环境不落盘。
- **GPU 预算**：不新增纹理、采样器或缓冲（ADR-0026 的纹理/缓冲记账不变），只多几条标量指令。
- **兼容性**：`pbr: None` 的用户可见输出完全不变；`pbr: Some` 在默认开时**变亮**（这正是本 ADR
  的目的），关闭 `IBL` 可回到旧观感。既有 PBR 像素断言必须按 §3.1 收窄，不得继续断言
  "PBR 批次像素不变"。
- **已知代价**：解析天空是相机相对的（D6），旋转视角时太阳跟着动；环境质量不如真实 HDR，
  粗糙表面的预滤波是解析近似而非真实 mip 链。
- **后续重构项**：`assemble_status` 的参数继续膨胀（本轮再加 `iblEnabled`）。把会话令牌聚成
  一个 `SessionToggles` 结构体是明确的后续项，**不在本 ADR 内做**，以免把着色改动与 wire 重构
  混在同一个可回退单元里。
- **证据状态**：所有验证**待补**（`docs/validation-ibl-2026-11-21.md` 全部 NOT RUN）。
  真实 GPU、Metal、DX12、Android/Vulkan、浏览器 GPU 一律 **NOT VERIFIED**。

---

## 6. 修订（2026-11-21，HDR 环境与色调映射）

本条修订 §1 D2 的两处措辞，其余决策（D1、D3–D6、§2、§3）**不变**：环境仍是 WGSL 内联的解析
函数，仍无资产、无 cubemap、无预滤波 pass、无新 binding、无 uniform 布局变化。

### 6.1 环境改为 HDR（含太阳盘与辉光）

`mesh_pbr.wgsl` 的解析天空从 LDR 改为 **HDR**，仍是纯 ALU 的确定性函数（无纹理、无循环）：

| 常量 | 值（线性 RGB） | 亮度 |
|---|---|---|
| `SKY_ZENITH` | `(1.35, 1.70, 2.20)` | ≈1.66（偏冷） |
| `SKY_HORIZON` | `(1.45, 1.32, 1.18)` | ≈1.34（偏暖） |
| `SKY_GROUND` | `(0.33, 0.30, 0.26)` | ≈0.30 |
| `SUN_DISC_POWER` / `SUN_DISC_INTENSITY` | `900` / `60` | 窄太阳盘，暖白 `(1.00, 0.94, 0.85)` |
| `SUN_GLOW_POWER` / `SUN_GLOW_INTENSITY` | `24` / `2.4` | 宽辉光，同一暖白 |

- **原因（可测量的缺陷，不是观感偏好）**：LDR 天空（亮度约 0.6–0.7）没有任何方向能接近显示上限，
  所以低粗糙度金属**没有真正的镜面峰**，反射里的地平线读不出来，白色漆面也永远不出现高光。
  实测 `DamagedHelmet` 等距视图：模型均值亮度 0.167、p95 0.388、>0.9 的像素仅 0.005%，白漆窗口
  `p95 − mean` 只有 0.04–0.07（参考实现同一视图约 0.428）。
- `sky_irradiance` 同步升到同一能量级：仍是余弦瓣的解析拟合（半球坡，形状不变），但不再停在 LDR
  数值。**太阳盘与辉光不计入漫反射辐照度**——它们的能量已经是直接光 `camera.light`，
  折进间接漫反射就是重复计能。
- 环境仍**相机相对**（D6）：太阳沿 `camera.light`，随视角移动。

### 6.2 色调映射只作用于 IBL 分支

IBL 分支在**光照求和之后、写入 sRGB 目标之前**（并在选择/悬停 tint **之前**——那是 UI 颜色，
不是场景辐亮度）应用 ACES 拟合曲线：

```
rgb   = lit * vertex_colour * batch_tint
rgb   = tonemap_aces(rgb * IBL_EXPOSURE)          // IBL_EXPOSURE = 2.2
tonemap_aces(x) = clamp(x*(2.51x+0.03) / (x*(2.43x+0.59)+0.14), 0, 1)
```

- `IBL_EXPOSURE` 是**命名常量**，也是唯一旋钮。它不是相机 f-stop：PBR 路径的漫反射沿用
  ADR-0039 的 `albedo * n_dot_l`（没有 1/π）显示约定，因此需要这个场景线性补偿量。
- **alpha 不变**；曲线写在 sRGB 目标之前，`Rgba8UnormSrgb` 的编码仍是最后一次写入变换。
- **IBL 开启时跳过 ink-flip 的规则不变**（D2 的例外）；legacy 分支的 ink-flip 逐字保留。

### 6.3 收窄 D2/D6 声明的「不做 tonemap」

D2 原文「**不做 tonemap / exposure**」收窄为：**legacy（IBL OFF）路径与所有非 PBR 通道**
（`mesh`/`mesh_textured`/`mesh_merged`/`mesh_pulled`、线、文字、图像、掩膜）仍**不做**
tonemap/exposure，输出与 ADR-0039 D6 逐字节一致；**只有 IBL 路径**做 6.2 的曲线。D6 表格里
「tonemap / exposure」一行按同一收窄理解（不再是一刀切）。

### 6.4 契约测试的变化

| 用例 | 断言 |
|---|---|
| `tonemapping_is_ibl_only`（**新增**） | 自发光夹具（`-Z` 法线 + `ao = 0` ⇒ `lit == emissive` 精确）：IBL OFF 时逐字节等于线性→sRGB 闭式值，且**不等于**曲线值（曲线不得泄漏）；IBL ON 时 x1<x2 ⇒ out(x1)≤out(x2)，1.0 的输入落在肩部（<255，而非直接削平），曲线 clamp 保证任何输入都在 `[0,1]`，alpha 不变 |
| `ibl_roughness_selects_the_specular_lobe`（重推） | 改用**暗 `f0`** 的金属夹具：白 `f0` 在 HDR 下被推到曲线肩部后，两个瓣会落进同一字节（≈252 vs ≈246），契约看不出差异；暗 `f0` 让镜面瓣（反射地平线）与粗糙瓣（余弦辐照度）仍可区分 |
| `metal_is_not_black_with_ibl_on` / `ibl_occlusion_scales_the_indirect_term` / `ibl_toggle_reshades_*` | 意图不变，阈值按 HDR + 曲线重推（`ibl_luma > 0.05` 等仍然成立） |
| `ibl_off_matches_the_legacy_lighting` | 闭式期望 `srgb_byte(AMBIENT)` 不变：曲线不得泄漏到 legacy |

### 6.5 已知残余（本修订不做）

- **均匀亮天空 ≠ 真实 studio HDR**：本环境是**大面积均匀**亮天空，漫反射辐照度高，因此
  **全亮**的白漆会停在曲线肩部（显示值 ~0.97），只有漆面的明暗过渡区才读得到镜面/环境高光。
  真实 studio HDR 的亮源立体角小，漫反射辐照度低，白漆会留在中灰而高光更锐利。要消除这一差异
  需要资产式 HDR 或分区光源，属于 D1/§2 明确否决的路线。
- 仍是**单方向光 + 解析环境**：没有阴影、没有 SSAO、没有多次弹射，粗糙表面的预滤波是解析近似
  而非真实 mip 链。
- 曲线是显示变换，不是相机的物理曝光/白平衡模型；不写文档、不进 `.ymodel`、不参与导出。

**证据**：`cargo test -p cad-render-wgpu --lib`（真实 GPU 用例）与
`cargo test -p cad-render-wgpu --test wgsl_validation`（离线 WGSL 解析）的逐条结果、以及
HDR/色调映射前后的实测亮度表，记在 `docs/validation-ibl-2026-11-21.md`。这些仍是**数值/公式**
证据，不是像素、golden、性能或产品验收；Metal/DX12/Android/浏览器 GPU 一律 **NOT VERIFIED**。
