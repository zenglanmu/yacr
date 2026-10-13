# 架构与公共契约

需求权威：`CAD_IMPLEMENTATION_SPEC.md` v2.0。

数据流：UI/CLI → Application/权限 → Command/DrawingTransaction → DB transaction →
ChangeSet → 查询/历史/依赖失效 → Representation → SceneDelta → Renderer → RenderHost。

已接线：导入→数据库、显示表示→场景→wgpu、绘图事务→撤销/重做、依赖失效、查询分页、
Slint↔wgpu 合成（`cad-ui-slint/src/bridge.rs`）。仍返回 NotImplemented 的业务路径：
文件选择/导入导出（宿主）、后端切换、3D 渲染、ACIS 离散。详见 `docs/validation.md`。

> 2026-10-05 变更：用户批注子系统整体移除（提交 `0120d09`），`cad-annotations` crate
> 不再存在，F07/F08/F09 退出产品范围；本文档只描述当前树中真实存在的 crate 与契约。

| 层 | 包与关键契约 |
|---|---|
| 领域 | cad-domain：强类型 ID、f64 几何、单位、容差、来源/完整性、TaskStamp |
| 权威数据库 | cad-db：DrawingDatabase/Builder、DrawingTransaction、ChangeSet |
| 数据一致性 | cad-dependencies：DependencyIndex/Invalidation；cad-history：UndoRecord/RecoveryJournal |
| 几何 | cad-geometry：GeometryEngine；cad-kernel-adapter：SolidTessellator/SAT/SAB 隔离 |
| 导入资源 | cad-import-acadrust：Importer/ImportReport；cad-proxy：ProxyRecordDecoder/ProxyPlayer；cad-resources：ResourceResolver |
| 显示 | cad-representation：ProviderRegistry/DisplayRepresentation；cad-spatial：SpatialIndex/PickHit |
| 场景/GPU | cad-scene：CacheKey/SceneDelta/SceneCache；cad-render-wgpu：Renderer/RenderTarget |
| 渲染会话 | cad-engine：UI 无关的 RenderSessionHandle、会话状态与 typed render/session protocol；直接依赖 cad-app/cad-db/cad-domain/cad-render-wgpu/cad-representation |
| 工作能力 | cad-measure：MeasurementEngine/SnapCandidate |
| 应用 | cad-app：Document/SessionState/Viewport/Workspace、CommandHandler/Tool |
| 查询/UI | cad-query：分页查询/Mixed/Unset；cad-ui-slint：UiAdapter/UiCommandSink/Slint shell |
| 宿主编排 | cad-host：HostCore（命令路由、打开/保存/出图管线、状态推送、导航/绘制 sink）；只依赖 cad-ui-slint，不直接依赖 slint |
| 宿主 | cad-platform：PlatformFiles/ConfigStore/SurfaceSource/FontLoader/Persistence/TaskExecutor/Clipboard/HostLifecycle/RenderHost；apps：组合根 |
| 自动化 | cad-diagnostics：统计/脱敏报告；cad-cli-tools：共用 Application 命令路径 |

## 必须保持的边界

- 底图只能由 Builder 导入；领域修改只能通过事务正式提交。会话导航不使文档变脏。
- Document 共享不可变底图；Viewport 保存独立相机，不复制数据库。
- GPU/内核/Slint/平台类型不得进入 domain/db；第三方类型不随意 re-export。
- `cad-engine` 是 UI 无关的渲染会话协调层：直接依赖 `cad-app`、`cad-db`、`cad-domain`、
  `cad-render-wgpu` 与 `cad-representation`。`cad-app` 提供应用会话/工具契约，`cad-db` 与
  `cad-domain` 提供权威数据和领域类型，`cad-representation` 提供显示表示，
  `cad-render-wgpu` 提供 renderer；这些都是 UI 无关契约，并非 UI/宿主实现依赖。不依赖
  `cad-ui-slint`、`cad-host`、Slint 或直接依赖 wgpu；wgpu 类型仅由 renderer 通过其 API/re-export
  提供。`cad-app` 不依赖 `cad-engine`，`cad-ui-slint` 不依赖 `cad-host`。这些方向由
  `scripts/check-architecture.py` 按 Cargo manifest 显式校验。
- 渲染协调分阶段实施中（ADR 0005）：P1 会话边界已通过审查；当前 host 仍依赖
  `cad-ui-slint`，Slint 仍作为单 surface 协调者与 presenter；不得将此过渡状态描述为 host
  已完成去 UI 化或已完成跨平台性能验收。整体项目仍实施中：P2 才将工具状态归入 SessionState
  并移除 host 对具体 UI 的依赖；P3 再落地有界语义队列与 event-driven dirty 唤醒。增量算法本轮不扩张。
- `cad-host` 不得直接依赖 `slint`/`wgpu`/`web-sys`/Android 类型；它只通过 `cad-ui-slint`
  的固有 API 与 sink trait 推送状态，平台 I/O 一律经 `cad-platform` 的
  `PlatformFiles`/`ConfigStore`/`SurfaceSource` 契约（事件式、无 futures、无 `Send`），
  宿主特有的路径/对话框/表面度量只存在于 `app-*`。
- ChangeSet revision 不连续时重建快照；元数据不默认引发曲线重新离散。
- 测量结果锚点由原始 handle + instance path + subelement + 坐标回退构成；GPU 索引不是身份。
- TaskStamp 不匹配拒绝发布；取消不可中断库调用时至少阻止结果进入当前会话。
- DB revision、渲染设备 generation、任务 generation 分开处理。
- 注册冲突须明确排序/拒绝；当前注册表并未实现扩展分发。

`scripts/check-architecture.py` 校验当前依赖图、循环和核心隔离。
Web 宿主与 Slint 渲染桥的文件职责划分见 `docs/code-structure.md`。
traits 是契约草案（0.1），实现前可通过 ADR 修正不足，不代表已稳定公共 API。
