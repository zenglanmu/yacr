# ADR 0019：引线标注族（MULTILEADER）——文本输入通道、一等表示、真实导出与创建子集（P7）

- 状态：**提案（Proposed）**，设计门禁 **SG-P7 attempt 1 = APPROVE-WITH-FIXES**（B1–B5 与全部非阻塞项已并入本修订；
  待 attempt 2 复核）。本文件是 P7 的**唯一设计权威**；实施进度另起小节追加，**不改写决策文本**。
- 范围：`crates/cad-ui-core`（wire protocol / 文本通道 / 命令选择子 / 状态 / i18n）、`crates/cad-app`（TEXT 与
  MLEADER 命令、工具、文本状态、input policy）、`crates/cad-domain`（MultiLeader 变体；`Text` 变换）、
  `crates/cad-geometry`（`engine.rs` 的 Text 面与 `transform_geometry` 对齐）、`crates/cad-db`（Text 变换校验）、
  `crates/cad-import-acadrust`（导入迁移 + 导出）、`crates/cad-representation`、React 宿主 UI。
- **不**覆盖：样条引线、块 / 公差内容引线的**变体化**（见决策 C.3）、多引线簇/`MLEADERCOLLECT`/`MLEADERALIGN`/
  `MLEADEREDIT`、注释性缩放的**应用**、背景遮罩、分栏、引线打断、`QLEADER`/`LEADER`/`TOLERANCE`、
  关联性、**画布内就地文字编辑器**、`-MLEADER`（**该命令不存在**，U-10）。
- 相关：`CAD_IMPLEMENTATION_SPEC.md` §3.2；ADR-0017 / ADR-0018（一等实体 + 真实导出 + 声明账本 + 匹配点清单）；
  ADR-0010 `:27`（**不允许跨 CAD 分区的 DOM 浮层**）；`docs/compatibility.md`（**当前无 MULTILEADER 行**为已知欠账）。

---

## 0. 事实与约束（勘察 + 门禁核实）

1. **没有字符串通道**：`ToolValue` 只有 `Number|Flag`；`InputKind` 只有指针/滚轮/尺寸；`InteractionRequest` 无字符串；
   `CadToolParams` 全数字/布尔；`cad_command(String)` 只收**命令名**（归一化 + 白名单）。
   **且 `apps/app-linux/src/host/commands.rs` 已明确记录"拒绝从命令行载荷解析文本"** ⇒ 必须新增**专用 IPC**。
2. **MULTILEADER 非一等**：导入展平为 `Compound`；导出把 `Compound` 摊平 ⇒ 创建的引线标注存盘会失去实体类别
   （P5a、P6a 之后第三次）。
3. **`Text` 不可变换**，但 `engine.rs` 的**捕捉面**已经变换 Text（`height × s`、`rotation + Δ`、对齐不变）**且无镜像/
   非均匀守卫** ⇒ 两个变换面语义不一致（ADR-0017 A.5 的教训）。
4. **两块既有行为建立在"Text 不可变换"之上**：`compatibility.md` 的 **STRETCH 整命令拒绝 TEXT**、
   **EXPLODE of INSERT 含 Text 成员整命令失败**。Text 一可变，两者都会**静默改变**，必须更新文档并加钉。
5. **`MTEXT` 折叠成 `Text` 且报 `Complete`**，而列/背景/行距样式/绘制方向被静默丢弃 ⇒ 既有诚实性缺口。
6. **无 MLEADERSTYLE 表**；导入仅在内存临时读 acadrust `MultiLeaderStyle`。
7. **DXF 结构官方文档薄**：嵌套段语法不在 DXF 参考；`90` 是**属性覆盖掩码**；`MLEADERSTYLE` 是符号表记录；
   `MLEADER`/`MLEADERSTYLE` 需 `CLASSES` 条目（真实文件 group 1 为 **`MULTILEADER`**，非 `MLEADER`）。
8. **acadrust 0.6.5 写入器可用**（已核实）：`write_multileader`（含嵌套段）、`write_multileader_style`、
   **`90` 掩码会写**；`CLASSES` 由 `document.classes` 生成——**2026-10-20 更正：并非默认空**，
   `CadDocument::new()` 已预注册约 260 条，**含本族**（见下方 P7b-2 实施小节对 U-5 的勘误）；
   `DxfClass::new_entity` /
   `DxfClassCollection::add_or_update` **是公开 API** ⇒ 可插入两条 class 而**不改 acadrust**。
9. **无 MULTILEADER 夹具**；现有测试全为内存构造的合成实体。

---

## 决策 A：文本输入通道（P7a）

### A.1 形态（**终端语义**，门禁建议）

- **`DrawToolKind::requires_text() -> bool`**；**`InteractionRequest::SetToolText { text }`**。
- **`SetToolText` 是终端的**：文本是**最后一步**。宿主提交文本后，若 `is_ready()`（点数已齐）则**立即完成命令**，
  **不再要求第二次 Confirm**。这样从根上消除"设两次文本""两次 Enter"的歧义：
  - 文本在**未待文本**状态到达 ⇒ **按名拒绝**（不缓存）；
  - 点未齐时到达 ⇒ 拒绝（F.3）；
  - 提交空/纯空白 ⇒ 拒绝；
  - **Esc 一次** = `DismissTextFocus`（仅失焦，工具仍待文本）；**Esc 两次** = `Cancel`（重置工具）。
- **专用 IPC**：新增 **`cad_set_tool_text`**，与 `cad_set_tool_parameter` **同型**（有界队列准入、未知/越界拒绝），
  **绝不复用 `cad_command`**。
- **长度上限**：**1024 字符**（有限且声明；超出按名拒绝）。

### A.2 状态投影

- `CadToolStatus` 新增 **`awaiting_text: bool`**（或等价的 `prompt_phase`）。**不得**让 React 从 `key` + 点数**推断**
  ——`ready` 目前把"缺点"与"缺文本"混为一谈，推断会与 Rust 的 `requires_text()` 漂移。

### A.3 UI（**诚实声明**）

- AutoCAD 在**画布内**开就地编辑器；本仓库 React 宿主**无法**在原生 CAD 分区上嵌入 **DOM** 编辑器（ADR-0010 `:27`）——就地编辑**不交付**（DOM 路线被几何证据否决、原生 GTK 路线被决定否决，见 [ADR-0020](0020-in-canvas-text-editing.md)）。
  采用**命令行/状态栏文本输入**：待文本时 `CommandLine.jsx` 切为文本模式，Enter 提交并 `SetToolText`。
  这是**传统 `-TEXT` 工作流**，可用但**不是** AutoCAD 的就地编辑。
- 必须遵守既有规则：`isTyping(event.target)`、`InputPolicy::shortcuts_suppressed`、`EscapeAction::DismissTextFocus`；
  IME `composing` 期间不提交。**声明**：无实时排版预览。

---

## 决策 B：`Text` 的变换语义（P7a，**两个变换面一起对齐**）

- **平移**：位置。**旋转**：位置 + `rotation` 叠加。**均匀缩放**：位置 + `height × s`。
- **非均匀缩放 / 镜像**：**整实体拒绝**（`Unsupported` + 诊断）——平面文字域无法表达真反射与拉伸字形。
- **两面对齐（门禁点名）**：`engine.rs` 的捕捉面**也必须**加上同样的镜像/非均匀拒绝，否则两个变换面对同一输入
  给出不同结果。**ADR-0017 A.5 的"两变换面语义一致"在此照搬**，并有**跨面一致性测试**（同一变换分别经
  `engine.rs` 与 `transform_geometry`，结果必须相同）。
- **B.2 两块依赖"Text 不可变"的既有行为：决定**保持行为不变、只更正理由**（attempt 2 的 F2/F3）：
  - **STRETCH 选中 TEXT**：**仍然整命令拒绝**（`stretch.rs` 的守卫保留）；但守卫的**理由**不再成立，
    改为"TEXT 的相似变换语义受限（见 B）"，并**由测试钉死"仍然整命令拒绝"**。
  - **EXPLODE of INSERT 含 Text 成员**：**仍然整命令失败**，但改为**显式**（`push_produced` / `child_refusal`
    的具名拒绝），**不再**依赖 `transform_geometry` 拒绝 Text 这一副作用。
  ⇒ 因此 **§0.4 的措辞更正**：Text 一变，这两处**本来会**静默改变；本 ADR **决定保持二者行为不变**，
  所以**本相不对这两条产生行为变更**——但必须**显式化理由**并由测试钉住。

---

## 决策 C：MultiLeader 一等表示（P7b）

```rust
SemanticGeometry::MultiLeader(Box<MultiLeaderGeometry>)

pub struct MultiLeaderGeometry {          // 字段私有；写入路径 new / from_import / transformed
    style: MLeaderStyleValues,            // 解析快照（含 text_style: StyleId）
    style_name: String,                   // 往返；导出按名去重生成 MLEADERSTYLE
    override_mask: u32,                   // DXF 90，**原样保存**
    content: MLeaderContent,              // **只有 Text**（见 C.3）
    text: MLeaderText,                    // string / location / rotation / height / width / attachment
    roots: Vec<MLeaderRoot>,              // **不限 2 簇**（门禁）；创建恒为 1
    lines: Vec<MLeaderLine>,              // 每条：vertices（箭头…朝内容）、path_type、index、
                                          //   **per-line override_flags / arrowhead_size / arrowhead_handle**
    reserved: MLeaderReserved,            // **不透明往返袋**（见 C.2）：breaks、background/columns、
                                          //   未建模的 per-line 覆盖位
    plane: PlaneFrame,                    // 110/111/112 + 297
    display: Vec<SemanticGeometry>,       // 派生
}
```

### C.1 必须保留（"不得省略"）

引线**顶点**、**落点**、**狗腿方向 + 长度**、**文本内容/位置/高度/样式/附着**、**箭头尺寸**、**landing gap**、
**`90` 掩码原样**、**`172` 内容类型**、**每簇 `leader_index`(90) 与每线 `index`(91)**。

### C.2 "保留存储"必须真的存（门禁 B1）

决策 E 承诺"块内容/打断/背景/分栏/覆盖位**保留存储并声明**"——那就必须有字段。**用不透明 `reserved` 袋**
（ADR-0018 渐变 `reserved`/`angle_raw` 的先例）承载：引线打断点、背景与分栏参数、未建模的 per-line 覆盖位。
**分区规则（attempt 2 的 F5）**：`reserved` **只存源文件的逐字数据**，**不得**存合成值，**不得**与已建模字段重叠；**每个源组码恰好导出一次**（由测试断言"无重复写出"）。`MLeaderLine.override_flags` 是**已建模**字段（逐字保存），因此**不**再进 `reserved`。
**不允许**"声称保留但无字段"。

### C.3 块 / 公差内容：**不迁移**（门禁 B2 —— 避免静默渲染回归）

今日**块内容引线会作为 `Insert` 子项渲染**（`dimension/multileader.rs` 的 block 分支）。若把整类迁入变体并标
`Unsupported`，就是**静默的渲染回归**。因此：

- **只有 MText 内容（`172 = 2`）与无内容（`172 = 0`）迁入 `MultiLeader` 变体**；
- **块 / 公差内容保持今日的 `Compound` 展平路径**（`Partial`，原因具名），**不变**；
- **声明**：这些实体仍**不能**以 `MULTILEADER` 往返（导出仍摊平）——**与今日相同**，不是新损失；
  变体化为未来工作（需 `MLeaderContent::Block{ insert }` + 块句柄导出）。
- **按内容类型分叉的**表面（attempt 2 要求逐条列出）：**导出往返**（变体→`MULTILEADER`；块/公差→摊平）、
  **STRETCH**（两者都整命令拒绝 ⇒ **不分叉**）、**导入 `Completeness`**（见下）。
- **迁移后变体的导入 `Completeness`**：背景/分栏/打断等**存进 `reserved` 但不渲染** ⇒ 按决策 H 的同一原则报
  **`Partial` + 具名原因**，**不得**报 `Complete`（否则 MTEXT 的诚实性缺口会在上一层复发）。

### C.4 显示派生与狗腿公式（门禁 B4 —— 以**代码现状**为准）

- **狗腿端点采用今日实现的公式**（`multileader.rs`）：`reach = (landing_distance − landing_gap).max(0)`，
  `end = <该引线的最后顶点> + 方向 × reach`；**即 landing gap 会缩短狗腿，基点是引线顶点而非落点**。（**实施更正**：代码用 `find_map` 取**第一条有末顶点的引线**，而非"最后一条"引线；以**代码现状**为准以满足显示等价。）
  **并须连同这些细节一起照搬（attempt 2 的 F8）**：`landing_distance` 缺省时走 `default_length` 回退链，且 **`reach < 1e-9` 时完全不画狗腿**。
  ⇒ **迁移不改渲染**（这是本决策的最大价值：P7b **不产生像素变化**）。
- **U-1 重新归因**：`落点 + 长度 × 方向` 是 **GDAL 逆推**（**第三方证据**），与本实现不同；记为
  **U-1'（第三方证据 vs 我们的实现，冲突未解，需真实夹具）**。
- **U-2 措辞更正**：landing gap 在本实现中**缩短狗腿**；"推开文字"是**第三方推断**，不采用。
- 显示 = 引线 `Polyline` + 箭头实心 `Mesh`（尺寸来自样式）+ 狗腿 `Line` + `Text`，由**唯一构造器**派生。

### C.5 变换（门禁 B3 —— 避免掩码与几何自相矛盾）

- **平移 / 旋转**：全部几何 + 定义点 + `display` 重建。
- **缩放（含均匀）/ 镜像 / 非均匀**：**拒绝**——注意 `transform_selection` 是**全有或全无**，所以实际表现是**含 MultiLeader 的选择整命令失败**（连其它实体一起）。
  **理由**：均匀缩放会改文字高度/箭头尺寸/狗腿长度，而 `90` 掩码若**未置**对应位，AutoCAD 加载时会从样式
  **重新解析**这些值 ⇒ 重开后的实体与我们显示的不一致。拒绝缩放**规避**该陷阱，并**显式声明**。
- **未来路径（attempt 2 的 Q4）**：缩放并非几何上不可行——正确做法是**同时置位**对应覆盖位（`TEXT_HEIGHT`、
  `ARROWHEAD_SIZE`、`LANDING_DISTANCE`、`LANDING_GAP`）并记入 `ExportReport`；本相**选择拒绝**作为取舍。
  **注意**：该规避只覆盖**我们施加的**变换；源文件本身在 U-11 下不一致时**仍会**往返进同一陷阱（见 J 的断言作用域）。

---

## 决策 D：样式（P7b 最小化，P7c 再建表）

- **P7b 只做**：每个实体携带 `MLeaderStyleValues` 快照 + 导出时按 `style_name` **去重生成** `MLEADERSTYLE` 记录
  （先例 = DIMSTYLE 的 `add_or_replace`），`Standard` 为首个。
- **P7c 再建 `cad-db` 的 `MLeaderStyle` 表**（`standard_metric()` + `resolve` 先例 = `DimStyle`），供创建解析默认；
  `resolve_dim_style` 的对称物。**P7b 不引入表**，避免无消费者。

---

## 决策 E：导出（P7b）

- 用 acadrust `EntityType::MultiLeader` + `ObjectType::MultiLeaderStyle` 写：`CONTEXT_DATA`（内容基点、文字高度、
  箭头尺寸、landing gap、文本串、文本样式 handle、文本位置/方向/旋转/宽度/行距、颜色、附着、flow direction、plane）、
  `LEADER{}`（落点、狗腿向量、**每簇狗腿长度**、`leader_index`）、`LEADER_LINE{}`（**顶点逐点**、`index`、per-line
  覆盖位/箭头）、实体尾部（`340`、**`90` 掩码原样**、`170`、`290/291`、`41/42/43`、`172`、`343`、`173/95/174/175`、
  `92`、`292`、`45`（**尾部语义 = scale_factor**）、`271`–`273`）。
- **`CLASSES`：由 acadrust 的默认注册保证（U-5 已勘误）**。`acadrust-0.6.5/src/classes/mod.rs:338` 的默认表
  已含 `("MULTILEADER", "AcDbMLeader", 1025, "ACDB_MLEADER_CLASS", true)` 与 `MLEADERSTYLE` 对应项，
  写 `CLASSES` 段时自动带上。写入器**只在缺失时补**（防御路径用同一组取值）；group 1 **从不**是 `MLEADER`；
  写出的精确字符串与 flags **由测试钉住**（不依赖"上游默认"这一事实而不加断言）。
- **具名导出偏差**：`91/341/171`（引线颜色/线型/线宽）写默认值（导入解析但未应用）⇒ `ExportReport` 声明；
  `271`–`273` 在 **context 与尾部两处都写**（与 acadrust 一致）；`90` 掩码原样、**不擅自置位**（因为缩放已拒绝，
  见 C.5）。
- **不写**"计算后的文字排版"（AutoCAD 打开时重算，与 HATCH 同一原则）。

---

## 决策 F：创建命令（P7a / P7c）

### F.1 `TEXT`（P7a）

- 流程：**插入点 → （高度参数，缺省取样式）→ 文本**；`PointArity::Exact(1)` + `requires_text`；`SetToolText` 终端完成。
- 插入 `SemanticGeometry::Text`（`style` = 当前文本样式、对齐全取样式默认），单一漏斗、一条事务/一步撤销/活动图层；
  `type_key`/`class_name` = **`AcDbText`**。
- **声明**：单行、无宽度因子/倾斜/字距/背景；MTEXT 的列/背景/行距不支持（决策 H）。

### F.2 `MLEADER`（P7c）

- 流程：**箭头位置 → 落点 → 文本**；`Exact(2)` + `requires_text`；**直引线、单簇、单引线、2 顶点**。
- 狗腿/landing gap/箭头尺寸/文字高度全取样式（决策 D 的表）；`content = Text`。
- **无 `Options` 子提示**（引线类型/内容类型/最大点数/角度约束）⇒ **声明**，UI 不得暗示。
- 单一漏斗插入**一个** `AcDbMultiLeader`；一条事务/一步撤销/活动图层。
- **创建必须有文本**（attempt 2 的 Q3）：AutoCAD 允许"在文本前退出"得到**无内容** MULTILEADER，而本设计**不提供**该路径（空文本拒绝、Esc 两次取消）⇒ 明确声明：**无内容 MULTILEADER 仅导入/显示/导出**，**不创建**。理由：为它引入"无文本"标志会把"无文本确认"的歧义重新带回来，而收益为零。

### F.3 拒绝面（全部具名、且不开事务）

空/纯空白/超 1024 字符文本、`SetToolText` 在非待文本状态、点未齐即提交文本、**`awaiting_text` 期间收到 `Confirm`（attempt 2 的 F4 —— 否则命令会从后门不带文本完成）**、样条样式、块/公差内容、
非 `+Z` 平面、缺活动图层、Viewer、MultiLeader 的缩放/镜像/非均匀变换。

---

## 决策 G：匹配点清单（照 ADR-0017 A.8 / ADR-0018 决策 E）

**通配符臂**：`cad-import-acadrust/src/{support,geometry,entity/push}.rs`、`cad-spatial/src/pick/{hit,bounds}.rs`、
`cad-app/src/{select_box,select_all,selection}.rs`、`cad-measure/src/snap/{generate,resolve}.rs`、
`cad-platform/src/{font_keys,images}.rs`、`cad-proxy/src/lib.rs`、`cad-geometry/src/intersect.rs`。

**必须改动（attempt 2 的 F3）**：`cad-app/src/app_drawing/explode.rs`（显式按 `type_key` 拒绝 MultiLeader，顶层与块内都算）与 `cad-app/src/app_drawing/stretch.rs`（显式拒绝 + 更正理由）。这两处**不是**通配符臂。

**穷尽臂**：`cad-db/src/{bounds,transform,validate}.rs`、`cad-geometry/src/{tessellate,vector,engine}.rs`、
`export/entity.rs`（`kind_name`）、`app_drawing/helpers.rs`（`class_name` = `"AcDbMultiLeader"`）、
`cad-representation/provider/default.rs`。

**必须决定的行为**：
- **EXPLODE（门禁 B5）**：今日**顶层拒绝** `AcDbMultiLeader`（`refuse_compound_type_key`）⇒ **保持拒绝**（**不是**
  改成分解，那会是行为变更），且**块内**拒绝改为**显式按 `type_key` 拒绝**（不能再依赖 `transform_geometry` 的
  Compound 拒绝——MultiLeader 一旦有 `transformed` 臂，该依赖就消失）。`compatibility.md` 的 EXPLODE 行相应说明。
- **STRETCH（attempt 2 的 F1）**：**整命令拒绝**，即**保持今日行为**——MultiLeader 今日作为 `Compound` 在**部分与完全包围两种情形下都使 STRETCH 失败**，所以"沿用既有口径"是**误述**（`compatibility.md` 的通用规则是**逐顶点移动**，"部分包围不动 + 诊断"只属 SPLINE/ARC/HATCH/INSERT 专条）。**不新增** `Unchanged` 臂，以免 STRETCH 行为**按内容类型分叉**；需在 `stretch.rs` **显式按 `type_key` 拒绝**。
- **变换**：见 C.5。**捕捉**：引线顶点/落点/狗腿端点是捕捉点。**拾取**：引线/箭头/文字任一命中即选中整个实体。

---

## 决策 H：既有诚实性缺口（P7a，**仅诚实性**）

`MTEXT → Text` 今日报 `Complete`，但列/背景填充/行距样式/绘制方向被静默丢弃。P7a **只做**：改为 **`Partial` +
具名原因**（`text.background_unsupported`、`text.columns_unsupported`、`text.line_spacing_style_unsupported`、
`text.drawing_direction_unsupported`）。**不**新增域字段（域内 `Text` 无这些字段）——字段化留给未来。
任何测试**不得**再断言 `Complete`。

---

## 决策 I：未查证 / UNVERIFIED（能力表逐条标注）

- **U-1' 狗腿端点**：GDAL 逆推公式（**第三方证据**）与**本实现**（`reach = (landing_distance − landing_gap)`，
  基点为最后顶点）**冲突未解** ⇒ 需真实夹具；本相**以现状为准**（不改渲染）。
- **U-2 landing gap 的作用**：本实现**缩短狗腿**；"推开文字"仅为第三方推断。
- **U-3 文字附着 0–10 → 3×3 映射**：ObjectARX 枚举可查，与 MTEXT 附着的对应**无官方文档**。
- **U-4 最小有效 MULTILEADER**：**推断**，**未在 AutoCAD 打开验证**。
- **U-5 `CLASSES` 是否强制** 与 `ProxyFlags`/`instance_count` 取值：**已勘误（2026-10-20）**——原判断"默认空、必须插入"**错误**；
  acadrust 0.6.5 的 `CadDocument::new()` **已预注册**（含本族）。剩下的不确定只有"**AutoCAD 是否接受**"，
  仍**无真实文件/真实 AutoCAD 验证**（见下）。
- **U-6 `Maxpoints` 与"最后拾取点 → 落点"对应**：未述；我们只产 2 点以规避。
- **U-7 `142`/`45`/`40` 的 scale 语义**：按普通乘数建模（尾部 `45` = scale_factor）。
- **U-8 `MLEADERSTYLE` 记录是否必需**：按必需处理。
- **U-9 `170`/`172` 尾部含义**：Autodesk 两张表**互相矛盾**；按**实体表**实现（acadrust 注释称已对 R2018 样本验证，
  但**非我们独立验证**）。
- **U-10 `-MLEADER` 不存在**（无官方页 + 命令词典无此项）⇒ 不实现。
- **U-11（新）掩码 vs 样式优先级**：AutoCAD 加载/编辑时是否用样式**重解析**掩码未置位的属性 —— **未验证**；
  这正是决策 C.5 **拒绝缩放**的理由。
- **U-12（新）`CONTEXT_DATA` 41/140/145 与样式的关系**：是否以 context 为准 —— 未验证。
- **U-13（新）`271`–`273` 在 context 与尾部两处的优先级**：未验证；**两处都写**以规避。

---

## 决策 J：分阶段、验收、文档债与文件计划

| 子阶段 | 内容 | 验收 |
|---|---|---|
| **P7a** | 文本通道（`cad_set_tool_text` + `awaiting_text` + 终端语义 + UI 文本模式）+ `Text` 变换（两面对齐）+ `TEXT` 创建 + MTEXT 诚实性 | 通道拒绝面与幂等矩阵；IME/快捷键**行为不变**；Text 变换四情形 + **跨面一致性**；TEXT 创建数值 + 导出往返；MTEXT `Partial` 具名原因；**STRETCH/EXPLODE 两行文档更新并加钉** |
| **P7b** | MultiLeader 变体（仅 MText/无内容）+ 导入迁移 + 不透明 `reserved` + **真导出**（含 `CLASSES`）+ 样式记录去重 | **往返同一性**（顶点/落点/狗腿/文本/附着/**掩码原始位**，**逐字优先**）；**自洽断言只作用于自洽构造**（掩码未置的位，实体值 == 写出的样式记录值）——**不得**为满足自洽而规范化源文件的不一致数据；两个实体共用同名样式且值不一致时，**样式记录取首个**并在报告声明；**`CLASSES` 两条精确字符串**；**显示等价**（迁移前后 bbox 相同 ⇒ 证明未改渲染） |
| **P7c** | `cad-db` `MLeaderStyle` 表 + `MLEADER` 创建（2 点 + 文本，直引线/单簇/2 顶点） | 数值几何 + 拒绝面 + **导出往返（首次检验"创建的"引线标注）** + 真机像素判据 |

### 文档更正清单（门禁 e —— 本 ADR 必须自带）

`docs/compatibility.md`：**新增 MULTILEADER 行**（读取/显示/创建/导出 + 声明损失，并写明**含 MultiLeader 的缩放/镜像/ALIGN-scale 选择整命令拒绝**）；`:39` TEXT/MTEXT（MTEXT 的 `Partial` 原因）；`:214` STRETCH（TEXT 仍整命令拒绝，理由更新）；`:215` EXPLODE（MultiLeader 拒绝为显式）；`:178` ROTATE/SCALE 与 ALIGN 行（用户真正会撞到拒绝的地方，attempt 2 的 F6）。`docs/dxf-entity-coverage.md:32,72`
的 `Partial` 原因随变体化更新。

### 文件计划（门禁更正）

`cad-domain/src/multileader.rs`（新，**`lib.rs` 已 948 行** ⇒ 变体 + 匹配臂预算约 30 行，实现全部放新文件）、
`cad-app/src/app_drawing/{text,mleader}.rs`（新；**`lib.rs` 672 行**，安全）、`cad-import-acadrust/src/export/multileader.rs`
（新）、`cad-ui-core/src/protocol.rs`（**801 行**，`SetToolText` + 测试需谨慎）、`session/status.rs`（`awaiting_text`）、
`cad-db/src/transform.rs`（Text 臂）、`cad-geometry/src/engine.rs`（守卫对齐）。

### i18n

P7a：`draw.kind.text`、`tool.draw.text`、`tool.prompt.text.{pick_point,type_text}`、`status.tool_text.prompt`、
`status.tool_text.too_long`，以及**高度参数标签**（若 F.1 的高度走 `ToolParameter`：`status.tool_parameter.text_height`）。
P7c：`draw.kind.mleader`、`tool.draw.mleader`、`tool.prompt.mleader.{arrow,landing,type_text}`。两目录键集一致；
`cad-app` **不得**新增 CJK 字面量（基线只禁增长）。

### 证据边界

合成 DOM 指针事件 **+ 合成文本事件**（经 `cad_set_tool_text`，**非真实键盘/IME**）；`cad_status` 无几何通道 ⇒
真机只证"提交 + 计数 + 内容存在"；几何由本 ADR 的数值断言钉住；截图仅含应用窗口自身；单次有界本地运行。

### 已知欠账（门禁点名）

**没有真实 MULTILEADER 夹具**：U-1'/U-2/U-4/U-9 只能靠更多合成用例逼近，**不能**替代真实样本；
需要一份**授权可分发**的样本并记入 `fixtures/manifest`（provenance + SHA-256）。

---

## 实施状态（追加，不改写决策文本）

### P7a（文本通道 + `TEXT` + `Text` 变换 + MTEXT 诚实性，2026-10-20）

- **文本通道**：`SetToolText` **终端语义**（提交即完成，`ready && !awaiting_text`）；**专用 IPC
  `cad_set_tool_text`**（与 `cad_set_tool_parameter` 同型，**不走 `cad_command`**）；`awaiting_text` 用
  **专用 wire 名**；**1024 字符**上限（按 `chars()`）；拒绝面五类（空/超长/非待文本/点未齐/**待文本期间 Confirm**），
  全部具名且**不开事务**。
- **`Text` 可变换**：平移/旋转/均匀缩放；**镜像与非均匀拒绝**；**域侧与捕捉面同规则**
  （`cad-db/src/transform.rs` 与 `cad-geometry/src/engine.rs`），**跨面一致性有测试**。
- **决策 B.2 落实**：`STRETCH` 选中 TEXT 仍整命令拒绝、`EXPLODE` 含 Text 成员的 INSERT 仍整命令失败，
  **理由已更正**并有测试钉住（**无行为变更**）。
- **`TEXT` 创建**：`Exact(1)` + `requires_text` + `textHeight`；`AcDbText`；一条事务/一步撤销/活动图层。
- **MTEXT**：`Partial` + **四条具名原因**（列/背景/行距样式/绘制方向），不再误报 `Complete`。
- **UI**：命令行文本模式（1024 按 Unicode 标量值、IME 双信号、Esc 两步、`TEXT` 面板入口与字形）。
- **宿主默认行为**：CAD 表面**按设计对输入透明** ⇒ 右键落 WebView 并弹 WebKit 菜单（Reload 属状态分叉风险）；
  已抑制 `contextmenu`（输入框内保留）并拦掉 reload/zoom/print/`Ctrl`+wheel；**右键无动作**，只留
  `cadContextMenuRequest` 钩子（见 ADR-0020）。
- **文件规模**：`draw_tool/mod.rs` 曾 1079 行 ⇒ `DrawIntent` 迁至 `draw_tool/intent.rs`（266 行），`mod.rs` 744 行，
  **公开路径不变**。
- **门禁**：受影响 crate **2060 通过 / 0 失败**；clippy/fmt/architecture(27)/i18n(**542**)/file-size(**711**)；
  UI **185/185**。**NOT RUN**：全量 `cargo test --workspace`；跨平台；真机（本片未新增渲染几何）。
- **待办**：A+B 编辑入口（`TEXTEDIT` + 面板可编辑字段）、`textHeight` 的 UI 步进器、P7b。

### P7b-1（MultiLeader 一等表示 + 导入迁移 + 匹配点清单 + 显示等价，2026-10-20）

- **变体**：`SemanticGeometry::MultiLeader(Box<MultiLeaderGeometry>)`（`cad-domain/src/multileader.rs`），
  私有字段 + `new`/`from_import`/`transformed`；`override_mask` 原样、`reserved` 仅存**逐字源数据**
  （每个源组码只导一次，有测试）、环/线索引与 per-line 覆盖位保留。
- **显示等价（本片验收核心）**：派生显示是旧导入合成的**逐字搬迁**（同一 `Polyline`/`Spline`
  （degree `3.min(n-1).max(1)`、空 knots/weights）、同一箭头 Mesh、同一狗腿公式与 `Text`）⇒ **迁移不改变渲染**。
- **导入**：**仅 MText 与无内容**迁入变体；**块/公差内容仍走旧 `Compound`**（渲染不变，`Partial` + 具名原因）。
  变体路径的 `Completeness` 为 **`Partial` + 具名原因**（颜色/线宽覆盖、背景、分栏、打断、自定义箭头、非默认平面）。
- **变换**：平移/旋转；**缩放（含均匀）/镜像/非均匀 ⇒ 整选择 `Unsupported`**。
- **匹配点**：`EXPLODE` 顶层与**块内**均**显式按 `type_key` 拒绝**（不写任何东西）；`STRETCH` 显式整命令拒绝；
  `class_name = "AcDbMultiLeader"`、`kind_name = "MultiLeader"`；pick/bounds/snap 与等价 `Compound` 平价。
- **导出（临时）**：`export/entity.rs` 按旧 `Compound` 方式摊平 `display()`，记录 `export.converted.multileader`
  并注明由 P7b-2 替换；**尚未写 `MULTILEADER`**。
- **两处如实记录的偏离**：
  1. **`MLeaderStyleValues` 存的是"解析后的有效值"快照**，**不是**原始 MLEADERSTYLE 记录 ⇒ 掩码**未置**位但实体值
     与样式不同的原始值**未单独保留**。⇒ **P7b-2 不得声称这些字段逐字往返**；往返验收须比较**有效值**，
     且"自洽断言"在此语义下是**构造性成立**，不能证明原始字节相等。
  2. **狗腿起点**按代码取"**第一条有末顶点的引线**"（非 ADR 措辞的"最后一条"）——为满足显示等价以代码为准，
     ADR 正文已更正。
- **门禁**：受影响 crate **2177 通过 / 0 失败**；UI **200/200**；clippy/fmt/architecture(27)/i18n(**553**)/file-size(**728**)。
  **NOT RUN**：全量 `cargo test --workspace`；**无真实 MULTILEADER 夹具**（全部为合成）；真机/像素。

### P7b-2（真 `MULTILEADER` 导出 + `MLEADERSTYLE` 去重 + `CLASSES`，2026-10-20）

- **实体从域定义写，不写派生显示**：`export/multileader.rs` 用 `style + text + roots + lines + reserved` 组
  `CONTEXT_DATA` / `LEADER{}`（落点、狗腿向量、**每簇狗腿长度**、`leader_index`、附着方向、打断）/
  `LEADER_LINE{}`（**顶点逐点**、`index`、覆盖位、箭头尺寸/handle、颜色/线型/线宽、打断）/ 实体尾部；
  **`90` 掩码逐字写出**（含**零掩码**用例），**从不**重算。
- **`MLEADERSTYLE` 按名去重**（`Standard` 优先），同名不同快照时**取首个**并在报告中声明冲突；
  实体 `340` 指向所写记录。
- **U-5 勘误（重要）**：`CadDocument::new()` **已预注册** `MULTILEADER`（`AcDbMLeader`、proxy 1025、
  `ACDB_MLEADER_CLASS`、entity）与 `MLEADERSTYLE`（`AcDbMLeaderStyle`、4095、`ACDB_MLEADERSTYLE_CLASS`、非 entity）；
  因此 ADR 原文"必须插入两条 / 默认空"**不成立**。实现改为"**缺失才补**"，并且**仍然断言写出的精确字符串与 flags**
  （不因为上游默认就免于断言）。**group 1 从不写 `MLEADER`**。
- **块内放置（与 `Compound` 一致，不丢内容）**：刚性（平移/旋转）⇒ 写真 `MULTILEADER`；
  **缩放/镜像 ⇒ `MappingOutcome::Converted`**（`export.converted.multileader_flattened`）+ 摊平为显示图元。
  摊平复用与 `Compound` **同一个** `flatten_children`，每个子项记录**自己的**结果——例如镜像实例下的 `Text` 子项
  按域规则**单独**拒绝（`export.dropped.transform`，消息点名 `Text`），父项仍是 `Converted`。
  （**修正过的第一版曾整条 `Dropped`**，那会让缩放块内的标注从导出结果里**消失**，与其它图元类别不一致。）
- **具名导出偏差（`ExportReport` 全部声明）**——**声明范围只限"写入器能观察到的内容"，本文不再宣称"零丢失"**：
  1. `91/341/171`（引线颜色/线型/线宽）写默认——导入**解析但未应用**；
  2. **样式记录是"解析后的有效值"快照**（P7b-1 偏离 1）⇒ **掩码抑制的字段不是逐字往返**；
  3. 未建模的上下文取显式默认（文本色 ByBlock、flow Horizontal、对齐 Left、附着 Middle/Center、
     行距 `1.0`/`AtLeast`、样式 `142 = 1.0`）；`text_normal`/`text_direction` 由模型**派生**（`u×v`、`u·cos+v·sin`）；
  4. `content_base_point = text.location`（模型**无独立基点**）；
  5. **seeds** 未建模（`300` 空、`47` identity），且 **acadrust 读回把 `300` 映射进 `description`**（写读不对称）
     ⇒ 即便写也无法往返；
  6. per-line 箭头/线型 **handle 逐字写**，但**被引用对象未导出** ⇒ **悬空**（无对象可重映射，不伪造）；
  7. `271–273` 的**样式级**在 acadrust 中**无法表达**（只在实体主体写）；
  8. **解析但未建模的组被归一化为默认值**（**常设具名偏差**，`REASON_UNMODELLED_GROUPS`）：实体尾 `292` 文字框、
     `293` 标注比例、`294` 文字方向取负、`174` 文字角度类型、`178` IPE 内对齐、`295` 引线延伸到文字；
     上下文 `44` 文字边界高、`293` 自动字高、`295` 断词；
  9. **实体级 `342`（自定义箭头块）在导入侧具名**（**恢复 P7b 前的理由**）：
     `custom multileader arrowhead blocks are not expanded: the entity-level arrowhead block handle … (entity-tail 342) …`；
     与逐线 `341` **不一致**时**另加**一条具名理由（两种句柄都点名，**不静默择一**）。把 `342` 存进 `reserved` 是**后续**项；
  10. **尾部 `271` 由 root 0 的保留值派生**（源实体级 `text_attachment_direction` 未读）⇒ 源中两者不一致时会被归一化（已声明）。
- **往返验收**：合成 → 导出 → **acadrust 读回** → 再导入，比较**有效值**（掩码位、根/落点/狗腿/索引、
  per-line 覆盖、文本/附着/高度/位置、平面）；**不**断言掩码抑制字段的字节相等（数据不存在）。
- **门禁（编排者复跑）**：受影响 crate **2191 通过 / 0 失败**；UI **200/200**；clippy(`-D warnings`)、fmt、
  architecture(27)、i18n(**553**)、file-size(**732**)、fixture-manifest(**23**)。**NOT RUN**：全量
  `cargo test --workspace`；跨平台；真机/像素；**无真实 MULTILEADER 夹具**（全合成），**未在 AutoCAD 打开验证**
  ⇒ U-4（最小有效文件）与 U-5（AutoCAD 是否接受）**仍是推断**。

### P7c（`MLeaderStyle` 表 + `MLEADER` 创建，2026-10-20）

- **`cad-db` 的 `MLeaderStyle` 表**（照 `DimStyle` 的 `standard_metric()` + `resolve` 先例）：
  `MLeaderStyle { id, name, path_type, content_type, text_height, arrowhead_size, landing_gap, dogleg_length, text_style_name, left_attachment }`；
  `finish()` **始终 pin 一个 `Standard`**；按名解析**先精确、后忽略大小写**。
  **如实声明**：默认值是**常量**（`dogleg 0.36` / `landing gap 0.09` / `arrow 0.18` / `text height 0.18` / `Standard` 文本样式 /
  左附着 `1` / 直线路 / 内容 `MTEXT`），与 P7b-2 写入器的基础值一致，但**不是**真实/vendor 验证过的样式表
  （与 `TEXT` 的 2.5 同一类债务）；样式表里**未建模**的字段（颜色/线型/线宽、箭头块、块内容、绘制顺序、flags）**具名不存**。
- **`MLEADERSTYLE` 导入现已喂表**：`read_mleader_styles()` 遍历源 `MLEADERSTYLE` 对象并插入（id 按源顺序、同名 `Standard`
  覆盖 pin 行、文本样式 `342` 经 `TEXTSTYLE` 解析）。**此前只按实体临时解析并丢弃**——这是本轮补上的真实缺口。
- **`MLEADER` 创建**：**箭头位置 → 落点 → 文本**；`Exact(2)` + `requires_text`；**单簇 / 单引线 / 2 顶点 / 仅直线**；
  经 `MultiLeaderGeometry::created(style, …)` 单一漏斗插入**一个** `AcDbMultiLeader`，一条事务 / 一步撤销 / 活动图层，
  `type_key`/`class_name` = `AcDbMultiLeader`，掩码 `0`。
- **精确几何（已钉）**：箭头 `(0,0,0)`、落点 `(10,2,0)`、`Standard` 行 ⇒ 根 `landing (10,2,0)`、狗腿向量 `(1,0,0)`、长度 `0.36`；
  顶点 `[(0,0,0), (9.64,2,0)]`；狗腿线 `(9.64,2)→(9.91,2)`（`0.36 − 0.09` 缺口）；文字在落点、高度 `0.18`、
  附着点 **`2`（居中）**、左附着 `1`。
- **`CREATED_TEXT_ATTACHMENT_POINT = 2` 的真实语义（实施更正）**：它只是**水平**附着 ⇒ `TextAlignH::Center`；
  **垂直**来自所解析样式的 `left_attachment`（`Standard` 行 = `1`）⇒ `TextAlignV::Top`，
  `cad-representation::finalize_text` 对 `Top` 施加**一个字高的基线上移** ⇒ 文字渲染在**落点上方**（实测 37 px ≈ 1 字高）。
  即**复合锚是 top-centre，不是 middle-centre**（此前报告里的"居中"是错的，已更正）。
  与 AutoCAD 的复合附着码（U-3）**仍未验证**；若将来要中间锚，需要独立决定而不是改这一个常量。
- **修复一处既有潜伏 bug**：`MultiLeaderGeometry::new` 在 P7b-1 中**实际不可达**——它对 `from_import` 的**任何**
  理由都判失败，而 `build_display` **总是**返回"颜色/线宽覆盖未应用"这条**信息性**说明 ⇒ 创建路径永远失败。
  现在 `new` 只校验**结构性**缺陷（根/线/顶点/文本/空显示），`from_import` 语义**未改**。
- **拒绝面（具名、不写、不开事务）**：`mleader.payload`、`tool.text_empty`、`tool.text_too_long`（1024 `chars()`）、
  `tool.text_points_incomplete`、`tool.text_not_awaiting`、`tool.text_required`（待文本期间提交）、`mleader.style_missing`、
  `mleader.text_required`（无内容样式）、`mleader.content_unsupported`（块/公差/未知内容）、`mleader.path_type`
  （样条/不可见）、`mleader.points_not_finite`、`mleader.plane`（非 `+Z`）、`mleader.landing_too_close`（跨度 ≤ 狗腿长）、
  `mleader.leader_degenerate`、`mleader.style_invalid`、缺活动图层、Viewer、以及变体既有的缩放/镜像/非均匀。
- **无 `Options` 子提示**（引线类型/内容类型/最多点数/角度约束）：**UI 不得暗示**（P7c UI lane 已按此实现，
  MLEADER 按钮**无 caret、无 variants**）。
- **创建必须有文本**：无内容 MULTILEADER 仅**导入/显示/导出**，**不创建**。
- **门禁（编排者复跑）**：受影响 crate **2215 通过 / 0 失败**；UI **208/208（0 跳过）**；clippy(`-D warnings`)、fmt、
  architecture(27)、i18n(**558**)、file-size(**734**)、manifest(**23**)。**NOT RUN**：全量 `cargo test --workspace`；
  跨平台；**真机像素判据**（ADR J 对 P7c 的要求）——留作独立一步；**无真实 MULTILEADER 夹具**。

### P7c 真机证据（真实 GPU + X11/XWayland，2026-10-20）

- **环境**：GNOME/Mutter + Quadro P620（Vulkan）+ `GDK_BACKEND=x11`、`scale=1`、分区 926×341。**有界本地**证据。
- **命令**：`YACR_PROBE_DRAW_EVIDENCE=1 scripts/collect-evidence.sh --backend x11 --lifecycle 0 --capture-at 225 --timeout 300`。
  **注意 `--lifecycle 0` 才是"关闭"**：`--lifecycle exit` 会启用生命周期驱动并**提前 `app.exit(0)`**，首轮因此无截图（`NATURAL_EXIT`）。
- **硬指标**：`mleader_pass=1`、`delta=1`、`after_tool=-`、`after_awaiting=false`、`after_error=-`；`select_count=1`，
  选择行 `type:AcDbMultiLeader, layer:0_(0), space:model, style:Standard, roots:1, lines:1, display:4`。
- **像素判读（逐像素测量，observer）**：箭头尖 (269.5,240)（拾取 270,240，差 0.5 px）、实心三角 13×36 px、与引线夹角 0.2°；
  引线直线度最大残差 0.46 px、倾角 20.6°；**狗腿肘部可见**（y=89–90，x 272–326，斜接无过冲，末端距落点 17 px = 一个 landing gap）；
  文字 `LEAD` bbox 215–295 × 28–53（预期 214–296 × 29–54，逐边 ≤1 px），水平中心 x=255 与落点**精确对齐**、位于上方 37 px。
  **零碰撞**（最紧 13 px）。**结论：ACCEPT**。
- **证据边界**：像素只证"四个部分被画出"，**不**证实体类型；类型由同轮 `select_rows` 钉住（两者互补，不得单独引用）。
  单合成器/单卡/`scale=1`，**不**推广为其它合成器、DPI、vendor 或真机结论；**无真实 MULTILEADER 夹具**。
- **首轮失败的教训（已修）**：证据点原本选在既有一笔之上且两次拾取同 y ⇒ 引线**压在既有线上**、狗腿**与引线共线**，
  视觉不可判读，且点选命中 2 个实体。已重定位到空白走廊并使引线**倾斜**。
- **同时修掉的真实交互缺陷**：**在命令行打字后，点画布的第一下被静默吞掉**——`onFocus` 挂起指针通道，
  而真实 `pointerdown` **先于**焦点转移派发 ⇒ 必然被丢。现以 window **捕获阶段** `pointerdown` 先 `blur()`（解除挂起）
  再让**同一个**事件送达 `.native-input`（不取消事件、不影响命令行内部点击）。UI 测试 208 → **212**。
