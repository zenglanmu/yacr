# 恢复、后端选择与未保存决策（F12 / B07 / U09）

本文记录 **应用层与 Slint 桥接层** 的后端选择、未保存决策与设备丢失重建契约。
它只描述本工作stream（`cad-app` / `cad-ui-slint`）实现的纯逻辑；真实 GPU 设备初始化、
Slint 渲染与浏览器后端探测**未在本环境运行**，见文末“仍未验证”。

## 1. 后端选择与回退（F12）

后端选择的结果是一个可区分的结构，而不是一个通用字符串。`cad-app::recovery` 定义：

```rust
pub enum BackendOutcome {
    Initialized { preference: BackendChoice, actual: ActiveBackendKind },
    Failed { preference: BackendChoice, failure: BackendFailure },
}

pub enum ActiveBackendKind { WebGpu, WebGl2, Native }

pub enum BackendFailure {
    NoBackendAvailable,              // Auto 探测两档均失败
    ForcedUnavailable { reason },    // 强制后端被平台拒绝
    InitFailed { reason },           // 偏好被接受但设备/渲染器初始化失败
}
```

语义：

- **成功与失败可区分**：`BackendOutcome::is_live()` / `actual()` 只在真正初始化后返回
  `Some`；失败时 `actual()` 为 `None`，并携带具体 `BackendFailure`。
- **标签跟随真实设备，不跟随愿望**：`BackendOutcome::label()` 用 `actual` 生成标签。
  强制 WebGL2 但实际拿到 WebGPU 设备时，标签是 `webgpu`，绝不是 `webgl2`
  （审计 F12“GL 桥仍标 WebGPU”）。Slint 桥在 `RenderingSetup` 时用
  `BackendCapabilities::actual` 构造 `Initialized`，因此 GL 与 WebGPU 被如实标注。
- **失败给出可恢复动作**：`BackendFailure::recovery_hint()` 为每类失败给出 UI 应展示的
  具体动作（回退 Auto / 切换后端 / 重试），不是一句“错误”。

桥（`cad-ui-slint/src/bridge.rs`）不再保存 `Option<String> error`，而是保存
`Option<BackendOutcome>`。`CadView` 暴露：

| 方法 | 含义 |
|---|---|
| `backend_outcome()` | 完整结果（成功或结构化失败） |
| `backend_is_live()` | 是否真的有可用后端 |
| `backend_label()` | 如实标注真实后端的标签 |
| `last_error()` | 仅失败时给出具体原因，成功时为 `None` |
| `preference_choice()` | 请求的偏好（app 层枚举） |

Web 侧的探测与偏好持久化仍在 `cad-ui-slint/src/web.rs`（真实 adapter 请求、WebGL2 上下文
探测、recovery 快照先写后 reload）。桥接层只负责把“实际初始化结果”结构化。

## 2. 未保存决策模型（U09 / B07）

`cad-app::recovery::UnsavedFlow` 是纯决策模型，不写任何文件：

```rust
pub enum UnsavedDecision { Save, PreserveRecovery, Discard, Cancel }
pub enum UnsavedOutcome { Proceed, SaveFailed, RecoveryFailed, Cancelled }
```

`UnsavedFlow::new(dirty).apply(decision, save_succeeded, recovery_succeeded)`：

| 决策 | 干净文档 | 脏文档 |
|---|---|---|
| `Save` | 继续 | 仅当 `save_succeeded` 为真才继续，否则 `SaveFailed` |
| `PreserveRecovery` | 继续 | 仅当 `recovery_succeeded` 为真才继续，否则 `RecoveryFailed` |
| `Discard` | 继续 | 继续（用户明确丢弃） |
| `Cancel` | `Cancelled` | `Cancelled`（保留当前文档与恢复数据） |

关键不变量：

- **保存失败绝不报告为已保存**（B07）。`SaveFailed` 不推进，不替换文档，不清除 dirty。
- `Cancel` 永远保留当前文档与任何恢复数据（U09）。
- 主机（`HostController`）通过 `Application::resolve_leave(doc, decision, save, recovery)`
  应用该模型；`prepare_leave` 现在委托给同一模型，因此 `Save` 在未确认写入时返回
  `Unsupported`，`PreserveRecovery` 在快照未持久化时返回 `Invariant`。
- 文档切换/打开走 `HostController::open_bytes_leaving(bytes, label, decision, save, recovery)`：
  失败或取消都保持当前文档、会话、历史与批注不变；`open_bytes_decided` 是不做写入的
  保守入口（`save=false, recovery=false`）。

### 2.1 恢复快照（原子导出）

`HostController::capture_recovery_snapshot(camera_center, world_per_px)` 复用与 sidecar 导出
**相同的原子编码器**（`AnnotationService::encode`），因此不存在“半个快照”：

- 编码失败返回错误，不产生快照，也不把文档标记为已保存；
- 快照携带文档身份（`DocumentIdentity`）、名称提示、批注 JSON、相机；
- `RecoverySnapshot::encode` / `decode` 是纯逻辑，可持久化到 localStorage 或恢复文件；
  `decode` 对损坏/版本不符/身份截断的输入返回 `None`，绝不伪造“无未保存工作”。

`HostController::restore_recovery_snapshot(snapshot)` 用严格指纹策略
（`FingerprintPolicy::RejectMismatch`）经与 sidecar 导入相同的单事务路径恢复，身份不匹配即
报错且不应用任何内容。

## 3. 设备丢失 / 重建路径（F12）

设备丢失必须在桥接层闭环，且场景**从数据库重建**，而不是从丢失的 GPU 批次：

- 桥在每帧调用 `Renderer::render`。若返回 `RenderError` 且 `is_device_loss()` 为真：
  1. 调用 `Renderer::note_device_lost(reason)`，渲染器拆除所有派生 GPU 资源并置位；
  2. 桥把该次丢失记为 `BackendOutcome::Failed { InitFailed }`，`caps` 置空；
  3. 清空 `BridgeState` 的 `document`（场景身份标记）与 `built_generation`；
  4. 请求重绘。
- 下一帧 `BeforeRendering` 时 `document` 为空，桥从 `incoming` 槽的权威
  `DrawingDatabase`（未被清空）重新 `build_scene_with_overrides` 并 `upload`。文档、批注与
  相机都不是 GPU 状态，因此自然存活。
- 桥还跟踪 `built_generation`：`initialize_with_device` 会递增设备代次，代次变化即强制
  重建，覆盖“宿主提供新设备后未触发身份变化”的情况。
- `CadView::teardown()` / `RenderingTeardown` **不再清空文档槽**（旧实现会 `take()` 掉
  `incoming`，这正是审计指出的“重建后场景不恢复”）。它们只重置 GPU 状态标记，让下一帧
  从数据库重建。

`CadView::note_device_lost(detail)` 供宿主在直接观测到 `DeviceLostReason` 时调用，等价于
上面的第 1–4 步，并返回原因字符串供诊断展示。

## 4. 测试

纯逻辑测试（本环境已运行）：

- `cad-app::recovery`：后端结果映射（成功/失败可区分、强制 WebGL2 实跑 WebGPU 标签为
  webgpu、`ForcedUnavailable` 与 `InitFailed` 区分）；未保存决策转移（保存/恢复失败不推进、
  Cancel 永不推进、Discard 推进、干净文档短路）；恢复快照往返（sha256/temporary 身份、
  损坏输入拒绝）。
- `cad-app::host`：保存失败不替换文档且仍 dirty；恢复写失败不替换文档；恢复快照经宿主
  捕获→编码→解码→恢复后批注完整（含 id/text）。
- `cad-ui-slint::bridge`：`ActiveBackend → ActiveBackendKind` 与
  `BackendPreference → BackendChoice` 映射如实；`UI_DEFINITION` 结构断言。

## 5. 仍未验证（禁止当作已完成）

- **真实 GPU 设备初始化未运行**：本环境无适配器/共享设备，`RenderingSetup`、
  `initialize_with_device`、`BackendCapabilities` 运行时核对、GL 与 WebGPU 的真实标签都
  尚未在真实设备上执行。桥接逻辑只做了静态编译（wasm32）。
- **Slint 渲染未运行**：`cad-ui-slint` 宿主测试因缺少 fontconfig 开发头无法在本环境构建，
  桥接与 UI 结构断言只通过 wasm32 `cargo check` 与源码断言。
- **浏览器 Auto 回退未实测**：`web.rs` 的真实 adapter 请求与 WebGL2 上下文探测未在浏览器
  运行；“Auto 探测失败后的真实回退”仍需真实浏览器验收。
- **设备丢失的真实重建未实测**：`note_device_lost` → 从数据库重建的路径未在真实设备丢失
  事件下运行。
- **宿主自动保存 UI 缺失**：`Save` 决策的持久写入与确认由各宿主负责（Web 有
  `prepare_annotation_export` → 下载 → `confirm_annotation_export` 的分步路径；Android 的
  自动保存/文件 API 尚未接线）。UI 层尚未接入完整的 Save/PreserveRecovery/Discard/Cancel
  弹窗流程；本文只锁定其决策模型与主机 API。
