# 网格基色纹理验证记录（2026-11-20）

范围：[ADR-0038](adr/0038-mesh-base-color-texture.md) Phase A 的 UV0 + base-colour texture：
`cad-domain` 两字段、`cad-db` 文档纹理表、`cad-import-core` 登记与预算、glTF/IFC/SKP 纹理导入、
OBJ/PLY/RVT 命名拒绝、`cad-render-wgpu` 的 `mesh_textured` 通道、glTF 纹理导出、`.ymodel`
`textures/<key>` 条目与 `FORMAT_MINOR = 3`、`cad-platform` 解码与 Web 按名拒绝。
环境：Linux x86_64，Rust 1.99.1（`rust-toolchain.toml`），`--locked`；真实 GPU 为
NVIDIA Quadro P620 / Vulkan（非软件渲染）。

**记录方式**：下表计数由本轮实现 lane 在其工作树上执行后记录；本文档 lane 只做整理，**没有重跑
任何 cargo 命令**（当时全量构建门禁由另一进程持有）。因此下表的"结果"是实现 lane 的报告，不是
文档 lane 的复现；未执行项在 §5 逐条标注 **NOT RUN**。

## 1. 实现摘要

- **域**：`cad_domain::Mesh` 增 `uvs: Vec<[f64; 2]>`（UV0，空或逐顶点）与
  `texture: Option<String>`（文档自有键 `<sha256 前 32 hex>.png|.jpg`）。不变量：
  `uvs.is_empty() || uvs.len() == vertices.len()`；`texture.is_some()` 要求 `uvs` 非空，
  `cad-db::validate` 按 `InvalidInput` 拒绝违反者。
- **文档资源**：纹理字节存在 `cad-db`（`DrawingDatabase::textures()`），按 sha256 内容寻址、
  同字节去重；`ImportLimits` 默认单张 16 MiB / 合计 64 MiB / 256 张，超限为
  `import.texture.too_large`/`budget_exceeded`/`too_many`，非 PNG/JPEG 为
  `import.texture.unsupported_format`；拒绝后数据库不写入（无部分插入）。
- **`.ymodel`**：新增受管二进制条目 `textures/<key>`（manifest 记 `len`/`sha256` + zip CRC，
  逐条校验，`foreign: false`，**不写入 `raw/`**）；`FORMAT_MINOR` **2 → 3**。minor ≤ 2 归档
  仍可打开——唯一缺省例外是 `cad:Mesh` 缺 `uvs`/`texture` 时解码为空/`None`；旧构建读 minor 3
  按 `ymodel.unsupported_minor` 拒绝。
- **导入**：glTF 读 `baseColorTexture.texCoord` 的 UV 集与 GLB bufferView / `data:` URI 字节；
  IFC 用 `process_element_with_submeshes_textured` + RGBA→确定性 PNG；SKP 用正面 `uv_xform` +
  `Material::Textured.image_bytes`。OBJ/PLY/RVT 不导入纹理并命名拒绝。
- **导出**：glTF 写 `TEXCOORD_0`、内联 `images`（bufferView + `mimeType`）、`samplers` 与
  `materials[].pbrMetallicRoughness.baseColorTexture`；无纹理输出不变。其它写出器当前**不写
  纹理，也没有逐格式 `texture_not_written` 码**（只有 glTF 有），属报告缺口，见 §5。
- **渲染**：新增**独立** `mesh_textured` 通道（UV `@location(3)`、group-2 纹理 + sampler、
  无光照 `sampled × 顶点色 × tint`、alpha blending）；既有 mesh/merged/pulled/pick 通道不变。
  纹理批次不 flat-merge、不走 pulled（一个批次一次 draw）；缺纹理/解码失败按无纹理绘制，
  记 `image.unresolved` 并由 `Renderer::mesh_texture_fallbacks()` 报告。
- **宿主**：`cad_platform::images::local::load_image_cache_for_document` 从 `cad-db` 字节解码，
  无文件系统访问、无占位图；Web 载体没有解码器，按名拒绝
  `unsupported-resources: images: … document texture(s)`。

## 2. 已运行命令与结果（测试矩阵）

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-domain --locked` | **exit 0**（210 passed） |
| `cargo test -p cad-import-gltf --locked` | **exit 0**（22 passed） |
| `cargo test -p cad-import-ifc --locked` | **exit 0**（20 passed） |
| `cargo test -p cad-import-skp --locked` | **exit 0**（22 passed） |
| `cargo test -p cad-import-obj --locked` | **exit 0**（18 passed） |
| `cargo test -p cad-import-mesh --locked` | **exit 0**（32 passed） |
| `cargo test -p cad-import-rvt --locked` | **exit 0**（35 passed） |
| `cargo test -p cad-export-gltf --locked` | **exit 0**（30 passed） |
| `cargo test -p cad-representation --locked` | **exit 0**（363 passed） |
| `cargo test -p cad-scene --locked` | **exit 0**（77 passed） |
| `cargo test -p cad-render-wgpu --locked` | **exit 0**（330 lib + 集成，含 `mesh_textured_gpu_tests`） |
| `cargo test -p cad-engine --locked` | **exit 0**（128 passed） |
| `cargo test -p cad-platform --locked` | **exit 0**（86 passed） |
| `cargo test -p cad-host --locked` | **exit 0**（407 passed） |
| `cargo test -p cad-app --locked` | **exit 0**（3065 lib + 集成） |
| `cargo test -p cad-cli-tools --locked` | **exit 0**（33 + 15 + 3 + 3 + 1 + 16） |
| `cargo test -p cad-store-zip --locked texture` | **exit 0**（6 条纹理用例） |

**说明**：这些计数是实现 lane 报告的通过数，文档 lane 未复现（见文首"记录方式"）。跳过
（例如 `YACR_TEST_*` 未设置）**不是**已运行证据；本轮纹理用例都在合成夹具上。

## 3. 真实 GPU 证据

环境：Linux + NVIDIA Quadro P620 / Vulkan（真实 GPU，非 SwiftShader/lavapipe），
`cargo test -p cad-render-wgpu --lib --locked -- --test-threads=1 mesh_textured_gpu_tests`。

| 用例 | 观测 |
|---|---|
| 1×1 红色纹理 + 白色顶点色/tint 的四边形 | 中心像素为红色（纹理被实际采样，不是顶点色/占位图） |
| 同一场景的拾取 | 拾取仍解析到该实体（新通道不破坏 pick 通道） |
| 引用未上传（不可解析）键的批次 | 按无纹理绘制，`Renderer::mesh_texture_fallbacks()` 列出该键 |

边界：这是**单用例定点**，不是像素比对、golden、视觉验收或性能结论；未测多纹理/大纹理、
mipmap（未实现）、跨 GPU 驱动或其它平台。

## 4. `.ymodel` 往返证据

宿主契约测试（`cad-host`）对**带纹理的网格**执行保存→重开：

- 纹理字节（`textures/<key>`）随归档写出，重开后 SHA-256 与长度相同；
- 网格的 `uvs` 与 `texture` 键在重开后保持；
- minor ≤ 2 归档（无 `textures/`、`cad:Mesh` 无两字段）打开后 UV 为空、纹理为 `None`；
- minor 3 归档被旧构建拒绝由既有 mark 断言钉住（`ymodel.unsupported_minor`），没有真的运行旧
  二进制。

`.ymodel` 仍**只在 Linux** 接线（`apps/app-linux/src/ymodel_archive.rs`）；其它载体打开/另存
`.ymodel` 按 `unsupported-runtime: ymodel` 拒绝。

## 5. 端到端运行、NOT RUN / 未实现（不得据此宣称完成）

下面是 2026-11-20 实际执行的端到端样本运行，以及仍未执行/未实现的项；后者不得由 §2–§4 的
契约测试或本文的端到端结果推广：

- **端到端样本运行（2026-11-20 已执行）**：网络不可达（`curl`/`fetch` 均 `ECONNRESET`），
  因此样本全部为**合成文件**或本机上游夹具；渲染走 `cad-cli-tools render` 的真实 Vulkan
  适配器（NVIDIA Quadro P620），不是软件渲染：

  | 样本 | 结果 |
  |---|---|
  | `checker-textured.glb`（合成：2×2 四色贴图 + UV0） | `mesh_textures.uploaded=1`、`unresolved`/`fallbacks` 空、`distinct_colors=95`、`complete`；帧内可见四色棋盘与三角剖分边 |
  | `checker-textured.gltf`（同几何，buffer 为 data URI） | 与 GLB 相同（`distinct_colors=95`）、`complete` |
  | `checker-plain.glb`（无纹理对照） | `mesh_textures.requested=[]`、`distinct_colors=8`（纯色填充） |
  | `checker-embedded-texture.ifc`（`IfcPixelTexture` 2×1 红/绿） | 纹理 `830b1eea….png` 导入、`uploaded=1`、`distinct_colors=426`、`complete`；帧内为红→绿双线性渐变 |
  | `issue_1781_image_textures.ifc` / `section_textured.ifc`（上游真实夹具，`IfcImageTexture` 外链） | `import.ifc.external_texture_unavailable` + `partial`，不读外部文件 |

  合成样本由 `scripts/e2e-textures/make_textured_gltf.py`（GLB + JSON `.gltf`）与
  `scripts/e2e-textures/make_textured_ifc.py`（`IfcPixelTexture`）生成，均为确定性字节；
  上游 IFC 夹具取自 `ifc-lite-processing` / `ifc-lite-export` 的 `tests/fixtures`。

  - `.ymodel` **文件级**：`apps/app-linux/src/tests/ymodel_texture.rs` 写真实 zip，断言
    `textures/<key>` 条目存在、字节/UV/键往返一致、二次保存（`with_previous`）不丢失、
    缺字节按键名拒绝（`cad:Mesh.texture:`）。
  - SKP **文件级**：`crates/cad-import-skp/src/tests.rs` 用 `skpwright` 2026 写出器生成真实
    `.skp`（纹理材质 + `0x2710` 面纹理块 + 内嵌 PNG），读回后 UV 与 `uv_xform` 逐点一致、
    纹理字节相同；无纹理对照不产生任何纹理诊断。
  - **端到端发现并修复的缺陷**：首轮 glTF 渲染是纯白——CLI 的无头 `render`/光栅 `plot` 只上传
    图像四边形纹理，从未调用 `upload_mesh_textures`（应用侧的服务引擎路径已接线）。修复后
    CLI 上报 `mesh_textures{requested,uploaded,unresolved,fallbacks,bytes}`，未解析/回退键可见；
    矢量 `plot`（SVG/PDF）无纹理通道，记 `vector.mesh_texture_unsupported`。
- **厂商接受性**：three.js / Blender / glTF-Validator 未打开本仓库写出的带纹理 glTF；
  IfcOpenShell / buildingSMART 未验证 IFC；SketchUp / Trimble 未验证 SKP。**全部 NOT RUN**。
- **真实世界 `.skp` 带纹理样本**：离线无公开样本；只有 `skpwright` 2026 写出器生成的
  合成 `.skp` 文件往返（§5），真实 SketchUp 文件的纹理保真度 **NOT RUN**。
- **Windows / macOS / Android / Web 运行**：一律 **NOT VERIFIED**；wasm 纹理路径只到
  `cargo check`（Web 载体对文档纹理是**按名拒绝**，不是解码后渲染）。
- **像素/视觉验收与性能**：无 golden、无像素对比、无 draw-call/帧率测量；软件渲染未用于任何
  像素或性能结论。
- **RVT 纹理**：未实现（需扩展 ADR-0035 补丁台账）；只有防御性
  `import.rvt.texture_not_imported` 契约。
- **PBR 其余通道、UV1+、per-corner UV、`KHR_texture_transform`、MASK 镂空、mipmap/sampler
  选项、SKP/IFC 纹理导出、OBJ/MTL 字节**：未实现（ADR-0038 §7）。
- **非 glTF 写出的纹理损失报告（已补）**：STL/OBJ/PLY/OFF/IFC4/SKP/DXF/DWG 写出器仍**不写**
  纹理，但对每个写出了几何的带纹理网格各记一条 `export.<id>.texture_not_written`
  （`Partial`，消息含实体 id 与纹理键），字节与无纹理导出一致；已被丢弃的实体不重复记。
  这些格式依然**没有**纹理导出。

## 6. 依赖与许可

- `png` **0.18.1**（MIT OR Apache-2.0）：IFC RGBA→PNG 编码与既有图像解码路径；workspace 内
  `png = "0.18"`，未修改上游。
- `sha2` **0.11.1**（MIT OR Apache-2.0）：纹理内容寻址键与 `.ymodel` entry 校验的 SHA-256；
  workspace 内 `sha2 = "0.11"`，未修改上游。（`Cargo.lock` 另有 `sha2 0.10.9`，来自 `rvt`
  等传递依赖，不是本 lane 的直接依赖。）
- 两者均未打补丁、未做 vendoring；许可条目见 `THIRD_PARTY_NOTICES.md`。

## 7. 跟进项

1. 下载真实第三方样本（Khronos glTF-Sample-Assets、带纹理的 IFC/SKP）复跑 §5；本轮网络不可达，
   只有合成文件与本机上游夹具。
2. 厂商查看器接受性（three.js/Blender/glTF-Validator/IfcOpenShell/SketchUp）。
3. wasm 载体的纹理解码策略（当前按名拒绝）与跨载体运行证据。
4. 桌面宿主（GTK/Tauri）的纹理路径运行证据：本轮只到 CLI 无头渲染 + 契约测试。
5. ADR-0038 §7 的后续项（PBR 通道、UV1+、MASK、SKP/IFC 纹理导出、RVT 纹理）。