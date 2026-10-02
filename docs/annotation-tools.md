# 批注创建工具与管理面板（F07/F08/F09）

本轮在 `cad-app` 与 `cad-ui-slint` 落地了六类批注的**捕获状态机**与**管理通道**，
核心逻辑可测；渲染层叠加已接线（见 `docs/annotation-render.md`）。

## 已实现

### 捕获工具（F07）
`cad-app::annotation_tool::AnnotationTool`，六类（`AnnotationToolKind::ALL`）：

| 类 | key | 点需求 | 需要文本 | 自动完成 |
|---|---|---|---|---|
| 文本 | `text` | 1 | 是 | 是 |
| 引线 | `leader` | 2 | 是 | 是 |
| 矩形 | `rectangle` | 2 | 否 | 是 |
| 椭圆 | `ellipse` | 2 | 否 | 是 |
| 自由线 | `freehand` | ≥2（开放） | 否 | 否 |
| 修订云线 | `revision_cloud` | ≥3（闭合） | 否 | 否 |

- 每类显式声明 `min_points` / `exact_points` / `requires_text` / `auto_completes`；
- 步骤捕获（`push_point` / `set_text`）带参数校验，缺参**不会**提交事务；
- `preview()` 暴露 `can_confirm` / `status_line`，确认时恰好一次事务（复用历史/撤销路径）；
- `cancel` 恒为零事务。

### 管理通道（F09/F03 风格）
`cad-app::annotation_list`：
- `annotation_rows`：只读地列出批注库（id/类型/文本/几何种类），并叠加会话可见性与选中态；
- `AnnotationVisibilitySet`：临时隐藏/显示，**只存会话状态，不改批注库**；
- 选中用于编辑/删除；`CommandPayload::DeleteAnnotation(id)` 走既有事务路径。

### UI 通道
- `ui/app.slint`：批注类型选择、步骤提示、文本输入（`annotation-text-input`）、确认/取消、
  管理列表（可见性开关 + 选中 + 删除）；
- `cad-ui-slint`：`AnnotationUiState` + 适配器回调，行索引经推送顺序精确映射回 `AnnotationId`，
  不做有损转换；空列表是显式空态。

## 未完成（显式）

- **批注叠加渲染**已由后续工作接线（见 `docs/annotation-render.md`）：`cad-scene` 把批注几何
  转成线段批次，桥接合并进场景并随批注 revision/可见性重建。仍未做：RGB 颜色（批次只带
  alpha）、绘制顺序（`draw_order` 已填充但渲染按上传顺序）、修订云扇贝造型、
  测量几何叠加（属测量路径）。真机/GPU 运行未验证。
- **Slint 渲染未运行**：本机缺 fontconfig，`cad-ui-slint` 不能原生构建；其测试为字符串/结构断言，
  仅由 wasm32 `cargo check` 保证可编译，无视觉/真机验收。
- **Web 宿主连接器已接线**：`apps/app-web` 的 `browser/state_push.rs` 已调用
  `set_annotation_state`（含有序 `AnnotationId`）以及图层/布局/属性/诊断推送，并在导入/
  导出/恢复后刷新；`apps/app-android` 仍未接线。空面板文案取自目录
  `annotation.empty`。仅源码接线，浏览器/真机验收待补。

## 验证

```bash
cargo test -p cad-app --locked                                   # 107 passed
cargo check --workspace --lib --target wasm32-unknown-unknown --locked
cargo clippy -p cad-app --all-targets --locked                   # clean
```
