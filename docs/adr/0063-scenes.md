# ADR-0063：场景（SketchUp Pages / Scenes）

- 状态：**已接受（决策，2026-11-27）**；**已实现（2026-11-27，文档表 + 命令 + host read +
  托盘面板 + `.ymodel` minor 20；无真实 GUI/GPU 证据）**
- 关联：ADR-0037（`.ymodel`）、ADR-0042 D11（request-carrying host read）、ADR-0053（应用域）、
  ADR-0062（文档材质，同一“文档表 + 命令 + 面板”形状）

## 背景

SketchUp 的 Scenes（旧称 Pages）是“一键回到某个展示状态”的命名快照。`concept` 域的 Scenes
托盘面板此前是 pending，`tables/` 里没有场景表。已有的 named view（`tables/views.json`，minor
18）保存相机、2D/3D 模式、工作平面与空间，但没有样式；场景需要同样的相机载荷再加样式，并且
托盘要能**读到**列表（不是靠命令回显），点击能应用。

本仓库只有两种真实显示样式（`VSCURRENT` 的 `WIREFRAME`/`SHADED`）；阴影、雾、剖面、软边、
可见 Tags 都还没有模型。保存一个渲染器会忽略的样式标志是假数据，因此本 ADR 只保存可恢复、
可渲染的状态。

## 决策

### D1 场景行 = named view + 样式 token

`cad-domain::scene::Scene { view: NamedView, style: SceneVisualStyle }`：

- `view` 复用 named view 的相机/2D-3D/工作平面/空间与它严格的 JSON 编解码；
- `style` 只有 `WIREFRAME`/`SHADED` 两个 token（与 `VisualStyleToken` 同一拼写）。

因此场景行就是一个 view 对象加上 `"style"` 键；读取时先取出 `style`，再交给 view 解码器（它会
拒绝其它未知键）。上限 **32** 行（`SCENE_CAP`），名称 trimmed、非空、大小写不敏感唯一，保存同名
覆盖并移到末尾；场景与 named view 各自独立的命名空间。

### D2 命令面：`SCENE`

typed argument 行（与 `VIEW` 同一套机制）：

- `SCENE SAVE <name>`：捕获当前 viewport 的相机/2D-3D/工作平面 + 会话空间 + `VSCURRENT`
  样式，写入 `tables/scenes.json`；一次 revision、一步撤销。相同状态重复保存是
  `scene.unchanged`（不写、不记撤销）。
- `SCENE DELETE <name>`：一行删除，一次 revision/一步撤销。
- `SCENE RESTORE <name>`：**会话应用**（相机、2D/3D、工作平面、空间、样式），不写文档；有变化时
  先推入 `ZOOM PREVIOUS` 历史，`VIEWBACK` 可回退。与 `VIEW RESTORE` 同一形状。
- `SCENE LIST`：只读报告。
- Viewer 模式：`SAVE`/`DELETE` 按 `PermissionDenied` 拒绝（与 `VIEW SAVE` 同一规则），
  `RESTORE`/`LIST` 合法。

错误标记：`scene.name_required`、`scene.unknown`、`scene.invalid_view`、`scene.style_invalid`、
`scene.cap_exceeded`、`scene.duplicate_name`、`scene.table_too_large`。

### D3 托盘面板走 request-carrying host read

新增第 5 个 host-read kind `scenes`（`HostReadRequest::Scenes` → `HostReadAnswer::Scenes`）：

- `cad-host` 的 `HostCore::scenes()` 投影 `tables/scenes.json`（名称、3D 标志、空间显示名、
  样式 token），无文档是空列表，从不写文档；
- 新 wire 行 `cad_scenes`（registry 的 `reads` 通道）两端共用，桌面经现有 `HostReadFn`
  闭包、`dom-canvas` 经 `CadSession::host_read`；
- 没有 seam 的载体答 `unsupported-runtime: scenes`，不是空列表；
- 面板（`DomainTray` 的 `ScenesBody`）读 `cad_scenes` 列行：点名称 dispatch
  `SCENE RESTORE <name>`，× dispatch `SCENE DELETE <name>`，输入框 + 按钮 dispatch
  `SCENE SAVE <name>`。读取永不改相机/文档，所有写入都走普通命令路径。

### D4 持久化与撤销

- `tables/scenes.json`，`FORMAT_MINOR` 19 → **20**；无场景文档保持原档位；`cad-store-zip`
  把 scenes 放在最高档；19 及以下照常打开。
- 撤销/重做：`cad-history` 新增 `scene_patches` 车道（`ScenePatch` before/after，含
  `inverse()`），`cad-app` 的 replay 走同一 `apply_scene_patches`。

## 不在本 ADR 范围

- **可见 Tags/图层、阴影、雾、剖面、软边**：没有对应模型，**不**存进场景；
- 场景缩略图、动画/连续播放、按场景更新文档设置（例如场景切换自动写图层可见性）；
- 场景改名/重排命令（覆盖保存可用，改名需 delete+save）；
- 真实 GUI/GPU/像素证据与其它载体运行时证据。

## 后果

- 文档多一张小表和一个格式档位；没有场景的文档零成本。
- 场景与 named view 字段重复但语义不同（一个带样式、一个不带），因此不合并成一张表：合并会让
  `VIEW LIST` 出现场景，或让场景丢样式。
- history 增加一个车道字段，所有 `DrawingHistoryStep` 字面量显式列出；`record` 漏斗已拆到
  `cad-history/src/record.rs` 以守住文件行数上限。
- 托盘 Scenes 面板从 pending 变成真实面板；覆盖表的 Tray 行随之改为 `partial`（缺阴影/雾/
  可见 Tags）。
