# ADR 0045：网格线框按表面边界绘制（三角剖分对角线不是边）

- 状态：**Accepted（2026-11-22）**。实施状态见 §5；渲染证据见 §5.3（同轮 `cad-cli-tools render`，
  真实 GPU `Quadro P620` + Vulkan，`native-below` 宿主本身的截图验收 **NOT RUN**）。
- 日期：2026-11-22。
- 相关：[ADR-0044](0044-procedural-ibl.md)（PBR 网格的边叠加抑制，本 ADR 修订其 §1 的显示类推导
  方式）、[ADR-0007](0007-entity-as-data-renderer.md)（实体即数据、显示为派生）、
  [ADR-0038](0038-mesh-base-color-texture.md)（UV0）、[ADR-0039](0039-mesh-pbr-channels.md)
  （`mesh_pbr` 通道）、`docs/3d-solidcore-and-mesh-import.md` §5（SKP 映射，P-SKP-3 仍未实现）。
- 影响：`cad-scene`（`cache.rs` 的边叠加来源、`RenderBatch::text_glyph` 与 `is_text_glyph`）、
  `cad-representation`（`DisplayPrimitive::Mesh` 的文档契约）、`cad-render-wgpu`（`GpuBatch::text_glyph`
  的推导来源不变，数值来源改为显式类）。
  **不改变**：`cad-db` 唯一权威、`.ymodel` schema 与 `FORMAT_MINOR`、载体所有权、patch 禁令、
  任何着色/光照公式、B-rep 实体的已存边集（`MeshWithStoredEdges` 仍然优先）。

---

## 0. 摘要

导入的 SketchUp 模型（`house.skp`）在**着色视图**里每个面上都有一条沿三角剖分对角线的亮线，
用户报为"三角面接缝很明显，不应该有这种效果"。实测（`cad-cli-tools render --view isometric`，
真实 GPU）确认：

- 亮线在**每个**三角形边界上，包括四边形被剖分出的对角线；面上没有颜色断层（同面三角形法向一致，
  observer 测得两侧 fill 差 1–3/255），所以不是 flat shading 接缝。
- 亮线与面的**边界线同色、同宽（1–2 px）**，是"未受光的多边形叠加线"而不是几何裂缝：
  模型内部没有背景色渗出（背景为纯黑，裂缝会发黑），亮线比两侧都亮。
- 三角剖分越密的地方（带窗洞的墙面、薄板）亮线越密，形成"梳状/波浪状"的假影线。

因此问题不在导入器、法向、深度或 MSAA，而在显示层的一个**过期规则**：

```rust
// 修改前（crates/cad-scene/src/cache.rs）
let include_edge_overlay = matches!(&fragment.primitive, DisplayPrimitive::Mesh(_)) && pbr.is_none();
let edge_positions = triangle_edge_positions(&kept, &verts); // 每个唯一三角形边一条线段
```

即：任何没有 PBR 的三角网格都会被**凭空合成**一套线框，来源是三角形拓扑本身。对三角形汤来说，
它的"边"里绝大多数是剖分内部的对角线，源文件里**并不存在**这些边——这是占位式假数据（铁律 #2）。

ADR-0044 已经为 PBR 网格做过同一判断的一半（"a triangle-edge overlay would paint over and hide
the shaded surface"），但只覆盖了 `pbr.is_some()` 的一类。本 ADR 把规则收口为一条几何判据，
并补上随之暴露的显示类推导漏洞。

---

## 1. 决策

### D1 — 叠加线 = 表面**边界**，判据是"三角形按**索引**使用次数为奇数"

一个边是表面边界边，当且仅当网格三角形使用它奇数次：

- 同一面上两个三角形共享的对角线被使用两次 → 抵消，不画；
- 面外轮廓、洞口轮廓、开曲面边缘被使用一次 → 画；
- 非流形边（三次）留在边界里，不静默丢弃。

比较按**顶点索引**，不做浮点位置相等判断（SKP 导入器为每个面复制顶点，两侧坐标只有 `f32`
级差异，按位置去重会漏）。这条判据与 `cad-representation::provider::display_modes::mesh_boundary_pairs`
（`FILL OFF` 已用）是同一规则，本 ADR 只是在场景层对批次的 `f32` 相对坐标再实现一次。

效果：

- SKP：每个面的外轮廓（SketchUp 里真实的边）仍然画，剖分对角线消失；
- 封闭网格（STL、焊接后的 glTF/OBJ）：没有边界边 → 纯着色，**不画线框**（源格式本来没有边）；
- 2D SOLID（`AcDbSolid`）：4 条外轮廓，对角线消失（与 `FILL OFF` 的行为一致）；
- 带洞/复杂面（`even_odd_fill` 的稠密剖分）：只剩洞与剖面轮廓，假影线消失。

`DisplayPrimitive::MeshWithStoredEdges`（B-rep 实体）不受影响：它继续用自己的已存边集，场景层
仅在"已存边为空"的防御分支里退回三角剖分线框。

### D2 — `text_glyph` 变成**显式**显示类，不再由"有没有边"反推

修改前：

```rust
pub fn is_text_glyph(&self) -> bool {
    self.topology == RenderTopology::Mesh && !self.indices.is_empty() && self.edges.is_empty() && self.pbr.is_none()
}
```

这个谓词依赖一个当时成立的巧合：只有文字字形（`MeshWithoutEdgeOverlay`）会得到"有索引、无边、
无 PBR"的批次；PBR 网格靠 `pbr.is_none()` 的守卫幸免。D1 之后，一个**封闭导入网格**自然就是这个
形状——它会被判成文字，进而在导航期被 `hide_text` 隐藏（模型在拖动中凭空消失）。

因此 `RenderBatch` 增加显式字段：

```rust
pub text_glyph: bool, // 只有 DisplayPrimitive::MeshWithoutEdgeOverlay（文字成形）为 true
```

`is_text_glyph()` 直接返回该字段。这正是 `cad-render-wgpu` 侧 `GpuBatch::text_glyph` 已经采用的
形状（它本来就是显式 bool），现在把事实来源下移一层，删掉"从边集反推"的推断。ADR-0044 的 PBR
守卫随之变成冗余（PBR 网格的 `text_glyph` 为 `false`），不再承担正确性。

### D3 — 不新造边数据、不伪造 SKP 边

不为导入器补"边界边"实体或 `LineSegments`：`.skp` 的真实边仍按
`import.skp.preview_geometry` / `P-SKP-3`（见 `docs/3d-solidcore-and-mesh-import.md` §5.4）
**未实现**处理，本 ADR 只保证**不把剖分对角线冒充成边**。显示层从三角形拓扑推出来的边界是
几何上可证明的表面边界（奇偶判据），不是新建的文档数据。

---

## 2. 不变量

1. `cad-db` 仍是唯一权威；本 ADR 只改**派生**的显示表示，不写数据库、不升 `FORMAT_MINOR`。
2. 不新增"只用一次"的显示通道：D1 复用既有边叠加管线（`RenderBatch::edges` → AA 线段），
   D2 复用既有 `text_glyph` 位。
3. 不允许静默：一个网格没有线框是可解释的（源没有边），不是丢弃——D2 保证它仍然是着色几何，
   在导航期照常绘制。
4. B-rep 实体的已存边永远优先于推导（`MeshWithStoredEdges` 的顺序不变）。
5. 奇偶判据不引入浮点比较；索引越界的段被跳过（输入已是畸形网格，不发明坐标）。

---

## 3. 代码落点

| 位置 | 变化 |
|---|---|
| `crates/cad-scene/src/wireframe.rs`（新模块） | 新增 `mesh_boundary_edge_positions`；`triangle_edge_positions`/`stored_edge_positions`/`dedup_pairs` 等叠加辅助从 `cache.rs` 迁入（`cache.rs` 已 976 行，触 1000 行上限门禁） |
| `crates/cad-scene/src/cache.rs` | `Mesh` 分支的叠加来源改为 `mesh_boundary_edge_positions`；`MeshWithoutEdgeOverlay` 拆成独立分支并置 `text_glyph: true`；其余分支 `false`；`MeshWithStoredEdges` 仍优先用已存边、仅空集时回退 `triangle_edge_positions` |
| `crates/cad-scene/src/batch.rs` | 新增 `RenderBatch::text_glyph`；`is_text_glyph()` 改读字段；`edges` 字段文档改为"表面边界" |
| `crates/cad-scene/src/highlight.rs` | 面高亮叠加批继承源 primitive 的 `text_glyph`（高亮文字必须与文字一同在导航期隐藏） |
| `crates/cad-representation/src/primitive.rs` | 文档：`Mesh` 画表面边界，`MeshWithoutEdgeOverlay` 连边界也不画（文字类） |
| `crates/cad-scene/src/tests/mesh.rs`·`budget.rs` | 契约测试：四边形 = 4 条边界（不是 5）；共享边全抵消 → 空边集且**不是**文字；字形仍是文字；已存边优先 |
| 全仓 `RenderBatch { .. }` 字面量 | 补 `text_glyph: false`（机械） |

`triangle_edge_positions` 保留，只作为 `MeshWithStoredEdges` 的防御回退（其文档写明这一点）。

---

## 4. 被否决的方案

- **给网格完全不画线框（等价于全部当成 `MeshWithoutEdgeOverlay`）**：会把 SKP 面轮廓一起删掉，
  让模型看起来是一堆没有分界的色块；而且要复用文字类，语义会串（`is_text_glyph` 把模型当文字）。
- **上游保留"全部唯一三角形边"，只对 SKP 关闭**：需要按 `type_key` 分支，把显示规则绑到单个
  导入器上，其余三角形汤（STL/OBJ/glTF/AcDbSolid）继续显示假对角线；违反铁律 #9（跨载体/跨格式
  的契约要一份）。
- **给 `edges` 补 MSAA/深度偏移让线不显眼**：只是把假数据画得更淡，问题仍在；且管线当前
  `sample_count = 1`，改 MSAA 是全局渲染决策，与本问题无关。
- **把剖分对角线改成"细到看不见"**：同上，属于掩盖。
- **在导入器里合成 `face_sources` 再用它挑边**：能表达"面内 vs 面间"，但要动 `cad-domain::Mesh`
  与 `.ymodel` 落盘（`face_sources` 目前为空且未序列化），收益与 D1 相同而成本高得多；留作
  将来 P-SKP-3 的输入。

---

## 5. 实施与证据

### 5.1 已完成

- D1、D2 的代码与契约测试（见 §3 表）。
- 文档：本 ADR、`crates/cad-scene/src/batch.rs` 与 `crates/cad-representation/src/primitive.rs`
  的字段/变体文档。

### 5.2 契约测试

`cargo test -p cad-scene --locked` → 87 passed（四边形边界 = 4 段、对角线证明缺席、全共享边 → 空边界且**不是**文字类、
`MeshWithoutEdgeOverlay` 仍是文字类、`MeshWithStoredEdges` 的已存边优先）；`cargo test -p cad-representation --locked`
→ 365 passed。

### 5.3 渲染证据（2026-11-22）

- 输入：`/home/zenglanmu/下载/house.skp`（用户报告的文件，2.2 MB）。
- 命令：`cad-cli-tools render <skp> --view isometric --width 1400 --height 1000 --png ...`
  （IBL 默认开）。
- 适配器：Vulkan / `Quadro P620` / NVIDIA 580.178.04（**真实 GPU，不是 lavapipe/SwiftShader**）。
- **A/B 方法**：同一构建内只把叠加来源在 `mesh_boundary_edge_positions` 与 `triangle_edge_positions`
  之间切换后重新渲染同一相机帧（避免拿旧二进制当基线——旧二进制早于 HEAD，帧统计
  `triangles` 9874 vs HEAD 10468，不可直接对比）。
- 结果：逐像素差分 16790 px 不同，全部落在三角边界上；裁剪放大后三角剖分对角线在 A 里有、
  在 B 里消失，面外轮廓（墙面白色描边）保留；`front`/`top` 视图几何完整、无线框异常。
- 边界：这是 `cad-cli-tools` 的无头帧，**不是** `native-below` 宿主窗口截图验收；宿主内的实际观感
  仍以用户复核为准。
- 附注：该校色墙面上可见的细密花纹是 `.skp` 自带的基色贴图（ADR-0038），A/B 两侧都存在，
  与本 ADR 无关。

### 5.4 NOT RUN

- `native-below` Linux 宿主（`yacr --open house.skp`）窗口级截图复核：**NOT RUN**。
- Windows/macOS/Android/Web 载体：**NOT VERIFIED**（本改动是载体无关的场景层规则，无平台代码）。
- 全量门禁 / 全量测试：**NOT RUN**（只在改动范围内验证）。
