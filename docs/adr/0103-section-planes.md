# ADR-0103：文档剖面（SECTIONPLANE + 模型空间裁剪）

- 状态：**已接受（决策，2026-11-28）**；**已实现（2026-11-28，模型/命令/场景/CPU 场景裁剪；无真实 GUI/GPU 证据）**
- 关联：ADR-0063（scenes）、`tables/sections.json`、`tables/scenes.json` 的 `section`

## 背景

SketchUp 的 Section Plane 用一个无限平面切开模型；此前 `SECTIONPLANE` 是具名拒绝，场景也
无法保存剖面。渲染器有 WGPU 管线但所有 shader 各自携带 uniform，逐 shader 加裁剪平面会扩大
每个着色路径的契约；场景构建已经产出 CPU 顶点/索引批次，因此把裁剪放在**批次边界**是更小的
接入面，也能覆盖线、弧、网格和文本字形。

## 决策

- 文档持有**一个**可选平面 `SectionPlane { origin, normal (unit), active }`，行落在
  `tables/sections.json`；有行即写 **minor 32**。`active=false` 保留平面但不裁剪。
- `SECTIONPLANE` 是真实命令（`CommandId::SectionPlane`）：
  `ON | OFF | TOGGLE | CLEAR | LIST | SET <ox> <oy> <oz> <nx> <ny> <nz>`；裸命令 = `TOGGLE`。
  `SET` 归一化法向；零/非有限输入按 `section.invalid` 拒绝；`ON/OFF/TOGGLE` 无平面按
  `section.plane_missing` 拒绝；每次写入一 revision/一步撤销，`LIST` 只读、Viewer 合法。
- **真实裁剪**：`cad-scene` 的 `ClipPlane`/`clip_delta` 在模型空间批次上保留
  `dot(normal, p - origin) <= 0`：
  - `Mesh` 逐三角形 Sutherland–Hodgman 裁剪、扇形重三角化，位置/法向/颜色/UV 线性插值；
  - `Lines`/`MeshEdges` 逐段裁剪；
  - `Conics` 在剖面激活时按线段采样后裁剪（SDF conic 在激活剖面下退化为线段，避免一条
    未被裁剪的解析弧）；
  - 网格切边追加到 `edges` 流；切边闭合环用耳切法生成 cap 三角（非流形边界或
    嵌套孔洞整批跳过 cap，避免把孔填死）；切面法向取 `+normal`。
  - 图片/wipeout mask 不参与裁剪；纸空间不裁剪。
- 场景保存 `Some(plane)`/`Some(None)`（真实“无平面”捕获）/`None`（旧档不触碰）；恢复时按
  display 同样的方式写一次 section slot，Viewer 有差异时在改动前拒绝。`section` 行使归档
  minor 32。
- UI：View → Section Planes 从 pending 行变成真实 `SECTIONPLANE` 菜单命令；命令线同样可用。

## 不在本 ADR 范围

- 多个剖面、剖切盒（section box）、剖面 jog/settings/转块、带孔/非流形切面的 cap、隐藏几何；
- 图片/点云裁剪、真实 GUI 交互/GPU/像素与其它载体运行时。

## 后果

- 用户可以通过命令/菜单建立并开关一个文档剖面，模型空间批次被真实裁剪并显示切边；
- 场景可保存/恢复该剖面；档位升到 32；
- 简单闭合切面有 cap；带孔/非流形切面只画切边（明确边界，不能声称完整填充）。
