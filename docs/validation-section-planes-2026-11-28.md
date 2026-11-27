# 文档剖面（SECTIONPLANE，`.ymodel` minor 32）（2026-11-28）

一个文档级剖面：`tables/sections.json` 的 origin/unit normal/active；`SECTIONPLANE` 写入，
模型空间场景构建按半空间**真实裁剪**批次，场景可保存/恢复。设计见
[ADR-0103](adr/0103-section-planes.md)。

## 1. 已接线

- **模型/持久化**：`SectionPlane { origin, normal, active }`（法向归一化），`tables/sections.json`
  一行；有行写 **minor 32**；`scene.section` 捕获 `Some(plane)`/`Some(None)`，旧档 `None`。
- **命令**：`SECTIONPLANE ON|OFF|TOGGLE|CLEAR|LIST|SET ox oy oz nx ny nz`；裸命令 = TOGGLE。
  每次写一 revision/一步撤销；`LIST` 只读、Viewer 合法；无平面 ON/OFF/TOGGLE 按
  `section.plane_missing` 拒绝。
- **渲染裁剪**：`cad-scene::clip_delta` 裁剪 Mesh（插值位置/法向/颜色/UV + 重三角化）、
  Lines/MeshEdges、Conics（激活时转线段）；切边进入 `edges`；简单闭合切边环耳切成 cap
  三角，嵌套孔洞/非流形边界跳过 cap；光栅图片四边形/既有 clip 多边形按半空间裁剪并插值 UV；
  wipeout mask 与纸空间不裁剪。
- **场景**：`SCENE SAVE` 捕获剖面，`SCENE RESTORE` 有差异时写一次 section slot 并产生一步
  撤销；Viewer 有差异在改动前拒绝。
- **UI**：View → Section Planes 是真实 `SECTIONPLANE` 命令（不再是 pending）。
- **边界**：单平面；简单闭合切面有 cap，带孔/非流形只画切边；图片几何裁剪、mask/点云不裁剪。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-scene --lib --locked` | PASS（**95 passed**：clip 三角/线段/整批丢弃/conic/box cap/image+UV） |
| `cargo test -p cad-db --lib --locked` | PASS（**529 passed**：section table/snapshot/undo replay） |
| `cargo test -p cad-history --lib --locked` | PASS（**43 passed**：section step 校验） |
| `cargo test -p cad-app --lib --locked` | PASS（**3578 passed**：命令/撤销/Viewer/场景 + 场景裁剪集成） |
| `cargo test -p cad-store-zip --test scenes_minor --locked` | PASS（**8 passed**：minor-32 section/场景 section） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**999 passed**：SECTIONPLANE 语法/selector） |
| `(cd apps/desktop-ui && npm test)` | PASS（**2162 passed**：本地 admission + menu 行） |
| `cargo fmt --all -- --check` / file-size / i18n / architecture / command-docs | PASS |

**边界**：没有真实 GUI 点击/GPU/像素证据；裁剪是 CPU 场景批次的契约测试。Linux 之外的载体
运行时 **NOT VERIFIED**。

## 3. NOT VERIFIED / 不在范围

- 多剖面/section box/jog/settings/转块、带孔/非流形切面的 cap、wipeout mask/点云裁剪；
- 真实 GUI/GPU 交互与所有非 Linux 载体。
