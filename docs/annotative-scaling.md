# 注释性缩放（标注的比例驱动显示）

`注释性缩放` 支持窗口内，**TEXT/MTEXT 的注释性缩放**已实现；标注样式、
多重引线、图案填充、块的注释性缩放为 **Partial/未支持**，明确建模且不伪造
几何。

> 状态：**契约 + 合成测试**。端到端 DWG 写入 → 回读的合成样本受 acadrust
> 写入器能力限制，本轮以“内存合成 acadrust 文档 + 纯映射函数 + 表示层单测”
> 验证导入与显示两条路径。

## 1. 数据来源（acadrust，只读）

- `objects/scale.rs` 的 `Scale { handle, name, paper_units, drawing_units,
  is_temporary, ... }`：图纸 `ACAD_SCALELIST` 的命名比例。
- 头变量 `CANNOSCALE`（当前注释比例名）与 `CANNOSCALEVALUE`（原始纸/图比值）。
- 实体通用标志 `annotative`（是否参与注释性缩放）。
- 每个实体的 `AcDb*ObjectContextData` 叶子携带**按比例**的位置/旋转/字高
  覆盖，引用一个 `Scale` 句柄。

## 2. 数据库模型（cad-db）

`DrawingDatabase` 新增：

```rust
pub struct Scale {
    pub id: ScaleId,
    pub name: String,          // 原样，例如 "1:100"
    pub paper_units: f64,
    pub drawing_units: f64,
}
impl Scale {
    pub fn factor(&self) -> f64;          // paper/drawing，退化时回退 1.0
    pub fn inverse_factor(&self) -> f64;  // drawing/paper
    pub fn is_unit_scale(&self) -> bool;
    pub fn is_reduction(&self) -> bool;   // 缩小，如 1:100
    pub fn is_enlargement(&self) -> bool; // 放大，如 2:1
    pub fn is_well_formed(&self) -> bool; // 比值有限且 drawing != 0
}

pub struct ActiveAnnotationScale {
    pub name: String,
    pub value: f64,   // CANNOSCALEVALUE 原始值
    pub named: bool,  // 名字是否解析到 Scale 表；未解析仍保留真实 value
}

pub struct AnnotativePlacement {          // 每条覆盖的几何
    pub position: Point3,
    pub rotation: f64,
    pub height: Option<f64>,
}
pub struct AnnotativeScaleOverride {
    pub scale: String,                    // 匹配 Scale 名
    pub placement: AnnotativePlacement,
}
pub struct AnnotativeAttributes {
    pub annotative: bool,
    pub overrides: Vec<AnnotativeScaleOverride>,
}
```

`EntityRenderAttributes` 新增 `annotative: AnnotativeAttributes`（默认
`annotative: false`、空覆盖），既有调用方行为不变。

查询/写入：

- `DrawingDatabase::scales()`、`scale(id)`、`scale_by_name(name)`；
- `active_annotation_scale()`、`annotation_scale_name()`、
  `annotation_scale() -> (Option<String>, f64, bool)`；
- `DrawingDatabaseBuilder::insert_scale(scale)`：拒绝空名与非有限比值；
- `DrawingDatabaseBuilder::set_active_annotation_scale(name, value)`：拒绝空名
  与非有限 value；**不**要求名字已在表中（未知名正是 `Partial` 场景）。

## 3. 表示层（cad-representation）

`RepresentationContext` 新增 `annotation_scale: Option<AnnotationScaleRef>`：

```rust
pub struct AnnotationScaleRef { pub name: String, pub factor: f64 }
```

- `with_annotation_scale(ref)` 设置；默认 `None`，既有调用方不变。
- 只有**被导入标记为 annotative** 的 TEXT/MTEXT 才缩放：
  * 字高按 `factor` 缩放（`height.abs() * factor`）；
  * 有该比例的 `AnnotativeScaleOverride` 时，位置/旋转/字高用覆盖值；
  * 非注释性实体、或 context 无 `annotation_scale` 时，几何完全不变。
- `ProviderRegistry::rebuild_annotative(db, context, scale)`：对数据库中所有
  注释性实体用给定比例重建 `(EntityId, DisplayRepresentation)`，供宿主在
  比例切换时做增量重建。

## 4. 明确 Partial / Unsupported

| 场景 | 结果 |
|---|---|
| 注释性 TEXT/MTEXT | 精确缩放（`Complete`） |
| 注释性但非 TEXT/MTEXT（标注、引线、填充、块） | `Partial` + `annotative.unsupported_entity`，按基础尺寸绘制 |
| `factor` 非有限或 ≤ 0 | `Partial` + `annotative.invalid_scale`，按基础尺寸绘制 |
| 导入期 `CANNOSCALE` 名不在 Scale 表 | 导入诊断 `import.annotation_scale_unknown`，保留真实 value |
| 导入期 `CANNOSCALEVALUE` 非正/非有限 | 导入诊断 `import.annotation_scale_invalid`，存安全的 1.0 |

诊断码（表示层）：`annotative.unsupported_entity`、`annotative.invalid_scale`。
导入诊断码：`import.annotation_scale_unknown`、`import.annotation_scale_invalid`。

## 5. 未接线（显式待办）

- 宿主的比例切换 UI/命令尚未调用 `rebuild_annotative`；本轮只有 DB +
  表示层契约与单测。
- 标注样式 / 多重引线 / 填充 / 块的注释性缩放未实现，按上表 `Partial`。

## 6. 测试

- `cad-db`：`factor` 数学（1:1 / 1:100 / 2:1 / 退化回退 / 非有限）、表名解析、
  活动比例 `named` 真/假、builder 拒绝空名/非有限输入。
- `cad-representation`：注释性文本按 `factor` 缩放（比值断言，避免依赖无字体
  占位几何的绝对尺寸）、非注释文本忽略比例、按比例覆盖生效、
  非法 factor → `Partial` + `annotative.invalid_scale`、
  非文本注释实体 → `Partial` + `annotative.unsupported_entity`、
  `rebuild_annotative` 只返回注释性实体。
