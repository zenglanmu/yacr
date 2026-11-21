# ADR 0038：网格基色纹理（UV0 + base-colour texture）

- 状态：**Accepted（2026-11-20）**。Phase A 范围已确认：导入 + 渲染 + `.ymodel` 持久化 +
  glTF 导出。全文的"纹理"只指**基色（base colour）贴图**；PBR 其余通道、UV1+、MASK 镂空、
  SKP/IFC 纹理导出、RVT 纹理不在本 ADR 范围内（见 §7）。
- 日期：2026-11-20。
- 相关：[ADR-0037](0037-ymodel-schema-and-persistence.md)（ymodel 容器/分片/FORMAT_MINOR）、
  [ADR-0033](0033-multi-format-import.md)（多格式导入）、
  [ADR-0035](0035-rvt-vendored-patch-policy.md)（`vendor/rvt` 补丁台账）、
  `docs/compatibility.md`、`docs/validation-*-2026-11-20.md`。
- 影响：`cad-domain`（`Mesh` 增两字段）、`cad-db`（纹理字节表 + 校验）、`cad-import-core`
  （纹理登记与预算）、`cad-import-{gltf,ifc,skp}`、`cad-export-gltf`、`cad-representation`、
  `cad-scene`、`cad-render-wgpu`、`cad-platform`、`cad-host`、`cad-store-zip`；
  `FORMAT_MINOR` **2 → 3**。
  **不改变**：`cad-db` 唯一权威、载体文档所有权、patch 禁令（仍只有 acadrust/rvt）、
  wasm 不链接第三方 C 库、报告词汇。

**实现状态（2026-11-20）**：A1 核心（`Mesh.uvs`/`texture`、`DrawingDatabase` 纹理表、
`MeshImport::register_texture` + 预算、`textures/<key>` 二进制条目、`FORMAT_MINOR = 3`）、
A2 渲染（`mesh_textured` 独立通道 + 合并/pulled 排除）、A3 导入（glTF / IFC / SKP 携带
UV0 + 基色字节；OBJ/PLY/RVT 命名拒绝）、A4 导出与宿主（glTF `TEXCOORD_0`/`images`/
`samplers`/`baseColorTexture`；`cad-platform` 从 `cad-db` 字节解码、Web 按名拒绝）**均已实现**，
契约测试见 `docs/validation-textures-2026-11-20.md`。A5 证据只有**库层/宿主契约测试 + 一次
真实 GPU（Quadro P620 / Vulkan）定点用例**；真实样本端到端、厂商接受性、跨载体运行**未运行**。
§7 的各项（PBR 其余通道、UV1+、`KHR_texture_transform`、MASK 镂空、SKP/IFC 纹理导出、
RVT 纹理、OBJ/MTL 字节）**仍不在本轮范围**，不得因为 A1–A4 已落地而宣称它们完成。

---

## 0. 摘要

本轮之前，纹理在每一层都不可表示：`cad_domain::Mesh` 没有 UV，`cad-db` 没有材质/纹理类型，
`RenderBatch` 与 mesh pipeline 没有 UV 顶点属性也没有 texture bind group，`.ymodel` 没有
可写的二进制条目（`raw/` 只做外来条目原样保留）。因此三个"其实拿得到纹理"的导入器都只能
显式降级：glTF 记 `import.gltf.texture_not_imported`，SKP 记
`import.skp.textures_not_sampled`，IFC 记 `import.ifc.style_not_imported`。

本 ADR 只加**一条**能力：**每个网格可带一组 UV0，并引用一张文档自有的基色纹理**。
纹理字节进入 `cad-db`（文档权威），随 `.ymodel` 以 `textures/<key>` 二进制条目保存，
由既有 `DecodedImageCache` 解码后交给渲染器；渲染器新增一条独立的
`mesh_textured` 通道，**不修改**现有 mesh/merged/pulled 三条通道的行为。

这是"诚实的最小闭环"：能看见贴图的格式就真的看见，拿不到字节的格式继续按名拒绝，
不采样、不猜、不用平均色伪装贴图。

---

## 1. 现状证据（2026-11-20 recon）

| 格式 | UV | 纹理字节 | 现在怎么丢的 |
|---|---|---|---|
| glTF 2.0/GLB | `Reader::read_tex_coords(n)` | GLB BIN / `data:` buffer 可解；外部文件不可得 | `scene.rs` 只测 `base_color_texture().is_some()` |
| IFC (ifc-lite 28.2.0) | `SubMesh.uvs` | `MeshTexture.rgba` 或 `ImageTextureRef` | 只调无纹理 router，颜色一律 `ByLayer` |
| SKP (skpwright 0.6.0) | `FaceTexture` + `UvXform::apply` | `Material::Textured{ image_bytes }` | 只用 `avg_rgba`（`import.skp.textures_not_sampled`） |
| OBJ (tobj 4.0.5) | `Mesh::texcoords` | MTL 从不加载，只有路径字符串 | `import.obj.mtl_not_loaded` |
| PLY / OFF / STL | PLY 可声明 `s`/`t`；其余无标准 | 无 | 静默跳过（PLY）/ 无通道 |
| RVT (rvt 0.24.0) | 无 | 无 | 上游 `ifc/gltf.rs` 不写 `images`/`TEXCOORD_0` |

仓库内**没有**任何 UV/材质/纹理承载点，所以缺口在下游，不在格式 crate。

> 2026-11-21：IFC 依赖由 `ifc-lite` 28.2.0 升到 **28.3.0**（本表该行是 2026-11-20 recon 的
> 时点记录），决策本身不变。`SubMesh.uvs` 与纹理 API 不变；28.3.0 的 CSG/void-frame/weld
> 修正可能改变三角化输出，纹理契约测试通过**不是**几何 golden 验收。详见
> `docs/validation-deps-2026-11-21.md`。

---

## 2. 决策（D1–D12）

### D1 — UV 放在 `Mesh` 上，只有 UV0

```rust
pub struct Mesh {
    // …既有 vertices/triangles/normals/face_sources/colors
    /// 逐顶点 UV0，与 `vertices` 一一对应；空表示无 UV。
    /// f64 只为与 `Point3`/ymodel `VecF64x2` 一致，GPU 侧转 f32。
    pub uvs: Vec<[f64; 2]>,
    /// 文档自有基色纹理的键（安全相对名，见 D4）；`None` 表示无纹理。
    pub texture: Option<String>,
}
```

不变量：`uvs.is_empty() || uvs.len() == vertices.len()`；
`texture.is_some()` **要求** `uvs` 非空（否则无法采样，按 `InvalidInput` 拒绝而不是忽略）。

不做 per-corner UV、不做多 UV 集、不做 UV transform（glTF `KHR_texture_transform` 记
`import.gltf.texture_transform_not_imported`）。

### D2 — 纹理是文档资源，不是外部引用

`Mesh.texture` 与既有 `SemanticGeometry::Image.file` 同构：一个**逻辑键**，绝不是平台路径。
区别在于 `Image.file` 由宿主机在受控根目录下解析磁盘文件，而网格纹理的字节**属于文档**，
由 `cad-db` 持有。这样保存/重开/换载体都不依赖源文件是否还在。

### D3 — 字节存在 `cad-db`，`ResourceKey` 只用于下游缓存

```rust
impl DrawingDatabase {
    pub fn textures(&self) -> impl Iterator<Item = (&str, &Arc<[u8]>)>;
    pub fn texture(&self, key: &str) -> Option<&Arc<[u8]>>;
}
```

`cad-db` 不依赖 `cad-resources`（分层禁止反向边），所以这里用 `String` 键；
`cad-scene`/`cad-render-wgpu` 已有 `cad-resources` 依赖，`RenderBatch.texture` 用
`ResourceKey`，转换发生在 representation/scene 边界。

`DocumentSnapshot` 增加同一张表，`cad-store-zip` 负责落盘/读回。

### D4 — `.ymodel`：受管二进制条目 `textures/<key>`，`FORMAT_MINOR = 3`

- 新 `Body::Binary { path, bytes }`：字节原样写入（PNG/JPEG 已压缩，不再二次加工）。
- 条目名 `textures/<key>`，`key` 必须是安全相对名（无 `/`、无 `..`、无盘符/协议、
  非空、仅 ASCII 字母数字与 `._-`）。键由导入器按内容生成：`<sha256 前 32 hex>.png`，
  因此同一张图被多个面集引用时只存一份。
- manifest 记录 `len` + `sha256` + zip CRC，读取时逐条校验；`foreign:false`。
- `is_modelled` 增加 `textures/` 前缀；**未在 manifest 列出的 `textures/` 条目仍按外来条目
  保留**，与现状一致。
- `FORMAT_MINOR` 2 → 3。**新读旧**：minor ≤ 2 的归档没有纹理条目，`cad:Mesh` 缺
  `uvs`/`texture` 两个字段——这是格式里唯一的"缺省"例外，只在 `decode_mesh` 内、
  只对这两个字段生效，并在 §5 用契约测试钉住。**旧读新**：仍由
  `ymodel.unsupported_minor` 拒绝（D11 原规则不变）。

理由：把 PNG 塞进 JSON（hex/base64）会让单张 1 MB 贴图膨胀成数 MB 文本，且破坏
"二进制大对象"这一容器设计；`raw/` 是外来条目区，不能由本构建写入。

### D5 — 导入契约：登记即内容寻址

```rust
impl MeshImport<'_> {
    /// 登记一张纹理，返回可写入 `Mesh.texture` 的键。
    /// 相同字节返回相同键（去重）；超出预算按 `InvalidInput` 拒绝。
    pub fn register_texture(&mut self, bytes: &[u8]) -> CadResult<String>;
}
```

`ImportLimits` 增加 `max_texture_bytes`、`max_total_texture_bytes`、`max_textures`，
默认值写进 `cad-import-core`（单张 16 MiB、合计 64 MiB、最多 256 张），超限是**命名拒绝**
（`import.texture.*`），不是截断。键由 `sha2` 计算，`sha2` 已是 workspace 依赖。

### D6 — 连接器范围（Phase A）

- **glTF**：读 `base_color_texture()` 的 `texCoord` 指向的集合（`read_tex_coords(n)`）与
  `Image::source()`。`Source::View` 从已解析 buffer 切片；`Source::Uri` 只接受
  `data:` base64；外部路径 → `import.gltf.external_image_unavailable`，UV 保留、纹理缺省、
  `Partial`。非基色纹理槽（normal/emissive/occlusion/metallicRoughness）各记一条
  `import.gltf.texture_channel_not_imported`。退化的 UV/长度不符整组丢弃并记
  `import.gltf.uvs_dropped`。
- **IFC**：用 `process_element_with_submeshes_textured` + `build_texture_index`。
  `Decoded(MeshTexture)` → RGBA8 经 `png` 编码后登记；`Image(ImageTextureRef)` 是外部
  URL → `import.ifc.external_texture_unavailable`（不取文件）。上游明文限制：
  带 void 的构件拿不到 UV，此时回退样式颜色并记 `import.ifc.textured_void_fallback`。
  样式颜色（`IfcSurfaceStyle`→`IfcColourRgb`）仍不在本 ADR 内，继续
  `import.ifc.style_not_imported`。
- **SKP**：`Material::Textured.image_bytes` 原样登记（PNG/JPEG 都是合法共享 MIME），
  UV 由 `face.uv_xform(Side::Front, applied_size_in).apply(world_pos_m)` 逐顶点求值。
  背面纹理不导入（`import.skp.back_texture_not_imported`）。
- **OBJ / PLY / STL / OFF**：Phase A 不导入 UV 或纹理。OBJ 记
  `import.obj.texture_not_imported`（MTL 未加载，字节不可得），PLY 记
  `import.ply.texture_coords_not_imported`（不再静默）。
- **RVT**：**不加纹理**。`vendor/rvt` 的 glTF 写出没有 `images`/`textures`/`TEXCOORD_0`，
  要支持必须先扩展 ADR-0035 的补丁台账并重算 `vendor/rvt/SHA256SUMS`；本轮明确不做，
  `docs/compatibility.md` 保留"纹理不存在是设计边界"。`cad-import-rvt` 若在 GLB 里看到
  `TEXCOORD_0`/`images`，记 `import.rvt.texture_not_imported` 而不是静默。

### D7 — 表示层：`Mesh` 原样流动

`DisplayPrimitive::Mesh(Arc<Mesh>)` 已携带整个 `Mesh`，因此不新增 primitive 变体；
必须改的是两处**重建**路径：`cad-representation/src/transform.rs` 的 `transform_mesh`
与 `layout/clip.rs` 的网格裁剪（两者都逐字段重建 `Mesh`）。漏改会让纹理在变换/裁剪后消失，
由契约测试钉住。

### D8 — 场景：纹理批次不合并、不走 pulled 通道

```rust
pub struct RenderBatch {
    // …既有字段
    pub uvs: Vec<[f32; 2]>,
    pub texture: Option<ResourceKey>,
}
```

- `uvs` 只在 `texture.is_some()` 时非空，长度 == `vertices.len()`；否则两者都为空。
- `approx_bytes` / `upload_size_bytes` / `vertex_bytes` 计入 UV（每顶点 8 B）。
- `mesh_batch_is_mergeable` 与 pulled 分流都必须排除 `texture.is_some()`：
  合并通道把 per-batch 着色烘进单一顶点流、只绑一个 uniform，无法表达逐批次纹理。
  这是**已知的批处理退化**，写进文档，不是静默行为。

### D9 — 渲染器：新增独立 `mesh_textured` 通道

- 顶点布局：位置(0)/法线(1)/颜色(2)/**UV(3, `Float32x2`)**，UV 只在纹理批次上传。
- bind group 2 与 image 通道同构：`texture_2d<f32>` + `Filtering` sampler。
- 片元：`sampled.rgb * vertex_color * tint`；alpha = `sampled.a * batch.alpha`。
  纹理批次开启 alpha blending；`alphaMode` 的 MASK 镂空不实现（导入时已按
  `import.gltf.alpha_mask_not_imported` 记丢失）。
- **不修改** 既有 `mesh` / `mesh_merged` / `mesh_pulled` 通道与 pick 通道。
- WGSL 必须通过离线 `wgsl_validation`（`Capabilities::empty()`）；不使用 storage buffer、
  不使用 `textureSampleLevel` 之外的扩展能力，保持 WebGL2 安全。
- 纹理缺失/解码失败：该批次按无纹理绘制（基色/顶点色），并记进图像加载报告，
  绝不画占位贴图。

### D10 — 宿主：解码顺序与预算

`cad_platform::images::load_image_cache` 先解码**文档自有纹理**（受同一 `ResourceLimits`
约束），再解析 `SemanticGeometry::Image.file` 的文件引用。文档纹理不经过
`is_safe_reference` 的磁盘解析（无文件系统访问），但仍校验键的安全形式。
解码失败按既有 `ImageLoadReport` 逐条记录，不替换成占位图。

### D11 — 导出：只有 glTF 写纹理

`cad-export-gltf` 在网格带 UV + 纹理时写 `TEXCOORD_0`、`images`（同一 buffer 的
bufferView + `mimeType`，`.gltf` 因此仍是单个 data-URI buffer）、`textures`、`samplers`、
`materials[].pbrMetallicRoughness.baseColorTexture`。材质去重键扩展为
`(rgb, alpha, texture)`，没有纹理的网格行为不变。

其余导出器按名记 `export.<id>.texture_not_written`（`Partial` 或 `Missing`）：
STL/PLY/OFF/OBJ 没有承载通道，IFC/SKP 写出了纹理属于本轮之外。

### D12 — 证据边界

- 自洽往返（导出→导入）是**唯一**自动化证据；three.js/Blender/IfcOpenShell/SketchUp
  验收、像素对比、真实 GPU 性能一律 **NOT RUN**，写进 `docs/validation-*.md`。
- 渲染证据只允许"布局/DOM/几何"级别；软件渲染（SwiftShader/lavapipe）不得用于像素或
  性能结论。
- `.ymodel` 必须补：纹理字节固定点（保存→重开字节相同）、minor 2 归档可读、
  minor 3 归档被旧构建按 `unsupported_minor` 拒绝（用现有 mark 断言，不必真的跑旧二进制）。

---

## 3. 备选方案与否决理由

1. **纹理烘焙成顶点色**（不引入 UV/纹理）：会伪造映射关系，且对高分辨率贴图直接失真。
   否决——违反铁律 2（不得把降级伪装成完成）。
2. **纹理存 JSON hex/base64**：不新增归档机制，但体积膨胀、破坏二进制大对象设计。
   否决（D4）。
3. **复用 `raw/` 存纹理**：`raw/` 是外来条目原样保留区，任何新写入都破坏其语义与
   raw-copy 判定。否决。
4. **把纹理当独立 `SemanticGeometry::Image` 实体**：图元没有 UV，等于把贴图贴在错误的
   几何上。否决。
5. **扩展 `vendor/rvt` 让 RVT 出纹理**：本轮收益不确定（上游外观资源解析本身未实现），
   且要动补丁台账。推迟，见 §7。

---

## 4. 实施分期（本 ADR 的执行顺序）

| 阶段 | 内容 | 依赖 |
|---|---|---|
| A1 核心 | `Mesh` 两字段 + 校验；`DrawingDatabase` 纹理表 + builder + 校验；`MeshImport::register_texture` + 预算；`DocumentSnapshot.textures`；`cad-store-zip` `textures/` 二进制条目 + minor 3 + 读 minor 2；`decode_mesh` 可选字段 | — |
| A2 渲染 | `transform_mesh`/clip 携带；`RenderBatch` 字段与字节账；`mesh_streams`；UV 顶点流；`mesh_textured` pipeline + WGSL；纹理解码上传；合并/pulled 排除 | A1 |
| A3 导入 | glTF / IFC / SKP 三条纹理导入；OBJ/PLY/RVT 命名拒绝 | A1 |
| A4 导出与宿主 | glTF `TEXCOORD_0`+`images`+`textures`+`materials`；`cad-platform` 解码顺序；`cad-host` snapshot 往返 | A1（导出另需 A3 的夹具） |
| A5 证据 | `.ymodel` 固定点、minor 兼容、全量门禁、`docs/compatibility.md`/validation 更新 | A1–A4 |

---

## 5. 不变量与契约测试（必须存在）

1. `uvs.is_empty() || uvs.len() == vertices.len()`；`texture.is_some()` 时 `uvs` 非空 —
   `cad-db::validate` 拒绝违反者，`VALIDATE` 诊断稳定。
2. `Mesh` 里 `texture` 的键必须是安全相对名；含 `/`、`..`、盘符、协议、控制字符的键
   **不进库**（导入/反序列化双向拒绝）。
3. `.ymodel`：`textures/<key>` 字节保存→重开 SHA-256 相同；manifest 记录 `len`/`sha256`；
   条目被篡改则拒绝打开（既有条目校验路径）。
4. `.ymodel`：minor 2 归档（无 `uvs`/`texture` 字段、无 `textures/`）打开后 `uvs` 为空、
   `texture` 为 `None`，再保存升为 minor 3 且行为不变。
5. `decode_mesh` 的"缺省"只允许 `uvs`/`texture` 两个字段；其它字段缺失仍是错误
   （用一条负向测试钉住，防止放松扩散）。
6. 纹理去重：同一 PNG 登记两次只有一个键、一份字节。
7. 预算：超 `max_texture_bytes`/`max_total_texture_bytes`/`max_textures` 是命名拒绝，
   数据库保持未写入状态（no partial insert）。
8. glTF 导入→导出→导入：UV 与纹理字节回到同一内容键；无 `TEXCOORD_0` 的网格不因导出
   凭空获得 UV。
9. 变换/镜像/裁剪后的网格：UV 与纹理仍在，数值符合变换语义（镜像不翻转 U 除非有明确
   约定——本轮约定**不翻转**，因为 UV 属于表面参数化）。
10. 合并/pulled 通道：带纹理批次永不进入（用 `mesh_batch_is_mergeable` 与分流断言钉住）。
11. 渲染器：`wgsl_validation` 覆盖新 shader，且 `Capabilities::empty()` 通过。
12. 能力声明诚实：格式 crate 的 `EntityCapability.render` 不因纹理支持而升级为
    "像素已验证"；报告里逐条列出未导入的通道。

---

## 6. 兼容性与迁移

- **旧文档**：minor ≤ 2 全量可读（D4 例外条款）。没有纹理的网格行为完全不变。
- **旧构建读新文档**：`ymodel.unsupported_minor`，退出码映射仍不存在（ADR-0037 D13 的
  `8` 仍是计划），宿主按稳定标记提示。
- **无 `.ymodel` 的载体**（Windows/macOS/Android/Web 当前）：纹理只在会话内存中随
  `DrawingDatabase` 存活；不做静默导出或旁路文件，`.ymodel` 仍按
  `unsupported-runtime: ymodel` 拒绝。
- **性能**：纹理批次不合并会显著增加 draw call；`RenderBatch::approx_bytes` 计入 UV，
  预算测试同步更新。软件渲染下的帧率**不作为**结论。

---

## 7. 明确不在本轮范围

| 项 | 原因 | 后续 |
|---|---|---|
| metallic/roughness/normal/emissive/occlusion | 域模型与 shader 均无通道；收益对 CAD 显示有限 | 另立 ADR |
| UV1+、per-corner UV、`KHR_texture_transform` | 需要新 `FieldKind` 与纹理矩阵；先证明 UV0 闭环 | ADR-0038 增补 |
| MASK 镂空 | 需要 `discard` 与 cutoff 参数 | 同上 |
| SKP/IFC 纹理**导出** | SKP 需要投影矩阵反解，IFC 依赖 `ifc-lite-export` 无纹理 plan | 另立 ADR |
| RVT 纹理 | 必须改 `vendor/rvt` 并扩展 ADR-0035 台账 | 视 Revit 外观资源可得性再评估 |
| OBJ/MTL 纹理字节 | MTL 不在导入字节里（需要宿主文件访问策略） | 与 xref/资源加载策略一起设计 |

---

## 8. 状态

Accepted（2026-11-20）。A1–A4 已实现，A5 的证据是库层/宿主契约测试与一次真实 GPU 定点用例，
逐条记录在 `docs/validation-textures-2026-11-20.md`；未运行的验证一律写 **NOT RUN**。§7 的
排除项仍不实现，新增能力按"另立 ADR / 增补 ADR-0038"推进。
