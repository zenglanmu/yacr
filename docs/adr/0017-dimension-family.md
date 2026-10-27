# ADR 0017：标注族创建与 DIMSTYLE 子集（P5）

- 状态：**提案（Proposed）**，设计门禁 **SG-P5 attempt 1 = APPROVE-WITH-FIXES**（B1–B4 与 N1–N14 已写入本修订）、
  **attempt 2 = ACCEPT-WITH-FIXES**（决策**无需改动**；F-1–F-7 文本项与 A.8 补臂已随本次修订落地；待 **attempt 3**
  做纯文档复核）。本文件是 P5 的**唯一设计权威**；实施进度另起小节追加，**不改写决策文本**。
- 范围：`crates/cad-domain`（新域变体）、`crates/cad-db`（校验/变换/包围盒/样式表）、`crates/cad-geometry`
  （离散化/第二变换面）、`crates/cad-representation`（显示分派）、`crates/cad-import-acadrust`（导入迁移 +
  **导出**）、`crates/cad-app`（创建命令/工具/宿主映射）、`crates/cad-ui-core`（选择子/参数/状态/i18n）、
  React 宿主 UI。
- **不**覆盖：关联性（`DIMASSOC=2`）、`DIMEDIT`/`DIMTEDIT`/`DIMOVERRIDE`、`DIMSTYLE` 编辑命令、
  注释性缩放下的标注、`MLEADER`、`DIMBREAK`/`DIMSPACE`/`QDIM`/`DIMARC`、`DIMJOGGED`、容差/极限/备用单位、
  3D/OCS 平面外标注、自定义箭头块、`DIMBASELINE`/`DIMCONTINUE`（**降级为 NOT IMPLEMENTED**，见决策 C）。
  > **就地更正（2026-10-21，不改写上面的历史记录）**：`DIMBASELINE`/`DIMCONTINUE` 此后由本文件末尾的
  > **「决策 C（2026-10-21 解禁 P5f）」**解禁并**已实装**——**显式拾取基准线性标注**、命令**零跨命令状态**、
  > 一条事务/一步撤销；范围**仅线性基准**、**不写 `DIMCONTINUEMODE`**、**非关联**。本行上面的「降级」是 P5
  > 设计当时的决定，**不是现状**；现状以该节与 `docs/compatibility.md` 的 DIMENSION 族逐行表为准。
- 相关：`docs/adr/0015-new-draw-commands.md`、`docs/adr/0016-edit-command-extensions.md`（EXPLODE 按
  `type_key` 拒绝 Dimension 的既有结论）、`docs/compatibility.md`、`CAD_IMPLEMENTATION_SPEC.md` §3.2/§3.3、
  `docs/dxf-entity-coverage.md`。

---

## 0. 问题与约束（为什么这不是"再加几条命令"）

1. **我们的 DB 是"已解析的显示模型"，不是源文档。** 导入把 DIMENSION **烤成** `SemanticGeometry::Compound`
   （`cad-import-acadrust/src/dimension/convert.rs`：定义点 + DIMSTYLE → 延长线/标注线/箭头/文字的合成几何），
   随后**丢弃**定义点与样式；`DbObject.type_key` 仅留字符串 `"AcDbDimension"`。
2. **导出按几何分派，`Compound` 被摊平、`Opaque` 被丢弃**（`cad-import-acadrust/src/export/entity.rs:210`、
   `:512`）。⇒ 只要 DB 里存的还是合成后的 `Compound`，存盘就会把标注退化成散落的线+文字（或丢失）。
3. **acadrust 的写入侧完整支持标注**（0.6.3 的 `io/dxf/writer/section_writer.rs:4196 write_dimension`；
   0.6.5 为同文件 `:4327`，写入路径逐行复核未变），**无需修改 acadrust**。
4. **spec 权威缺口**：§3.2 只要求"**常用 DIMENSION 可见表示**"（读取/显示）；§3.1 的批注类特性（F07–F09）
   已于 2026-10-05 移除。**"创建标注"是项目自选范围，无 spec 条款授权**——能力表必须**逐行**写明这一点
   （不是脚注），全部行为选择标为**我们的选择**而非"符合规范"。
5. **不存在可复用的域类型**：`SemanticGeometry` 无 `Dimension` 变体、无 `DimStyle` 表；命令表有
   `EXPECTED_COMMAND_COUNT = 60` 硬断言（`crates/cad-ui-core/src/session/tests/.../command_vocabulary.rs:95`）。
6. **导出必须有内容块（内容块不是可选项）**：第三方权威文档明确——*"For AutoCAD this graphical representation
   is mandatory, otherwise AutoCAD will not open the DXF document."*（ezdxf 1.4.5 `Dimension` 文档；
   同页：*"AutoCAD does not render DIMENSION entities automatically"*，BricsCAD 则会自行渲染）。
   ⇒ **不生成 `*D…` 内容块的 DIMENSION 导出会产出 AutoCAD 打不开的文件**，而它在我们自己的查看器里看起来完全正常。
   这是比 §0.2 更深一层的"假成功"，因此**本 ADR 选择生成内容块**（AutoCAD 自身写出的标注也总是带内容块）。
   该依据**非 Autodesk 官方文档**，出处如实标注为第三方。

---

## 决策 A：域表示 —— 新增 `SemanticGeometry::Dimension`（**拒绝** sidecar 方案）

### A.1 选择

在 `cad-domain` 新增（**放在新文件 `cad-domain/src/dimension.rs`**，见 N13 文件行数）：

```rust
SemanticGeometry::Dimension(DimensionGeometry)   // 注意：须先解决与导入侧既有 DimensionGeometry 的重名（A.7）

pub struct DimensionGeometry {                   // display 为私有字段，仅经单一 API 构造/变换
    kind: DimensionKind,             // Linear{rotation} | Aligned | Angular2Ln | Angular3Pt
                                     // | Radius | Diameter | Ordinate{x_ordinate: bool}
    definition: DimensionDefinition, // 按 kind 存放 DXF 等价的定义点（A.3）
    style_name: String,              // DXF 组 3
    style: DimensionStyleValues,     // 已解析的样式快照（决策 B）
    measurement: f64,                // 创建时算出；**导出写组 42**（B2）
    text_override: Option<String>,   // DXF 组 1；None = 画测量值
    display: Vec<SemanticGeometry>,  // 派生：延长线/标注线/箭头/文字
}
```

公开 API（**唯一写入路径**；字段全部私有）：
- `synthesized(definition, style_name, style, text_override) -> CadResult<Self>`——**自己算 `display`**（共用合成
  构造器），故"调用方传入与定义不一致的显示"在**结构上不可能**；
- `from_picture(definition, style_name, style, text_override, members) -> CadResult<Self>`——用于 A.6 的
  **有块分支**（显示 = 外部图片块成员，本就不由我们合成；其不变量见 A.4）；
- `transformed(&self, transform) -> CadResult<Self>`——同时改 `definition`、`display`、`measurement` 与文字内容
  （见 A.5）。

### A.2 为什么不用 sidecar（`BTreeMap<EntityId, DimensionRecord>`）

`DbEntity` 确有 sidecar 先例（`EntityRenderAttributes` 在 `Drawing.render_attributes`，
`cad-db/src/entity.rs:237`），且**不用改匹配点**。但两个**决定性缺陷**（门禁已核实）：

- `transform_geometry` 只变换**几何**。若语义记录在 sidecar，变换后**显示几何已移动、定义点仍在原处**，
  导出即写出**位置错误**的 DIMENSION——**静默数据损坏**。
- sidecar 的**生命周期不在事务内**：删除会剪掉 sidecar，撤销删除需调用方显式
  `entity_display_state`/`restore_entity_display_state` 快照恢复（`cad-db/src/drawing/access.rs:112-157`）。

补充有利事实：撤销/补丁携带**整值** `DbEntity`（`cad-history/src/lib.rs:24-33`），故变体的撤销故事是干净的；
`DimensionGeometry` 的**构造/变更面很小且可枚举**：导入（`dimension/convert.rs:72`）、P5c+ 创建命令、
`cad-db/src/transform.rs:48`、`cad-geometry/src/engine.rs:202`（第二变换面，捕捉路径用）。

### A.3 定义点模型（**逐 kind**）与**未建模字段的处置**

| kind | 点（DXF 组） |
|---|---|
| Linear / Aligned | `13` 第一延长线原点、`14` 第二延长线原点、`10` 标注线位置、**`11` 文字中点**；Linear 另有组 `50` 旋转 |
| Radius / Diameter | `15` 圆/弧上第一点、`10` 圆心（Radius）或对径点（Diameter）、**`11` 文字中点**、组 `40` 引线长 |
| Angular2Ln | 两线端点 `13`/`14` 与 `10`/`15`；`16` 标注弧位置、**`11` 文字中点** |
| Angular3Pt | `15` 顶点、`13`/`14` 两延长线端点、`10` 标注弧位置、**`11` 文字中点** |
| Ordinate | `13` 特征点、`14` 引线端点、`10` 创建时基准、**`11` 文字中点** |

**未建模但写入器会写/合成会读的字段，逐个处置（不得沉默）**：

| 组 | 含义 | 处置 |
|---|---|---|
| `40` | 引线长（Radius/Diameter） | **建模**（写入器无条件写，`section_writer.rs:4351/4360`） |
| `53` | 文字旋转 | **建模**（合成 `place_dimension_text` 消费 `text_rotation`，`convert.rs:39-49`；否则往返漂移） |
| `11` | 文字中点 | **建模**（所有 kind 均写、均读，见上表） |
| `51` / `52` | 水平方向 / 倾斜角 | **声明丢弃**（零值 ⇒ 写入器省略；读取忽略；能力表记录） |
| `71` / `72` | 文字附着点 / 行距样式 | **声明丢弃**（写入器**总是写固定默认值** `MiddleCenter`=5 / 行距=1，`section_writer.rs:4297-4298`；读取忽略） |
| `210` | 挤出方向 | **声明拒绝**：创建时定义点必须在 XY 平面（决策 D-6）；导入侧沿用既有 `Opaque`+`Partial` |

### A.4 不变量（**修正 B4**：原表述几何上不可能成立）

`display` 的包围盒**严格包含** `definition`（延长线按 `DIMEXO`/`DIMEXE` 偏移、文字离线
`DIMGAP + DIMTXT/2`），故"变换后两者包围盒一致"**永远不可能通过**（除恒等）。改为两条**可测**不变量：

1. **构造期（仅适用于"由我们合成的显示"）**：`display == synthesis(definition, style)`——创建与**无块导入**
   共用同一个合成构造器（`synthesized`），由测试在两条路径上逐点断言（最强的一条）。
   **不适用于 A.6 的"有块分支"**：那里的 `display` 是**外部图片块成员**，按构造就不等于我们的合成结果；
   该分支改为钉住"**导出的块成员 == `display` 逐项**"。
2. **变换期（增量性质，两分支都适用）**：平移下二者位移**完全相同**；相似变换下 `display'` 的包围盒 ==
   变换后 `display` 的包围盒（即"显示与定义同步移动"，不要求二者相等）。

**统一写入路径**：`display` 为**私有字段**，只经 `synthesized`/`from_picture`/`transformed` 写入 ⇒
"调用方传入不一致的显示"在结构上不可能（前者自己算，后者的输入本就是外部图片）。

### A.5 显示、变换、拾取、包围盒、导出

- **显示**：`cad-representation` 分派臂**递归 `display` 子项**，与 `Compound` 同路——**不新增
  `DisplayPrimitive`、不新增 overlay、不动渲染器**。
- **变换（修正 B3）**：显示子项**含 `SemanticGeometry::Text`**（`dimension/convert.rs:17-27`），而
  `cad_db::transform_geometry` **拒绝 Text**（`cad-db/src/transform.rs:226`）⇒ 原计划"变换每个显示子项"
  会让**任何 MOVE/COPY/ARRAY/ALIGN/STRETCH** 报错。修正为：
  - **两个变换面都必须有 `Dimension` 臂且语义一致**：`cad-db/src/transform.rs`（包围盒/编辑路径）与
    `cad-geometry/src/engine.rs:202`（捕捉路径，**已能变换 Text**，`:363-381`）。
  - `Dimension` 臂**自行变换其显示子项（含 Text）**，语义对齐 `engine.rs` 的 Text 规则
    （位置平移、`height * uniform_scale`、`rotation + rotation_of`）。
  - **记录该不对称**：独立 `Text` 实体仍**拒绝**变换（未实现），而标注**内部**的文字必须随标注一起移动——
    两者是不同的能力，不得混为一谈。
  - **标量与文字内容必须同步（F-2）**：均匀缩放下 `measurement` **必须由变换后的 `definition` 重算**，
    文字子项的内容经**同一条测量值格式化路径**重新生成——否则导出会写出**陈旧的组 42**，而屏幕上仍显示旧数字
    （3 倍缩放后标注量的是 30，文字却还写 "10"）。`kind` 的 `Linear{rotation}`（组 50）须随旋转施加角度增量。
  - **有块分支的不可变换成员**：外部图片块可能含 `Spline`/`Shape`/`Opaque` ⇒ **整实体 `Unsupported` + 诊断**，
    **绝不部分变换**。
  - **非均匀缩放 / 镜像**：无法保持标注语义 ⇒ **整实体显式 `Unsupported` + 诊断**，不产生畸变几何。
  - **备选方案（记录为未来改进，不在 P5a）**：把合成下沉、`display` 改为"**变更时重建**"。**不取的准确原因**：
    ① 合成与 `ImporterBuilder` 的样式/字体解析**纠缠**（`place_dimension_text` → `self.dimension_text` →
    `self.style_id`/`self.style_font`，`convert.rs:17-27/51`）⇒ 要做到 "display = 纯 `f(definition, style)`"
    得先把这层析出；② 可行的落点是 **`cad-db` 自身**（它已在 `transform.rs` 的 `math` 模块里带线性代数，
    **不涉及层序变动**）；③ "变更时重建"每帧零成本，故**性能不是**反对理由。
    **但**：A.6 的**有块分支根本无法重算**（外部图片块不是我们的合成结果）⇒ "存储显示"对那一半是**必需**而非取舍。
    故 P5a 取"存储 + 私有字段 + 单写入路径 + 双变换臂"，并把"下沉重算（**仅合成分支**）"记入未来改进。
- **拾取/包围盒**：委托 `display` 子项（与 `Compound` 同口径）。
- **导出（修正 B1/B2/N4/N5）**：
  - **必须生成内容块**：为每个标注写 `BLOCK_RECORD` + `BLOCK`（成员 = `display` 子项）+ 组 2 = 块名
    （`DimensionBase.block_name`）。依据见 §0.6；acadrust 写入/读取侧均支持块记录，**不改 acadrust**。
  - **组 42 = `measurement`**：写入器**总是**写组 42（0.6.3 `section_writer.rs:4289`；0.6.5 `:4367`
    `write_double(42, base.actual_measurement)`，Ordinate 路径仍重算），不写等于写出 **`42 = 0.0` 的假测量值**。
    "只读"指**用户不能输入**，不是"不写"。**0.6.5 复核**：`actual_measurement` 的语义/单位未变（全 crate diff
    只命中一处测试），0.6.5 的"dimension flag mapping 修正"只改 DWG 标志字节，**不改组 42**。
  - **组 70 的 32 位不写**：该位语义是"组 2 的块仅被本标注引用"（R13+ 恒置），而 acadrust 写入器**无法置位**
    （按 kind 的常量只含 `0x40`/`0x80`，0.6.3 `section_writer.rs:4283-4288`/`4397`；0.6.5 `:4360-4366`/`4473`
    **逐行相同**）⇒ **删去原"类型标志 + 32 位"的表述**，改为**显式偏离记录**："32 位不写（acadrust 写入器
    约束）"。`0x40`（坐标 X 型）/`0x80`（文字被用户定位）与 Autodesk 参考一致。**0.6.5 复核结论：仍然不能
    置位**——DXF 写入器只写 `type_flags | 0x80`，`dwg_flags_byte` 仅由 DWG 写入器消费、DXF 侧完全不读；
    故本次**不改变**该偏离（否则必须用非公开/伪造记录绕过写入器，违反"不修改 acadrust"）。
  - **导出 DIMSTYLE 表**（N5）：写入器支持（0.6.3 `section_writer.rs:1500-1534`；0.6.5 `:1620-1660`）；我们为
    每个被写出的样式名写一条 DIMSTYLE 记录（含决策 B 子集值），**否则组 3 悬空、外部 CAD 用它自己的
    `Standard` 解析** ⇒ 静默显示漂移。若某样式无法写出，记入 `ExportReport` 的 `Converted/Dropped` 条目，
    **不得沉默**。**0.6.5 复核**：`write_dimstyle_entry` 的组码/组 3 行为逐行未变，`DimStyle` 结构与
    `DimStyle::new` 默认值**零 diff**；唯一写入变化是 DIMSTYLE 组 50（`DIMJOGANG`）改按**度**写出
    ——本项目**不建模** `DIMJOGANG`（默认 `π/4` ⇒ 该组不写出），故对我们无影响。
- **校验**：`cad-db` validate 臂要求定义点与显示子项均有限；`display` 为空视为合法（退化但可显示为空）。

### A.6 导入迁移（**两个分支都迁**，以保住往返）

- **无匿名块的 DIMENSION**：现产出 `Compound` ⇒ 改为**同一批合成子项**包进 `Dimension`
  （定义点取自 acadrust 结构、样式取自既有 `DimStyleValues`）。
- **有匿名块（`*D`）的 DIMENSION**：现在展开为 `Insert`（`entity/convert.rs:166`）⇒ 改为 `Dimension`，
  `display` = **展开后的块成员**（沿用今天的展开语义，但放进变体）。
  **这是刻意的行为变更，不是"与今天一致"（F-4）**：今天该分支导入为 `Insert` 且**保留块引用**（成员**不**在
  导入时摊平），`EXPLODE` 对它走 `explode_insert` **会成功**（`type_key` 门只在 `Compound` 臂触发，
  `explode.rs:254/465-471`）；迁移后 `EXPLODE` **拒绝**（符合 ADR-0016），且**图片块不再存在于块表**。
  三者（实体类型、EXPLODE 结果、块表）都记入能力表。
  另：此分支的显示**无法重算**（外部图片块不是 `synthesis(definition, style)`）⇒ "显示改为变更时重建"的方案
  对**两种显示中的这一种根本不可用**，故"存储显示"在此不是取舍而是**必需**。
  > 这一分支**必须迁**，否则我们导出的标注（A.5 要求带内容块）再导入时会变回 `Insert`，**往返立刻破**。
- **平面外/退化**：保持现状（`Opaque` + `Partial`）。
- **测试改动面（门禁点名，须一并处理）**：`tests_display_dimension_multileader.rs`、`dimension/mod.rs`
  的合成测试、`export/tests/report_limits.rs:29` 的夹具丢失期望、`docs/dxf-entity-coverage.md`、
  `docs/compatibility.md:33`。

### A.7 命名冲突（N14）与文件行数（N13）

- **重名**：`DimensionGeometry` **已被占用**（`crates/cad-import-acadrust/src/dimension/geom.rs:9`，合成中间体）。
  ⇒ 把**导入侧的合成中间体改名为 `SynthesizedDimension`**，域类型保留 `DimensionGeometry`
  （它是几何，名字更贴切）；改名波及 `dimension/{convert,linear,angular,multileader}.rs` 与其测试。
- **文件行数**：`crates/cad-domain/src/lib.rs` 已 **908** 行、`crates/cad-app/src/app_drawing/helpers.rs`
  已 **911** 行。变体 + `DimensionDefinition` + `DimensionStyleValues`（~30 字段）**必然破 1000** ⇒
  **新建 `crates/cad-domain/src/dimension.rs`**（并预留后续 `export/dimension.rs`；`export/entity.rs` 已 755 行）。
  **不加白名单豁免。**

### A.8 通配符臂：编译器**不会**强制的地方（N1，必须显式补臂）

"编译器强制每个匹配点表态"**只对穷尽 match 成立**；以下 `_ =>` 会**静默漏处理**新变体，必须逐个补显式臂：

| 位置 | 现状 | 若不管的后果 |
|---|---|---|
| `crates/cad-platform/src/fonts.rs:124` `_ => {}` | 只递归 `Compound` | **标注文字字体静默不注册 ⇒ 文字静默不绘制** |
| `crates/cad-import-acadrust/src/support.rs:50` `_ => (Verified, Verified)` | 只特判 `Compound` | 带 Text 子项的标注被报 **Verified**（今天报 `Unverified`）⇒ **静默过度声称** |
| `crates/cad-app/src/select_box.rs:377` `_ => Some(MatchPath::Aabb)` | 只特判 `Compound` | 框选**从精确逐子项匹配退化为 AABB** |
| `crates/cad-geometry/src/engine.rs:151` `_ => 0.0` | `fallback_scale` | 标注在捕捉回退弦上静默取 0.0 |
| `crates/cad-import-acadrust/src/geometry.rs:389` `_ => Vec::new()` | `referenced_blocks` | 标注的块引用不被登记 |
| `crates/cad-platform/src/images.rs:99` `_ => {}` | 只递归 `Compound` | 同 fonts |
| `crates/cad-import-acadrust/src/entity/push.rs:253` `_ => semantic` | MEASURE 支持度 | 标注被报可测（标注不是 MEASURE 目标） |
| `crates/cad-app/src/app_drawing/stretch.rs:264` `other =>` | 整命令拒绝 | **须显式声明** STRETCH 对标注的行为：部分包围 → 不动 + 诊断（同 ARC）、完全包围 → 整移 |
| `crates/cad-measure/src/snap/generate.rs:20` | **穷尽**（编译器强制），但**诱人的落点是 `Mesh\|Insert\|Text\|… => {}` 空臂** | 今天 `Compound` **递归子项**（`:53-57`）⇒ 标注线端点**是**捕捉点；并入空臂会**静默删除**它们 |
| `crates/cad-measure/src/snap/resolve.rs:16` `is_intersectable` | `_ => false` + `Compound` 递归 | 标注线上的**交点捕捉静默消失** |
| `crates/cad-spatial/src/pick/bounds.rs:88` `_ => return None`（+ `:130` 的 `Compound` 并集） | 通配符 | 标注失去精确局部包围盒 ⇒ 落到 AABB 精度路径（与 N1c 同族） |
| `crates/cad-proxy/src/lib.rs:490` `vertex_count` `_ => 4` | 通配符 | 仅估算值，外观级 |

**声明项（不是臂，但必须记入能力表）**：`crates/cad-app/src/host/databases.rs:260`——捕捉目标经
`transform_geometry` 烘焙，今天 `Compound` 被拒（`:255`）⇒ **标注自 P5a 起会首次成为捕捉目标**（**行为新增**）；
`crates/cad-app/src/picking/mod.rs:179` `_ => Analytic` 无害且正确。
（`cad-cli-tools`/`cad-engine`/`cad-history`/`cad-host`/`cad-ui-core`/`cad-scene` **无**几何 kind 匹配，
其 `_ =>` 命中的是 `SpaceId`/`DisplayPrimitive`，**这些 crate 是干净的**。）

**穷尽（编译器强制，照改即可）**：`cad-db/{validate.rs:42,transform.rs:48,bounds.rs:71}`、
`cad-geometry/{tessellate.rs:9,vector.rs:78,engine.rs:211}`、`cad-spatial/pick/{hit.rs:110,300,bounds.rs:269,429}`、
`cad-representation/provider/default.rs:228`（`Compound` 早分支）+ 主 match、`export/entity.rs:258` 与
**`export/entity.rs:738 kind_name`**、`app_drawing/{helpers.rs:15 class_name,explode.rs:244}`、`selection.rs:419`。
**`class_name` 必须映射为 `"AcDbDimension"`**（与导入侧 `support.rs:142` 的 `type_key` 一致；否则
`Compound` 会写成 `"AcDbCompound"`，与既有约定冲突）。**EXPLODE 继续按 `type_key` 拒绝 Dimension**
（ADR-0016 结论延续）。

---

## 决策 B：DIMSTYLE 子集

### B.1 存储与解析

- `cad-db` 新增 `dim_styles: BTreeMap<DimStyleId, DimStyle>` + 名称索引；**默认含 `Standard`**。
- 实体存**已解析快照**（`DimensionGeometry.style`），与"ByLayer 在导入时解析"同一理由。
- **刻意偏离并声明**：AutoCAD 中编辑 DIMSTYLE 会**追溯更新**所有使用者；我们是**快照**，**样式编辑不追溯**。
  能力表必须写明；追溯需要"重新解析"通道，不在 P5 范围。
- **默认值来源必须钉住（N7）**：仓库内已有**两套互相矛盾**的默认（acadrust `dxf_defaults`
  `tables/dimstyle.rs:359-378` 与 `DimStyle::new`；导入侧 `DimStyleValues::from_style` 的回退
  2.5/0.625/1.25 又与两者都不同，`dimension/values.rs:46-50`）。⇒ 本 ADR **选定一套并说明来源**：
  以 **Autodesk 文档的 *Standard* 初值页**为准，**公制一套**（仓库其余部分按公制走），并把差异写入
  `docs/compatibility.md`。**已知需纠正的错行**：`DIMTIH`/`DIMTOH` 不是 "ON / OFF"（acadrust `dxf_defaults`
  为 `true/true`、`DimStyle::new` 为 `false/false`）；`DIMPOST` 默认与 acadrust 的 `"<>"` 冲突；
  `DIMTOFL` 的 "OFF–ON" 记法含义不明，须写成单一值。

### B.2 变量子集：**必须区分"驱动显示"与"仅快照"**（修正 N6）

原表把两者混为一谈。**违反铁律 2 的是"未声明地存储却不消费"**；(b) 之所以存在，是为了**DIMSTYLE 表导出的保真**
（组 3 必须能在表里解析），其**归宿是 `cad-db::DimStyle`（表）**，**实体快照只携带 (a)**。故拆两张表，并逐条声明。

**(a) P5 驱动显示（合成/格式化真正消费；实现 + 测试）**

| 变量 | 组 | 控制 | 消费点 |
|---|---|---|---|
| `DIMTXT` | 140 | 文字高度 | 合成文字子项 |
| `DIMASZ` | 41 | 箭头尺寸 | 箭头网格 |
| `DIMEXO` | 42 | 延长线自原点偏移 | 延长线起点 |
| `DIMEXE` | 44 | 延长线超出标注线 | 延长线终点 |
| `DIMGAP` | 147 | 文字间隙 / 标注线断开 | 文字位置 + 标注线断口 |
| `DIMTAD` | 77 | 文字垂直位置（0 居中/1 上方/2 外侧/3 JIS/4 下方） | 文字位置 |
| `DIMTVP` | 145 | `DIMTAD=0` 时文字垂直偏移比例 | 文字位置 |
| `DIMSCALE` | 40 | 尺寸/距离/偏移的整体比例（**不含测量值**） | 上述尺寸变量 |
| `DIMDEC` | 271 | 主单位小数位 | 测量值格式化（`linear.rs:117-128`） |
| `DIMZIN` | 78 | 零抑制（**当前仅消费 bit 3**） | 格式化 |
| `DIMLFAC` | 144 | 线性测量比例因子（非关联） | 测量值 |
| `DIMRND` | 45 | 距离取整 | 测量值 |
| `DIMPOST` | 3 | 测量值前后缀 | 格式化 |
| `DIMCEN` | 141 | 中心标记/中心线尺寸（>0 标记 / <0 线 / 0 无） | `DIMRADIUS`/`DIMDIAMETER`/`DIMCENTER` |
| `DIMTXSTY` | 340 | 文字样式名 | 文字子项 |
| `DIMSE1`/`DIMSE2` | 75/76 | 抑制第一/二条**延长线** | 合成（**必须实现，否则删除该行**） |
| `DIMSD1`/`DIMSD2` | 281/282 | 抑制第一/二段**标注线+箭头** | 同上 |
| `DIMDLI` | 43 | 基线间距 | **P5f 已降级 ⇒ 本行移入 (b)** |

**(b) 仅快照、显示按"忽略"处理并**显式声明**（不得假装生效）**

`DIMLUNIT`（**≠2 时须实现或显式降级为小数**，`DIMFRAC`/英尺-英寸未实现）、`DIMDSEP`、`DIMJUST`、
`DIMTIH`/`DIMTOH`、`DIMATFIT`、`DIMTIX`/`DIMTOFL`、`DIMCLRD`/`DIMCLRE`/`DIMCLRT`、`DIMDLE`、`DIMDLI`、
`DIMCONTINUEMODE`。

**明确声明 NOT IMPLEMENTED**：备用单位（`DIMALT*`）、容差/极限（`DIMTOL`/`DIMLIM`/`DIMTP`/`DIMTM`/
`DIMTDEC`/`DIMTFAC`）、箭头块（`DIMBLK*`/`DIMSAH`/`DIMLDRBLK`）、标注线型（`DIMLTYPE`/`DIMLTEX1/2`）、
固定延长线长（`DIMFXL*`）、文字背景（`DIMTFILL*`）、弧符号/折角（`DIMARCSYM`/`DIMJOGANG`）、
注释性缩放（`DIMSCALE=0` + `CANNOSCALE`）、`DIMASSOC`/`DIMASO`/`DIMSHO`（图纸级，非样式成员）。

### B.3 样式编辑命令

`DIMSTYLE`（对话框）/`-DIMSTYLE`/`DIMOVERRIDE` **均 NOT IMPLEMENTED**。P5 只提供**样式存储 + `Standard`
默认 + 创建时解析**，创建命令用**当前样式**（= `Standard`）。⇒ **"P5 的 DIMSTYLE 子集"指变量子集，
不是可交互编辑的样式管理器**，能力表必须这样写。

---

## 决策 C：命令族与分阶段

| 子阶段 | 内容 | 关键点 |
|---|---|---|
| **P5a** | 域变体 + 全部穷尽臂 + **A.8 通配符臂** + 导出（含**内容块** + 42 + DIMSTYLE 表）+ **导入两分支迁移** + 重命名/拆文件（**无命令**） | 验收：**往返保住 DIMENSION**；渲染与迁移前**逐字节一致**；变换同步移动定义点与显示；A.4 两条不变量有测试 |
| **P5b** | DIMSTYLE 存储 + 子集（B.2(a)/(b) 分开）+ 默认（公制一套）+ i18n | 无 UI 命令；解析与格式化通道有契约测试；**i18n 行须列出具体新增键** |
| **P5c** | `DIMLINEAR` + `DIMALIGNED` | 三点流程（两延长线原点 + 标注线位置）；**线性/垂直由标注线位置决定** |
| **P5d** | `DIMRADIUS` + `DIMDIAMETER` + `DIMCENTER` | `DIMCENTER` 产出**普通 LINE/ARC**（文档明确"非关联中心标记"，**不产 DIMENSION 实体**）；`DIMCEN` 符号读数须钉住 |
| **P5e** | `DIMANGULAR`（3 点 + 两线）+ `DIMORDINATE` | 前置条件**不是**"补角度合成"（见下），而是 **U-1/U-2/U-5/U-7/U-8** 的钉住 |
| ~~P5f~~ | ~~`DIMBASELINE` + `DIMCONTINUE`~~ | **降级为 NOT IMPLEMENTED（N11）**：收益是便利，成本是会话级 `last_dimension` 状态（撤销/删除/切换文档时失效）——与 sidecar 同类生命周期风险，项目一直刻意回避。记录为未来工作。 |

**各命令共同的选择（我们的选择，非规范）**：
- **仅点选流程**：线性/对齐/角度的**对象选择变体**（选直线/圆/弧自动取端点）**NOT IMPLEMENTED**
  ⇒ 能力表**单列一行**；半径/直径的**对象拾取是唯一入口**（文档如此），故 `DIMRADIUS`/`DIMDIAMETER` 接受实体拾取。
- **文字覆盖**：`Text`/`Mtext` 提示 **NOT IMPLEMENTED**（只画自动测量值）；`text_override` 字段保留以备导入。
- **创建即非关联**：不建立 `ACAD_DIMASSOC`；**导出生成内容块**（§0.6）但不建立关联。
- **测量值**：由定义点直接算（线性/对齐 = 两点距离；角度 = 两向量夹角；半径/直径 = 半径/直径），
  经 `DIMLFAC`/`DIMRND`/`DIMDEC`/`DIMPOST`/`DIMZIN` 格式化后进 `Text` 显示子项。
- **图层**：新标注在**活动图层**（与 P4 的 CHAMFER/FILLET 一致）。**事务**：一条事务、一步撤销。

**P5a 必须顺带纠正的既有不一致（N9）**：`docs/compatibility.md:33` 称"角度/坐标/圆弧长/大半径仍
Partial"，而代码对 `Angular2Ln`/`Angular3Pt`/`Ordinate` 报 **`Completeness::Complete`**
（`dimension/convert.rs:153-155`），仅 `LargeRadial`/部分弧为 Partial（`:156-176`）。**表与代码矛盾** ⇒
P5a 必须核实并统一（哪个是真的就改另一个），不得两处并存。

---

## 决策 D：诚实边界（**逐条进入能力表，按行而非脚注**）

1. **关联性未实现**：创建与迁移的标注都是**非关联单实体**；**编辑被测量几何不会更新标注**；导出不写
   `ACAD_DIMASSOC`（**不写即等于声明"不关联"**）。
2. **文字受宿主字体门控**：测量文字是 `SemanticGeometry::Text`，无宿主字体则"不由场景绘制"
   （既有 `Partial`，`cad-representation/src/representation.rs:176`）。标注**继承**此限制；A.8 的
   `fonts.rs` 臂必须补上，否则是**静默不绘制**而非"有诊断的 Partial"。
3. **spec 权威缺口**：创建是项目自选范围（§3.2 只要求可见表示）。
4. **导出形状与已知损失**：写出**内容块**（AutoCAD 必需）、**组 42 = 测量值**、DIMSTYLE 表；
   **不写组 70 的 32 位**（acadrust 写入器约束；0.6.5 复核仍无法置位）、**丢弃**组 51/52/71/72；非关联、
   无 `ACAD_DIMASSOC`。
5. **样式不追溯**（B.1）。**`DIMSTYLE` 编辑命令未实现**（B.3）。
6. **平面外拒绝**：定义点不在 XY 平面时创建**整命令拒绝**并诊断，不猜工作平面。
7. **`DIMCENTER` 不是标注**：产出普通几何（文档明确），能力表按"绘图辅助"而非 DIMENSION 记录。
8. **导出兼容性的证据边界**：内容块的必要性依据**第三方（ezdxf）文档**而非 Autodesk 官方文档；
   **不得**据此宣称"AutoCAD 兼容"，只宣称"按该依据采用 AutoCAD 所要求的结构"。

---

## 决策 E：未查证 / UNVERIFIED 清单（**必须在能力表标注**）

- **U-1 角度标注的"四选一"规则**：标注线位置拾取如何在四角中选一、`Quadrant` 如何锁定——权威句在
  Autodesk 仅 JS 渲染的概念页，静态镜像无此页 ⇒ **未查证**。
- ~~U-2 角度子类型的 DXF 类型码~~ ⇒ **升级为文档化**：Autodesk DXF 参考**逐字**列出 "2 = Angular；
  5 = Angular 3-point"（不再是我方推断）。**残留未查证**：弧/圆变体归入 `2` 的**对映**仍属推断。
- **U-3 关联性的编辑传播矩阵**：Autodesk 只述原则，不枚举"哪种编辑更新哪些定义点"（我们未实现关联，仅记录）。
- **U-4 `DIMCEN` 默认值**：变量页给 0.09/2.5，部分命令页只述符号约定 ⇒ 落常量前需确认目标版本。
- **U-5 `DIMORDINATE` 自动 X/Y 规则的边界**：文档给"Y 差更大则量 X"，**相等的边界情形未述**。
- **U-6 逐标注覆盖的 XDATA 编码**（`DSTYLE` 实数/字符串变量编码无完整样例）。（P5 未实现覆盖。）
- **U-7 组 70 的 `2` vs `5`**：现代版本是否仍对三点角度发 `5`（还是归一为 `2`）未述。
- **U-8 `Angular2Ln` 的点语义冲突（N10，P5e 前置）**：acadrust 结构注释称 line1 = `13→14`、line2 = `15→10`
  （**顶点是交点**），而合成把 `15` 当顶点并**忽略 `14`**（`dimension/convert.rs:105-112`），
  `new(vertex,…)` 亦把 `15` 当交点（`acadrust …/dimension.rs:452-457`）。⇒ **P5e 前须用一张真实
  AutoCAD 写的两线角度标注钉住唯一读法**，否则是"先实现后争论"。
- **我们的推断（同样标注）**：`DIMCENTER` 输出为普通 LINE/ARC（文档只说"非关联中心标记/中心线"，未点名实体）；
  线性标注的水平/垂直由标注线位置决定（文档述，但**提示序列的严格状态机**未给出）。

---

## 决策 F：验收与证据要求

- **契约测试（数值）**：
  1. **往返诚实性**：无块 DIMENSION 夹具 → 导出 → 再导入 → 断言 kind/定义点/样式名/文字覆盖**保全**，
     并断言导出形状（组 2 + 块成员 == `display` 子项）+ 组码 **70/1/3/10/11/13/14/42** 与 ADR 所述**逐字一致**。
     另断言 `ExportReport` **含有**声明的条目（生成内容块、写入 DIMSTYLE 记录、51/52/71/72 的处置）——
     零成本钉住诚实性。
     并补**有块分支的第二段往返**：再导入后的 `display` 与导出前的 `display` **逐成员相等**。
  2. **不变量**：创建/导入期 `display == synthesis(definition, style)`；平移/旋转/均匀缩放下的增量性质；
     镜像/非均匀 ⇒ 整实体 `Unsupported` + 诊断（替换原 B4 的伪测试）。
  3. **通配符回归三连**：`collect_fonts` 返回标注文字字体；带 Text 的标注 `display_support == Unverified`；
     `select_box` 分类与等价 `Compound` **一致**。
  4. **格式化诚实性**：`DIMLFAC`/`DIMDEC`/`DIMZIN` bit3 测试 + 一条**显式测试**证明 `DIMLUNIT≠2`
     （或 `DIMPOST`/`DIMDSEP`）被**声明未实现**而非静默按小数处理。
  5. **角度点语义钉住（P5e 前）**：一张已知点的两线角度夹具 → 断言合成显示取向（关闭 U-8）。
- **真机证据**：用既有 `YACR_PROBE_DRAW_EVIDENCE` 扩一条标注相位（合成 DOM 指针事件、**非真实鼠标**），
  断言 `entityCount` 增量与无错误；**诚实边界**：`cad_status` 无几何通道 ⇒ 真机只证"提交成功 + 计数 +
  内容存在"，**几何正确性一律由契约测试钉住**（沿用 P4 结论）。
- **门禁**：受影响 crate 测试 + clippy/fmt/check+wasm/architecture/i18n/fixture/file-size/workflow + UI 测试构建；
  全量 `cargo test --workspace` 仍为 NOT RUN（`accelerated_tier` 超时）。
- **能力表**：`docs/compatibility.md` 的 DIMENSION 行拆为"读取/显示"与"**创建**"两栏，**逐行**写明子阶段状态、
  决策 D 的八条边界、决策 E 的未查证项，以及"创建无 spec 授权"；并声明 A.8 的**两项行为变更**
  （标注**首次成为捕捉目标**；有块分支的 `EXPLODE` 由**成功转为拒绝**）。

---

## 实施状态（追加，不改写决策文本）

### P5a（域变体 + 全部匹配臂 + 合成下沉 + 导入两分支迁移 + 导出；**无创建命令**）

- **域表示（决策 A）**：`cad-domain` 新增 `SemanticGeometry::Dimension(Box<DimensionGeometry>)`
  （`crates/cad-domain/src/dimension.rs`）。`DimensionGeometry` 字段**全部私有**，唯一写入路径为
  `synthesized` / `from_picture` / `transformed`；`display` 由构造器自算（或来自外部图片块），故
  "调用方传入与定义不一致的显示"在结构上不可能。`DimensionDefinition` 携带 `kind` + 逐 kind 定义点
  （`DimensionPoints`）+ 组 11/53 + 组 210 法向。
- **合成下沉（决策 A.5/A.7）**：合成从 `cad-import-acadrust::dimension` **原样迁入** `cad-domain`
  （`dimension.rs` + `dimension_synthesis.rs`），创建路径与无块导入路径共用同一构造器；旧导入侧中间体改名
  `SynthesizedDimension`，域类型保留 `DimensionGeometry`。迁移前后数值不变（有测试逐点断言）。
- **导入迁移（决策 A.6，两分支都迁）**：无匿名块的 DIMENSION → 同一批合成子项包进 `Dimension`；
  有匿名块（`*D…`）的 → `from_picture`，`display` = 展开后的块成员。**圆弧长/大半径不走域变体**，
  保留 P5a 前的 `Compound` 合成。
- **导出（决策 A.5）**：写出真实 DXF `DIMENSION` + **生成的 `*D…` 内容块**（成员 = `display` 子项）+
  组 42 = `measurement()` + 为每个样式名 `add_or_replace` 一条 DIMSTYLE 记录（组 3 不悬空）。
  `ExportReport` 逐条记录形状与声明的丢弃（`export.converted.dimension`）。
- **匹配臂**：全部**穷尽**臂 + A.8 的 12 处**通配符**臂均已补显式 `Dimension` 臂
  （`fonts.rs`/`images.rs`/`support.rs`/`select_box.rs`/`engine.rs`/`geometry.rs`/`push.rs`/`stretch.rs`/
  `snap`/`pick/bounds.rs`/`proxy`）；`class_name` 映射 `"AcDbDimension"`。
- **测试**：`cad-domain/src/dimension_tests.rs`（合成 / A.4 两条不变量 / 变换）；`cad-import-acadrust`
  的 `dimension/mod.rs` 合成测试与 `export/tests/dimension.rs`（F.1 往返、内容块 == display、组码、
  picture 分支逐成员往返）；`export/tests/report_limits.rs` 夹具期望（`export.converted.dimension`
  取代旧的 `export.converted.mesh`）。

### 偏离 / 决策（逐条，均须进能力表）

1. **Kind 集合**：`DimensionKind` 恰为 A.1/A.3 的 **7** 个 kind。**ArcLength 与 LargeRadial 未迁移**——
   ADR 未给它们点槽与组码，凭空造点槽是**未声明选择**，故保留 P5a 前的 `Compound` 合成。
   **后果须明说：它们的导出仍与从前一样有损；P5 不声称 DIMENSION 完整往返。**
2. `SemanticGeometry::Dimension(Box<DimensionGeometry>)` —— **boxed**，为仓库 `-D warnings` clippy 门禁
   （仓库**无** `large_enum_variant` allow）。
3. `definition` 携带 kind；**没有**可与之失同步的独立 `kind` 字段（`kind()` 委托 `definition.kind`）。
4. 组 11/53 的**零哨兵**在变换下保留（`0` = "从标注线推导"），故"真实原点放置"与"未设置"被合并——
   沿用自旧合成，**声明**。
5. 导出写 `DIMSCALE=1` + 已解析值（**DIMSCALE 不保留**）；报告有对应条目。
6. 导出总是 `add_or_replace` 该实体已解析快照的 DIMSTYLE 记录，故组 3 永不解析到陈旧条目。
7. **Picture 分支**：成员不可放置（`Spline`/`Shape`/`Opaque`，或含 `Text` 的嵌套 `Compound`/`Insert`）⇒
   **整个**标注 `Opaque`+`Partial`（A.5"绝不部分变换"）——**比 P5a 前更严**（旧代码里一个含坏成员的
   `Insert` 仍会渲染其余部分）。
8. **Picture 分支策略（B1）**：picture 分支的 `measurement` 与文字内容**冻结**（picture 是权威显示），
   使组 42 与画面上的数字彼此一致，而不是让导出凭空造一个 picture 未显示的数字；**合成分支**则重算
   measurement 并重推文字。**诚实残差**：均匀缩放后，picture 分支的**画面数字**不再匹配被缩放的几何
   （picture 是不透明的外部几何）。
9. **Ordinate 语义（B2）**：`measurement` 与画面文字**相对基准**（feature − datum 在 X 或 Y 轴上的投影），
   与 acadrust 写入器重算进组 42 的口径一致，故文字、`measurement()` 与 42 一致。对**基准不在原点**的
   导入 ordinate 标注，这是**渲染正确性修复**（此前取绝对坐标，与写出的 42 不符）。**核实**：导入侧
   **从不做 datum 重基**（`dimension_definition` 原样取组 13/10），故旧的 `feature.x` 是**绝对值**。
   **取绝对值**（而非带符号投影），因为共享的 `format_measurement` 与 acadrust 写入器
   （`DimensionOrdinate::measurement` → `projected.abs()`）**都只有量值** ⇒ 这样三者对**任意输入**都一致。
   **代价（声明）**：**带符号 ordinate 坐标未建模**。
10. **递归护栏（B4）**：导入摊平现携带块栈（`dimension_picture_stack`，与导出侧对应），遇循环/超深回退
    `Opaque`+`Partial` 而非递归。**P5a 前的代码没有等价护栏，只因为那时这条递归不存在**（旧路径不把
    块成员放进变体）。
11. **样式子集拆分（偏离 9）**：`DimensionStyleValues` 只携带 B.2(a) 中**被合成消费**的子集；其余 (a)
    与全部 (b) 是 **P5b**。故凡"导出写已解析样式"的说法**必须软化**：DIMSTYLE 记录对 (b) 变量写的是
    **未钉住的默认值**，且 **DIMTAD 2/3/4 在导入时塌缩为 1**。
12. 组 70 **32 位不写**（acadrust 写入器无法置位；0.6.5 复核仍无法置位，见 P5d 小节），**0x80 位**
    （文字被用户定位）**从不置位**：对自建标注正确（我们从不用户定位文字），但导入的、曾置该位的标注会
    **丢掉**它——报告中的**已声明丢弃**。
13. 组 `210`（挤出）**不写**；−Z 挤出的角度标注保持 P5a 前的渲染（合成**重新取源法向**），但导出文件对
    它们**丢失 210**（报告条目）。
13b. **模型补充（A.3 修订项）**：`DimensionDefinition` 新增 **`normal: Point3`** 字段——**A.3 的点表原本
    没有这一项**，但恢复"−Z 挤出角度标注的旧渲染"必须携带源法向（合成据其定弧的绕向），否则渲染会变。
    该字段同时被 `is_consistent` 校验、被 `transformed` 映射。**A.3 的模型描述应按此补一行**。
14. **F.4 拆分**：P5a 落地 = F.1 往返（含 picture 分支第二段 + **定义点** + **组 70** 断言）+ F.2 不变量 +
    F.3 通配符回归三连 + **`DIMLFAC ≠ 1` 格式化测试**（已落地）+ picture 分支的不可放置成员拒绝、
    `Linear{rotation}`/`text_rotation` 旋转增量、EXPLODE 的 `Dimension` 臂、角度法向、组 210 丢弃等补测。
    **推迟到 P5b** 的只有 `DIMLUNIT ≠ 2` 诚实测试——且它是**结构性推迟**：`DimensionStyleValues`
    **根本没有** `dimlunit`/`dimpost`/`dimdsep` 字段，故不存在"静默按小数处理"的回退面可断言。
15. **N3 更正**：`flange_dimension_display_matches_shared_synthesis` 原先断言 `synthesized(d) == synthesized(d)`
    （**自证**）且注释宣称"与改动前逐字节相同"。已改为**非平凡**断言（导入标注的显示 == 用其**自身定义 +
    导入的样式值**做合成的结果，钉住"导入→合成"的交接），并把"逐字节相同"的措辞改为诚实表述——
    数值不变的证据是**保留的合成数值测试**（另经 Oracle 逐字 diff 核对）。

### 未用第三方读取器验证

内容块的必要性依据**第三方文档**（ezdxf，引于 §0.6），但本机**没有**第三方读取器（无
`ezdxf`/`dwgread`/`dxfgrabber`），故我们的往返只是**内部一致性**——只声称"采用了该出处所述 AutoCAD
所需的结构"，**从不**声称 AutoCAD 兼容。**NOT RUN**。

### P5b（DIMSTYLE 表 + B.2(a)/(b) 子集 + 默认 + 导入/导出接线；**无 UI 命令**，**0 个新 i18n 键**）

- **`cad-db` 新增 `DimStyle` 表**（`crates/cad-db/src/dim_style.rs`）：承载 **B.2(a) + B.2(b) 的全量变量集**。
  - `DimStyle::standard_metric`——**钉住的公制默认**（决策 B.1/N7 选定的一套）。
  - `DimStyle::resolved`——把 `DIMSCALE` **烘进各尺寸**（距离/偏移），**不含测量值**。
  - 创建时 API：`resolve_dim_style`（按名解析 + 把 `DIMTXSTY` 解析到 `StyleId`）、`dim_style_by_name`、
    `dim_style(s)`——**P5c 将使用**这三者（创建命令用"当前样式"= `Standard`，B.3）。
  - `finish` 保证库里**总有一个 `Standard`**（缺失即补建）。
- **`DimensionStyleValues`（`cad-domain`）扩到 B.2(a) 全集，且每个变量都被合成/格式化真正消费**（不再有"存而不消费"）：

  | 变量 | 消费点 | 适用 kind |
  |---|---|---|
  | `DIMTXT` | 合成文字子项高度 | 全部 |
  | `DIMASZ` | 箭头网格尺寸 | 有箭头的 kind（linear/aligned/angular/radial/diameter） |
  | `DIMEXO` / `DIMEXE` | 延长线起点 / 终点 | linear/aligned |
  | `DIMGAP` | 文字离标注线偏移（`text_offset` / leader 文本位置） | 全部（文本位置） |
  | `DIMTAD` | 文字垂直位置（`text_offset`） | **仅** linear/aligned |
  | `DIMTVP` | `DIMTAD=0` 时的文字垂直偏移比例 | **仅** linear/aligned |
  | `DIMSCALE` | 经 `resolved` 烘进各尺寸（构造期） | 全部 |
  | `DIMDEC` | 测量值格式化小数位 | 全部（经 `format_measurement`） |
  | `DIMZIN` | 零抑制（**现消费 bits 2+3**，见下） | 全部（格式化） |
  | `DIMLFAC` | 线性测量比例（乘进格式化输入） | 全部（经 `resolve_measurement_text`） |
  | `DIMRND` | 显示取整（**仅文字**，见偏离） | 全部 |
  | `DIMPOST` | 测量值前后缀 | 全部 |
  | `DIMDSEP` | 小数分隔符 | 全部（格式化） |
  | `DIMLUNIT` | 单位制判定（≠2 声明降级） | 全部 |
  | `DIMCEN` | 中心标记 / 中心线 | radius/diameter |
  | `DIMTXSTY` | 文字样式名（解析到 `StyleId`） | 全部 |
  | `DIMSE1`/`DIMSE2` | 抑制第一/二条延长线 | **仅** linear/aligned |
  | `DIMSD1`/`DIMSD2` | 抑制第一/二段标注线 + 箭头 | **仅** linear/aligned |

- **导入填充**：`read_dim_styles`（`crates/cad-import-acadrust/src/dimension/values.rs`）读入 DIMSTYLE 表并
  转成 `cad-db` 的 `DimStyle`；**缺失样式回退到钉住的 `Standard`**（不是沉默的任意默认）。
- **导出（N1 拆分）**：DIMSTYLE 记录携带**表中**的 B.2(b) 值，而**两个驱动显示的变量（`DIMLUNIT`/`DIMDSEP`）
  从实体快照写出**——因为它们在实体里真正喂了测量值格式化；表里的值可能与该实体已解析的样式不同。
  该拆分**显式声明**（不是"全从表写"）。
- **`transformed` 按均匀缩放同步缩放样式快照（N2）**：使导出记录里的尺寸与**被缩放后**的显示一致。
- **`DIMCEN` 直径修复（B1）**：中心标记画在**真实圆心**——对 `Diameter`，圆心是**两弦点的中点**，
  **不是**组 10（组 10 是**对径点**）；中心线**触及半径相对**（`radius + |DIMCEN|`），不是引线全长。
- **F.4 债务在此结清**：两条 `DIMLUNIT` 诚实测试落地——域
  `dimlunit_other_than_decimal_is_a_declared_degradation`（`crates/cad-domain/src/dimension_tests.rs`）、
  导入 `non_decimal_dimlunit_is_declared_partial_not_a_silent_decimal`
  （`crates/cad-import-acadrust/src/tests/dimension.rs`）。
- **i18n：0 个新键**——P5b **没有 UI 表面**（无命令、无面板、无状态投影）。计划里 P5c 行写的 "+ i18n" 指的是
  **P5b**；诚实的计数是**零**，这与决策 B.3（P5b 只做样式存储 + `Standard` + 创建时解析，不做可交互样式管理器）
  一致。

#### B.2 拆分更正（追加注记，不改写决策文本）

- `DIMLUNIT`/`DIMDSEP` **被格式化消费**，故实为 **(a)**（B.2 原表把它们列在 (b)）。
- `DIMZIN` 现**消费 bits 2+3**，故 B.2(a) 表里"仅消费 bit 3"的格子**已过时**。
- `DIMSE1/2` / `DIMSD1/2` 的消费**仅限 linear/aligned**（radial/angular 不读）。
- B.2(a) 的 `DIMGAP` 行**过度承诺**：文字周围的**标注线断口未被建模**，`DIMGAP` 只影响**文字位置**。

#### 偏离 / 决策（逐条，均须进能力表）

1. **渲染增量（门禁 B2）**：P5b **确实会改变已导入标注的渲染输出**（一般而言）——`DIMTAD` 2/3/4 现**真正实现**
   而不再塌缩为"上方"，且 `DIMSE1/2`、`DIMSD1/2`、`DIMCEN≠0`、`DIMRND`、`DIMPOST`、`DIMZIN` bit 2、`DIMDSEP`
   **新被消费**。这是**正确性改进，也是像素变化**。**已提交的 QCAD flange 经验证不变**：其 `QCADDimStyle` 只设
   `DIMASZ 2.5` / `DIMEXO 0.625` / `DIMEXE 1.25` / `DIMTIH 0` / **`DIMTAD 1`** / `DIMZIN 8` / `DIMTXT 2.5` /
   `DIMGAP 0.625`，且其六条标注**全是 aligned/rotated linear（无 radial）**。**任何"不变"的断言只限于该夹具**。
2. **`DIMCEN` 默认钉为 `0.0`**（而非文档化的公制 2.5，见下方 U-4 更新）；它**不会**静默改变已导入标注
   （导入从源样式解析；钉住的默认只到达手工构造的库与缺失样式回退），但**新建**的 radial 标注（P5c/P5d）
   在 AutoCAD 公制会画 2.5 中心标记处**不画中心标记**。
3. **`DIMCEN` 除放置 bug 之外的近似**：AutoCAD 只在**标注线落在圆外**时为 radius/diameter 画标记，**我们总是画**；
   负 `DIMCEN` 在 AutoCAD 是"中心标记 + 带缺口的中心线"，我们画**普通长十字**。
4. **`DIMTAD=3`（JIS）**实现为一个**独立代码**，其**垂直放置等于"上方"**；JIS 特有的**水平放置与文字适配规则**
   为**声明未实现**。
5. **`DIMSD1`/`DIMSD2`** 在**文字中点**处分割标注线——是 AutoCAD 文字间隙断口的**近似**。
6. **`DIMRND` 只取整显示文字**；组 42 保留**未取整**的测量值。
7. **`DIMPOST`**：无 `<>` 的模板按**后缀**处理（我们对未言明情形的读法）；AutoCAD 的 `DIMPOST = "."` 表示
   "无后缀"**未被遵循**（我们会渲染出 `42.5.`）。
8. **`DIMLUNIT ≠ 2` 是声明降级**，导入时以 `Completeness::Partial` 浮出；`DIMZIN` 的英尺/英寸位（1/2/16）
   **未实现**。
9. **`DIMCONTINUEMODE` 是图纸级**，acadrust 未建模：表里带文档化默认值，导出**无法写出**。
10. **`DIMTAD`/`DIMTVP` 只作用于 linear/aligned 文字**；radial/angular 保留各自的 leader/弧放置，**不读**它们。
11. **DIMSTYLE 记录的来源拆分**：B.2(a) 驱动显示的值来自**实体快照**，B.2(b) 来自**表**，缺失样式名回退到
    钉住的 `Standard`（一条**声明过的**报告行）。声明为 NOT IMPLEMENTED 的样式变量按 `DimStyle::new` 的
    **中性值**写出，**不是源值**。`DIMDLI`/`DIMDLE` **未缩放**地写进 `DIMSCALE=1` 记录，`DIMSCALE` 本身
    **不保留**（P5a 行保留）。
12. **`insert_dim_style` 拒绝**一个**已被别的名字占用**的样式 id（调用方 bug），而不是覆盖并**遗弃**旧名的索引项。
13. **取代 P5a 偏离 11 的两条现已错误的表述**："DIMTAD 2/3/4 塌缩为 1" 与 "(b) 写未钉住默认值" **都不再成立**
    ——P5a 偏离 11 的这两句**已被本 P5b 作废**。

#### 决策 E 更新（追加注记，不改写）

- **U-4 结清**：变量页写明公制 `DIMCEN` 初值是 **2.5**；我们**刻意钉 0.0**（中性：它不给否则不变的显示添加中心
  标记）。记为"**已文档化、刻意偏离**"，**不是**"未查证"。
- **`DIMTAD = 2` 从推断升级为文档化**：Autodesk 的措辞是"在**背离定义点**的一侧的标注线上"
  （on the side of the dimension line farthest away from the defining points），正是我们实现的。
- **新增 U 行**：`DIMTIH`/`DIMTOH`/`DIMTOFL` 在 `standard_metric` 里被断言为"Autodesk 公制 Standard"，但只有
  **英制**初值可被核实（On/On，与 acadrust `dxf_defaults` 一致）；文档化的公制样式（ISO-25）让文字**与标注线
  对齐**（off/off）。这些是 B.2(b)（显示忽略）故**显示安全**，但该断言需要**引用出处**或**降级**。

### P5c（`DIMLINEAR` + `DIMALIGNED` 创建命令 + React 标注面板/步进提示；**首批创建命令**）

- **命令面**：`CommandId::{DimLinear,DimAligned}` 贯通命令面——`crates/cad-app/src/command_id.rs` 新增变体
  （`requires_work_mode()`）、`crates/cad-ui-core` 的选择子表（`commands.rs`：`("DIMLINEAR", …)`/`("DIMALIGNED", …)`）
  与 `session/mod.rs` 的 `InteractionRequest::BeginDraw(DrawToolKind::{DimLinear,DimAligned})` 映射；
  `EXPECTED_COMMAND_COUNT` **60 → 62** 断言与选择子契约（每个变体在 `COMMAND_TABLE` 恰出现一次）同步更新。
- **绘制工具种类**：`DrawToolKind::{DimLinear,DimAligned}`（`ALL` 当时 25 种；**当前 30 种**，以 `kind.rs` 为准），`arity = PointArity::Exact(3)`
  （两次尺寸界线原点 + 尺寸线位置）；机器键 `"dimlinear"`/`"dimaligned"`（永不翻译），标签走 i18n。
- **`DrawIntent` 变体**：`DrawIntent::DimLinear{first,second,location}` / `DrawIntent::DimAligned{…}`；宿主
  `draw_intent_map.rs` 把两者映射为 `CommandPayload::Points([first, second, location])`（**复用**既有 payload，
  不新增变体）。
- **命令实现**（`crates/cad-app/src/app_drawing/dimension.rs`）：校验 → 从**当前样式**（`Standard`，经 P5b 的
  `DrawingDatabase::resolve_dim_style`）解析样式快照 → 经**共享合成** `DimensionGeometry::synthesized` 构造
  → 经**单一漏斗** `commit_new_entity` 插入**一个**实体（`type_key` `"AcDbDimension"`，一条事务、一步撤销、
  活动图层）。**非关联**（不写 `ACAD_DIMASSOC`）。
- **React 面**：`ribbon-model.mjs` 把标注族放在既有 annotate 面板**首位**（linear 在前、aligned 在后），
  `commands.mjs` 目录项、`icons.jsx` 专用图标（两条竖直尺寸界线 + 一条尺寸线；aligned 旋转 45°）；
  `cad.mjs` 的 `TOOL_STEP_PROMPTS` 按**宿主真实** `tool.key`/`tool.points` 派生每步提示（`StatusBar.jsx` 经
  `toolStepPromptKey`），从不来自本地镜像。**i18n 新增 9 键**：`tool.prompt.dim.{origin_first,origin_second,
  location_linear,location_aligned,ready}`、`tool.draw.{dimlinear,dimaligned}`、`draw.kind.{dimlinear,dimaligned}`。

#### 声明的决策（实现者逐条标记，未静默选择）

1. **三个拾取复用 `CommandPayload::Points([first, second, location])`**，**不新增** payload 变体（构造输入，
   与 ELLIPSE/RECTANG/POLYGON 一致；预构造几何臂会是第二个真值源）。
2. **`DIMLINEAR` 的 H/V 规则**：以两原点**中点**为基准，`|Δy| > |Δx|` ⇒ 水平（`rotation 0`，量 X），否则垂直
   （`rotation π/2`，量 Y）；**平局（含恰在中点拾取）是显式拒绝**，不做任意打破平局。平局规则标记为**我们的
   选择**（AutoCAD 对该退化拾取的行为**未查证**）。
3. **"非平面视图"拒绝**实现为 ADR **D.6 的平面外**情形（定义点必须共一个 `+Z` 平面），**不是**视口相机检查
   ——命令接收世界点、**无视口依赖**。显式声明，以免 ADR 措辞被读成相机检查。
4. **ADR 清单之外的额外拒绝**：**零测量值**（所选方向把两不同原点投影到同一点，`measurement ≤ 1e-9`）**按名
   拒绝**——否则会存下一个结构性退化的标注。
5. **当前样式硬编码为 `"Standard"`**（ADR B.3：P5 **无交互式样式管理器**）；数据库**没有** `Standard` 时是
   **防御性的显式拒绝**。
6. **每步提示派生自宿主真实工具状态**，从不来自本地镜像；`DIMLINEAR` 的位置提示写明该拾取**决定水平/垂直**，
   `DIMALIGNED` 的**刻意不写**（该说法对它为假）。
7. **声明的缺口——放置期间没有真正的标注预览**。实时预览是后端**橡皮筋链**
   （`crates/cad-app/src/render_scene/overlay/preview.rs` 把两种 kind 都路由到 `preview_chain`），故用户看到已
   捕获的原点与到光标的链，但**看不到**合成后的标注（偏移的尺寸线、尺寸界线、箭头、测量文字）。真正的预览需要
   **后端 overlay 分支**；React UI **根本无法**绘制 overlay（原生 CAD 分区是 X11 子窗口 / Wayland 子表面，DOM 不得浮于其上——ADR-0010 `:27`）。
   记为**声明限制**并附原因，并注明 React 宿主因此**改为获得步进提示**。**不**把它当作已完成。

   > **2026-10-23 前提更正**：句中“React UI 根本无法绘制 overlay / DOM 不得浮于其上”的**几何前提已失效**
   > （[ADR 0023](0023-dom-on-top-cross-host-composition.md) D1：DOM 在上、CAD 子表面在下）。**但缺口本身
   > 仍然存在**：真正的标注预览缺的是**后端 overlay 分支**（合成后的尺寸线/箭头/文字），不是前端绘制能力；
   > 该限制以新理由保留，**不得**据此认为预览已实现。

8. **未加 ribbon 激活态高亮**——今天没有任何 ribbon 命令有激活态；这两个命令**与邻居一致**，而不是为两个按钮
   发明高亮。
9. **一处值得记录的潜在发现**：后端 `DrawPreview::status_line()` 携带**硬编码中文**的每步文本，它**不在 React
   线上**（`CadTool` 只携带 `key/points/ready/openEnded/params`）。它是**既有且已冻结**的：
   `crates/cad-app/src/tests/cjk_baseline.rs` 守护一份记录的 CJK 基线，仅在**增长**时失败。P5c **未新增**任何
   CJK。记为**已基线化的债务**，不是新的违规。

#### 真机证据（2026-10-19）

- **真机证据（2026-10-19）**：真实硬件 GPU（Vulkan / **Quadro P620** / `device_type=discrete_gpu`）+ X11；驱动扩展
  `YACR_PROBE_DRAW_EVIDENCE` 的标注相位（合成 DOM 指针事件、**非真实鼠标**）：派发选择子 → 三次合成点击 → CONFIRM。
  **`DIMLINEAR` 30→31（+1）、`DIMALIGNED` 31→32（+1）**，`after_tool` 为 `null`（工具清空）、`after_error` 为 `-`、
  无任何 `*_failed`；总点击 50。运行目录 `/tmp/opencode/yacr-evidence-20261019T111646Z`（verdict `PLANNED_TIMEOUT` /
  exit 143 属预期，截图 `captured`）。**像素判据**：墨迹恰在两条标注的世界区域（0.1003 / 0.0405，对比此前无标注帧的
  0.0015 / 0.0017）；`DIMLINEAR` 尺寸线横跨 cols 839–944，**两端有箭头**（端点带墨迹 0.42 / 0.40）。
  **诚实边界**：① 输入是 Rust 经 `webview.eval` 派发的**合成 DOM 指针事件**（不经 evdev/XTest）；② `cad_status`
  **无几何通道** ⇒ 真机只证"提交成功 + 计数 + 无错误 + 内容存在"，**几何正确性由契约测试钉住**；③ 截图仅含
  **应用窗口自身**；④ 对比基线帧**窗口尺寸不同**（926×341 vs 960×439）⇒ **世界区域对齐**比较，非像素相同。

#### 门禁（编排者复跑，2026-10-19）

- **门禁（编排者复跑，2026-10-19）**：受影响 crate **1814 通过 / 0 失败**（`cad-app` 595、`cad-ui-core` 140）；
  workspace check(all-targets)、wasm lib、clippy(workspace `-D warnings`)、fmt、architecture(27)、i18n(**498**，P5c **+9**)、
  fixture(22)、file-size(**668**)、workflow、UI **133/133** + build 全通过。
- **受影响 crate 契约测试**：见 `crates/cad-app/src/tests/drawing/dimension.rs`、`crates/cad-app/src/draw_tool/tests.rs`、
  `crates/cad-app/src/host/tests/dimension.rs`、`crates/cad-ui-core/.../command_vocabulary.rs`、React
  `apps/app-linux/ui/test/{dimension-tools,commands,i18n}.test.ts`。
- **NOT RUN / 未证**：全量 `cargo test --workspace`（`accelerated_tier` 超时）；跨平台；
  **第三方读取器验证**（本机无 `ezdxf`/`dwgread`/`dxfgrabber`）。

### P5d（`DIMRADIUS` + `DIMDIAMETER` + `DIMCENTER`；**首批需要实体拾取的创建命令**）

- **命令面**：`CommandId::{DimRadius,DimDiameter,DimCenter}` 贯通命令面——`crates/cad-app/src/command_id.rs`
  新增变体（`requires_work_mode()`）、`crates/cad-ui-core` 的选择子表（`commands.rs`：
  `("DIMRADIUS", …)`/`("DIMDIAMETER", …)`/`("DIMCENTER", …)`）与 `session/mod.rs` 的
  `InteractionRequest::BeginDraw(DrawToolKind::{DimRadius,DimDiameter,DimCenter})` 映射；
  `EXPECTED_COMMAND_COUNT` **62 → 65** 断言与选择子契约同步更新。
- **绘制工具种类**：`DrawToolKind::{DimRadius,DimDiameter,DimCenter}`（`ALL` 当时 28 种；**当前 30 种**，以 `kind.rs` 为准）；
  DimRadius/DimDiameter `arity = PointArity::Exact(2)`（实体拾取 + 尺寸线位置），DimCenter `Exact(1)`（一次实体拾取）；
  机器键 `"dimradius"`/`"dimdiameter"`/`"dimcenter"`（永不翻译），标签走 i18n。
- **混合拾取（新机制）**：新增 `DrawToolKind::picked_step_count()`——**前导实体拾取步数**
  （TRIM/EXTEND/CHAMFER/FILLET = 全部步；radial 族 = 1；其余 0）。`needs_picked_entities()` 改为
  `picked_step_count() > 0`；`push_point`/`push_pick`/`requires_entity_pick`/`is_ready`/`intent` 改读它。
  故 radial 捕获：首击经 `push_pick`（宿主 `capture_tool_point` 的 `draw_requires_entity_pick` 路由），次击为自由点。
- **`DrawIntent` 变体**：`DimRadius{target,location}`/`DimDiameter{…}`/`DimCenter{target}`；宿主
  `draw_intent_map.rs` 映射为**新增**的 `CommandPayload::DimRadial(Box<RadialDimensionCommand>)`（radial 对）
  与 `CommandPayload::DimCenter(SelectionRef)`。
- **命令实现**（`crates/cad-app/src/app_drawing/dimension.rs`）：解析实体（`Arc`/`Circle`）→ 由位置拾取定方向 →
  构造 `DimensionDefinition`（组 15 = 圆上点；组 10 = 圆心（半径）或对径点（直径）；组 40 = 0.0；组 11/53 零哨兵）→
  经共享合成 `DimensionGeometry::synthesized`（`Standard` 快照）→ 经单一漏斗 `commit_new_entity` 插入**一个**
  `AcDbDimension`。`DIMCENTER` 经**新增**的 `commit_new_entities`（批量单一漏斗，一条事务/一步撤销）插入**普通 LINE**。
- **i18n 新增 9 键**：`tool.prompt.dim.{select_arc,location_radius,location_diameter}`、
  `tool.draw.{dimradius,dimdiameter,dimcenter}`、`draw.kind.{dimradius,dimdiameter,dimcenter}`（两目录键集一致）。

#### 声明的决策（实现者逐条标记，未静默选择）

1. **radial 的位置拾取只定方向**：`point_on_arc` = 圆心 + `半径·单位(位置−圆心)`；组 11/53 保持零哨兵，文字由合成放在
   引线中点。**"位置拾取定文字位置"未实现**——合成（P5a）把文字放在引线中点，不消费位置拾取。**声明**。
2. **`Arc` 不钳制扫掠**：引线点落在**该弧的圆**上（方向由拾取定），**不**钳制到弧的扫掠范围；方向落在弧外时引线仍指向圆。
   **我们的读法**（ADR 未述）。
3. **组 40（引线长）写 `0.0`**：合成不消费它、不画引线延长；`0.0` 是 AutoCAD 中性值。**声明**（非"符合规范"）。
4. **`DIMCEN` 符号读数**：`> 0` 中心标记（十字 LINE 对，臂 `|DIMCEN|`）；`< 0` 中心线（十字 LINE 对，达 `radius+|DIMCEN|`）；
   `0` 无。**近似（声明）**：AutoCAD 的负值"中心标记 + **带缺口**中心线"变体**未建模**（我们画普通十字，与径向合成
   P5b 偏离 3 同一声明）；ADR 决策 C/E 述"LINE/ARC"，**我们只产 LINE**（ARC 变体未建模）。
5. **`DIMCENTER` 十字对齐世界 X/Y 轴**（无径向拾取，故用世界轴；与 radial 合成的"沿引线方向"取向不同，**声明**）。
6. **`DIMCEN = 0`**：产出 0 实体 + 显式诊断 + **无事务**（同 ARRAY 1×1），**绝不静默空成功**。
7. **平面外拒绝**实现为 ADR **D.6**：实体法向含 X/Y 分量即拒（`−Z` 法向**接受**——仍与 XY 平行、径向合成不读法向）；
   位置拾取的 `z` 必须与圆心共面。
8. **退化半径**：命令层按名拒绝，但**经校验的数据库不可达**（`cad-db` validate 已拒非正/非有限半径）⇒ 该拒绝在
   **纯辅助函数**上以单测钉住（同一谓词、同名），命令层为**防御性**。
9. **`DIMCENTER` 不是标注**：能力表按**绘图辅助**而非 DIMENSION 记录（决策 D.7）；测试断言产出为 `AcDbLine`、**非** `Dimension`。
10. **无真正标注预览**（沿用 P5c 声明）：radial/DIMCENTER 的实时预览是后端橡皮筋链；React 宿主经每步提示。

#### P5 阶段门禁后的更正（F1：Diameter 组 10/15 曾写反）

- **事实**：Diameter 的导出曾把 `definition_point`（→ 组 10）填成**圆上点**、`angle_vertex`（→ 组 15）填成**对径点**，
  与 acadrust 读取器的约定（"组 15 = 第一弦点、组 10 = 其对径点"）及本 ADR A.3 **相反**。因为 radial 合成对
  `from`/`to` **对称**、且导入侧**镜像了同一个反向**，往返自洽、显示不受影响 ⇒ **无测试抓到**（这是"自洽 ≠ 正确"的
  典型案例）。**已改代码对齐约定**（导出与导入两向各交换 Diameter 的两个赋值），并新增 **Radius 与 Diameter 的
  组 10/15 逐值断言**（此前组码往返只覆盖 Aligned）。**不是**改文档迁就缺陷。

#### 真机证据（2026-10-19）

- **真机证据（2026-10-19）**：真实硬件 GPU（Vulkan / **Quadro P620**）+ X11；驱动新增 P5d 相位（每相先 `CIRCLE` 造圆 →
  在圆周上**实体拾取** → radial 再点定位 → CONFIRM；合成 DOM 指针事件、**非真实鼠标**）。
  **`DIMRADIUS` 33→34（+1）、`DIMDIAMETER` 35→36（+1）、`DIMCENTER` 37→39（+2）**，三者 `after_tool` 为 `null`
  （工具清空）、`after_error` 为 `-`、所有 `*_failed` 为 `-`。运行目录
  `/tmp/opencode/yacr-evidence-20261019T132645Z`（verdict `PLANNED_TIMEOUT` / exit 143 属预期，截图 `captured`）。
  **`DIMCENTER` 的 +2 是正确值**（产出**普通 LINE 对**而非标注）；报告带
  `dimcenter_output=plain_line_geometry_not_a_dimension`。**像素判据**：三个目标圆的环 ≈**100%** 命中、半径引线 100%、
  中心标记十字臂 100%、文字框 33–38%（文字稀疏）；空对照区 0.000。
- **一处更正（证据 lane 自查并重跑）**：`DIMCENTER` 的增量**不是 0**——夹具自带 `Standard` DIMSTYLE 且
  `DIMCEN = 0.09`（acadrust `DimStyle::new`），故**导入路径从源样式解析**，钉住的 0.0 默认只到达手工构造的库；
  与该 lane 先前"假设 `DIMCEN=0`"的注释矛盾，已删除该字段并重跑。**`cad_status` 无 DIMSTYLE 通道** ⇒ 有效 `DIMCEN`
  是由增量**推断**的，不是读到的。
- **诚实边界**：① 输入是 Rust 经 `webview.eval` 派发的**合成 DOM 指针事件**（不经 evdev/XTest）；② `cad_status`
  **无几何通道** ⇒ 真机只证"提交成功 + 计数 + 工具清空 + 无错误 + 内容存在"，**几何正确性由契约测试钉住**；
  ③ 截图仅含**应用窗口自身**（X11 `XGetImage`），非合成器帧、不含视觉正确性/vendor 结论；④ 抓帧里另有两条横/竖
  覆盖线（最后一次合成光标位置与世界原点），非驱动命令所产，已排除在存在性测量外；⑤ radial 的**位置拾取只定引线方向**，
  文字位置由合成推导——此处**不**暗示用户定位了文字。

#### 门禁（编排者复跑，2026-10-19）

- **门禁（编排者复跑，2026-10-19）**：受影响 crate 全绿（`cad-app` **663**、`cad-ui-core` **140**、`cad-host` **143**、
  `cad-domain` **53**、`cad-db` **84**、`cad-import-acadrust` **195**，合计 **1832 通过 / 0 失败**）；
  workspace check(all-targets)、wasm lib、clippy(workspace `-D warnings`)、fmt、architecture(27)、i18n(**P5d 时点 508**：P5d **+9**，P5e 后为 **517**、
  后续字形/文案修正 **+1**)、fixture(22)、file-size(**673**)、workflow、UI **145/145** + build 全通过。
- **受影响 crate 契约测试**：见 `crates/cad-app/src/tests/drawing/dimension_radial.rs`、
  `crates/cad-app/src/draw_tool/tests/dimension.rs`、`crates/cad-app/src/host/tests/dimension.rs`、
  `crates/cad-app/src/app_drawing/dimension.rs`（纯辅助单测）、
  `crates/cad-ui-core/src/session/tests/tests_commands/command_vocabulary.rs`、React
  `apps/app-linux/ui/test/{dimension-tools,dimension-radial-tools,commands,i18n}.test.ts`（**React 面由 designer lane 落地**）。
- **NOT RUN / 未证**：全量 `cargo test --workspace`（`accelerated_tier` 超时）；跨平台；
  **第三方读取器验证**（本机无 `ezdxf`/`dwgread`/`dxfgrabber`）。

### P5e（`DIMANGULAR` + `DIMORDINATE`；**U-8 以真实夹具钉住**；修复一处潜伏单位损坏）

#### 落地

- **命令面**：`CommandId::{DimAngular,DimOrdinate}` 贯通命令面（`EXPECTED_COMMAND_COUNT` 65 → 67）、draw-tool 种类、
  `DrawIntent`、宿主映射、单一插入漏斗（`AcDbDimension`、一条事务/一步撤销、活动图层）、显式命名拒绝且拒绝时不开事务。
- **`DIMANGULAR`**：**两线交互形式**（两次实体拾取 + 标注弧位置）；**三点形式**在命令/载荷层支持并有测试
  （`AngularDimensionInput::ThreePoint`）但**尚无交互捕获**——固定步数状态机表达不了 AutoCAD 的"选一条线
  **或**指定顶点"首步。**声明**（`DrawToolKind::DimAngular` 与命令文档均写明）。
- **`DIMORDINATE`**：特征点 + 引线端点；`OrdinateDimensionCommand::axis` 可强制 X 或 Y。
- **i18n**：+9 键（两目录键集一致）。

#### U-8 结案（**真实工件**，非"选一种读法"）

- **证据**：解码 `fixtures/dxf/qcad-examples/{entities,example01,isometric_grid}.dxf` 中 **9 条 type flag = 34**
  （块位 32 + 角度 2）的 `DIMENSION`。判别性用例 `*D65`（`14 ≠ 15` 且 `15` **不是**交点）：**组 42 =
  `1.975688113` rad = 113.19859°**，**恰好等于"顶点=两线交点"**的读法；把 `15` 当顶点得 112.56°，
  "忽略 14"的读法复现不出 42。且其**匿名块的 `ARC` 圆心在 `(90, 40)`**——两线交点——而非组 15 的 `(90, 39.375)`。
- **九条记录在该读法下全部数值复现组 42**，acadrust 自己的读取器与 `DimensionAnalg2Ln::measurement_radians`
  口径一致。
- **结论**：line 1 = 组 `13→14`、line 2 = `15→10`，**顶点是两条无限直线的交点**，**组 `16` 选择被测扇区**。
  新增 `angular_two_line_reading_is_pinned_by_the_committed_qcad_fixture`（导入夹具、断言组 42 弧度与块 ARC 圆心）
  与 `angular_lines_synthesis_uses_the_intersection_and_the_arc_sector`（域侧同数值）。
- ⇒ **决策 E 的 U-8 关闭**（不再是未查证项）。

#### 修复一处潜伏单位损坏（**重要**）

- **角度标注的组 42 曾按"度"写出**，而 DXF 语义是**弧度**（夹具全部为弧度）⇒ 此前导出的角度标注**数值是错的**。
  现 `measurement()` 仍以**度**用于文字格式化，但**导出把角度组 42 转为弧度**；报告有条目声明。新增
  `angular group 42 radians vs degrees-text` 测试。

#### 声明的决策（逐条，均须进能力表）

1. **U-1（四选一）**：被测角 = **包含标注弧位置拾取的扇区**；拾取**恰在边界射线上**时取**自该射线起的逆时针扇区**；
   三点形式同规则（可选**优角/劣角**）。**我们的规则**（U-1 仍标未查证）。
2. **U-5（自动 X/Y）**：按 feature 与 leader 的 `|Δy| > |Δx|` ⇒ **X**，否则 **Y** ⇒ **相等取 Y**（文档条件式的
   "otherwise"分支）。**我们的规则**（相等边界 Autodesk 未述）。
3. **仅接受 `Line` 拾取**：两线形式对 `Arc`/`Circle`/折线为显式 `Unsupported` 拒绝（ADR 范围排除弧/圆角度变体）。
4. **ordinate 基准 = 世界原点 `(0,0,0)`**，测量 = feature 的 X/Y **量值**（沿用 P5b 的基准相对取绝对值）；
   **零 ordinate 合法**（不拒绝）；引线长必须非零；输入必须为 `+Z` 平面。
5. **渲染变化（须声明）**：角度合成改为"交点顶点 + 扇区选择"后，**已导入的角度标注渲染输出会变**——
   这是**正确性修复 + 像素变化**（与 P5b 的渲染增量同性质）。文字仍是度，导出组 42 是弧度。
6. **三点形式无交互捕获**（见上）。

#### 真机证据（2026-10-19）

- 真实硬件 GPU（Vulkan / **Quadro P620** / `device_type=discrete_gpu`）+ X11；驱动新增 P5e 相位
  （先画两条共享顶点 `(−0.60,0.90)` 的直线：0° 射线与 45° 射线 → `DIMANGULAR` 点两条线**线身** + 弧位置
  （落在 0°…45° 扇区内）→ `CONFIRM`；再 `DIMORDINATE` 特征点 + 引线端点）。合成 DOM 指针事件、**非真实鼠标**。
- **`DIMANGULAR` 41→42（+1）、`DIMORDINATE` 42→43（+1）**；两者 `after_tool` 为 `null`、`after_error` 为 `-`、
  所有 `*_failed` 为 `-`；27 个 P5e 派发 **0 错误**；`clicks=70`。运行目录
  `/tmp/opencode/yacr-evidence-p5e-run2`（verdict `PLANNED_TIMEOUT` / exit 143 属预期，截图 `captured`）。
- **像素判据（空间性判据，较强）**：两条线的取样覆盖率 **110/110 = 1.000** 与 **71/71 = 1.000**；
  **角度弧在 `r = 0.25` 世界、3°…42° 处 40/40 = 1.000**，而**同半径 55°…85° 的空扇区对照弧 0/31 = 0.000**、
  `r = 0.60` 对照弧 0/36 = 0.000 ⇒ **画的是被选中的那个扇区**；ordinate 引线 53/53 = 1.000。区域对照：
  角度区墨迹 0.1086 vs 同尺寸空区 0.0000；ordinate 区 0.0403 vs 0.0000。
- **诚实边界**：① 输入是 Rust 经 `webview.eval` 派发的**合成 DOM 指针事件**（不经 evdev/XTest）；
  ② `cad_status` **无几何通道**（无定义点、无测量值、无扇区）⇒ 真机只证"提交成功 + 计数 + 无错误 + 工具清空 +
  墨迹存在"；**哪个扇区被量、U-8 点语义、组 42 弧度修复**均由契约测试钉住，**不由本帧证明**；
  ③ 截图仅含**应用窗口自身**（X11 `XGetImage`），非合成器帧；④ 单次有界本地运行（GNOME/Mutter + XWayland、
  P620、`scale=1`），无跨平台结论。
- **两处如实记录的未达成（其一已定位并修复）**：① 一次点击**被对象捕捉吸附**——目标是 `(−0.05,0.90)`，
  实际捕获到 `x ≈ 0.0`（落在夹具 `x=0` 竖线的 12 px 捕捉孔径内）；不影响结论（两次拾取仍各自落在自家线身上、
  顶点与扇区不变），但记录的 `−0.05` 是**点击目标**而非**捕获点**。② **标注测量文字**：独立全域判读确认该帧
  **全无字形簇**、**同帧所有标注都没有文字** ⇒ **不是分辨率问题，而是缺陷**。根因有**三道闸**：`load_engine`
  在 `requested` 与 `explicit` 皆空时**在使用 `font_dir` 之前**早退（连 `--fonts-dir` 也救不回）；表示层**只在
  `font: Some(..)` 时整形**（自建标注 `text_font: None` ⇒ 永不整形）；字体只在**打开漏斗**安装（新建标注后不重装）。
  **已修**（`needs_text_shaping(database)` 与"是否命名字体"解耦、`shape_unstyled` 走**既有回退链**且无引擎时报
  `ResourceMissing`（**绝不伪造字形**）、提交漏斗内 `ensure_fonts_for_text`），并保留"无文字就不加载多兆字体"的优化。
  **真机复验**：与修复前帧差分 **+1856 墨迹像素 / 0 移除**；读到 **`DIMORDINATE` "0.1"**（簇 `[102,190..147,216]`）、
  **`DIMALIGNED` "0.45"**、**`DIMRADIUS` "R0.18"`**；`DIMLINEAR`/`DIMANGULAR` 的文字**被画布顶边裁切**
  （驱动固定放置所致，非路径差异）⇒ **文字路径已证**，但 `DIMANGULAR` 的完整字形**未测量**（需改驱动放置）。

#### 门禁（编排者复跑，2026-10-19）

- **门禁（编排者复跑，2026-10-19）**：受影响 crate 全绿（`cad-app`、`cad-ui-core`、`cad-domain`、`cad-db`、
  `cad-import-acadrust`，**P5d 时点**合计 **1850 通过 / 0 失败**（P5e 之后为 1881，见下）；workspace check(all-targets)、wasm lib、
  clippy(workspace `-D warnings`)、fmt、architecture(27)、i18n(**517**，P5e **+9**、文案补正 **0 新键**)、
  fixture(22)、file-size(**682**)、workflow、UI **154/154** + build。
- **NOT RUN / 未证**：全量 `cargo test --workspace`（`accelerated_tier` 超时）；跨平台；第三方读取器验证；
  **标注文字渲染**已修并复验（见本节上方的"三道闸"与差分证据）；仅 `DIMANGULAR` 的完整字形因**驱动固定放置被画布顶边裁切**而未测量。

---

## P5d：acadrust 0.6.3 → 0.6.5 复核（2026-10-20，**不改写决策文本**）

**触发**：依赖从 `acadrust 0.6.3` 升到 `0.6.5`（上游 0.6.5 说明含 *"corrects dimension flag mapping"*、
更完整地保留标注数据）。本 ADR 中若干**偏离**是以"acadrust 写入器只能这样"为依据记录的；按项目规则，
被后续变更影响的决定**必须在文中更正**，不能悄悄失效。本节逐条给出复核结论与依据。

**方法**：对照 `~/.cargo/registry/src/rsproxy.cn-*/acadrust-0.6.3` 与 `acadrust-0.6.5` 的源码逐函数 diff
（`write_dimension*`、`write_dimstyle_entry`、`entities/dimension.rs`、`tables/dimstyle.rs`、DWG/DXF 读取器），
再用契约测试读回**导出字节**，最后跑真实夹具往返。**未**修改 acadrust，**无** `[patch]`。

### 结论矩阵（"原记录" → "0.6.5 实测"）

| 记录项 | 原依据（0.6.3） | 0.6.5 复核结果 | 处置 |
|---|---|---|---|
| **组 70 位 32**（内容块仅被本标注引用） | 写入器按 kind 的常量只含 `0x40`/`0x80`，**无法置位** ⇒ "32 位不写" | **仍然无法置位**：`write_dimension_base` 只写 `type_flags \| 0x80`；`type_flags` 常量逐行相同；`dwg_flags_byte` 只被 DWG 写入器消费，DXF 写入器**不读**它 | **偏离保持**（决策 D.4 / 偏离 12 已加复核注记）。新增断言：角度两种子类型的组 70 恰为 **2 / 5**（不是 34/37） |
| **组 42** 总写出、取自 `actual_measurement` | `write_double(42, base.actual_measurement)`；Ordinate 另重算 | **未变**：0.6.5 同行仍是 `write_double(42, base.actual_measurement)`；全 crate diff 中 `actual_measurement` 只命中一处测试；"corrected dimension flag mapping" 改的是 **DWG 标志字节**，与组 42 无关 | 不需要改；角度"弧度写 42 / 度写文字"的断言继续成立 |
| **角度组 70 = 2 / 5**（U-2） | Autodesk DXF 参考逐字列出；写入器常量 2/5 | **未变**：`write_dimension_angular_2line(..., 2, ...)`（`:4448`）、`..._angular_3point(..., 5, ...)`（`:4463`）两行逐字相同 | 新增契约测试钉住真实导出字节的标志值 |
| **DIMSTYLE 表**（组 3 解析） | `write_dimstyle_entry` 写出全部记录；我们按样式名各写一条 | **未变**：组码、组 3（`DIMPOST`）、xref 标志逐行相同；`DimStyle` 结构与 `new()` 默认值**零 diff**；`write_dimstyle_table` 零 diff。标注相关的唯一变化：写侧 DIMSTYLE 组 50 `DIMJOGANG` 改按**度**写（0.6.3 误把弧度夹在度区间），读侧对应把组 50 由度转弧度存储 | 我们**不建模** `DIMJOGANG`（默认 `π/4` ⇒ 不写出、不消费）⇒ 输出/导入无变化；导出 DIMSTYLE 往返测试继续通过 |
| **DIMJOGANG（0.6.4）** | —（0.6.3 不写 `$DIMJOGANG`，DIMSTYLE 组 50 单位有误） | 0.6.5 为 AC1021+ 写 `$DIMJOGANG`（弧度、夹在 5°..90°），DIMSTYLE 组 50 写度 | 本项目**不读不写** `DIMJOGANG`：导出新建空文档 ⇒ 只多出一个默认值头变量；**无**导入/导出语义变化 |
| **DXF 头保留（#127，0.6.4）** | —（`$ANGBASE`/`$ANGDIR`/`$ELEVATION`/`$THICKNESS`/`$FILLETRAD`/`$CHAMFER*` 不写出） | 0.6.5 写出这些变量（另加摘要信息、`$HYPERLINKBASE`、若干 R2004+ 变量） | 我们导出**从不复用源 header**（`CadDocument::new()`，只写 `$INSUNITS`）⇒ **无行为变化**；导入侧这些字段本就不消费 |
| **DWG 标志字节位 0**（"corrected dimension flag mapping"） | 0.6.3 把位 0 当作"文字被用户定位" | 0.6.5 **反转**：位 0 置位 = 文字在默认位置（`text_user_positioned = (flags & 0x01) == 0`），并同步 `dwg_flags_byte` | yacr **不使用** `text_user_positioned`/`dwg_flags_byte`（DXF 组 70 的 `0x80` 读取路径未变）⇒ 无影响；记录为上游语义修正 |

**结论**：**没有任何一条以"acadrust 写入器约束"为依据的偏离被 0.6.5 推翻**。最需要警惕的组 70 位 32
仍然不可置位（也没有公开 API 可绕开，绕过就只能伪造记录，违反"不修改 acadrust"），故能力表与
`docs/compatibility.md` 的表述**保持**，但已补上"0.6.5 复核"字样，避免读者以为是未复核的旧结论。

### 往返证据（真实夹具，本机 CPU/无 GPU）

- `fixtures/dxf/qcad-flange/flange.dxf`（**6 条无内容块的 DIMENSION**，组 70 = 32/36/35，`-1.0` 测量值）：
  `blockless_dimensions_round_trip_as_dimensions` 通过 ⇒ 数量、kind、测量值、样式名、
  **定义点 + 组 11/53**、文字覆盖全部保持。
- `fixtures/dxf/qcad-examples/entities.dxf`（44 条 K 块内标注，其中 **8 条角度**，组 70 = 34/162，
  组 42 为弧度）：新增 `qcad_angular_dimensions_round_trip_with_radian_group_42` 通过 ⇒ kind、样式名、
  测量值保持，且重新导出的**角度组 42 仍是弧度**（与 `measurement().to_radians()` 逐条一致）。
  **诚实边界**：该夹具的角度标注是**块成员**、由 INSERT 放置，导出会**烘焙 INSERT 变换**（既有、已声明的
  有损导出行为）⇒ 定义点由块坐标变为世界坐标，因此该测试**不**断言点相等；点保持由无块的法兰夹具覆盖。
- 本轮**未**采真实 GPU 帧（无渲染/UI 行为变化，纯依赖升级）；不声称任何像素结论。

### 本轮实际代码改动（仅 API 适配）

- `Dimension(Box<Dimension>)`、`MText(Box<MText>)`（0.6.5 把大实体装箱）：导出与测试构造改 `Box::new`，
  测试匹配改 `dimension.as_ref()`。
- `GradientColorEntry` 新增 `color_name`/`book_name`（颜色簿名）：测试构造用 `..Default::default()`（不伪造来源）。
- **未**新增/删除依赖；`Cargo.lock` 由 `cargo update -p acadrust --precise 0.6.5` 生成（传递新增
  `oem_cp 2.1.2` 及其 `phf 0.11.3`/`phf_shared 0.11.3` 依赖，用于 0.6.4 的旧代码页精确解码）。

## P5 阶段门禁结论（Oracle，2026-10-20）：**CLOSED**

- **门禁判定**：`closed-with-fixes`（1 个阻塞项 F1 + 文档腐化 F4 + 两条非阻塞 F2/F3）⇒ 全部修复后
  **P5 关闭**。门禁自行重跑了受影响套件与全部卫生门禁，并**用程序比对** `COMMAND_TABLE`(67) 与
  `EXPECTED_COMMAND_COUNT`(67) **完全一致**。
- **F1（阻塞，已修）**：Diameter 的组 10/15 **曾写反**（详见 P5d 小节的更正记录）。已**改代码对齐约定**
  （导出与导入双向），新增 **Radius/Diameter 的组 10/15 逐值断言** + **手工约定 DXF 的导入方向测试**；
  显示不变（radial 合成对称）。**回归证明**：临时回退改动后新测试**失败**，恢复后通过。
- **F2（已修）**：presenter 的尺寸守卫改为**有界等待**（`ProbeError::TextureNotReady` 前 2 个连续 tick，
  第 3 个才 `TextureInvalid`），`frame_cad` 把 not-ready 映射为 `Ok(None)`（等同"本帧无帧"），**绝不拉伸**。
  **残留（门禁已指出、实现者如实上报未扩大范围）**：tick 的另一处守卫 `drawing_area_matches_dom`
  （原点+尺寸超 ±1 逻辑像素）仍可能在中途 resize 的 tick 上给出 `cad present failed`；那是**一次性布局不变量检查**，
  不是 F2 指的宽高比守卫，扩宽它需要 per-tick 宿主状态并改动刻意的布局不变量 ⇒ **记录为已知残留**。
- **F3（已修）**：`ensure_fonts_for_text` 现在也挂在**所有编辑请求都经过的漏斗** `HostCore::execute_engine_request`
  上（以"确有事务提交"为门槛，只读请求不付代价）；测试证明**非 confirm 的提交**（同 paste/undo 形态）也会安装引擎。
  **仍未覆盖（声明）**：宿主自有的 Open/New/Plot/Save 漏斗（Open 自装）、以及不经 `HostCore` 直接使用 `cad-app`
  的场景（无渲染句柄可装）。
- **F4（文档腐化，已改）**：P5e 的 NOT RUN 列表不再说"独立判读中"（已改为"已修并复验，仅 `DIMANGULAR`
  被画布裁切未测量"）；handoff 的 P5a 偏离 ⑪ 加"**此句已被 P5b 作废**"行内标记；能力表 `P5c/P5d` → `P5d`；
  ADR 的 `ALL` 种类数标为"当时 28 / **当前 30**，以 `kind.rs` 为准"；门禁数字标注为 **P5d 时点** 并与 P5e 后区分。
- **P6 可否开工**：**可以**。可复用的机制（编译强制的命令词表、单一创建漏斗、共享合成 + 私有字段、声明账本）
  均健全；**必须带入 P6 的显式债务**：① **无真正的创建预览**（仅橡皮筋链；DOM 不得跨 CAD 分区浮动，ADR-0010 `:27`）——
  HATCH 会遇到同一堵墙；② 样式快照**不追溯**；③ `DIMPOST="."` 会渲染成 `42.5.`；④ DIMSD 的断口是**文字中点**近似；
  ⑤ F2 的**布局守卫残留**；⑥ F3 的**未覆盖路径**。

  > **2026-10-23 注**：括号内“DOM 不得跨 CAD 分区浮动”的几何依据已被
  > [ADR 0023](0023-dom-on-top-cross-host-composition.md) D1 取代。① 的债务**仍然成立**（缺后端 overlay 分支），
  > 只是原因不再是“前端画不了”。其余 ②–⑥ 与本轮无关，原样保留。


---

## 决策 C（2026-10-21 解禁 P5f）：`DIMBASELINE` / `DIMCONTINUE`

**为什么现在能做**：原文把 P5f 降级的理由是"**会话级 `last_dimension` 状态**在撤销/删除/切换文档时失效"（见 `~~P5f~~` 行）。
该风险的来源是**隐式**的"沿用上一次标注"。本决策改为**显式拾取基准标注**，于是：

- 命令**不持有任何跨命令状态**（基准是本次工具内的一个拾取，`picked_step_count = 1`）；
- 撤销/删除基准后，下一次命令**自然**要求重新拾取，**不存在**失效的悬挂引用；
- 与项目的**非关联**边界一致（实体里不存"第几行"或"基准是谁"）。

### 交互（两者相同的前半段）

**拾取基准标注 → N 个点 → 确认（Enter）**；`PointArity::AtLeast(2)` + `picked_step_count = 1`。
（**实施更正**：`push_pick` 把基准拾取**也存为一个点**（`draw_tool/mod.rs:400`），所以"至少 1 个**测量**点"
实际是 `AtLeast(2)`；写成 `AtLeast(1)` 会让 `is_ready()` 在**刚拾完基准、零个测量点**时就为真，
而命令自身会拒绝该确认——UI 也会过早显示"可以确认"。）
`Confirm` 一次性提交 **N 个**新标注（**一条事务/一步撤销**，用既有的批量插入路径 `commit_new_entities`），
每次确认后工具回到 `Idle`（**不做"重复到 Esc"的循环**）。

### `DIMBASELINE`（基线）

- 基准必须是 `DimensionKind::Linear` 的 `Dimension`（**对齐/角度/半径/坐标均拒绝**，具名）。
- 新标注 `i`（按拾取顺序 `i = 1..N`）：`first = base.first`，`second = P_i`，`rotation = base.rotation`，
  `dimension_line = base.location + n̂ · (i · dim_line_increment)`，
  其中 `n̂` 是**背离被测几何**的法向：`n̂ = ±rot90(direction)`，符号取使 `dot(base.location − midpoint(first, second), n̂) > 0` 的那个；
  点积为 0（基准落在几何上）⇒ 具名拒绝（**没有"随便挑一边"**）。
- 每行测量由既有合成重算（`synthesized` 已算 `measurement`/文字/显示）——**不新增实体字段**。

### `DIMCONTINUE`（连续）

- 基准同上（`Linear`）。
- 链式：第 1 段 `first = base.second, second = P_1`；第 `i` 段 `first = P_{i−1}, second = P_i`；
  `dimension_line = base.location`（**不堆叠**，与基线区分）。
- **拾取顺序即链方向**；某段沿 `direction` 的投影长度 ≤ 0（含 0）⇒ **具名拒绝**（不允许回折）。

### `DIMDLI` 的位置语义（一次性偏差，已声明）

`DIMDLI`（组 43，默认 `3.75`）今天**不进解析快照**且被 ADR 明确列为**显示忽略**（见上文 `:274`）。
本决策把它作为**创建期放置参数**引入：`DimensionStyleValues` 增加 `dim_line_increment`，
**只被 `DIMBASELINE` 的行距使用**，**显示仍忽略**。这是一处**显式偏差**，不是顺手改语义。

### 拒绝面（全部具名、不写、不开事务）

基准缺失/已删/跨文档/块内成员/子元素、基准不是 `Linear`、点为 0 个、点非有限、
某段长度 ≤ 0（基线：`first == P_i`；连续：投影 ≤ 0）、基线法向不可定（点积 0）、非 `+Z` 平面、缺活动图层、Viewer。

### v1 范围声明（**不**隐藏）

- 仅**线性**基准；**支持**角度/对齐/坐标基准的基线/连续 ⇒ 未实现；
- **不写** `DIMCONTINUEMODE`（acadrust 无法表达，本 ADR 早已记录）；
- 无关联性、无文字覆盖（沿用 `:309`）；
- **不做**"重复到 Esc"的循环；**不做**每点撤销（沿用 `AtLeast` 的 all-or-nothing 边界）。

**实施补充（2026-10-21）**

- **`AtLeast(2)` 的由来**：基准拾取计入 `points`（`push_pick` 的既有语义），因此"基准 + 至少一个测量点"= 2；
  UI 的提示表据此在 `points <= 1` 时**永不**显示可确认。
- **i18n 键**由 UI lane 先行落地（7 个，两目录一致）；Rust lane 不得重复插入，措辞如需改则保留**一套**。

### 实施（2026-10-21，已落地）

- **入口**：`CommandId::DimBaseline/DimContinue`（`EXPECTED_COMMAND_COUNT` 71→73）、`DrawToolKind` 键
  `"dimbaseline"/"dimcontinue"`、`AtLeast(2)` + `picked_step_count = 1`；路由别名 `DBA`/`DCO`（**声明的便利别名，不改行为**）。
- **载荷**：`CommandPayload::DimChain(DimensionChainCommand { base: SelectionRef, points: Vec<Point3> })`；
  `points` **不含**基准拾取（拾取是单独的一次）。
- **几何**：`d = (cos r, sin r, 0)`，`n = rot90(d)`；基线行 `i`：`s_i = dot(base.location − mid_i, n)`，
  `n̂_i = s_i > 0 ? n : −n`，`dimension_line_i = base.location + n̂_i·(i·DIMDLI)`；连续段 `i`：
  `advance_i = dot(P_i − prev, d)`，`dimension_line = base.location`。
- **`DIMDLI`**：`DimensionStyleValues.dim_line_increment`，来自 `DimStyle` 且**按 DIMSCALE 缩放**（与其他距离变量一致），
  非正/非有限回落 `3.75`；**只用于基线行距，显示仍忽略**。
- **提交**：`commit_new_entities_in` —— 一条 `begin_drawing_transaction`、**一个** `DrawingUndoRecord`（每实体一个 patch、同一事务 id），
  一次 revision 递增，全有或全无。
- **修的共享缺陷**：`push_point` 的"就绪后拒绝继续加点"规则**此前对开放式工具也生效**；`PLINE` 无前导拾取所以从未触发，
  P5f 一撞即暴露（拾取后点立即被拒）。现按**固定/开放式**分派，开放式继续接收点。
- **拒绝面 14 条**（全部具名、开事务前）：跨文档基准、块内成员/子元素、缺失、非 `Dimension`、非 `Linear`（含逐类命名）、
  载荷缺失、零测量点、非有限点、缺 `Standard` 样式、`+Z` 平面（点与基准各自一条）、`DIMDLI` 非正非有限、
  基线行退化、**法向不可定**、连续不前进。
- **门禁**：crate **2283/0**、UI **286/286**、clippy/fmt/architecture(27)、i18n **578**（**无重复键**）、file-size **763**、manifest 23。

### 真机证据（2026-10-21，有界）

- **硬指标**：`bc_pass=1`；`DIMLINEAR` 基准 `+1`；`DIMBASELINE` 拾取成功、3 个测量点、**`+3`**、无错误；
  `DIMCONTINUE` 拾取成功、3 个链式点、**`+3`**、无错误；选中行 `type:AcDbDimension, subtype:Linear{rotation:0.0},
  measurement:0.603834, display:6`。
- **像素**：一条**共线**尺寸线贯穿 `x 249→610`（y≈315），8 个外向箭头、关节处**背对背**（链式，非堆叠），
  段长与文字 `0.6 / 0.4 / 0.4 / 0.4` 一致，文字中心与各段中点重合 ⇒ **`DIMCONTINUE` 的链式语义由像素证明**。
- **基线行不可见（已声明）**：该夹具 `DIMDLI = 3.75` 单位 ≈ **745 px**，而画布高 341 px ⇒ 堆叠行**必然在视野外**；
  基线的证据是**计数 + 无错误**，契约测试钉几何。
- **一处必须说清的"看起来坏了"**（**归因：既有，不是 P5f**）：文字在线上方约 160 px、延伸线看似缺失。
  实测与公式**逐项吻合**（误差 1–2 px）：文字偏移 = `DIMGAP + DIMTXT` + 半个 em 中锚位移 = 0.805 单位 ≈ 160 px；
  延伸线**确实画了**，但 `DIMEXO 0.625` 比"原点到尺寸线"深度（22 px）大 5.6 倍 ⇒ 起终点被推到线上方 102/248 px，
  横跨文字。
  **根因**：该夹具的 `Standard` DIMSTYLE **量级自相矛盾**（`DIMTXT/DIMASZ = 0.18` 英制量级配
  `DIMGAP/DIMEXO/DIMEXE = 0.625/1.25` 公制量级，图形仅 ~1 单位，来自 acadrust 写入器默认值）。
  `DIMLINEAR` 与 P5f 走**同一条**`resolve_dim_style` + `synthesized`，故**行为共享**。
  **结论**：这不是 P5f 的几何错误；未来尺寸类**视觉证据应使用单位量级自洽的图纸**（本夹具不适合判定标注排布）。

### 夹具 `Standard` DIMSTYLE 更正（2026-10-21）

上一节把"文字浮 160 px、延伸线看似缺失、基线行不可见"归因到**夹具 DIMSTYLE 量级自相矛盾**。现已更正：

- **可复现生成器**：`crates/cad-import-acadrust/tests/fixtures_generate.rs`（`#[ignore]`，幂等；读→只改 DIMSTYLE→写回；
  跨 DWG 读写迭代到字节固定点；**几何不一致则拒绝覆盖**）。运行：
  `cargo test -p cad-import-acadrust --test fixtures_generate -- --ignored`。
- **只改量级**：`DIMGAP 0.625→0.06`、`DIMEXO 0.625→0.06`、`DIMEXE 1.25→0.12`、`DIMDLI 3.75→0.5`；
  `DIMTXT/DIMASZ 0.18`、`DIMSCALE 1.0`、`DIMDEC 2`、`DIMZIN 8`、`DIMPOST "<>"`、`DIMTAD 1` **不变**。
  **四条 `LINE` 坐标与数量不变**（生成器在写前/写后/每次往返各断言一次）。
- **顺带如实记录**（与内容无关，稳定后幂等）：首次 DWG 往返会把 `CLASSES` 表按哈希序重排、并丢弃一个 DWG 对象写入器
  不持久化的 TABLESTYLE 行抑制标志（组 71）；已写入 `fixtures/manifest` 的 `known_limits`。
- **复跑结果（同一脚本、同一点击）**：文字偏移 **160 → 49 px**；**三行基线首次出现在画面上**
  （y = 215/116/16.5，**共左端 x=249**，间距 99–99.5 px，长度 201/281/361 px，各带外向箭头）；
  连续仍为背对背链式（369→450→530→610）。
- **仍未"完美"，且明确归因于证据版式而非产品**：最高一行（y=16.5）的文字被**画面上缘裁掉**
  （其 49 px 偏移 + 字高超出 341 px 画布），故**三行渲染、两行带标注**；标注文字与夹具既有家具（填充矩形边、`LEAD` 注释）
  有交叠；两条外侧延伸线因**共享原点**必须贯穿整叠，被上缘裁切。**结论**：夹具修复达成了目的（排布可判读），
  进一步的"干净截图"应换更稀疏的图纸或调整点击位置，**不是产品缺陷**。
