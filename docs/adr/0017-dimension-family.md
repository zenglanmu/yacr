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
3. **acadrust 0.6.3 的写入侧完整支持标注**（`io/dxf/writer/section_writer.rs:4196 write_dimension`），
   **无需修改 acadrust**。
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
  - **组 42 = `measurement`**：写入器**总是**写组 42（`section_writer.rs:4289`，Ordinate 路径还重算），
    不写等于写出 **`42 = 0.0` 的假测量值**。"只读"指**用户不能输入**，不是"不写"。
  - **组 70 的 32 位不写**：该位语义是"组 2 的块仅被本标注引用"（R13+ 恒置），而 acadrust 写入器**无法置位**
    （按 kind 的常量只含 `0x40`/`0x80`，`section_writer.rs:4283-4288`/`4397`）⇒ **删去原"类型标志 + 32 位"
    的表述**，改为**显式偏离记录**："32 位不写（acadrust 写入器约束）"。`0x40`（坐标 X 型）/`0x80`
    （文字被用户定位）与 Autodesk 参考一致。
  - **导出 DIMSTYLE 表**（N5）：写入器支持（`section_writer.rs:1500-1534`）；我们为每个被写出的样式名写一条
    DIMSTYLE 记录（含决策 B 子集值），**否则组 3 悬空、外部 CAD 用它自己的 `Standard` 解析** ⇒ 静默显示漂移。
    若某样式无法写出，记入 `ExportReport` 的 `Converted/Dropped` 条目，**不得沉默**。
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
   **不写组 70 的 32 位**（acadrust 写入器约束）、**丢弃**组 51/52/71/72；非关联、无 `ACAD_DIMASSOC`。
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
