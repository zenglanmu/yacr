# 实施追踪（全部产品功能仍待验收）

“契约已定义”不表示功能完成。测试栏为后续必须补充的测试，不是已执行证明。

| 编号 | 实施入口 | 后续契约/验收测试 |
|---|---|---|
| F01 | Importer、FileAccess、CancellationToken | 合法/损坏 DWG、取消、旧任务丢弃 |
| F02 | Camera/Viewport、CommandId | DPI、鼠标/触控、适应图纸、大坐标 |
| F03 | SessionState.layer_overrides、QueryService.layers | 显隐不改底图、不重解析、搜索/恢复 |
| F04 | Layout/PaperViewport、SpaceId | 视口裁剪/比例/不支持报告 |
| F05 | SelectionRef、SpatialIndex、properties | 不同 INSERT 独立选择、只读属性 |
| F06 | MeasurementEngine、MeasurementRecord | 单位未知、自交/共面、2D/3D/纸空间、代理精度 |
| F07 | AnnotationGeometry、AnnotationService | 六类批注、预览确认、手势冲突 |
| F08 | AnnotationTransaction、History、authorize | 创建/修改/删除、撤销重做、模式权限 |
| F09 | AnnotationFile、Persistence、AnnotationRow | 指纹映射、迁移、往返、导出失败不标保存 |
| F10 | ResourceResolver、ResourceLimits | 缺字体/SHX/外参、恶意路径与资源预算 |
| F11 | ImportReport、ProxyOutput、DiagnosticsModel | 缺缓存/未知状态不报完整、脱敏 |
| F12 | BackendPreference、Renderer、UnsavedDecision | Auto 初始化检测、强制失败、恢复/设备丢失 |
| F13 | Projection、StandardView、WorkPlane | 轨道、正交/透视、回到 2D、近平面 |
| F14 | Mesh、DisplayPrimitive、PickHit | 法向/绕序/透明/镜像、后端对照 |
| F15 | SolidTessellator、TessellationResult | ACIS 分项样本、缺面/边线降级、误差 |

## 横切要求

| 规范章节 | 契约/文档 | 后续验证 |
|---|---|---|
| §4、§18.1 | DB、ChangeSet、history、dependencies、query | 原子失败、乱序恢复、撤销全链一致 |
| §4.10、§8.3 | TaskStamp、CancellationToken、TaskExecutor | 文档切换、取消/背压、Worker 无共享内存路径 |
| §5–6 | render-backends.md、RenderHost | 同设备/纹理/sRGB/alpha/MSAA/呈现顺序 |
| §7 | proxy-support.md、ImportLimits/DecodeLimits | 有界解码、深层块、来源与资源权限 |
| §8、§11 | performance.md、BenchmarkContext | 真实设备同样本/质量的分阶段基准 |
| §9 | build.md、platform traits | SAF、IndexedDB/导出、生命周期/退出保护 |
| §10、§17 | migration-map.md、THIRD_PARTY_NOTICES.md | 上游 commit/许可、最小输入输出测试 |
| §16 | TolerancePolicy、AnnotationFile、anchors | 数值稳健性、未知字段迁移、稳定来源 |
| §19 | CLI、diagnostics | 与 UI 同命令路径、非零错误、脱敏 |

当前实际测试与运行证据见 `docs/validation.md`；尚不存在黄金图、真实兼容样本或性能验收。
