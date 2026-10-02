# 架构与公共契约

需求权威：`CAD_IMPLEMENTATION_SPEC.md` v2.0。

数据流：UI/CLI → Application/权限 → AnnotationService → DB transaction →
ChangeSet → 查询/历史/依赖失效 → Representation → SceneDelta → Renderer → RenderHost。

已接线：导入→数据库、显示表示→场景→wgpu、批注事务→撤销/重做、依赖失效、查询分页、
Slint↔wgpu 合成（`cad-ui-slint/src/bridge.rs`）。仍返回 NotImplemented 的业务路径：
文件选择/导入导出（宿主）、后端切换、3D 渲染、ACIS 离散。详见 `docs/validation.md`。

| 层 | 包与关键契约 |
|---|---|
| 领域 | cad-domain：强类型 ID、f64 几何、单位、容差、来源/完整性、TaskStamp |
| 权威数据库 | cad-db：DrawingDatabase/Builder、AnnotationDatabase/Transaction、ChangeSet |
| 数据一致性 | cad-dependencies：DependencyIndex/Invalidation；cad-history：UndoRecord/RecoveryJournal |
| 几何 | cad-geometry：GeometryEngine；cad-kernel-adapter：SolidTessellator/SAT/SAB 隔离 |
| 导入资源 | cad-import-acadrust：Importer/ImportReport；cad-proxy：ProxyRecordDecoder/ProxyPlayer；cad-resources：ResourceResolver |
| 显示 | cad-representation：ProviderRegistry/DisplayRepresentation；cad-spatial：SpatialIndex/PickHit |
| 场景/GPU | cad-scene：CacheKey/SceneDelta/SceneCache；cad-render-wgpu：Renderer/RenderTarget |
| 工作能力 | cad-measure：MeasurementEngine/SnapCandidate；cad-annotations：AnnotationService/AnnotationFile |
| 应用 | cad-app：Document/SessionState/Viewport/Workspace、CommandHandler/Tool |
| 查询/UI | cad-query：分页查询/Mixed/Unset；cad-ui-slint：UiAdapter/UiCommandSink/Slint shell |
| 宿主 | cad-platform：FileAccess/Persistence/TaskExecutor/Clipboard/HostLifecycle/RenderHost；apps：组合根 |
| 自动化 | cad-diagnostics：统计/脱敏报告；cad-cli-tools：共用 Application 命令路径 |

## 必须保持的边界

- 底图只能由 Builder 导入；批注只能通过事务正式修改。会话导航不使文档变脏。
- Document 共享不可变底图；Viewport 保存独立相机，不复制数据库。
- GPU/内核/Slint/平台类型不得进入 domain/db；第三方类型不随意 re-export。
- ChangeSet revision 不连续时重建快照；元数据不默认引发曲线重新离散。
- 原始 handle + instance path + subelement + 坐标回退构成锚点；GPU 索引不是身份。
- TaskStamp 不匹配拒绝发布；取消不可中断库调用时至少阻止结果进入当前会话。
- DB revision、渲染设备 generation、任务 generation 分开处理。
- 注册冲突须明确排序/拒绝；当前注册表并未实现扩展分发。

`scripts/check-architecture.py` 校验当前依赖图、循环和核心隔离。
Web 宿主与 Slint 渲染桥的文件职责划分见 `docs/code-structure.md`。
traits 是契约草案（0.1），实现前可通过 ADR 修正不足，不代表已稳定公共 API。
