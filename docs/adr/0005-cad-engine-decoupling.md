# ADR 0005：cad-engine 渲染会话与宿主解耦

状态：P1 审查通过；整体分阶段实施中。范围：渲染会话归属、UI/渲染协议边界及宿主解耦；本 ADR 不是跨平台运行或性能验收结论。

## 背景与现状

当前 `cad-render-wgpu` 封装 wgpu 渲染器，但渲染会话协调仍分散在渲染桥和 UI/宿主周边。
`cad-host` 当前依赖 `cad-ui-slint` 并实现其 sink；`cad-ui-slint` 当前直接使用渲染相关 API。
因此本次抽取须允许可编译过渡：P1 建立 `cad-engine` 会话边界，不把尚未完成的 host/UI
拆分写成既成事实。依赖方向及门禁由 Cargo manifest 的架构检查明确锁定。

当前 `cad-engine` manifest 的直接依赖为 `cad-app`、`cad-db`、`cad-domain`、
`cad-render-wgpu`、`cad-representation`：分别提供应用会话/工具契约、权威数据库、领域类型、
renderer 与显示表示，均为 UI 无关的核心契约而非 UI/宿主实现。依赖边为
`cad-engine → cad-app/cad-db/cad-domain/cad-render-wgpu/cad-representation`；`cad-app` 不依赖 engine；
engine 不依赖 `cad-ui-slint`、`cad-host`、Slint 或
直接依赖 wgpu；UI 不依赖 host。acadrust 仍仅由
`cad-import-acadrust` 依赖，wgpu 仍仅由 `cad-render-wgpu` 依赖，不扩宽现有唯一依赖边界。

## 决策

### P1：会话与 renderer 边界

- 新增 UI 无关的 `cad-engine`。由 owner-thread 持有 `RenderSessionHandle`；handle 管理
  CAD 快照、准备流程、dirty runtime、资源与 overlays。所有权不承诺 `Send`，UI 线程之外的
  调度由适配层负责。
- 对外提供 typed render/session API 与 typed protocol，例如 `RenderWake`，不以 UI 消息或
  未约定的跨语言序列化作为稳定接口。渲染帧绑定与纹理通过 API 交给呈现适配层。
- Device/Queue 等 GPU 类型由 `cad-render-wgpu` 的 renderer API/re-export 暴露；engine
  只消费 renderer 边界，不直接依赖 wgpu。GPU 提交和资源操作继续封装在 renderer 内。
- Slint 继续作为单 surface 协调者和 presenter：拥有 surface/事件循环，提供共享 GPU
  Device/Queue，并呈现 engine/renderer 提供的 FrameBinding 与 Texture。CAD 不建立第二套
  surface 或事件循环，不争抢 present，也不做整幅 GPU→CPU 回读。
- P1 期间 `cad-host` 暂保留对 `cad-ui-slint` 的依赖，以维持现有可编译过渡；不得添加
  host 无 UI 依赖断言，亦不得声称 host 已完成去 UI 化。

### P2：SessionState 与 host/UI 分离

- 工具状态统一归入 `SessionState`，在现有 `ToolState` 扩展 `Drawing(DrawCapture)`，捕获
  `DrawTool` 与开始时的 `SelectionRef` 快照。确认成功后清除捕获；失败保留；取消不写事务；
  模式/文档切换清理状态。测量沿用现有实现。
- `cad-host` 保持 HostCore，消费 RenderSessionHandle、配置/度量及语义更新；移除
  `UiHandle`、`CadView`、`MessageSource`、UI 专属 trait 实现和对 `cad-ui-slint` 的依赖。
  平台适配器由 apps 持有，Slint 本地化继续留在 Slint。借用 RefCell 期间不回调拉取
  HostUpdates。

### P3：有界语义队列与唤醒

- 队列默认容量 256；单次 drain 最多处理 64 个请求或约 4ms（非实时、不可抢占）。只合并
  相邻且 context 相同的 `MoveCursor`，采用 latest-wins；命令、其他输入及 context 变化均为
  顺序屏障。每个已接受请求有完成结果；过期 context 明确取消；队列满显式报错并为终结输入
  保留空间，避免手势卡住。执行时先释放共享队列锁。
- 队列由空转非空时唤醒，仍有待办时重置唤醒；不得用永久 10ms tick 轮询 CAD dirty。平台
  已有 I/O polling 可保留，但不得无条件触发 CAD dirty；UI-only frame 复用 CAD texture。
- dirty 采用 event-driven 失效/唤醒，不改变既有 ChangeSet → 依赖失效 → 显示表示重建语义。
  本轮不扩张或重写增量算法。

## 明确不在本轮

- 不实现 React 或任何未来 UI 框架接入；typed protocol 不等于承诺跨语言序列化协议。
- 不改场景/事务/历史语义，不重写增量算法，不宣称跨平台性能、真实 GPU、模拟器或真机运行已通过。
- 各阶段仍须按对应契约补测试并执行其独立 gate；未运行的验证须如实标为未运行。

## 后果

P1 可以在保持当前 UI/host 过渡可编译的同时抽取 UI 无关的会话 API；P2 才建立 host 不依赖
具体 UI 的边界，P3 才收敛输入队列和空闲唤醒成本。阶段门禁覆盖依赖方向、会话与队列
契约，以及 workspace 全目标、WASM 和 Android 原生编译；运行证据须单独记录环境与范围。
