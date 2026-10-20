# ADR 0019：引线标注族（MULTILEADER）——文本输入通道、一等表示、真实导出与创建子集（P7）

- 状态：**提案（Proposed）**，待设计门禁（Oracle SG-P7 attempt 1）。
- 范围：`crates/cad-ui-core`（wire protocol / 文本输入 / 命令选择子 / i18n）、`crates/cad-app`（TEXT 与 MLEADER 命令、
  工具、文本状态）、`crates/cad-domain`（MultiLeader 变体；`Text` 变换）、`crates/cad-db`（MLeaderStyle 表）、
  `crates/cad-import-acadrust`（导入迁移 + 导出）、`crates/cad-representation`（显示）、React 宿主 UI。
- **不**覆盖：样条引线、块内容 / 公差内容、多引线簇与 `MLEADERCOLLECT`/`MLEADERALIGN`/`MLEADEREDIT`、
  注释性缩放的**应用**、背景遮罩、分栏、引线打断（break）、`QLEADER`/`LEADER`/`TOLERANCE` 命令、
  关联性、画布内**就地文字编辑器**。
- 相关：`CAD_IMPLEMENTATION_SPEC.md` §3.2；ADR-0017（一等实体 + 声明账本）、ADR-0018（同名纪律 + 匹配点清单）、
  ADR-0002（React 宿主无法绘制原生 overlay）；`docs/compatibility.md`；`docs/dxf-entity-coverage.md:32,72`。

---

## 0. 事实与约束（勘察结论，决定本 ADR 的形状）

1. **没有任何字符串通道**。`ToolValue` 只有 `Number|Flag`；`InputKind` 只有指针/滚轮/尺寸；`InteractionRequest`
   无字符串；`CadToolParams` 全是数字/布尔；唯一字符串入口 `cad_command(String)` **只收命令名**（归一化 + 白名单）。
   ⇒ "让用户输入文字"本身是**平台级新增**，不是某个命令的细节。
2. **MULTILEADER 不是一等实体**：导入展平为 `Compound([引线折线, 箭头 Mesh, 狗腿 Line, Text, …])`。
   而**导出把 `Compound` 摊平** ⇒ 创建出的引线标注存盘会变成散线 + 文字，**实体类别与语义全丢**。
   这是 P5a（标注）、P6a（填充）之后**同一教训的第三次**。
3. **`Text` 不可变换**（`cad-domain`/`cad-db` 的 transform 拒绝 Text）⇒ 文字一旦能创建，`MOVE`/`COPY`/`ROTATE`
   立刻失败。**必须一并解决**，否则交付的是"画得上、动不了"的东西。
4. **`MTEXT` 被折叠成 `Text` 且未报告**：列、背景填充、行距样式、绘制方向被静默丢弃
   （`entity/convert.rs` 报 `Completeness::Complete`）⇒ **既有诚实性缺口**，与本 ADR 的"声明账本"标准不符。
5. **无 MLEADERSTYLE 表**：`cad-db` 只有 `DimStyle`（P5b 先例）与文本 `Style`；导入只在内存里临时读 acadrust 的
   `MultiLeaderStyle`，**不存储**。创建需要一个可解析的默认样式。
6. **DXF 结构复杂且官方文档薄**：嵌套段语法（`300 CONTEXT_DATA{ … 302 LEADER{ … 304 LEADER_LINE{ … 305 } 303 } 301 }`）
   **不在 DXF 参考里**（仅实现共识）；`90` 是**属性覆盖掩码**（写 0 会让实体值被样式默认值取代）；
   `MLEADERSTYLE` 是**符号表记录**，`MLEADER`/`MLEADERSTYLE` 还需 `CLASSES` 条目。
7. **acadrust 0.6.5 写入器可用**（已核实源码）：`write_multileader` 写嵌套段、`write_multileader_style` 写样式记录、
   **`90` 掩码会写出**（`property_override_flags.bits()`）；`CLASSES` 由 `document.classes` 生成且**默认为空**
   ⇒ 我们必须自行插入两个 `DxfClass`。
8. **无 MULTILEADER 夹具**：现有测试全是内存构造的合成实体；`compatibility.md` **没有 MULTILEADER 行**。

---

## 决策 A：文本输入通道（P7a，**新增平台能力**）

### A.1 形态

- **`DrawToolKind::requires_text() -> bool`**：为 `true` 的工具在**点捕获完成后**进入"待文本"状态。
- **新增 `InteractionRequest::SetToolText { text: String }`**：把文本放进工具状态（与 `push_point` 同级），
  **不是** `ToolValue` 的第四种（参数是数值/闭集，文本是**内容**）。
- **`InputKind` 不变**：文本不是指针事件；宿主在用户提交时发 `SetToolText`。
- **载荷**：`CommandPayload::{Text, MLeader}` 各带 `String`（`text`）；点部分沿用 `Points`。
- **就绪条件**：`is_ready()` 对需要文本的工具要求"点数齐 **且** 文本非空"。

### A.2 UI（**诚实声明**：不是 AutoCAD 的就地编辑器）

- AutoCAD 在**画布内**开 MTEXT 就地编辑器；本仓库的 React 宿主**无法在原生 CAD 分区上绘制/嵌入 DOM 编辑器**
  （ADR-0002：原生分区无子窗口、DOM 不得浮于其上）。因此采用**状态栏/命令行文本输入**：
  复用既有 `CommandLine.jsx`，在"待文本"时切换为**文本提示模式**，Enter 提交并 `SetToolText`。
- **必须遵守既有规则**：`isTyping(event.target)` 的标记、`InputPolicy::shortcuts_suppressed`、
  `EscapeAction::DismissTextFocus`（输入框聚焦时画布快捷键不得触发）；IME `composing` 期间不得提交。
- **声明**：无画布内就地编辑；文字在提交后**一次性**出现，**无实时排版预览**（与 P5/P6 的"无真正预览"同源）。

### A.3 拒绝面

空/纯空白文本 ⇒ 按名拒绝；超长文本（上限待定，须有限且声明）；宿主在工具未处于"待文本"时发 `SetToolText`
⇒ 拒绝（不得静默缓存）。

---

## 决策 B：`Text` 的变换语义（P7a 一并修）

`SemanticGeometry::Text` 当前被 `transform_geometry` 拒绝 ⇒ 创建的文字无法 MOVE/COPY/ROTATE。改为：

- **平移**：位置。
- **旋转**：位置 + `rotation` 叠加。
- **均匀缩放**：位置 + `height × s` + `rotation` 不变。
- **非均匀缩放 / 镜像**：**整实体拒绝**（`Unsupported` + 诊断）——理由同 Dimension A.5：字形本身会被拉伸/镜像，
  而平面文字域无法表达真反射（与 P6b 的 `-Z` 文字结论一致）。
- **对齐**在旋转/缩放下保持不变（不重排）；声明。

---

## 决策 C：MultiLeader 一等表示（P7b）

```rust
SemanticGeometry::MultiLeader(Box<MultiLeaderGeometry>)

pub struct MultiLeaderGeometry {       // 字段私有；写入路径 new / from_import / transformed
    style: MLeaderStyleValues,         // 解析后的快照（见决策 D），`text_style: StyleId`
    style_name: String,                // 源样式名（往返）
    override_mask: u32,                // DXF 90；**必须原样保留**（见 §0.6）
    content_type: MLeaderContentType,  // MText | None（Block/Tolerance ⇒ Unsupported）
    text: MLeaderText,                 // string / location / rotation / height / width / attachment
    roots: Vec<MLeaderRoot>,           // ≤2 簇；每簇：landing point、dogleg vector、dogleg length
    lines: Vec<MLeaderLine>,           // 每条：vertices（箭头…朝内容）、path_type、索引
    plane: PlaneFrame,                 // 110/111/112 + 297
    display: Vec<SemanticGeometry>,    // 派生：引线折线 + 箭头 Mesh + 狗腿 Line + Text
}
```

- **必须保留的已解析量**（`§C.2` 的"不得省略"）：**引线顶点**、**落点**、**狗腿方向 + 长度**、
  **文本内容/位置/高度/样式/附着**、**箭头尺寸**、**landing gap**、**`90` 掩码**、**`172` 内容类型**。
- **显示**由唯一构造器派生（`display`）：引线 → `Polyline`；箭头 → 实心三角 `Mesh`（复用
  `dimension_synthesis` 的箭头构造，尺寸来自样式）；狗腿 → `Line`；文字 → `Text`（经 `text_style` 解析）。
  **狗腿端点 = 落点 + 长度 × 方向**（GDAL 逆推公式，标 **U-1 未查证**）。
- **变换**：与 Dimension 同构（平移/旋转/均匀缩放全部几何与定义点 + 重建 `display`；非均匀/镜像拒绝）。

---

## 决策 D：MLeaderStyle 表（P7b，`cad-db`）

- 新增 `MLeaderStyle` 表与 `standard_metric()` 默认（**先例 = P5b 的 `DimStyle`**），字段取 DXF MLEADERSTYLE 子集：
  引线类型（直/样条）、箭头尺寸、landing gap、dogleg 开关与长度、内容类型、文字样式、文字高度、文字颜色、
  附着（左/右 = 0–10 的子集）、最大引线点数、`Scale`。
- **创建从存储样式解析快照**（与 `resolve_dim_style` 同构），**不接受**隐式常量散布。
- **只实现"直引线"**：样条样式**可存储但创建拒绝**（显示/导入仍支持）。

---

## 决策 E：导出（P7b）

- 用 acadrust 的 `EntityType::MultiLeader` + `ObjectType::MultiLeaderStyle`：
  - 写 `CONTEXT_DATA`（内容基点、文本高度、箭头尺寸、landing gap、文本串、文本样式 handle、文本位置/方向/旋转/
    宽度/行距、颜色、附着、flow direction、plane）。
  - 写 `LEADER{}`（落点、狗腿向量、**每簇狗腿长度**）与 `LEADER_LINE{}`（**顶点逐点**）。
  - 写实体尾部：`340` 样式 handle、**`90` 掩码（原样）**、`170` 引线类型、`290/291`、`41`/`42`/`43`、`172` 内容类型、
    `343` 文字样式、`173`/`95`/`174`/`175`、`92`、`292`、`45`、`271`–`273`。
- **`CLASSES` 必须插入两条 `DxfClass`**（`MLEADER`/`AcDbMLeader`/`ACDB_MLEADER_CLASS` 与
  `MLEADERSTYLE`/`AcDbMLeaderStyle`/`ACDB_MLEADERSTYLE_CLASS`）——`document.classes` 默认空，
  **不插则实体可能被当成 proxy 或丢弃**（U-5，推断）。
- **`ExportReport` 逐条声明**：样式快照、`90` 掩码原样、含未建模属性时 `Converted`；
  块/公差内容、多簇/多样条、打断、背景/分栏 → 若源文件有则**保留存储并声明**，不静默丢。
- **不写**"计算后的文字排版"（AutoCAD 打开时重算，与 HATCH 同一原则）。

---

## 决策 F：创建命令（P7c）

### F.1 `TEXT`（P7a，最小消费者）

- 流程：**插入点 → （高度参数，缺省取样式）→ 文本**；`PointArity::Exact(1)` + `requires_text`。
- 插入 `SemanticGeometry::Text`（`style` = 当前文本样式，`h_align/v_align` 取样式默认），经单一漏斗、
  一条事务/一步撤销/活动图层；`type_key`/`class_name` = **`AcDbText`**。
- **声明**：无宽度因子/倾斜/字距/背景；单行；`MTEXT` 的列/背景/行距仍不支持（见决策 H）。

### F.2 `MLEADER`（P7c）

- 流程：**箭头位置 → 落点 → 文本**（`Exact(2)` + `requires_text`）；**直引线、单簇、单引线、2 顶点**。
- 狗腿/落点间隙/箭头尺寸/文字高度**全部来自存储样式**（决策 D）；`content_type = MText`。
- **不提供** `Options` 子提示（引线类型/内容类型/最大点数/角度约束）——**声明**为未实现，
  且 UI 不得暗示可切换。
- 经单一漏斗插入**一个** `AcDbMultiLeader`；一条事务/一步撤销/活动图层。
- **无就地编辑器、无实时预览**（决策 A.2）。

### F.3 拒绝面（全部具名、且不开事务）

空/空白文本、超长文本、样条样式、块/公差内容、非 `+Z` 平面、缺活动图层、Viewer、
`SetToolText` 在非待文本状态下到达、点捕获未完成即提交文本。

---

## 决策 G：匹配点清单（照 ADR-0017 A.8 / ADR-0018 决策 E）

**通配符臂**（不补就静默错）：`cad-import-acadrust/src/support.rs`、`cad-spatial/src/pick/{hit,bounds}.rs`、
`cad-app/src/{select_box,select_all,selection}.rs`、`cad-measure/src/snap/{generate,resolve}.rs`、
`cad-platform/src/{font_keys,images}.rs`、`cad-import-acadrust/src/{geometry,entity/push}.rs`、
`cad-proxy/src/lib.rs`、`cad-geometry/src/intersect.rs`、`cad-app/src/app_drawing/explode.rs`
（**块内 MultiLeader 的拒绝必须保持**，顶层 EXPLODE 按现有 `Compound` 口径**分解**）。

**穷尽臂**：`cad-db/src/{bounds,transform,validate}.rs`、`cad-geometry/src/{tessellate,vector,engine}.rs`、
`export/entity.rs`（`kind_name`）、`app_drawing/helpers.rs`（`class_name` = `"AcDbMultiLeader"`）、
`cad-representation/provider/default.rs`。

**必须决定的行为**：`transform_geometry`（同 Dimension）；`EXPLODE`（分解，非拒绝）；`STRETCH`
（沿用 `compatibility.md` 既有口径：部分包围不动 + 诊断、完全包围整移）；捕捉（引线顶点是捕捉点）；
拾取（引线/箭头/文字任一被点中即选中整个实体）。

---

## 决策 H：既有诚实性缺口（P7a 一并修）

`MTEXT` 被折叠为 `Text` 且报 `Complete`，但**列、背景填充、行距样式、绘制方向被静默丢弃** ⇒ 改为：
能保留的**保留**（行距因子、绘制方向若域可承载），不能保留的**显式 `Partial` + 具名原因**。
**不得**再把"丢了一半"报成 `Complete`。

---

## 决策 I：未查证 / UNVERIFIED（能力表逐条标注）

- **U-1 狗腿端点公式**：`端点 = 落点 + `长度` × 方向`（GDAL 逆推），Autodesk **未给几何定义**。
- **U-2 landing gap 的作用方式**：推断为**推开文字**而非**缩短引线**；未述。
- **U-3 文字附着 0–10 → 我们的 3×3 对齐映射**：ObjectARX 枚举可查，但**与 MTEXT 附着的对应**无官方文档。
- **U-4 最小有效 MULTILEADER**：本 ADR 与规格研究给的是**推断**，**未在 AutoCAD 中打开验证**。
- **U-5 `CLASSES` 条目是否强制**：缺失后果（proxy / 丢弃 / audit 修复）**未验证**；按"必须写"处理。
- **U-6 `Maxpoints` 与"最后拾取点 → 落点"的对应**：未述。我们**只产 2 点**，规避该歧义。
- **U-7 `142`/`45`/`40` 的 scale 语义**：按**普通乘数**建模并声明。
- **U-8 `MLEADERSTYLE` 记录是否必需**：按"必需"处理（真实文件都有 `Standard`）。
- **U-9 `170`/`172` 在实体尾部 vs 样式记录的含义**：Autodesk 两张表**互相矛盾**；按**实体表（与真实文件一致）**实现。
- **U-10 `-MLEADER` 不存在**（无官方页 + 命令词典无此项）⇒ **不实现**该别名。

---

## 决策 J：分阶段与验收

| 子阶段 | 内容 | 验收 |
|---|---|---|
| **P7a** | 文本通道（`SetToolText` + UI 文本提示模式 + IME/快捷键规则）+ `Text` 变换 + `TEXT` 创建 + MTEXT 诚实性修复 | 通道契约测试（含 IME/快捷键不改行为）、`Text` 变换四情形、TEXT 创建数值 + 拒绝面 + 导出往返、MTEXT `Partial` 原因。**真机**：输入文本 → 文字出现 → MOVE 后仍在正确位置 |
| **P7b** | MultiLeader 一等表示 + 导入迁移 + `MLeaderStyle` 表 + **真导出**（含 `CLASSES`） | 往返保全（顶点/落点/狗腿/文本/附着/掩码）、导出组码断言（含 `90` 掩码、`CLASSES` 两条）、显示等价、匹配点清单回归 |
| **P7c** | `MLEADER` 创建（2 点 + 文本，直引线/单簇/2 顶点） | 数值几何 + 拒绝面 + 导出往返（**首次检验"创建的"引线标注**）+ 真机像素判据 |

### 证据边界

沿用既有：合成 DOM 指针事件（**非真实鼠标**）+ **本次新增的合成文本事件**（经 `SetToolText`，**非真实键盘/IME**）；
`cad_status` **无几何通道** ⇒ 真机只证"提交成功 + 计数 + 内容存在"；几何正确性由契约测试与本 ADR 的数值断言钉住；
截图仅含应用窗口自身；单次有界本地运行。

### 文件计划与 i18n

`cad-domain/src/multileader.rs`（新）、`cad-db/src/mleader_style.rs`（新）、`cad-app/src/app_drawing/{text,mleader}.rs`（新）、
`cad-import-acadrust/src/export/multileader.rs`（新）；`cad-ui-core/src/session/{commands,status}.rs`、
`protocol.rs` 增量。`cad-app/src/lib.rs` 变体增加（注意 `lib.rs` 已接近 1000 行）。
i18n：P7a 需 `draw.kind.text`/`tool.draw.text`/`tool.prompt.text.{pick_point,type_text}`/`status.tool_text` 等；
P7c 需 `draw.kind.mleader`/`tool.draw.mleader`/`tool.prompt.mleader.{arrow,landing,type_text}`。
