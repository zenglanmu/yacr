# 网格 PBR 通道验证记录（2026-11-20）

范围：[ADR-0039](adr/0039-mesh-pbr-channels.md) P1–P4 的 metallic-roughness + normal/emissive/
occlusion 通道：`cad-domain::MeshPbr`、`cad-db` 校验、`.ymodel` `cad:Mesh.pbr` 与
`FORMAT_MINOR = 4`、glTF/RVT 导入、glTF 导出、`cad-scene` 批次、`cad-render-wgpu` 的
`mesh_pbr` 通道与宿主纹理上传。另附本轮 `rvt` 0.24.0 → 0.25.0 升级证据（§6）。

环境：Linux x86_64，Rust 1.99.1（`rust-toolchain.toml`），`--locked`；真实 GPU 为
NVIDIA Quadro P620 / Vulkan（非软件渲染）。

**记录方式**：§2 的计数由实现 lane 在其工作树上执行后记录；本文档 lane 只做整理，
**没有重跑任何 cargo 命令**（构建门禁由另一进程持有）。因此计数是实现 lane 的报告，不是
文档 lane 的复现；端到端渲染状态按仓库内实际产物记录（§5），未执行项在 §5/§8 逐条标注
**NOT RUN**。

## 1. 实现摘要

- **域**：`cad_domain::MeshPbr { metallic, roughness, emissive, occlusion_strength,
  metallic_roughness_texture, normal_texture, emissive_texture, occlusion_texture }`，挂在
  `Mesh.pbr: Option<MeshPbr>`。**`None` = 源没有声明 PBR**（不是“默认材质”）：仅
  `baseColorFactor`/`baseColorTexture` 的 glTF 材质保持 `None`，因为 `metallicFactor = 1`
  是规范默认值，套到无光照 CAD 显示会伪造黑色金属。`emissive` 是**线性** RGB。
- **文档校验**：标量有限且 ∈ `[0,1]`；纹理键必须是合法文档键且字节存在于 `cad-db`；
  任一 PBR 贴图要求网格有非空 `uvs`。违反者是 `cad:Mesh.pbr:` 前缀的 `InvalidInput`
  （不钳制、不猜测）。
- **`.ymodel`**：`cad:Mesh.pbr` 是可选嵌套对象（`null`/缺失 → `None`），新增
  `FieldKind::MeshPbr`；`FORMAT_MINOR` **4**。minor ≤ 3 的归档仍可读（缺 `pbr` → `None`）；
  旧构建读 minor 4 按 `ymodel.unsupported_minor` 拒绝。纹理复用 `textures/<key>`。
- **导入**：
  - glTF 读五个通道（metallicRoughness/normal/emissive/occlusion 贴图 + metallic/
    roughness/emissive 标量），**只导入原始 JSON 确实声明的槽**。逐槽 `texCoord != 0` →
    该槽不导入 + `import.gltf.uv_set_ignored`；`normalTexture.scale != 1` →
    `import.gltf.normal_scale_not_imported`；`KHR_materials_emissive_strength` →
    `import.gltf.emissive_strength_not_imported`；非法因子 → `import.gltf.pbr_ignored`；
    外部图像仍按名拒绝（不联网）。
  - RVT 读 `metallicFactor`/`roughnessFactor`（上游 0.25.0 按材质**名**启发式生成），
    每文档一条 `import.rvt.pbr_heuristic` note；非有限/越界值不建 `MeshPbr` 并记
    `import.rvt.pbr_ignored`（`Partial`）。RVT 仍无纹理/UV。
  - IFC/SKP/OBJ 不产生 PBR（`pbr: None`）；IFC 样式颜色仍是
    `import.ifc.style_not_imported`，SKP/OBJ 无 PBR 数据也不产生 PBR 诊断。
- **导出（glTF）**：写 `metallicFactor`/`roughnessFactor`/`metallicRoughnessTexture`/
  `normalTexture(scale 1)`/`occlusionTexture(strength)`/`emissiveTexture`/
  `emissiveFactor`；材质去重包含整个 PBR 元组；`pbr: None` 的输出与改动前**逐字节相同**。
  其它写出器仍**不写** PBR（也没有独立的 PBR 丢失码；基色纹理损失由既有
  `export.<id>.texture_not_written` 覆盖，PBR 标量/贴图不在其消息内）。
- **渲染**：独立 `mesh_pbr` 通道（Cook-Torrance GGX，单方向光 `camera.light`，
  `F0 = mix(0.04, base, metallic)`，`diffuse = base * (1 - metallic)`，metallicRoughness 的
  G/B 缩放 roughness/metallic，occlusion 只乘漫反射与环境项，emissive 相加，再乘顶点色与
  批次 tint，沿用 ink-flip/sRGB 约定）。切线由 CPU 按 UV 生成；退化 UV 关闭法线通道并记
  `render.pbr.tangent_unavailable`（**不**发明切线）。缺失通道绑定精确恒等纹理
  （白 / 黑 / 平面法线）＝“通道关闭”，不是伪造数据。PBR 批次**永不** flat-merge 或进入
  pulled；原地覆盖拒绝 `pbr_not_updatable`。
- **宿主**：`SceneBatch.pbr` 携带 4 个额外贴图键；`mesh_texture_keys` 收集全部 5 个键，
  `load_image_cache_for_document` 按 `db.textures()` 全量解码；无占位图，缺键按无贴图
  通道处理并报告。Web 载体仍按名拒绝文档纹理。

## 2. 已运行命令与结果（测试矩阵）

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-domain --locked` | **exit 0**（212 passed） |
| `cargo test -p cad-db --locked` | **exit 0**（348 passed） |
| `cargo test -p cad-store-zip --locked` | **exit 0**（33 passed） |
| `cargo test -p cad-import-gltf --locked` | **exit 0**（35 passed） |
| `cargo test -p cad-export-gltf --locked` | **exit 0**（36 passed） |
| `cargo test -p cad-import-rvt --locked` | **exit 0**（41 passed） |
| `cargo test -p cad-representation --locked` | **exit 0**（365 passed） |
| `cargo test -p cad-scene --locked` | **exit 0**（82 passed） |
| `cargo test -p cad-render-wgpu --locked` | **exit 0**（342 passed，含 `mesh_pbr_gpu_tests` 与切线用例） |
| `cargo test -p cad-render-wgpu --test wgsl_validation --locked` | **exit 0**（14 passed，含 `mesh_pbr_shader_is_valid_wgsl`） |
| `cargo test -p cad-engine --locked` | **exit 0**（128 passed） |

**说明**：计数来自实现 lane 报告，文档 lane 未复现。`YACR_TEST_*` 未设置而跳过的用例
**不是**已运行证据；本轮 PBR 用例都在合成夹具上。

## 3. 真实 GPU 证据

环境：Linux + NVIDIA Quadro P620 / Vulkan（真实 GPU，非 SwiftShader/lavapipe），
`cargo test -p cad-render-wgpu --lib --locked -- --test-threads=1 mesh_pbr_gpu_tests`。

| 用例 | 观测 |
|---|---|
| `a_normal_map_changes_pixels_versus_the_flat_identity_normal` | 有法线贴图与恒等平面法线的帧像素不同 |
| `metallic_one_differs_from_metallic_zero_under_the_same_light` | 同一光照下 `metallic=1` ≠ `metallic=0` |
| `emissive_is_visible_with_the_light_facing_away` | 光源背向时自发光仍可见 |
| `an_occlusion_map_darkens_the_surface` | AO 贴图使环境项变暗 |
| `the_base_colour_map_tints_the_pbr_response` | 基色贴图确实参与 PBR 结果 |
| `the_metallic_roughness_map_modulates_the_scalar_factors` | MR 贴图 G/B 调制 roughness/metallic |
| `degenerate_uvs_disable_the_normal_map_instead_of_inventing_a_tangent` | 退化 UV 不生成切线，计数器记 `render.pbr.tangent_unavailable` |
| 拾取 | 实现 lane 报告 PBR 批次仍可拾取；本文件未保存该断言对应的 GPU 用例名 |

边界：这些是**定点用例**，不是像素比对、golden、视觉验收或性能结论；未测多光源、IBL、
阴影、tonemap、跨 GPU 驱动或其它平台。

## 4. `.ymodel` 往返证据

- `cad-domain` 的 payload 测试覆盖 `uvs`/`texture`/`pbr` 的编码→解码往返，以及手写的
  minor 3 mesh payload（无 `pbr` 键）解码为 `pbr: None`、再编码写 `"pbr": null`。
- `cad-store-zip` 的 schema 测试断言 `cad:Mesh` 声明 `pbr` 字段且
  `FieldKind::MeshPbr` 的拼写为 `mesh_pbr`；`FORMAT_MINOR = 4`，读取时 minor > 4 按
  `ymodel.unsupported_minor` 拒绝。
- **NOT RUN**：本文件**没有**宿主级“保存→重开带 PBR 的 `.ymodel` 文件”记录（`cad-host`
  的纹理往返用例只覆盖基色纹理）；旧二进制读取 minor 4 的拒绝由 mark 断言模拟，没有真的
  运行旧构建。

`.ymodel` 仍**只在 Linux** 接线；其它载体打开/另存 `.ymodel` 按 `unsupported-runtime:
ymodel` 拒绝。

## 5. 端到端运行、NOT RUN / 未实现（不得据此宣称完成）

- **合成样本已生成**：`scripts/e2e-textures/make_pbr_gltf.py` 产出
  `pbr-channels.glb`（1 m 四边形，UV0；64×64 内嵌贴图：四象限基色、roughness/metallic 分半、
  左平右斜法线、中心绿色自发光、底边暗带 AO）。脚本是确定性字节，样本**未入库**。
- **端到端渲染（2026-11-20 已执行）**：`cad-cli-tools render /tmp/…/pbr-channels.glb`
  走真实 Vulkan 适配器（NVIDIA Quadro P620，`device_type=discrete_gpu`）：
  `mesh_textures.requested` 5 张全部 `uploaded`、`unresolved`/`fallbacks` 空、
  `completeness = complete`、`pixels.distinct_colors = 322`。帧内可见：四象限基色、
  中心绿色自发光方块、底边 AO 暗带；**右半（metal=1、roughness≈0.08）呈近黑色**——
  单方向光且**没有 IBL/环境贴图**时金属只剩镜面项，这是 ADR-0039 §3 已声明的边界，
  不是通道丢失。法线贴图的分半倾斜在该分辨率下不能单独用肉眼判定，其像素级证据来自
  `cad-render-wgpu` 的真实 GPU 契约测试（法线贴图 vs 平面法线像素不同）。
- **样本性质**：这是合成文件，不是厂商/真实资产；即使渲染落盘，也只证明该固定场景的
  通道可见性，不是 golden、像素或性能验收。
- **NOT RUN / 未实现**：
  - IBL/环境贴图、阴影、tonemap、多光源；
  - `KHR_materials_*` 扩展族、`alphaMode = MASK` 镂空、UV1+/`KHR_texture_transform`；
  - SKP/IFC/OBJ 的 PBR、RVT 纹理；
  - 非 glTF 导出器（STL/OBJ/PLY/OFF/IFC4/SKP/DXF/DWG）的 PBR——它们仍不写 PBR，也没有
    独立的 PBR 丢失码；
  - three.js / Blender / glTF-Validator 等厂商查看器接受性；
  - Windows/macOS/Android/Web 的运行（wasm 只到编译；Web 对文档纹理按名拒绝）；
  - 像素/golden/性能结论；软件渲染未用于任何像素或性能结论。

## 6. `rvt` 0.24.0 → 0.25.0 升级证据

本轮把 `rvt` 从 0.24.0 升到 **0.25.0**（2026-11-15 的 crates.io 版本），`vendor/rvt` 是同一份
固定路径源码副本，**同一个** alpha 补丁按原逻辑重挂到 `src/ifc/gltf.rs`（`most_transparent_alpha`
+ `element_alpha` 的 constituent/profile/layer-set 构造 + `category_alpha_materials` 回退 +
`element_alpha_tests`）。台账见 `docs/rvt-patches.md`，决策见
[ADR-0035](adr/0035-rvt-vendored-patch-policy.md)。

- `bash scripts/test-rvt-patch.sh`：**7 passed**（上游 3 + 补丁 4）。
- `vendor/rvt` 自带测试套件：**331 passed**。
- `cargo test -p cad-import-rvt --locked`：**41 passed**（PBR 变更后的工作树；0.24.0 轮次为
  33 passed）。
- `python3 scripts/check-architecture.py`：**OK**（`rvt` patch 0.25.0；`vendor/rvt` 210 个
  vendored 文件；`Cargo.lock` 的 `rvt` 条目无 `source`，即解析自路径）。
- `Cargo.toml`：`rvt = "=0.25.0"`、`[patch.crates-io] rvt = { path = "vendor/rvt" }`；
  `vendor/rvt/SHA256SUMS` 已重生成（210 个文件，除 `src/ifc/gltf.rs` 外逐文件摘要）。
- 上游 0.24 → 0.25：metallic/roughness 名称启发式抽到 `src/ifc/pbr.rs`；构件体
  placement/extrusion 数学改动（预览几何变化，glTF 交接结构不变）；本补丁触及的材质选择链
  不变。**0.25.0 仍然没有** `images`/`textures`/`TEXCOORD_0`，`appearance_asset_id` 依旧只
  解码不解析位图——升级**不**带来 RVT 纹理。
- **NOT RUN**：真实 Revit 样本（`YACR_TEST_RVT`）在 0.25.0 上的重基线、逐像素/GPU 证据、
  厂商接受性。0.24.0 上的真实样本数字仍是历史记录（`docs/rvt-patches.md`）。

## 7. 依赖与许可

PBR 通道**没有新增依赖**：glTF 读写复用既有 `gltf 1.4.1`，纹理字节复用既有 `png 0.18.1`
与 `sha2 0.11.1`（纹理内容寻址）。`rvt` 只做版本号升级（0.25.0，仍是 Apache-2.0，仍是同一份
固定路径补丁），见 `THIRD_PARTY_NOTICES.md`。

## 8. 跟进项

1. 落盘 `pbr-channels.glb` 的真实 GPU 渲染结果，并记录 `mesh_pbr` 上传/回退统计。
2. 用真实第三方资产（Khronos glTF-Sample-Assets 等）复跑 PBR 导入/导出与渲染。
3. 厂商查看器接受性（three.js/Blender/glTF-Validator）。
4. 宿主级 `.ymodel` 保存→重开 PBR 文件证据（当前只有 payload/schema 契约）。
5. 逐项评估 ADR-0039 §3 的未实现项（IBL、阴影、MASK、UV1+、RVT/非 glTF PBR）。
