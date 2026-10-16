# yacr — Rust CAD 系统（开发中）

[English](README.md) | [中文](README.zh-CN.md)

> **项目仍处于开发搭建阶段，尚无可交付的产品。** yacr 是一个用 Rust 从零实现的 CAD 系统，
> 目标是打开 DWG/DXF 图纸。桌面 UI 已于 2026-10-16 从 Slint 迁移为 **React + Tauri 原生宿主**
> （Linux 优先）；Web/Android UI 与 Windows/macOS 桌面宿主当前是**显式 NOT IMPLEMENTED 占位**。
> 当前能力以**契约测试、合成样本与部分宿主的实际运行**为证据；尚未完成真机、真实 GPU 与真实
> 图纸的视觉验收，也没有任何已验收的 DWG 兼容性结论。请勿把它当作可用或生产级的 CAD 软件。
>
> **范围变更**：用户批注（annotation）功能已于 2026-10-05 整体移除，本项目聚焦
> **查看 + 测量**（测量 F06 与 DXF 注释性缩放保留）。2026-10-16 桌面 UI 由 Slint 迁移为
> React/Tauri，`cad-ui-slint` 已删除。

## yacr 是什么

图纸数据与业务逻辑只操作数据库、事务和命令，渲染是数据库的派生结果；同一套核心
（`cad-*` crate：领域模型、数据库/事务、几何、语义表示、场景、wgpu 渲染）加上平台无关 UI 层
（`cad-ui-core`：wire protocol、布局、CAD 会话、shell 状态、i18n）被宿主复用，因此行为可独立
测试，并可用于无头 CLI 与出图。

技术底座：DWG/DXF 解析用 **未修改的 acadrust 0.6.3**，绘制用 **wgpu**（renderer-only：只有
`cad-render-wgpu` 依赖 wgpu），桌面 UI 是 **Tauri 2 WebView 里的 React（Vite）chrome** 合成在
**原生 GTK/wgpu CAD 分区**之上（`apps/app-linux`，二进制 `yacr`）。数据库（`cad-db`）是唯一权威
数据源，UI 与渲染均为派生。详细边界见 `docs/architecture.md`，迁移决策见
[`docs/adr/0010-react-linux-host-replaces-slint.md`](docs/adr/0010-react-linux-host-replaces-slint.md)。

## 今天能做什么

以下能力描述的是**共享核心 / 无头 CLI 的能力集**（有契约/合成测试覆盖，历史上曾在**已退役的
Slint** 宿主上运行），**不是**当前 GUI 范围：当前 **React/Tauri 桌面宿主只接线 OPEN 与 FIT 以及原生
导航**，其余控件一律显式禁用；Windows/macOS/Android/Web 宿主均为 **NOT IMPLEMENTED 占位**。
**保真度不一**，不少图元只是近似表示或仍有缺口，逐项状态以 `docs/compatibility.md`、
`docs/dxf-entity-coverage.md` 为准。

- **打开与解析**：本地 DWG / DXF（ASCII 与二进制），共用同一语义/数据库转换；异步
  导入带进度与取消。
- **查看**：模型空间与受支持布局切换；图层显隐、搜索、恢复；图元选择、高亮与基本属性。
- **导航**：平移、缩放、适应图纸、重置视图，鼠标与触控行为一致；**2D 与 3D 观察**
  （轨道旋转、标准视图、正交/透视切换）。
- **测量**：距离、折线长度、角度、多边形面积，单位/精度/几何来源可追踪。
- **图元显示**：覆盖全部可绘制 DXF 图元（LINE/POLYLINE/CIRCLE/ARC/ELLIPSE/SPLINE/
  INSERT/TEXT/MTEXT/HATCH/DIMENSION/LEADER/MULTILEADER/MLINE/TABLE/…）。
  TEXT/MTEXT 用真实字体整形为线段（TTF/OTF/WOFF 与 SHX）；HATCH 支持多环实心填充与
  渐变；DIMENSION 对线性/对齐/半径/直径/角度/坐标/弧长合成显示几何（部分子类为近似）。
- **光栅图像**：RASTERIMAGE 端到端纹理显示（相对路径解析 + PNG/JPEG 解码 + GPU 纹理，
  每键去重、UV 方向修正、裁剪多边形）。
- **ACIS 实体（子集）**：3DSOLID / BODY / REGION / SURFACE 解析为中性 B-rep，仅平面
  （含孔）/球/柱/环面/锥面离散；其余显式标记未支持。
- **字体**：运行时解析捆绑/配置的字体目录，缺字体时回退默认轮廓面。（发布打包**当前已退役**，
  见 `docs/build.md`；此处描述的是可用资源/配置，不是可分发的发布包。）
- **出图**：模型/布局光栅 PNG（CLI 另有 SVG/PDF 纯矢量路径）；无矢量打印样式表（CTB）。
- **大图保护**：场景批次/顶点硬预算，超限**显式失败**而不是 OOM。
- **代理图元**：天正、探索者等自定义图元仅按公开的**代理图形缓存记录**显示，无缓存几何
  不显示。
- **多平台复用**：一套核心 + `cad-ui-core` 共享 UI 层；桌面宿主当前仅构建 Linux，Windows/macOS/
  Android/Web 宿主均为显式 NOT IMPLEMENTED 占位（见下表）。

## 平台与运行状态

**编译通过不等于运行验收。** 各端实际运行到什么程度如下（`docs/validation*.md` 为已退役 Slint
宿主的历史证据）：

| 平台 | 宿主 | 当前实际状态 |
|---|---|---|
| **Linux 桌面（首选，当前）** | `apps/app-linux`（`yacr`） | React/Tauri chrome + 原生 GTK/wgpu CAD 分区。**仅 OPEN 与 FIT + 原生导航接线**，其余控件一律显式禁用并标注原因。P6 最终验证**已执行（有界）**；Gate P6 attempt2 **ACCEPT-WITH-FIXES**（文档限定已应用），observer 终判所观察截图**无重大视觉缺陷**——见 [`docs/validation-react-linux-migration-2026-10-16.md`](docs/validation-react-linux-migration-2026-10-16.md)。**仅有界本地 Linux 迁移验收**，**不是**完整 CAD 产品/跨平台/vendor/golden/性能验收。 |
| **Windows 桌面** | `apps/app-windows`（`yacr-windows`） | **NOT IMPLEMENTED** 失败桩（无 Tauri 依赖，打印诊断并非零退出）。CI 只编译（`windows-check`），**没有可运行 GUI 产品**。 |
| **macOS 桌面** | `apps/app-macos`（`yacr-macos`） | **NOT IMPLEMENTED** 失败桩。CI 只编译（`macos-check`）；**没有可运行 GUI 产品**。 |
| **Android** | `apps/app-android` | **NOT IMPLEMENTED** 占位（UI 已退役；原生入口打印诊断后 abort）。 |
| **Web** | `apps/app-web` | **NOT IMPLEMENTED** 占位（UI 已退役；wasm 入口返回错误）。 |
| **无头 CLI** | `cad-cli-tools` | 保留：经共享应用路径无头打开/出图。 |

> 历史 Slint 宿主证据（离屏/软件 Vulkan/模拟器/浏览器）保留在 `docs/validation*.md` 中**作为历史**，
> **不**迁移为 React/Tauri 宿主的结论。

## 快速开始

工具链 1.99.0（`rust-toolchain.toml`），`Cargo.lock` 已锁定；若 `PATH` 无 cargo，加上
`$HOME/.cargo/bin`。

### Linux 桌面（首选宿主）

```bash
sudo apt-get install -y pkgconf libgtk-3-dev libwebkit2gtk-4.1-dev libwayland-dev \
  libfontconfig-dev libfreetype-dev mesa-vulkan-drivers
# 推荐入口：不强制 GDK_BACKEND（Auto 优先原生 Wayland；显式 x11/wayland 保留），构建 Vite 前端，
# 以 custom-protocol 运行
bash scripts/run-react-linux.sh --open /absolute/drawing.dwg
```

直接运行（需先 `npm run build` 出 `ui/dist`）。原生 Wayland（真实原生，非 XWayland）：

```bash
env -u DISPLAY -u XAUTHORITY GDK_BACKEND=wayland WAYLAND_DISPLAY=wayland-0 \
    XDG_RUNTIME_DIR=/run/user/1000 ./target/release/yacr --open /absolute/drawing.dwg
# 显式 X11/XWayland 替代：GDK_BACKEND=x11 DISPLAY=:0 ./target/release/yacr --open ...
```

配置目录为 `~/.config/yacr/`（由旧的 `~/.config/yacr-react/` 迁移，**不自动迁移**）。后端策略、
原生 Wayland 子表面与完整参数/限制见 `docs/react-linux-app.md` 与
`docs/adr/0011-native-wayland-subsurface.md`。

### 无头 CLI

```bash
cargo build -p cad-cli-tools --release --locked
./target/release/cad-cli-tools render /absolute/drawing.dwg --png /tmp/opencode/out.png
./target/release/cad-cli-tools --help
```

成功时 stdout 只输出一份 JSON 结果文档，失败为非零退出码 + 结构化错误。操作与选项见
`docs/cli.md`。

### 其他平台

Windows/macOS/Android/Web 本轮 **NOT IMPLEMENTED**；其构建/打包/部署脚本非零失败（已退役），
CI 发布 job 已硬禁用。当前真实状态见 `docs/build.md` 与 `docs/ci.md`——**不要**把已退役脚本当作
可用流程。

## 现状与边界（诚实说明）

- **没有已验收的 DWG 兼容性、实体或平台。** `fixtures/manifest` 含合成夹具与一个开源
  QCAD `flange` 样本（`Partial`）；构建证据不等于兼容性。
- **验证证据的边界**：静态门禁与契约/合成测试，以及 P6 可执行证据（前端测试、`--locked` 编译/测试、
  合成夹具上的硬件 Vulkan 真实窗口 smoke——见 `docs/validation-react-linux-migration-2026-10-16.md`）；
  已退役 Slint 宿主的运行仍属历史。当前宿主的真实 GPU 像素矩阵、真机、精确视觉 golden、真实图纸
  验收 **未运行**，不得从历史证据推广。Gate P6 attempt2 **ACCEPT-WITH-FIXES**（文档限定已应用）；
  **有界本地 Linux 迁移验收通过**，**不是**完整 CAD 产品/跨平台/vendor/golden/性能验收。
- **明确未支持 / 未实现**（不伪造成功）：
  - 代理图元的厂商 opcode 记录（仅公开缓存记录）；无缓存几何不显示。
  - 外部内容：PDF/DWF/DGN underlay、OLE2 嵌入对象、CoordinationModel / Navisworks NWD 不加载。
  - ACIS 完整几何内核（3DSOLID/REGION/BODY/SURFACE 仅子集离散）、点云点数据（仅范围盒）。
  - 绘制顺序 `draw_order` 未贯通（=上传顺序）；WIPEOUT 掩码填充未渲染；线宽显式不绘制。
  - 图像不支持编解码器（TIFF/CCITT/EPS）、亮度/对比/淡出、outside/mask 裁剪。
  - 出图仅光栅 PNG（CLI 另有 SVG/PDF）；无 CTB/HPGL；不写回 DWG、不修改原始实体。
  - 用户批注已整体移除；增强模式只含测量。
- 未支持项逐项标注在 `docs/compatibility.md`，图元逐项状态见 `docs/dxf-entity-coverage.md`；
  **未验证即未验证**，不冒充完成。

## 文档索引

**总览**

- 需求权威：`CAD_IMPLEMENTATION_SPEC.md`（v2.0）
- 逐轮交接入口：`docs/handoff.md`
- 架构边界与不变量：`docs/architecture.md`、`docs/core-invariants.md`
- 兼容性与能力矩阵：`docs/compatibility.md`
- 验证与运行证据：`docs/validation.md`、`docs/validation-dwg.md`

**构建与平台**

- 各平台构建：`docs/build.md`
- Linux 桌面宿主（当前）：`docs/react-linux-app.md`
- 决策记录：`docs/adr/`（ADR 0010 = React 宿主取代 Slint）
- 无头渲染：`docs/headless-render.md`　CI 分层：`docs/ci.md`
- CLI：`docs/cli.md`　字体：`docs/fonts.md`　应用图标：`docs/app-icon.md`
- 已退役（历史，含横幅）：`docs/linux-app.md`、`docs/windows-app.md`、`docs/macos-app.md`、
  `docs/flatpak.md`、`docs/verify-ui.md`

**能力与专题**

- 图元覆盖：`docs/dxf-entity-coverage.md`　代理支持：`docs/proxy-support.md`
- 曲线几何：`docs/curve-geometry.md`　ACIS：`docs/kernel-acis.md`
- 三维观察：`docs/view-3d.md`　绘制顺序：`docs/render-order.md`　渲染后端：`docs/render-backends.md`
- 测量：`docs/measure.md`　布局：`docs/layouts.md`　出图：`docs/plot.md`
- 动态块：`docs/dynamic-blocks.md`　注释性缩放：`docs/annotative-scaling.md`
- 性能与预算：`docs/performance.md`
- DWG 测试流程：`docs/testing-dwg.md`　无头 UI 调试：`docs/verify-ui.md`

**参考与决策**

- OpenCADStudio 功能/交互历史参考（2026-10-15 已结束该参考练习，从未复制代码）：调查记录保留于 `docs/migration-map.md`；UI 设计现遵循 AutoCAD 桌面惯例（`docs/react-linux-app.md`）
- 决策记录：`docs/adr/`

## 许可与第三方

本仓库以 **AGPL-3.0** 发布（`LICENSE`）。依赖与捆绑资源（acadrust、wgpu/Tauri/GTK、QCAD 样本、
osifont、mlightcad 字体等）的来源与许可见 `THIRD_PARTY_NOTICES.md` 及 `fonts/SOURCE.md`。
