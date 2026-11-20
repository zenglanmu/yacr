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

## 5. NOT RUN / 未实现（不得据此宣称完成）

以下逐条为 **NOT RUN**（或未实现），不得由 §2–§4 的契约测试推广：

- **端到端样本运行（待补）**：glTF / IFC / SKP 的纹理导入→渲染与 `.ymodel` **文件级**保存→重开
  尚未执行，由本文档 lane 之后另一 lane 补跑后更新本文件；在此之前不得写"通过"。
  - glTF 纹理样本 → 渲染 → `.ymodel` 文件往返：**待补**
  - IFC 纹理样本 → 渲染 → `.ymodel` 文件往返：**待补**
  - SKP 纹理样本 → 渲染 → `.ymodel` 文件往返：**待补**
- **厂商接受性**：three.js / Blender / glTF-Validator 未打开本仓库写出的带纹理 glTF；
  IfcOpenShell / buildingSMART 未验证 IFC；SketchUp / Trimble 未验证 SKP。**全部 NOT RUN**。
- **真实 `.skp` 带纹理样本**：离线无样本，SKP 纹理只有合成夹具契约；**NOT RUN**。
- **Windows / macOS / Android / Web 运行**：一律 **NOT VERIFIED**；wasm 纹理路径只到
  `cargo check`（Web 载体对文档纹理是**按名拒绝**，不是解码后渲染）。
- **像素/视觉验收与性能**：无 golden、无像素对比、无 draw-call/帧率测量；软件渲染未用于任何
  像素或性能结论。
- **RVT 纹理**：未实现（需扩展 ADR-0035 补丁台账）；只有防御性
  `import.rvt.texture_not_imported` 契约。
- **PBR 其余通道、UV1+、per-corner UV、`KHR_texture_transform`、MASK 镂空、mipmap/sampler
  选项、SKP/IFC 纹理导出、OBJ/MTL 字节**：未实现（ADR-0038 §7）。
- **非 glTF 写出的纹理损失报告**：STL/OBJ/PLY/OFF/IFC4/SKP/DXF/DWG 写出器不写纹理，也**没有**
  `export.<id>.texture_not_written` 码（源码中只有 `export.gltf.texture_not_written`）；
  当前是静默丢失，属待补报告缺口，**不得**写成已无损。

## 6. 依赖与许可

- `png` **0.18.1**（MIT OR Apache-2.0）：IFC RGBA→PNG 编码与既有图像解码路径；workspace 内
  `png = "0.18"`，未修改上游。
- `sha2` **0.11.1**（MIT OR Apache-2.0）：纹理内容寻址键与 `.ymodel` entry 校验的 SHA-256；
  workspace 内 `sha2 = "0.11"`，未修改上游。（`Cargo.lock` 另有 `sha2 0.10.9`，来自 `rvt`
  等传递依赖，不是本 lane 的直接依赖。）
- 两者均未打补丁、未做 vendoring；许可条目见 `THIRD_PARTY_NOTICES.md`。

## 7. 跟进项

1. 用真实 glTF/IFC/SKP 样本做端到端渲染与 `.ymodel` 文件往返（§5 "待补"）。
2. 为 STL/OBJ/PLY/OFF/IFC4/SKP/DXF/DWG 写出器补纹理损失报告（或明确"该格式不支持纹理"的
   稳定码）。
3. 厂商查看器接受性（three.js/Blender/glTF-Validator/IfcOpenShell/SketchUp）。
4. wasm 载体的纹理解码策略（当前按名拒绝）与跨载体运行证据。
5. ADR-0038 §7 的后续项（PBR 通道、UV1+、MASK、SKP/IFC 纹理导出、RVT 纹理）。