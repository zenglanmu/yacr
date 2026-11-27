# ADR-0079：材质面板的文档材质行（cad_materials）

- 状态：**已接受（决策，2026-11-27）**；**已实现（2026-11-27，host read + 托盘重涂；无真实 GUI/GPU 证据）**
- 关联：ADR-0062（文档材质与 `PAINT`）、ADR-0042 D11（请求携带型 host read）、Materials 托盘

## 背景

Materials 托盘此前只有 12 个**内置**色块；文档自己的材质（`tables/materials.json`）只在
`MATERIALS` 命令的文本报告里出现，面板无法列出或重涂用户已经用过的材质。覆盖表把这条写成
“文档材质只在 MATERIALS 报告里列出，色块不随文档材质生成”。

## 决策

### D1 一个只读 host read

- `cad_materials`（`kind: "materials"`，无参数）返回文档材质表的行
  `{name, color: [r,g,b], transparency}` 及 `total`/`truncated`；空文档/无材质是空列表，
  不是错误；无通道载体答 `unsupported-runtime: materials`，不是空列表。
- 上限 256 行（与 `MATERIAL_CAP` 一致）；host 只读投影 `tables/materials.json`，不写。

### D2 托盘

- 面板上方列出**文档材质**（色块 + 名称 + 真实透明度），点击 dispatch 既有
  `PAINT <name>`；下方保留内置调色板（首次涂刷会落成文档行）。
- 文档变化（命令数/实体数/文档名）后重新读取；读取失败显示原因，不伪造空表。
- 提示文案明确：涂刷仍是**整实体**；逐面材质、材质编辑器、贴图/材质库未建模。

### D3 不改协议以外的东西

没有新命令、没有格式档位变化；`PAINT`/`MATERIALS` 命令与 `tables/materials.json`
（minor 19）都不变。

## 不在本 ADR 范围

- 逐面/逐子元素材质、材质编辑器（改名/改色/删除）、贴图与材质库；
- 材质缩略图渲染、拖放涂刷、按材质选择；
- 真实 GUI/GPU 交互与其它载体运行时。

## 后果

- 新增一个 host-read kind、一个 wire 响应、一个托盘读取与重涂路径；文档材质在 UI 中可见。
- Materials 仍是 `partial`（逐面/编辑器/贴图未建模），但“文档材质只在报告里”这一缺口关闭。
