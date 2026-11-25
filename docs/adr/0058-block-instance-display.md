# ADR-0058: block-reference display instancing (proposed)

- 状态：Proposed（2026-11-25）；**尚未实现**
- 关联：ADR-0007（实体即数据渲染）、ADR-0009（AA 线）、ADR-0057（CPU payload release）

## 背景

`fixtures/complex-test.dwg` 的 `build-representation` 报告 8,271,435 个 line primitive / 23.47M
顶点，但 `instances: 0` 是一个**死计数器**：CLI 调 `build_expanded`（
`cad-representation/src/provider/registry.rs`），而展开路径遇到 `DisplayPrimitive::Instance`
就递归展开，永远不会把该 variant 放进输出。INSERT 的真实成本因此完全体现在展开后的独特几何里。

现状：

- `SemanticGeometry::Insert` 在 `provider/default.rs` 产生一个 `DisplayPrimitive::Instance`；
- `provider/expand.rs` 组合 `outer × insert`、解析 INSERT 的有效颜色/线宽/线型/透明度、
  layer-0 链、XCLIP、ATTRIB 文本、嵌套/循环/深度上限，然后把成员几何展开成世界坐标；
- `cad-scene` 为每个展开 fragment 建 batch，`local_origin` 归约后上传；没有 block/template 概念；
- CPU 拾取始终解析到**成员**（`SelectionRef { entity, instance path }`），GPU pick key 是 per
  batch；编辑路径拒绝非空 instance path；
- plot/SVG/PDF 明确报告未展开的 Instance 为不支持；导出走 `cad-db`，不读显示 primitive。

## 决策（分两阶段）

### Phase 1（优先，无 shader/像素风险）

- `RenderBatch` 增加 block template 引用 + 每个 placement 的变换；resident CPU 场景只保留一份
  block-local 模板几何 + 每个引用的小变换。
- `ResidentScene::materialized` 在 staging 边界把模板 + 变换组合成与今天**逐字节相同**的世界
  坐标 batch；上传/renderer 不需要新 shader。
- 与 ADR-0057 的 payload release 兼容：release 后模板仍可按需重建；模板本身也受 release 管理。
- 收益：降低**构建/上传峰值**（重复块不再按 placement 复制顶点）和 append/rebuild 成本；
  GPU 常驻不变。

### Phase 2（后续，能力门控）

- 每 (block, style class) 一份共享 GPU 模板 buffer；per-instance 3×4 矩阵（translation 必须
  相对 chunk origin，f64 组合，f32 局部）与 per-batch pick key。
- line/mesh 两个 pipeline 变体；mirrored 使用现有 mirrored pipeline；透明/图像/文本先保持展开。
- 收益：8.27M 段的 56 B/段常驻可降到 `template + 48 B/placement`（量级取决于可实例化比例）。
- Phase 2 需要真实 GPU 逐像素等价证据；WebGL2/无 VS storage 的载体保持 Phase 1 或展开。

## 必须回退为展开的情况（带稳定诊断/计数，绝不静默）

- XCLIP（平面/depth band）与继承 clip；
- 世界空间生成的虚线 pattern（非 1 缩放会改变 dash 长度）；
- 透明 batch（排序按 centroid）；
- 非均匀/奇异缩放、镜像（Phase 1 回退；Phase 2 用 mirrored pipeline）；
- 嵌套超过 `MAX_INSTANCE_DEPTH`、循环、缺失 block；
- 每引用 ATTRIB 文本（随引用变化）；
- 单实体编辑/选择：instanced 行不进入 `entity_batches`，`update_candidate` 返回 `None`，
  走一次 full rebuild（显式、可接受）；成员 CPU 拾取保持现状。

## 测量（2026-11-25）

`cad-cli-tools build-representation fixtures/complex-test.dwg` 的新 `instancing` 块（只读 DB
walk；不改变既有 primitive/vertex 计数）：

- `model_insert_placements=1274`，`unique_blocks=128`，`direct_block_member_entities=19721`
- `attribute_carrying_placements=276`，`non_uniform_scale_placements=14`，
  `mirrored_placements=325`，`without_basic_fallbacks=672`
- 展开后仍是 `lines=8,271,435` / `vertices=23,470,089`，`kind_counts.instances=0`

这说明模板层有 128 个 unique block 可被 1,274 个 placement 共享；672 个 placement 不触发
ATTRIB/非均匀/镜像这三类基础回退。它**不是**可实例化字节数的证明：XCLIP、世界空间虚线、
透明、嵌套深度和每引用样式仍要在 Phase 1 的 eligibility 里过滤。

## 待办（实现前）

1. 一个 **GPU-free `instancing-report`**（或 `build-representation` 的独立字段，不改现有计数）：
   placements、unique templates、expanded vs template 字节、fallback 原因分布。先在
   `complex-test.dwg` 上量化可实例化比例。
2. 若比例足够：实现 Phase 1 的模板构建/共享 + `materialized` 组合，并用 synthetic fixture
   断言输出 batch 与展开基线逐字节一致；再在真实 GPU 上做 PNG diff。
3. Phase 2 单独 ADR/实现，按载体能力门控。

## POC 结果（2026-11-25，`YACR_BLOCK_INSTANCING=1`，默认关闭）

- 同一 `complex-test.dwg`：`placements=155` / `batches=776`（总 164,762 batch 的 **0.47%**）。
- 回退分布：`not_translation=701`、`attributes=276`、`representation_diagnostics=75`、
  `raster_or_mask=49`、`clip=18`。
- 峰值 RSS 2469 MiB（off 2448 MiB），稳态 1539 MiB（off 1530 MiB）：**没有可测的收益**。
- 同一相机页的 `actual-render` verts/tris/draw 与展开基线一致（883008 / 308960 / ~2707），
  组合路径没有引入可见错误；无 present 错误。
- 结论：**纯平移窄范围不足以覆盖这份图纸**；真正的剩余量集中在旋转/缩放（701）和
  ATTRIB（276）。前者需要 per-placement 矩阵组合（Phase 1 的真正主体），后者需要每引用文本，
  两者都回到高风险区。
- **POC 代码已删除**（`b5ee0b85`）；默认路径仍是展开。本文只保留负结果和 Phase 1/2 的设计，
  不要在没有旋转/缩放与像素 parity 的情况下重复实现同一窄范围。

## 证据边界

目前只有 recon 与 `build-representation` 的计数，没有实现、没有 PNG/像素/真机证据。
`instances: 0` 不能作为“没有块引用”的证据。Windows/macOS/Android/Web 未验证。
