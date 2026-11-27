# ADR-0065：阴影与雾（Shadows and Fog）

- 状态：**已接受（决策，2026-11-27）**；**已实现（2026-11-27，文档显示记录 +
  `SHADOWS`/`FOG` + 渲染 uniform + `.ymodel` minor 22 + 两个托盘面板；无真实 GUI/GPU 像素证据）**
- 关联：ADR-0037（`.ymodel`）、ADR-0044（光照/IBL）、ADR-0050（共享 `Camera` uniform）、
  ADR-0062/0063/0064（同属“文档显示属性 + 命令 + 面板”）

## 背景

SketchUp 的 Shadows 与 Fog 是两个**显示设置**面板：Shadows 打开太阳光并给出方位角/高度角，
Fog 在远距离淡出到一种颜色。参考软件还会画**投射阴影**（阴影贴图、软化、地面淡出）。

本仓库的渲染器只有**一个方向光**：`Camera.light`（2D 是 +Z 头灯，3D 是相机相对的 key light）
加常数环境光。没有深度/阴影贴图通道，也没有雾。

## 决策

### D1 一个文档显示记录（`tables/display.json`，minor 22）

`cad_domain::display::DisplaySettings` 携带：

```jsonc
{ "shadows": { "enabled": true, "azimuth": 135.0, "elevation": 45.0 },
  "fog":     { "enabled": true, "start": 100.0, "end": 1000.0, "color": [200, 210, 220] } }
```

- 方位角是**自世界 `+Y`（北）顺时针**的角度，`[0, 360)`；高度角是地平线以上 `[0, 90]`；
  朝太阳的单位向量是 `(cos(elev)·sin(az), cos(elev)·cos(az), sin(elev))`。
- 雾距离是**模型单位**，要求 `end > start ≥ 0`；颜色是 sRGB 字节。
- 记录只在**非默认**时写入；普通文档保持原档位。解码严格（未知键、错误类型、越界值都拒绝，
  标记 `display.invalid_row` / `display.invalid_value`），缺表 = 默认记录。

### D2 Shadows = 着色太阳，**不是**投影阴影

- `SHADOWS ON|OFF` 打开/关闭太阳；`SHADOWS SUN <azimuth> <elevation>` 改方向；
  裸 `SHADOWS` 只报告当前记录。
- 渲染器把 `Camera.light` 换成文档太阳的单位向量（`ViewSnapshot.sun`，逐帧 uniform 数据）。
  2D 视图仍保持世界 +Z 头灯，不受影响。
- **投射阴影（shadow map）、软化、地面淡出、逐物体投影开关一律 NOT IMPLEMENTED**：没有深度
  贴图通道，托盘面板与 `SHADOWS` 报告都显式写明。

### D3 Fog = 可见通道的**距离雾**

- `Camera` uniform 从 160 B 扩到 **192 B**：words 40..47 是 `fog_color`（rgb + enabled）与
  `fog_range`（start, end）。非雾帧写零，shader 以 `enabled == 0` 快路径直接返回。
- 每个可见颜色 shader（`mesh`、`mesh_merged`、`mesh_textured`、`mesh_pbr`、`line`、
  `line_instanced`、`arc`、顶点拉取的 `mesh_pulled`/`line_pulled`）在最终颜色上按
  “片元到视点距离”混合到雾色；视点在同一顶点空间（拉取通道是 chunk 相对几何 + 世界空间视点）。
- 拉取通道的 `PullCamera` 从 80 B 扩到 **128 B**（新增 `fog_color`、`fog_range`、`eye`）。
- 选择/悬停 tint 先于雾混合，雾只作用于几何颜色；id/mask/shadow 类通道不参与。

### D4 命令与撤销

- `SHADOWS [ON|OFF|SUN <az> <elev>]`、`FOG [ON|OFF|RANGE <start> <end>|COLOR <r> <g> <b>]`，
  两个都是 Work-only（写文档）。
- 一次写入替换整条记录 = **一次 revision + 一步撤销**；历史步只携带一个整记录 patch，
  撤销/重做走 `apply_display_patches`。无变化时 `display.unchanged` 不写。
- 裸选择器是报告（`display.shadows` / `display.fog`），不写、不进历史。

### D5 托盘面板与 host read

- `cad_display`（`kind: "display"`，无字段）走共享 host-read seam，返回当前记录；没有通道的载体
  按 `unsupported-runtime: display` 拒绝。
- 概念域托盘的 Shadows / Fog 从 pending 变成真实面板：勾选框 dispatch `SHADOWS ON|OFF` /
  `FOG ON|OFF`，数值输入 dispatch `SHADOWS SUN …` / `FOG RANGE …`，颜色选择器 dispatch
  `FOG COLOR r g b`。所有语义仍在命令里。

## 不在本 ADR 范围

- 投射阴影/阴影贴图、软阴影、地面淡出、逐物体阴影；
- 太阳的日期/地点/时间模型（只存方位角与高度角，不存地理坐标）；
- 雾的“距离百分比”表达与天空/背景混合（只有模型单位区间 + 平坦颜色）；
- 场景保存显示设置（场景仍只快照相机/空间/样式）；
- 真实 GUI/GPU/像素证据与其它载体运行时。

## 后果

- 共享 `Camera` uniform 变大（160 → 192 B），动态偏移槽 stride 仍是 256 B，槽数不变；
  拉取相机 uniform 80 → 128 B。
- 所有可见 shader 多一个 `fog_mix` 调用（enabled 为 0 时是一次 uniform 读取 + 早退）。
- 文档多一个可选表、格式多一个档位；普通文档零成本。此前 22 档的读取端会拒绝新表而不是丢掉
  设置（`unsupported_minor`）。
