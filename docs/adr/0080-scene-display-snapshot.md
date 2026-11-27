# ADR-0080：场景阴影/雾快照（Scenes capture Shadows/Fog）

- 状态：**已接受（决策，2026-11-27）**；**已实现（2026-11-27，场景行 `display` + 恢复写入；无真实 GUI/GPU 证据）**
- 关联：ADR-0063（场景表）、ADR-0065（`tables/display.json`）、ADR-0078（场景可见标签）

## 背景

场景行已经保存相机、2D/3D、工作平面、空间、样式与可见标签，但 `SHADOWS`/`FOG` 的文档记录
（`tables/display.json`，minor 22）不随场景切换。SketchUp 的页面会记住阴影/雾状态，因此这仍是
覆盖表 Scenes 行的明确缺口。

显示记录是**文档状态**（重开文档保持、所有载体一致），与场景标签的会话覆盖不同：恢复一条
不同的记录必须写 `tables/display.json`，而不能只改会话。

## 决策

### D1 行快照与档位

- `cad_domain::scene::Scene` 增加 `display: Option<DisplaySettings>`；行 JSON 的 `"display"`
  直接使用 `DisplaySettings::to_json/from_json`（严格：未知键/越界值拒绝）。
- 没有 `display` 的旧行（minor 20..23）解码为 `None`，恢复时不动显示记录。
- 非法记录是 `scene.display_invalid`；新行把归档升到 **minor 24**
  （`FORMAT_MINOR = 24`），没有该键的场景行保持原档位。

### D2 保存与恢复

- `SCENE SAVE` 总是捕获当前文档显示记录（与标签快照一样，没有“场景属性”开关）。
- `SCENE RESTORE`：
  - 记录相同 → 不写文档（保持原来的会话恢复语义）；
  - 记录不同且在 Work 模式 → 先用与 `SHADOWS`/`FOG` 相同的 `write_display` 写一次
    `tables/display.json`（一次 revision / 一步撤销，标签 `SCENE RESTORE <name>`），再应用
    相机/空间/样式/标签；
  - 记录不同且在 Viewer → 在任何改动**之前**返回 `PermissionDenied`（Viewer 不能写文档），
    不留下半应用场景。
- 显示写入在会话应用之前执行：它是唯一可失败的文档操作，失败时相机与标签都未动。

### D3 边界

- 场景仍不保存剖面、软边或缩略图（这些没有场景应用路径）。
- 没有“逐场景阴影/雾复选框”或场景延迟/动画。
- 显示记录的写回仍受 Work 权限与现有 undo 机制约束，不新增第二套显示状态。

## 后果

- 场景行可携带 `display`；`tables/scenes.json` 向后兼容（旧行没有该键）。
- 归档 minor 升到 **24**；旧读者按 `unsupported_minor` 拒绝新档，而不是静默丢失阴影/雾。
- Scenes 行从“阴影/雾不随场景切换”改为“有差异时恢复为一次显示写入”，其余未建模项照旧。
