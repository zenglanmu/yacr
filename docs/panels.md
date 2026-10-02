# 图层与属性面板接线（F03 / F05 / U03）

本文件记录 `cad-app` 与 `cad-ui-slint` 之间**图层列表（F03）**与**选择/只读属性
（F05）**的接线状态，以及**刻意未接线**的部分与精确的剩余接线缺口。规范权威是
`CAD_IMPLEMENTATION_SPEC.md` v2.0；需求追踪见 `docs/requirements.md`（F03/F05）与
`docs/code-audit-and-agent-handoff.md`（F03/F05/U03）；交互状态机与测量面板见
`docs/interaction.md`、`docs/ui.md`。

本文件只描述展示层与应用层的边界：面板把应用状态推入 Slint 组件，并把 Slint
回调翻译成 `Command`；不重实现数据库、拾取或渲染算法。

## 1. 图层（F03）

### 1.1 真实图层表（只读）

- `cad-app::layers::layer_rows(&DrawingDatabase, &LayerOverrideSet) -> Vec<LayerRow>`
  直接从 `DrawingDatabase::layers()` 读取 `(id, name, visible)`，**不重写**底图。
  投影顺序沿用数据库自身的 `BTreeMap<LayerId, _>` 顺序（确定、可按 id 复现）。
- `LayerRow` 同时给出三个字段，UI 不再猜测：
  - `database_visible`：DWG 图层表里的存储值（唯一权威）；
  - `override_visible: Option<bool>`：本会话的临时覆盖；
  - `effective_visible`：渲染应遵守的最终可见性（覆盖 ⊕ 存储值）。
- 空数据库 → 空列表；UI 显示显式空状态，不生成假行。
- 分页/修订绑定路径 `layers::query_layers(&QueryService, &DrawingDatabase, QueryRequest)`
  复用 `cad-query` 的修订绑定投影（§4.9），供需要丢弃陈旧投影的宿使用。
- 搜索：`layers::filter_layer_rows(&[LayerRow], needle)` 为大小写不敏感子串过滤，
  空 needle 返回全部，命中为空时返回空列表（不伪造行）。

### 1.2 临时覆盖模型（不改底图）

- `cad-app::layers::LayerOverrideSet`：`BTreeMap<LayerId, bool>` 的会话级覆盖。
  - `set(id, visible)` / `remove(id)` / `clear()`；`clear_changed()` 返回是否变化。
  - `effective(id, database_visible)`：`get(id).unwrap_or(database_visible)`。
  - `is_entity_visible(&DrawingDatabase, &DbEntity)`：唯一可见性判定，读底图存储值 + 覆盖。
  - `fingerprint() -> u64`：BTreeMap 顺序稳定的 FNV-1a 指纹，供渲染桥做变更检测。
- `LayerOverrideSet` 存放在 `SessionState.layer_overrides`（类型从裸 `BTreeMap`
  提升为该模型，语义唯一）。
- 命令路径：`CommandId::ToggleLayer` + `CommandPayload::Layer(LayerId, bool)` 写入覆盖；
  `CommandId::RestoreLayers` 清空覆盖。二者都**不写库、不记历史、不改 revision**。
- 单测：`crates/cad-app/src/layers.rs` 覆盖"覆盖可显示被隐藏图层而不动底图"、
  "覆盖可隐藏可见图层"、"restore 报告是否变化"、"指纹只随覆盖状态变化且与插入
  顺序无关"、"搜索命中/未命中"、"`visible_model_entities` 过滤隐藏图层"。

### 1.3 渲染/场景请求路径（已消费）

- `cad-ui-slint::bridge::build_scene_with_overrides(database, stamp, fonts, &overrides)`
  在遍历 `cad_app::layers::visible_model_entities(database, overrides)` 时跳过不可见
  图层的实体，再走既有 `cad-representation` → `cad-scene` 管线。**底图未重解析**，
  数据库 revision 不变；只重建受影响的显示批次。
- `CadView::set_layer_overrides(LayerOverrideSet)` 把新覆盖写入桥的共享槽并请求重绘；
  `BridgeState` 用 `overrides_fingerprint` 决定是否重建批次，覆盖未变时不重建。
- `build_scene` / `build_scene_with_fonts` 保持原签名（等价于空覆盖），未破坏既有调用方。
- 由于 `cad-representation`/`cad-scene` 无图层字段（本任务不可改），过滤放在
  `cad-app`（可测）并由 `cad-ui-slint` 调用——这是本轮的完整接线点，**不是**待办缺口。

## 2. 选择与属性（F05）

### 2.1 选择集与实体身份

- `cad-app::selection::SelectionSet` 包装 `Vec<SelectionRef>`，身份为
  `(entity, instance_path, sub_element)`：
  - 同一块实体经**不同 `InstancePath`** 是不同对象；子元素再次独立；
  - `insert` 去重、`toggle`、`remove`、`contains`、`replace`、`clear` 均按身份精确；
  - `identity_label(&SelectionRef)` 输出可读身份（`实体 @ [实例路径] #子元素`）。
- `SessionState.selection` 类型从裸 `Vec` 提升为 `SelectionSet`，语义唯一。
- 命令路径：`CommandId::Select` + `CommandPayload::Selection(Vec<SelectionRef>)`；
  空 Vec 清空选择。选择是**只读**操作：设置 `ToolState::Selecting`、记录引用，
  **不写库、不记历史、不改 revision**（`cad-app` 单测
  `select_command_records_selection_and_does_not_mutate_the_drawing`）。
- 单测：`selection.rs` 覆盖"不同 INSERT 实例独立"、"同实例重复被拒"、"子元素独立"、
  "toggle/remove 身份精确"。

### 2.2 只读属性视图

- `cad-app::selection::SelectionProperties::extract(&DrawingDatabase, &SelectionSet)`：
  - 空选择 → `empty = true`，无行（显式空状态）；
  - 单选且可解析 → `rows`：`id`/`handle`/`type`/`layer`/`space`/`draw_order`，
    有实例路径时追加 `instance`，有子元素时追加 `sub_element`，再叠加几何专属基础项；
  - 单选但引用已失效/不存在 → 显式 `status = unresolved` 行（不伪造实体）；
  - 多选 → `rows` 为空、`mixed_keys` 列出取值不同的键，UI 显示"已选 N + 多值键"。
- 几何专属基础项（真实计算值，不含臆造）：
  - `Line`：start/end/length（长度由端点欧氏距离计算）；
  - `Circle`：center/normal/radius；`Arc`：center/radius/start_angle/sweep；
  - `Ellipse`：center/ratio/start_angle/sweep；
  - `Polyline`：vertices/closed/bulges 数量；`Spline`：degree/control_points 数量；
  - `Point`：position；`Mesh`：vertices/triangles 数量；
  - `Insert`：block；`Text`：text/position/height/font（未知字体显式 `unknown`）；
  - `Opaque`：opaque_type/opaque_version/payload_bytes；`Compound`：children 数量。
- 曲线长度等需要离散精度的量**不在此臆造**；需要精确长度请走测量引擎（F06）。
- `handle` 缺失时显式 `none`，不合成编号。

## 3. UI 通道（Slint）

### 3.1 已接线

- `ui/app.slint` 新增两个结构体：`LayerRow { id, name, visible, overridden }`、
  `PropertyRow { key, value }`，以及紧凑的"图层 + 属性"面板（`Rectangle` 高 132px）。
- 图层面板：
  - `in property <[LayerRow]> layer-rows`、`layer-override-count`、`layer-empty-label`；
  - 每行 `CheckBox` 绑定 `visible`，切换回调 `layer-visibility-toggled(index, visible)`；
  - `restore-layers-requested` 按钮仅在存在覆盖时可用；
  - 覆盖行显示"临时"标记（`overridden`）。
- 属性面板：
  - `in property <[PropertyRow]> property-rows`、`property-mixed-label`、
    `property-empty-label`、`selection-count`；
  - `clear-selection-requested` 派发 `Select` + 空 `Selection`；空状态显示
    `property-empty-label`（不显示假行）。
- 适配器（`cad-ui-slint/src/lib.rs`）：
  - `on_layer_visibility_toggled` 用 `index` 回查 `UiHandle` 推送时保存的
    `Vec<LayerId>`，**不做 u128→i32 有损转换**，再派发 `ToggleLayer`；
  - `on_restore_layers_requested` → `RestoreLayers`；`on_clear_selection_requested`
    → `Select` + 空 `Selection`。
- 状态推送：
  - `LayerPanelState::from_rows(&[cad_app::layers::LayerRow], empty_label)`；
  - `PropertyPanelState::from_properties(&cad_app::SelectionProperties, empty_label, mixed_label_fn)`；
  - `UiHandle::set_layer_state(&state, &order)`、`UiHandle::set_property_state(&state)`。
- 面板不创建每实体控件；只按行迭代真实模型。

### 3.2 宿主连接器（本轮范围外，与测量面板同一交接）

宿主（`apps/app-web`、`apps/app-android`）尚未调用：

```rust
handle.set_layer_state(
    &LayerPanelState::from_rows(&controller.layer_rows()?, "无图层"),
    &controller.layer_ids()?,
);
handle.set_property_state(
    &PropertyPanelState::from_properties(
        &controller.selection_properties()?,
        "未选择",
        |keys| format!("多值: {}", keys.join(",")),
    ),
);
cad_view.set_layer_overrides(controller.session.layer_overrides.clone());
```

未调用时行为是**降级而非假装**：面板初始为空模型并显示显式空状态文案，图层切换
按钮不可用；这不是静默假成功。`HostController::layer_ids()` 与 `layer_rows()` 顺序
一致，宿主无需自行拼接。

### 3.2.1 Web 宿主接线（已接线；无头/真机验收待补）

`apps/app-web` 在 `browser/state_push.rs` 中已调用
`set_layer_state` / `set_property_state`（以及布局/批注/诊断面板），并传入真实
`LayerId`/`AnnotationId`/`LayoutId` 顺序；空状态与多值文案取自目录
（`layers.empty`、`properties.empty`、`properties.mixed`）。无捕获工具时的画布轻触经
`cad_app::pick_at_screen` 命中后派发 `Select`，属性面板随命令统一推送刷新。
`apps/app-android` 仍未接线，不在本轮范围。

### 3.3 刻意未做（原因）

- **布局面板（F04）**：`bridge` 只构建 model_space，纸空间/视口裁剪与比例未闭环，
  现在加面板只能是空壳；属独立工作流。
- **批注列表面板（F09）**：`QueryService.annotations` 有投影，但隐藏状态、列表
  UI 与映射策略尚未闭环（审计 F09）；属独立工作流。
- **资源 / 3D / 诊断抽屉（F10/F13/U08）**：对应核心闭环未完成或 `pending(...)`，
  加面板会是假数据。见 `docs/ui.md` 第 4 节的同一判断。
- **选择高亮（F05 高亮部分）**：面板展示选择属性，但把选择集画成高亮几何需要宿主
  把 `SelectionRef` 经场景/GPU 管线提交（`bridge` 当前不消费选择高亮）；这是独立于
  属性面板的渲染接线，未在本轮完成。`SelectionRef` 已带 `InstancePath`，渲染侧可直接
  区分实例。
- **精确拾取**：把画布点击变成 `SelectionRef` 需要宿主安装画布→世界映射并做命中
  测试（`cad-spatial::GridSpatialIndex`）；UI 侧只回传逻辑像素，宿主未接线时同样
  显式提示，不静默丢弃（与测量 `canvas-pick` 同一条路径）。

## 4. 测试与证据边界

- 纯逻辑全部在 `cad-app`，由 `cargo test -p cad-app` 覆盖：
  `layers::{LayerOverrideSet,layer_rows,filter_layer_rows,visible_model_entities,query_layers}`、
  `selection::{SelectionSet,SelectionProperties,identity_label}`，以及命令层
  `ToggleLayer`/`RestoreLayers`/`Select` 的"不改库、不记历史"断言。
- `cad-ui-slint` 的新测试位于该 crate 的 `#[cfg(test)]`：`LayerPanelState`/
  `PropertyPanelState` 的映射，以及外壳定义字符串断言（`layer-rows`、
  `layer-visibility-toggled`、`restore-layers-requested`、`property-rows`、
  `clear-selection-requested`）。**这些只是字符串/结构断言，且本机缺 fontconfig
  无法构建该 crate 的测试**，本轮未执行；`cargo check --workspace --lib --target
  wasm32-unknown-unknown` 是 `cad-ui-slint` 的编译门，已通过。
- Slint 渲染、窗口事件循环、真实指针→世界映射、GPU 合成与覆盖后的画面**本轮均未
  运行**；上述仅为源码接线与编译证据，不构成视觉/真机验收。
