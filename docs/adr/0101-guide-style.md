# ADR-0101：参考线样式（GUIDE STYLE COLOR/DASH）

- 状态：**已接受（决策，2026-11-28）**；**已实现（2026-11-28，style 行 + overlay + Styles 托盘控件；无真实 GUI 交互证据）**
- 关联：ADR-0084/0085/0086/0092（guide 表、虚线、相机 overlay）、`tables/guides.json`

## 背景

guide 的颜色和虚线开关此前是 overlay 常量；用户不能改，覆盖表把“guide styling”列为未建模。
guide 表已有 line/point 行，适合增加一条**文档级共享样式**，而不是每行重复颜色。

## 决策

- `GuideStyle { color: [u8;3], dashed: bool }`，默认 `#3373d9`（旧 overlay 常量）且 dashed=true。
- `tables/guides.json` 在非默认样式时多一行 `{"style":{"color":[r,g,b],"dashed":bool}}`；
  旧数组（无 style 行）解码为默认。样式行最多一条，未知键/类型按 `guide.style_invalid` 拒绝。
- 存在非默认 style 行时归档写 **minor 30**；只有 line/point 行仍是原档位；样式行本身也会让
  空 guide 表被持久化。
- `GUIDE STYLE COLOR <r> <g> <b>` / `GUIDE STYLE DASH ON|OFF` 是一次整表写入/一步撤销，
  Work-only；Guide LIST 报告当前颜色/虚线。无新命令 ID。
- 相机 overlay 使用 `GuideStyle`：颜色转为 sRGB 浮点；dashed=false 时 line 不切段、pattern
  continuous；point 十字始终实线。
- `cad_status.guideStyle {color, dashed}` 在**有文档时**投影该表样式（旧宿主缺字段时 UI 用默认）；
  concept 域 Styles 托盘提供原生颜色输入与虚线开关，分别 dispatch `GUIDE STYLE COLOR` /
  `GUIDE STYLE DASH`，不新增命令。

## 不在本 ADR 范围

- 每 guide 独立样式、屏幕空间恒定 dash（已由 ADR-0092 处理）；
- 真实 GUI 交互/GPU/像素与其它载体运行时。

## 后果

- guide 颜色/虚线可持久化并由 renderer 与 Styles 托盘消费；guide 表档位升到 30。
- 托盘控件只存在于 concept 域；其它域仍通过 typed `GUIDE STYLE` 设置。
