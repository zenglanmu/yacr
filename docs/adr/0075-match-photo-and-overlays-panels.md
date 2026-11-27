# ADR-0075：照片匹配与叠加层面板（Match Photo / Overlays）

- 状态：**已接受（决策，2026-11-27）**；**已实现（2026-11-27，host-read 面板 + 具名拒绝；无真实 GUI/GPU 证据）**
- 关联：ADR-0042 D11（请求携带型 host read）、ADR-0065/0066（面板模式）、概念域托盘

## 背景

概念域默认托盘最后两个 pending 面板是 Match Photo 与 Overlays：

- **Match Photo** 需要照片存储、灭点相机求解与照片背景，本仓库三样都没有；
- **Overlays** 在 SketchUp 里是用户管理的叠加定义；本仓库的叠加是**渲染器/场景拥有**的通道
  （坐标轴、网格、选中高亮、捕捉提示、选择框、悬停高亮、测量读数、绘制预览），没有可持久化的
  用户叠加模型。

规则禁止用空面板/空列表冒充能力，所以两个面板要么不是 pending、要么明确说清边界。

## 决策

### D1 两个新的 host-read kind

- `cad_match_photo`（`kind: "matchPhoto"`）：返回 `{ modeled: false, reason:
  "matchphoto.not_implemented: this build has no photo-matching model (no photo import, camera
  solve or photo background)" }`——这是能力陈述，不是空照片列表。
- `cad_overlays`（`kind: "overlays"`）：返回渲染器**当前真实**的 8 个通道
  `[{ key, active }]`，key 固定为 `axes`/`grid`/`selectionHighlight`/`snapHints`/`selectionBox`/
  `hoverTint`/`measurement`/`drawPreview`；值来自 `RenderSessionHandle::overlays()` 的可见性开关
  与瞬时 overlay 是否有内容。
- 两者都走共享 seam：没有通道的载体分别答 `unsupported-runtime: matchPhoto` /
  `unsupported-runtime: overlays`。

### D2 面板动作 = 具名拒绝

- 新增命令 `MATCHPHOTO` 与 `OVERLAYS`（无 payload）。它们不改写任何东西，只返回稳定原因：
  Match Photo 与 host read 同一字符串；Overlays 说明“没有用户管理的叠加定义模型，列出的通道由
  渲染器拥有且只读”。两个命令 Viewer 合法（只是拒绝）。
- 面板：Match Photo 显示原因 + “Try Match Photo”按钮；Overlays 只读列出通道及 on/off，并提供
  “Manage overlays”按钮。**没有可点的假开关**：通道的开关属于渲染器/宿主配置，不由该面板写入。

### D3 UI 接线

概念域托盘 `matchPhoto`/`overlays` 从 `pending` 变成真实 body；catalog 增加 `MATCHPHOTO` /
`OVERLAYS` 行与提示键；面板 body 拆到 `domains/ui/tray-panels.tsx`（DomainTray.tsx 的文件大小
门禁）。

## 不在本 ADR 范围

- 照片导入、灭点求解、相机/模型对齐、照片背景渲染；
- 用户自定义叠加定义、叠加排序/可见性持久化、脚本叠加；
- 把现有渲染器通道变成可持久化的文档设置（它们是宿主/会话配置）；
- 真实 GUI/GPU 交互与其它载体运行时。

## 后果

- 新增两个 host-read kind、两个拒绝命令与两个真实面板；**无格式档位变化**、无文档表。
- 托盘里不再有 `pending` 面板；Match Photo 仍明确未建模，Overlays 明确只读——都不是“已复刻”。
- `full` 计数仍是 54/69；这两个面板只把“没有实现且没有解释”的缺口变成“有真实读取、有具名
  拒绝、边界写明”的 partial。
