# PBR 程序化环境光照（IBL）验证记录（2026-11-21）

范围：[ADR-0044](adr/0044-procedural-ibl.md) 的 PBR 通道 IBL —— `mesh_pbr.wgsl` 的解析天空
（split-sum + 解析 BRDF 近似）、`Camera::pbr_eye.w` 开关位、`cad-scene` 的边叠加抑制、
`cad-app`/`cad-ui-core`/`cad-host` 的 `IBL` 会话开关与
`CadStatus.iblEnabled`。同时记录对 [ADR-0039](adr/0039-mesh-pbr-channels.md) §3/D6/不变量 1 的
修订（§6）。

环境：Linux x86_64，Rust 1.99.1（`rust-toolchain.toml`），`--locked`。

**记录方式（重要）**：设计摘要（§1）先于实现写成，不是证据。§2 由实现 lane 在集成后逐行补跑，
下表为**实际运行结果**（2026-11-21，Linux x86_64，Quadro P620 / Vulkan
`580.178.04`，`device_type=discrete_gpu`）。未执行项保持 **NOT RUN**。

## 1. 实现摘要（待验证的设计，不是证据）

- **环境**：WGSL 内联的**程序化解析天空**，无环境贴图资产、无启动期预滤波 pass。
  `sky_radiance(dir, sun)` 给方向辐亮度，`sky_irradiance(n)` 给半球辐照度近似，
  `env_brdf_approx(roughness, n_dot_v) -> (A, B)` 是 Karis 解析 BRDF LUT 近似（无查找表）。
- **着色**：`prefiltered = mix(sky_radiance(reflect(-v, n), sun), sky_irradiance(n),
  roughness²)`；间接镜面 = `prefiltered * (f0*A + B) * energy * occlusion`，其中
  `ess = A + B`、`energy = 1 + f0*max(1-ess, 0)/max(ess, 1e-4)`；间接漫反射 =
  `base*(1-metallic)*sky_irradiance(n)*occlusion`。IBL 开时不再叠加 `AMBIENT`。
  直接光项与 `f0 = mix(0.04, base, metallic)` 不变；无 tonemap/exposure；IBL 分支
  **不做 ink-flip**（Legacy 分支保持原规则）。
- **开关**：`CommandId::Ibl` / `CommandPayload::Ibl(bool)`；裸 `IBL` 翻转，
  `IBL ON|OFF|1|0`（含 `IMAGEBASED`/`LEGACY` 同义 token）显式设置，其余值拒绝
  `ibl.invalid_value`。会话态 `ViewExtras.pbr_environment: PbrEnvironment { Legacy,
  ImageBased }`，`CadStatus.iblEnabled` 上报。**会话默认 `ImageBased`，渲染器默认 `Legacy`**
  （CLI/直接调用者输出不变）。
- **OFF**：旧路径（单方向光 + `AMBIENT = 0.25` + `base*(1-metallic)*AMBIENT`）**逐字保留**，
  由 fixture 锁定。
- **uniform**：`pbr_eye.w`（word 39）为开关位（0 = Legacy，1 = ImageBased），
  `pbr_emissive.w`（word 35）保持 0；`Camera` 仍 160 B、槽步长 256 B。
- **场景**：`pbr.is_some()` 的网格**不生成三角边叠加**（`cad-scene` 把 `edges` 留空，与
  `MeshWithoutEdgeOverlay` 同形）；`pbr: None` 的网格与 `Body3d` 的边不变。
- **格式**：`.ymodel` 零改动，`FORMAT_MINOR` 不变；环境不落盘、不进 ChangeSet/撤销/导出。

## 2. 运行表

| 命令 / 检查项 | 结果 | 看到的 |
|---|---|---|
| `cargo test -p cad-render-wgpu --test wgsl_validation --locked` | **PASS** `14 passed; 0 failed` | `mesh_pbr` 离线 WGSL 校验通过（含解析天空函数）；`Camera` 布局断言 160 B、成员偏移 `0/64/80/96/112/128/144` |
| `cargo test -p cad-render-wgpu --lib --locked -- --test-threads=1` | **PASS** `348 passed; 0 failed; 4 ignored`（GPU 用例**已运行**，非跳过） | word 39 = 0/1 与 word 35 = 0；`metal_is_not_black_with_ibl_on`（metallic=1、无贴图、IBL 开 > 0.05 luma，关 < 0.01）；roughness 选瓣；AO 作用于间接项；`Legacy` 项与闭式解 `base*(1-metallic)*AMBIENT*occlusion = 0.25` 一致；**同一渲染器两帧之间切换 IBL 必须重新着色**（渐进遍历完成后与 retained 页两种情形，修复前两者均 FAILED） |
| `cargo test -p cad-scene --locked` | **PASS** `84 passed; 0 failed` | `pbr_mesh_emits_no_edge_overlay`（PBR 批 `edges` 为空且 `is_text_glyph()==false`，其余流一致）；`pbr: None` 仍带叠加；`Body3d` 存储边不变 |
| `cargo test -p cad-engine --lib --locked -- --test-threads=1` | **PASS** `130 passed; 0 failed; 16 ignored` | 开关是 live frame 输入：失效帧、`view_revision` +1、**不**置 `navigation_pause_pending`（`pause_after_page == false`）、不推进 `input_revision`、不重建场景；`#[ignore]` 的硬件用例 `ibl_toggle_reshades_the_next_presented_frame_without_a_scene_rebuild` 在真实 GPU 上单独跑过（`1 passed`） |
| `cargo test -p cad-app --locked -- --test-threads=1` | **PASS** lib `3119 passed; 0 failed; 3 ignored` + 6 个集成目标全绿 | 裸 `IBL` 翻转、`IBL ON/OFF/1/0` 显式设置、非法值 `ibl.invalid_value`、不产生事务/撤销/修订 |
| `cargo test -p cad-ui-core --locked -- --test-threads=1` | **PASS** lib `848 passed; 0 failed; 1 ignored` | wire 解析与 `iblEnabled` 投影；命令表 245 条 |
| `cargo test -p cad-host --locked` | **PASS** `451 passed; 0 failed` | 新增 `the_published_view_snapshot_carries_the_session_ibl_switch`：`ViewSnapshot.ibl_enabled` 默认 true → `IBL OFF` false → 裸 `IBL` true，文档修订不变且不可撤销 |
| `(cd apps/desktop-ui && npm test)` | **PASS** `tests 1930 … pass 1930; fail 0` | 视图选项卡 Display 面板 IBL 项（两个 i18n key）；裸/带参命令；按下态只读 `iblEnabled` |
| `cargo test -p cad-cli-tools --locked -- --test-threads=1` | **PASS** lib `42 passed`、`cli_contracts` `16 passed` | `--view` 预置解析、角度/`--fov` 拒绝矩阵；`oblique_views_render_a_real_angle_dependent_3d_projection`（同一 3D 楔体两个方位角帧不同、与 2D 帧不同） |
| 集成门禁：`cargo fmt --all -- --check`、`python3 scripts/check-file-size.py`、`python3 scripts/check-architecture.py`、`python3 scripts/check-i18n.py` | **PASS** | `file-size OK: 2602 code file(s)`；`Architecture OK: 44 packages`；`i18n OK: 3120 keys` |
| `cargo clippy -p cad-render-wgpu -p cad-engine -p cad-scene -p cad-app -p cad-ui-core -p cad-host --all-targets --locked -- -D warnings` | **PASS** `Finished` | 6 个受影响 crate 无警告 |
| `cad-cli-tools render DamagedHelmet.gltf --view isometric`（真实 GPU、headless） | **PASS** exit 0 | `camera = {mode:"3d", view:"isometric", fov_deg:45}`；5/5 贴图 `uploaded`、`fallbacks: []`；`pixels = {non_background:117998, coverage:0.1152, distinct_colors:13229}` |
| 同上但 `--legacy-shading` | **PASS** exit 0 | `non_background:104385, coverage:0.1019, distinct_colors:6736` —— 同一视角、同一资产，IBL 关时暗面直接归零（旧缺陷） |
| 同上但默认 2D 顶视（无 `--view`） | **PASS** exit 0 | `camera.mode:"2d"`、`distinct_colors:10030`；证明默认路径未变 |

以上 CLI 帧为 **headless、单适配器**证据：证明公式、uniform 开关位、批次与投影接线，**不是**
像素/golden/性能/跨厂商验收。

**emissive 的边界（实测）**：该资产的 emissive 贴图只有约 0.95% 的纹素亮于 lum 20，渲染后只在少数
角度出现稀疏原值像素（如 `(255,255,199)`、`(90,255,67)`），**没有**参考图那种 HUD 光晕——那需要
bloom/tonemap，本 ADR 明确不做。不得把"HUD 观感"当成已实现。

**同一会话里切换开关的缺陷（已修）**：首版 `Renderer::set_ibl_enabled` 只存 bool，没有重置
渐进遍历的 `progressive_frame.cursor`；`frame_key` 又不含该标志，所以遍历已完成（`cursor ==
ordered.len()`）时下一帧被规划成**零批次**，屏幕上保留旧像素——CLI（每次新进程、新遍历）看不出来，
只有长驻宿主会中招。修复为"标志真正变化时把 cursor 归零并保留颜色重绘"，并补了两条真实 GPU 回归
用例（遍历完成后 / 复用 retained 页）与一条 engine 契约用例；`progressive_restart_flash` 4 项仍过，
平移的渐进优化未动。**应用内端到端呈现仍为 NOT VERIFIED**（需要人工点击确认）。

## 3. 真实 GPU / 跨后端

**已运行（单适配器）**：`mesh_pbr_gpu_tests` 的 4 个 IBL 数值用例在本机
`Quadro P620` / Vulkan `580.178.04` / `device_type=discrete_gpu` 上**实际执行并通过**
（非 `gpu_or_skip` 跳过）。它们证明"metallic=1 无贴图时 IBL 开 → 非黑、关 → 近黑"这条**公式级**
结论，以及 uniform word 39 是活开关。

这**不是**像素验收、golden 对照、性能或跨厂商结论：只有一个适配器、一个驱动版本、一次运行。

**NOT RUN / NOT VERIFIED 的平台**：

- 其它 NVIDIA / AMD / Intel 真实 GPU（Vulkan）与不同驱动；
- macOS / Metal；
- Windows / DX12（含 WebView2 宿主）；
- Android / Vulkan；
- 真实浏览器 WebGPU / WebGL2（CI 的 `web-smoke` 是 **SwiftShader 软件渲染**，只作布局/DOM
  证据，不得写成 GPU 结论）；
- vendor 资产 / golden 图像对照；
- 任何帧时间、显存或吞吐测量。

## 4. NOT RUN / 未实现清单

- **NOT RUN**：跨厂商/跨后端 GPU、Metal、DX12、Android、真实浏览器 GPU、golden/像素对照、
  性能测量（见 §3）。
- **未实现（本 ADR 明确不做，不得当成待补）**：资产式 HDR 环境、预滤波 cubemap、
  tonemap/exposure、阴影、多光源、`KHR_materials_*`；环境的世界锚定（当前是相机相对的头灯）。
- **未改动且必须保持**：`pbr: None` 网格的 `mesh`/`mesh_textured` 输出、`.ymodel` 布局与
  `FORMAT_MINOR`、`Camera` 160 B / 256 B 步长、导出字节。

## 5. 跟进项

1. 单适配器（P620/Vulkan）已有数值证据；补跑 AMD/Intel、Metal、DX12、Android、真实浏览器 GPU 时
   逐平台登记，绝不把单适配器结论推广为跨厂商或像素验收。
2. `DamagedHelmet` 端到端观感记录已用 `cad-cli-tools render --view isometric` 取得（见 §2 末三行）；
   该帧只证明"金属不再全黑、贴图与自发光可见、三角边叠加已消失"，不是资产保真、像素或性能验收。
3. 逐载体登记：Windows/macOS/Android/Web 的 IBL 结论一律先写"仅编译 / 仅契约 /
   NOT VERIFIED"，直到有对应载体运行证据。
4. 评估把会话令牌聚成 `SessionToggles`（ADR-0044 §5 的重构项），避免 `assemble_status` 参数继续膨胀。

## 6. HDR 环境与色调映射（同日追加，ADR-0044 §6）

程序化天空改为 HDR 波段 + 太阳盘/光晕，IBL 分支输出加 ACES 拟合曲线与具名曝光
（`IBL_EXPOSURE = 2.2`）；**IBL 关闭路径逐字不变**（`tonemapping_is_ibl_only` 契约锁定）。

`cad-cli-tools render DamagedHelmet.gltf --width 1600 --height 1200 --view isometric`，
真实 GPU（Quadro P620 / Vulkan `580.178.04`），模型区域（非背景 265 477 px）：

| 指标 | 改前（LDR） | 改后（HDR + ACES） |
|---|---|---|
| mean | 0.167 | **0.366** |
| median | 0.140 | 0.323 |
| p95 | 0.388 | **0.869** |
| p99 | 0.667 | 0.972 |
| > 0.6 | 3.40 % | 11.69 % |
| > 0.9 | 0.00 % | **4.33 %** |
| > 0.99 | 0.00 % | 0.01 % |

未达成的子项（诚实记录）：最亮的"纯白漆"40 px 窗口 `p95−mean` 只有 **0.010**（目标 ≥0.20）。
原因是该资产白漆在曝光前已在 ACES 肩部（≈0.9 scene-linear），大面均匀天空无法同时满足
"模型均值 ≥0.33" 与"白漆留在中间调"。要真正达到参考图的观感需要**小立体角的 HDR 影棚环境/
面积光**（ADR-0044 D1/§2 已否决的资产式路线），不是继续调常数。

运行的检查：`cargo test -p cad-render-wgpu --lib`（361 passed，GPU 用例实跑）、
`--test wgsl_validation`（14 passed）、clippy `-D warnings`、fmt、file-size 均通过。
其它平台与多适配器仍为 **NOT VERIFIED**。

## 7. 采样器 wrap 缺陷（同日追加，ADR-0044 §7）

根因：mesh 材质的采样器 U/V 写死 `ClampToEdge`，而 glTF 2.0 的默认 wrap 是 **REPEAT**，该资产
`TEXCOORD_0` 的 **V ∈ [1.0006, 1.9987]**（全部越界，采样点被夹到 V=1.0），于是每张贴图只剩**最后
一行纹素**被采样：表面变成沿 U 拉直的一维色带，MR 最后一行 metal≈0.007（零金属）、emissive 全黑
（无 HUD/贴花自发光）、AO≈211（几乎无遮挡）。此前把条纹归因于"缺 mipmap"是误诊；mip/各向异性是
独立的真实质量改进。

修复：`mesh_sampler` U/V 改 `Repeat`（W 不用，保持 clamp；CAD 光栅图 `image_sampler` 不动）；
导入侧对声明了非 REPEAT 的 sampler 报 `import.gltf.sampler_wrap_not_imported`（不新增
`Mesh`/`MeshPbr` 字段，`FORMAT_MINOR` 不变）。

回归围栏（真实 GPU，Vulkan P620）：4×4 贴图最后一行红、其余绿，V∈[1,2] 的 quad ——
生产采样器 green-dominant 192 / red 64（共 256 px），同用例内 `ClampToEdge` 采样器
green-dominant 0 / red 256，缺陷被逐像素钉死。

DamagedHelmet isometric 1600×1200，模型区域（非背景），Rec.709+sRGB：

| 帧 | mean | median | p95 | >0.9 | cyan px | orange px | distinct colours |
|---|---|---|---|---|---|---|---|
| 修复前（ClampToEdge，曝光 2.2） | 0.366 | 0.323 | 0.869 | 4.33 % | 2 | 1866 | 45 161 |
| 只修 wrap（REPEAT，仍 2.2） | 0.663 | 0.700 | 0.976 | 21.01 % | 5 276 | 4 945 | 66 251 |
| 最终（REPEAT，曝光 **0.72**） | **0.442** | 0.395 | **0.897** | **4.33 %** | **8 002** | **3 919** | **66 347** |

纹理恢复后亮度整体上移，`IBL_EXPOSURE` 由 2.2 重标定为 **0.72**（只改这一个常数，环境不动）；
中间值 1.2 → 0.544/0.944/13.0 %、0.8 → 0.463/0.909/6.69 % 用于夹逼，最终落在 ADR-0044 §6 区间内。

运行的检查：`cargo test -p cad-render-wgpu --lib`（362 passed，GPU 用例实跑）、
`--test wgsl_validation`（14 passed）、`cargo test -p cad-import-gltf`（38 passed）、
clippy `-D warnings`、fmt、file-size 均通过。应用内端到端观感仍为 **NOT VERIFIED**。
