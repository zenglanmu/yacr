# ADR-0092：屏幕空间参考线虚线（Camera-scaled guide dashes）

- 状态：**已接受（决策，2026-11-28）**；**已实现（2026-11-28，guide 移入相机 overlay；无真实 GPU 证据）**
- 关联：ADR-0084/0085/0086（guide 表、虚线）、`OverlayInputs.view_scale`

## 背景

ADR-0086 在 **base scene** 里把 guide 切成虚线，dash 周期按模型包围盒取（extent/100），因此
缩放时 dash 的屏幕长度会变。base scene 与相机无关、不会在 zoom 时重建；要实现屏幕空间一致的
dash，必须把 guide 画到**相机相关、每次导航重建的 transient overlay** 中。

## 决策

- guide 批次从 `assembly` 的 base scene 移除，改由 `overlay/guides.rs` 生成：
  - line：有界端点（模型包围盒 ×10、≥1000）用 `subdivide_dashes` 切分，dash/gap =
    **6/4 逻辑像素 × `OverlayInputs.view_scale`**；`view_scale` 缺失/退化时回退到模型
    `extent/100`（≥5 单位）并保持真实 pattern。
  - point：三轴十字，臂长 = 6px × `view_scale`（回退 extent×0.005，钳制 1..25）。
  - 颜色/线宽/合成 source 不变；guide 仍不进入 base scene、实体索引、拾取或导出。
- `overlay_fingerprint` 加入 `view_scale`，缩放会重建 overlay，dash 屏幕长度保持稳定；base
  scene 的缓存与增量路径不变。
- `GUIDE_DRAW_ORDER = 840_000`（axes/grid 之上、POINT markers 之下）。

## 不在本 ADR 范围

- 用户自定义 dash 长度/颜色、屏幕空间抗锯齿/像素验证；
- guide 的 base-scene 选中/擦除（仍非实体）；
- 真实 GUI/GPU 与其它载体运行时。

## 后果

- guide 的虚线周期与缩放一致（CPU overlay 重建），但 guide 不再属于 base scene，base 场景测试
  改为 overlay 测试；没有格式/命令变化。
- `view_scale` 变化现在会触发 transient overlay 重建（连同既有 bounds），这是屏幕空间 dash 的
  必要代价。
