# ADR 0039：网格 PBR 通道（metallic-roughness + normal/emissive/occlusion）

- 状态：**Accepted（2026-11-20）**。实施状态：P1–P4 已完成（见 §5）；P5 证据已补契约测试
  与真实 GPU 用例，端到端 `pbr-channels.glb` 渲染记录 **待补**。验证边界见
  `docs/validation-pbr-2026-11-20.md`。
- 日期：2026-11-20。
- 相关：[ADR-0038](0038-mesh-base-color-texture.md)（UV0 + 基色纹理，本 ADR 在其上扩展）、
  [ADR-0037](0037-ymodel-schema-and-persistence.md)（`FORMAT_MINOR`）、
  [ADR-0035](0035-rvt-vendored-patch-policy.md)（`vendor/rvt`）、`docs/compatibility.md`。
- 影响：`cad-domain`（`MeshPbr` + 新 `FieldKind` + minor **4**）、`cad-db`（校验）、
  `cad-import-gltf` / `cad-import-rvt`（读入）、`cad-export-gltf`（写出）、
  `cad-representation`/`cad-scene`/`cad-render-wgpu`（PBR 着色通道）、
  `cad-platform`/`cad-host`（多张文档纹理解码）。
  **不改变**：`cad-db` 唯一权威、载体所有权、patch 禁令、wasm 不链接 C 库。

---

## 0. 摘要

ADR-0038 只支持"UV0 + 基色贴图"。本 ADR 增加 glTF 2.0 的 metallic-roughness 材质其余通道：

1. **标量**：`metallic`、`roughness`、`emissive`（线性 RGB）、`occlusion_strength`。
2. **贴图**：`metallicRoughnessTexture`、`normalTexture`、`emissiveTexture`、`occlusionTexture`
   （全部使用 UV0，字节存在既有文档纹理表）。
3. **着色**：新增独立 `mesh_pbr` 通道（GGX + 法线贴图 + 自发光 + 环境光遮蔽），
   复用渲染器已有的单方向光（`camera.light`）；既有 mesh/merged/pulled/textured 通道不变。

数据先于着色：所有通道都能**保真保存与导出**；渲染只在几何确实声明了 PBR 数据时启用。

### 现状证据（2026-11-20 核对）

- glTF：`gltf` 1.4.1 暴露 `pbr_metallic_roughness().metallic_factor()/roughness_factor()/
  metallic_roughness_texture()`、`normal_texture()`、`occlusion_texture()`、
  `emissive_texture()/emissive_factor()`；ADR 定稿前导入器**全部忽略**，导出器**全部不写**
  （P2 已接线，见 §5）。
- RVT：`vendor/rvt` 0.24.0/0.25.0 的 `src/ifc/pbr.rs` 按材质名启发式给出 `metallic/roughness`
  并写进 glTF 的 `metallicFactor`/`roughnessFactor`；`cad-import-rvt/src/glb.rs` 在 ADR 定稿时
  只读 `baseColorFactor`，**静默丢弃**这两个值。
- IFC：`IfcSurfaceStyleRendering` 有 `Transparency`/`SpecularHighlight`/`ReflectanceMethod`/
  `SpecularRoughness`，没有 metallic；伪造映射不做，样式颜色仍按名记
  `import.ifc.style_not_imported`（`pbr: None`）。
- SKP：`skpwright` 只给颜色/不透明度/贴图，无 PBR → `pbr: None`（无数据，不需要诊断）。
- OBJ：`tobj` 给 `emissive`、`shininess`、`optical_density`，但 MTL 未加载（字节流无文件源）
  → 仍不导入任何材质通道。

**`rvt` 版本核对**：上游最新版 **0.25.0**（2026-11-15）的 `src/ifc/gltf.rs` 仍然**没有**
`images`/`textures`/`TEXCOORD_0`；`appearance_asset_id` 依旧只解码不解析位图。升级到 0.25.0
**不带来 RVT 纹理**；纹理仍是显式边界（`import.rvt.texture_not_imported`）。本 ADR 定稿后，
`rvt` 已从 0.24.0 升级到 0.25.0（同一 alpha 补丁按原逻辑重挂，见 ADR-0035 与
`docs/rvt-patches.md`）；PBR 标量从 0.25.0 的 `src/ifc/pbr.rs` 读取，仍只是名称启发式，
不是 Revit 作者数据。

---

## 1. 决策

### D1 — `MeshPbr` 挂在 `Mesh` 上

```rust
pub struct MeshPbr {
    pub metallic: f64,            // 0..=1
    pub roughness: f64,           // 0..=1
    pub emissive: [f64; 3],       // 线性 RGB，0..=1
    pub occlusion_strength: f64,  // 0..=1，仅与 occlusion 贴图同用
    pub metallic_roughness_texture: Option<String>,
    pub normal_texture: Option<String>,
    pub emissive_texture: Option<String>,
    pub occlusion_texture: Option<String>,
}

pub struct Mesh {
    // …vertices/triangles/normals/face_sources/colors/uvs/texture
    pub pbr: Option<MeshPbr>,
}
```

- **`None` 表示"源没有声明 PBR"**，不是"默认值 0/1"。glTF 里只有
  `baseColorFactor`/`baseColorTexture` 的材质仍是 `None`（渲染走既有通道），
  因为 glTF 的默认 `metallicFactor = 1` 是规范默认值，套用到无光照 CAD 显示会把
  资产画成黑色金属——那不是"保真"，是伪造。显式声明了任一 PBR 标量/贴图才建 `MeshPbr`。
- 所有 PBR 贴图使用 **UV0**（我们只有一组 UV）。glTF 中 `texCoord != 0` 的槽记
  `import.gltf.uv_set_ignored`（已有码），该槽不导入。
- 贴图键与 ADR-0038 同规则（`<sha256 前 32 hex>.png|.jpg`，安全相对名，字节在 `cad-db`）。

### D2 — `.ymodel`：`cad:Mesh.pbr` 可选嵌套对象，`FORMAT_MINOR = 4`

- 新 `FieldKind::MeshPbr`（每个变体必须被至少一个字段使用，`schema/classes.json` 要能拼写）。
- payload：`"pbr": null | { metallic, roughness, emissive, occlusion_strength,
  metallic_roughness_texture, normal_texture, emissive_texture, occlusion_texture }`。
  与 ADR-0038 的例外一致：`pbr` **允许缺失**（→ `None`），其余字段仍严格。
- `FORMAT_MINOR` 3 → 4；minor ≤ 3 可读（缺 `pbr` → `None`）；旧构建读 4 按
  `ymodel.unsupported_minor` 拒绝。
- 纹理条目继续复用 `textures/<key>`（不新增条目族）。

### D3 — 校验

- `metallic`/`roughness`/`occlusion_strength`/`emissive` 各分量有限且 ∈ `[0,1]`；
- 每个贴图键必须是合法键，且 `db.texture(key)` 存在；有 `occlusion_texture` 时
  `occlusion_strength` 才生效（无贴图则不校验其"意义"，但仍要求有限）；
- 有 `pbr.normal_texture` 时要求 `Mesh` 有非空 `uvs`（法线贴图必须有参数化）；
  其余 PBR 贴图同样要求 `uvs`（着色器按 UV 采样）；
- 违反者是稳定 `InvalidInput`（`cad:Mesh.pbr:` 前缀）。

### D4 — 导入

**glTF**：读全部五个贴图槽与三个标量。图像字节解析复用 ADR-0038 的
`texture::resolve_base_colour_texture`（View / `data:` URI；外部文件按名拒绝）。
去重走 `MeshImport::register_texture`，因此同一张图在多槽/多网格只存一份。
`normalTexture.scale`、`occlusionTexture.strength`、`KHR_materials_emissive_strength`：
`scale` 非 1、`emissive_strength` 非 1 逐条按名报告（不猜）；
`occlusion_strength` 直接存。

**RVT**：读 `metallic_factor`/`roughness_factor`，`pbr: Some { metallic, roughness,
其余默认 }`，并记 `import.rvt.pbr_heuristic`（说明这两个值是 `rvt-rs` 按材质名启发式生成的，
不是 Revit 作者数据；`extras` 只标注颜色/透明来源）。非有限/越界 → 不建 `MeshPbr` +
`import.rvt.pbr_ignored`。

**IFC / SKP / OBJ**：不建 `MeshPbr`（`pbr: None`）；IFC 仍只记 `import.ifc.style_not_imported`
（样式里只有颜色/透明度/高光粗糙度，没有 metallic），SKP/OBJ 无 PBR 数据也不产生 PBR 诊断。

### D5 — 导出（glTF）

`materials[]` 写 `pbrMetallicRoughness.metallicFactor/roughnessFactor/
metallicRoughnessTexture`、`normalTexture{index,scale:1}`、
`occlusionTexture{index,strength}`、`emissiveTexture`、`emissiveFactor`。
材质去重键扩展为 `(rgb, alpha, base_texture, pbr_tuple)`。`pbr: None` 时输出与当前
逐字节相同（不写任何 PBR 字段）。

### D6 — 渲染：独立 `mesh_pbr` 通道

- 顶点：position/normal/color/uv + **tangent**（`@location(4)`，`vec4`，w = handedness）。
  glTF `TANGENT` 不存储（域模型没有切线）；切线由渲染器在打包时按 UV 生成
  （MikkTSpace 风格简化为每三角形累加正交化；退化 UV 整批回退为无切线并记
  `render.pbr.tangent_unavailable`，法线贴图通道按无贴图处理——不伪造切线）。
- bind group 2：5 张纹理 + 1 个 sampler（基色、metallicRoughness、normal、emissive、
  occlusion）。**缺失的通道绑定数学恒等纹理**（白 1×1 / 平面法线 `(0.5,0.5,1)`），
  这是"关闭通道"的标准做法且是精确恒等，不是伪造数据；报告中仍按"源未声明"记录。
- 着色：Cook-Torrance GGX，单方向光（`camera.light.rgb`，与既有 diffuse 同一个方向约定），
  `F0 = mix(0.04, baseColor, metallic)`，`diffuse = baseColor * (1 - metallic)`，
  环境项 = `baseColor * ambient`，`occlusion` 只乘环境项与漫反射，`emissive` 直接相加。
  粗糙度来自 `roughness * metallicRoughness.g`，金属度来自 `metallic * metallicRoughness.b`。
- 颜色空间：与既有 shader 一致——贴图解码为 sRGB→线性采样，输出沿用现有
  `mesh.wgsl` 的显示约定（不引入 tonemap，避免改变既有资产的观感）。若是 sRGB 贴图，
  bind group 需要 `Rgba8UnormSrgb` 视图；实现必须与基色通道一致。
- 既有通道**不修改**：`mesh_pbr` 只在 `batch.pbr.is_some()` 时使用；无 PBR 的批次仍走
  `mesh`/`mesh_textured`。

### D7 — 宿主

`SceneBatch.pbr` 携带 4 个额外贴图键；`mesh_texture_keys` 扩展为收集全部 5 个键；
`load_image_cache_for_document` 已经按 `db.textures()` 全量解码，无需再改。
上传接口扩展为按批次的键集合上传（仍无占位图：缺失键按无贴图通道处理并报告）。

---

## 2. 不变量与契约测试

1. `pbr: None` 的网格在 `.ymodel`、glTF 导出、渲染三条路径上与 ADR-0038 的当前输出
   逐字节/逐像素一致。
2. `metallic`/`roughness`/`occlusion_strength`/`emissive` 越界或非有限 → `InvalidInput`
   （不钳制、不猜测）。
3. PBR 贴图键缺字节 → 校验拒绝（保存前），导入时 → 命名诊断 + 该槽不导入。
4. minor ≤ 3 的归档读入后 `pbr == None`；minor 4 被旧构建按 `unsupported_minor` 拒绝。
5. glTF 导入→导出→导入：五个通道的标量与纹理内容键回到原值（±浮点舍入）。
6. RVT：`metallicFactor`/`roughnessFactor` 被读入并记 `import.rvt.pbr_heuristic`；
   非有限值不进入 `MeshPbr`。
7. 着色：真实 GPU 测试必须证明 (a) 法线贴图改变像素、(b) `metallic=1` 与 `metallic=0`
   在同光照下结果不同、(c) `emissive` 在无光照方向下仍可见、(d) `occlusion` 使环境项变暗；
   任一不成立即视为未实现，不得只靠"shader 编译通过"。

---

## 3. 明确不做

| 项 | 原因 |
|---|---|
| IFC 的 PBR/样式颜色 | `IfcSurfaceStyleRendering` 只有颜色/透明度/高光粗糙度；metallic 不存在，映射会伪造 |
| SKP/OBJ 的 PBR | 源无数据；OBJ 的 MTL 字节不可得 |
| `KHR_materials_*` 扩展族（clearcoat/transmission/sheen/ior…） | 需要额外通道与 BRDF；本轮只做核心 metallic-roughness |
| 多 UV 集（`TEXCOORD_1+`）与 `KHR_texture_transform` | 域模型只有 UV0；`texCoord != 0` 的槽按名报告后不导入 |
| `alphaMode = MASK` 镂空 | 需要裁剪/alpha-test 语义；本轮 PBR 通道沿用既有 alpha blending，不新增 cutout |
| 环境贴图/IBL、tonemap、阴影、多光源 | 会改变既有资产观感，且需要新资源与预算模型；着色只用既有单方向光 `camera.light` |
| RVT 纹理 | 上游 0.25.0 仍不产出 `images`/`TEXCOORD_0`；要支持必须扩展 ADR-0035 补丁并自行解析 Revit 外观资源位图 |
| 非 glTF 导出器的 PBR（STL/OBJ/PLY/OFF/IFC4/SKP/DXF/DWG） | 这些写出器仍不写 PBR；也**没有**独立的 PBR 丢失码——基色纹理的损失已由 `export.<id>.texture_not_written` 覆盖，PBR 标量/贴图不在其消息内 |

---

## 4. 分期

| 阶段 | 内容 |
|---|---|
| P1 核心 | `MeshPbr` + `FieldKind` + payload + descriptor + `FORMAT_MINOR = 4`；`cad-db` 校验；所有 `Mesh` 字面量 |
| P2 glTF | 导入五个通道与标量；导出；材质去重键 |
| P3 RVT | 读 metallic/roughness + 启发式诊断 |
| P4 渲染 | 切线生成、5 纹理 bind group、GGX shader、批次分派、宿主上传 |
| P5 证据 | 单元/往返/真实 GPU 测试、E2E 样本与校验记录 |

---

## 5. 实施状态（2026-11-20）

**Accepted；P1–P4 已完成，P5 部分完成。**

- **P1 核心（完成）**：`cad_domain::MeshPbr` + `Mesh.pbr: Option<MeshPbr>`，
  `FieldKind::MeshPbr`，`.ymodel` 的 `cad:Mesh.pbr`，`FORMAT_MINOR = 4`（minor ≤ 3 可读，
  旧构建读 4 按 `ymodel.unsupported_minor` 拒绝）；`cad-db::validate` 校验有限性/范围、
  纹理键存在与 UV0 要求（`cad:Mesh.pbr:` 前缀的 `InvalidInput`）。
- **P2 glTF（完成）**：导入五个通道（仅文件确实声明的槽），逐槽 `texCoord != 0` 丢弃并记
  `import.gltf.uv_set_ignored`；`normalTexture.scale != 1` 记
  `import.gltf.normal_scale_not_imported`；`KHR_materials_emissive_strength` 记
  `import.gltf.emissive_strength_not_imported`；非法因子记 `import.gltf.pbr_ignored`；外部
  图像仍按名拒绝。导出写 `metallicFactor`/`roughnessFactor`/`metallicRoughnessTexture`/
  `normalTexture(scale 1)`/`occlusionTexture(strength)`/`emissiveTexture`/`emissiveFactor`；
  材质去重键含整个 PBR 元组；`pbr: None` 输出逐字节不变。
- **P3 RVT（完成）**：读 `metallicFactor`/`roughnessFactor`（上游 0.25.0 的名称启发式），
  每个文档一条 `import.rvt.pbr_heuristic` note；非法值不建 `MeshPbr` 并记
  `import.rvt.pbr_ignored`。RVT 仍不提供纹理/UV。
- **P4 渲染（完成）**：独立 `mesh_pbr` 通道（Cook-Torrance GGX、单方向光 `camera.light`、
  `F0 = mix(0.04, base, metallic)`、`diffuse = base * (1 - metallic)`、
  metallicRoughness 的 G/B 缩放 roughness/metallic、occlusion 只乘漫反射与环境项、emissive
  相加、再乘顶点色与批次 tint、沿用 ink-flip/sRGB 约定）。切线由 CPU 按 UV 生成；退化 UV
  关闭法线通道并记 `render.pbr.tangent_unavailable`（不发明切线）。缺失通道绑定精确恒等的
  1×1 纹理（白/黑/平面法线）；PBR 批次永不 flat-merge 或进入 pulled，原地覆盖被拒绝
  （`pbr_not_updatable`）。
- **P5 证据（部分完成）**：契约测试与真实 GPU 用例已跑（见
  `docs/validation-pbr-2026-11-20.md`）；合成 `pbr-channels.glb` 的端到端渲染记录 **待补**。
  IBL/环境贴图、阴影、tonemap、多光源、`KHR_materials_*`、`alphaMode = MASK`、UV1+
  /`KHR_texture_transform`、SKP/IFC/OBJ 的 PBR、RVT 纹理、非 glTF 导出器的 PBR 仍
  **NOT RUN / 未实现**（§3）。
