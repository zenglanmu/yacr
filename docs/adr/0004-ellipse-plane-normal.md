# ADR 0004：`SemanticGeometry::Ellipse` 携带平面法向

状态：采用。范围：审计 B23（非均匀变换/椭圆语义）与规范 §16.1。

问题：域模型 `Ellipse { center, major_axis, ratio, start, sweep }` 没有平面法向，
只能约定 `minor = cross(world_z, major)`。这只覆盖“平面包含世界 Z 方向”的椭圆：

- 任意 OCS 平面上的 ELLIPSE 无法表达，导入器只能标记 `Partial` 并把它压平到世界 XY；
- 非均匀变换把法向不平行世界 Z 的 CIRCLE/ARC 变成倾斜椭圆后无法编码，只能近似；
- 文档在 `docs/compatibility.md` 声称“保留 normal/OCS”，与实际契约不符。

选项：

1. 新增独立变体 `Ellipse3d`：污染所有 `match SemanticGeometry`，且消费者必须同时处理两种椭圆。
2. 继续用有理 NURBS 承载倾斜椭圆：形状精确，但丢失椭圆语义（ratio/两轴），测量与拾取只能走样条。
3. 在现有 `Ellipse` 变体上加一个 `normal: Point3` 字段：语义最直接，改动面最小。

选择：选项 3。新增字段，约定 `minor = cross(normal, major_axis)`；世界 Z 椭圆不变
（`normal = +Z` 时退化为原约定）。所有既有匹配都使用 `..`，因此只有构造点需要更新。

兼容性与迁移：默认法向取 `+Z`；导入器对退化法向回退到 `+Z`，不再产生“压平但声称完成”
的假成功。世界 Z 的几何与测试结果逐位不变。`AnnotationGeometry::Ellipse` 使用
`axis_u/axis_v`，本就支持任意平面，不受影响。

测试：`crates/cad-geometry/tests/invariants.rs`（倾斜椭圆保持自身平面）、
`crates/cad-geometry/tests/curves.rs`（任意法向、非均匀圆→椭圆、回代校验）、
`crates/cad-import-acadrust/src/lib.rs`（倾斜椭圆 Complete 且保留法向、倾斜圆 OCS→WCS）。
