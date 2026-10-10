# yacr — Rust CAD 系统（开发中）

[English](README.md) | [中文](README.zh-CN.md)

> **项目仍处于开发搭建阶段，尚无可交付的产品。** yacr 是一个用 Rust 从零实现的 CAD 系统，
> 目标是在桌面、Android 与浏览器上打开 DWG/DXF 图纸。当前能力以**契约测试、合成样本与部分
> 宿主的实际运行**为证据；尚未完成真机、真实 GPU 与真实图纸的视觉验收，也没有任何已验收的
> DWG 兼容性结论。请勿把它当作可用或生产级的 CAD 软件。
>
> **范围变更**：用户批注（annotation）功能已于 2026-10-05 整体移除，本项目聚焦
> **查看 + 测量**（测量 F06 与 DXF 注释性缩放保留）。

## yacr 是什么

图纸数据与业务逻辑只操作数据库、事务和命令，渲染是数据库的派生结果；同一套核心
（`cad-*` crate：领域模型、数据库/事务、几何、语义表示、场景、wgpu 渲染、Slint UI）
被复用到各平台宿主，因此行为在各端一致、可独立测试，并可用于无头 CLI 与出图。

技术底座：DWG/DXF 解析用 **未修改的 acadrust 0.6.3**，绘制用 **wgpu**，界面用 **Slint**，
核心不含平台依赖（可编译到 `wasm32`）。数据库（`cad-db`）是唯一权威数据源，UI 与渲染均为派生。
详细边界见 `docs/architecture.md`。

## 今天能做什么

以下能力已在代码中存在并有契约/合成测试覆盖，部分宿主已实际运行；但**保真度不一**，
不少图元只是近似表示或仍有缺口，逐项状态以 `docs/compatibility.md`、
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
- **字体**：发布包自带字体目录，缺字体时回退默认轮廓面。
- **出图**：模型/布局光栅 PNG（CLI 另有 SVG/PDF 纯矢量路径）；无矢量打印样式表（CTB）。
- **大图保护**：场景批次/顶点硬预算，超限**显式失败**而不是 OOM。
- **代理图元**：天正、探索者等自定义图元仅按公开的**代理图形缓存记录**显示，无缓存几何
  不显示。
- **多平台复用**：一套核心 + Slint UI，构建到 Linux/Windows/macOS 桌面、Android 与 Web。

## 平台与运行状态

**编译通过不等于运行验收。** 各端实际运行到什么程度如下（证据见 `docs/validation*.md`）：

| 平台 | 宿主 | 当前实际状态 |
|---|---|---|
| **Linux 桌面（首选）** | `apps/app-linux`（`yacr-linux`） | 默认门禁为 debug 编译 + 静态检查；release GUI 打包与**离屏渲染（软件 Vulkan / lavapipe）**已实跑。真实窗口系统与真实 GPU 像素验收 **未运行**。 |
| **Windows 桌面** | `apps/app-windows`（`yacr.exe`） | 复用共享桌面实现；CI 用 `windows-latest` 原生 MSVC 编译并打含字体 zip，本机以 GNU 交叉/MSVC 复现并用 Wine 做参数解析 smoke。真实 Windows 窗口/文件对话框/GPU **未运行**。 |
| **macOS 桌面** | `apps/app-macos`（`yacr-macos`） | 复用共享桌面实现；CI 在 `macos-latest` 编译并打 universal（arm64+x86_64）`Yacr.app`。Linux 主机无法产出 Mach-O，真实 Mac / Metal GPU **未运行**。 |
| **Android** | `apps/app-android` | x86_64 release APK 已在无头**模拟器**（KVM + SwiftShader）安装、启动、渲染，画布平移/“适应”经像素 diff 验证。**真机**与 SAF 文件选择 **未运行**。 |
| **Web** | `apps/app-web`（`web-dist/`） | wasm 产物在无头 Chromium 以 WebGL2 运行，并在本机真实桌面浏览器（Playwright，真实 GPU）复验；CI `web-deploy` 已实际发布到 Cloudflare Pages（示例 <https://yacr-examples.pages.dev>）。真 WebGPU 硬件适配器与浏览器矩阵 **未运行**。 |

## 快速开始

工具链 1.99.0（`rust-toolchain.toml`），`Cargo.lock` 已锁定；若 `PATH` 无 cargo，加上
`$HOME/.cargo/bin`。

### Linux 桌面（首选宿主）

```bash
sudo apt-get install -y pkgconf libfontconfig-dev libfreetype-dev mesa-vulkan-drivers
cargo build -p app-linux --bin yacr-linux --release --locked
./target/release/yacr-linux                                  # 启动（需桌面环境）
./target/release/yacr-linux --open /absolute/drawing.dwg      # 打开图纸
./target/release/yacr-linux --open /absolute/drawing.dxf --locale en
bash scripts/check-linux-app.sh                              # 无窗口离屏运行验收（lavapipe）
```

`--headless --output <新目录>` 用同一套宿主/控制器/Slint 桥做无窗口出图；`--gpu auto|high|low`
选择适配器偏好（双显卡默认优先独显）。完整参数与限制见 `docs/linux-app.md`。

### 无头 CLI

```bash
cargo build -p cad-cli-tools --release --locked
./target/release/cad-cli-tools render /absolute/drawing.dwg --png /tmp/opencode/out.png
./target/release/cad-cli-tools --help
```

成功时 stdout 只输出一份 JSON 结果文档，失败为非零退出码 + 结构化错误。操作与选项见
`docs/cli.md`。

### 其他平台

- Android（`cargo-apk`，非 Gradle 工程）：`docs/build.md`、`docs/validation-android.md`。
- Web（wasm + 最小 JS 宿主）：`scripts/build-web.sh` 产出 `web-dist/`，再由
  `scripts/serve-web.py` 起本地服务；见 `docs/build.md`。
- Windows / macOS 打包（CI `windows-release` / `macos-release`）：`docs/windows-app.md`、
  `docs/macos-app.md`。

### 发布包

CI（`.github/workflows/build.yml`）在 `workflow_dispatch` 或 `v*` tag 触发生成 Linux、
Windows、macOS、Android 四类含字体发布包并上传 artifact（Linux 为 `dev.yacr.app` Flatpak
bundle；各平台打包脚本见 `docs/build.md`）。产物为 CI artifact，**当前未建立正式的分发渠道**。

## 现状与边界（诚实说明）

- **没有已验收的 DWG 兼容性、实体或平台。** `fixtures/manifest` 含合成夹具与一个开源
  QCAD `flange` 样本（`Partial`）；构建证据不等于兼容性。
- **验证证据的边界**：静态门禁与契约/合成测试、软件 Vulkan（lavapipe）离屏渲染、无头
  模拟器、无头/真实浏览器。真实 GPU 像素矩阵、真机、真实图纸视觉验收 **均未运行**，不得
  从上述证据推广。
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
- Linux 宿主：`docs/linux-app.md`　Windows 宿主：`docs/windows-app.md`　macOS 宿主：`docs/macos-app.md`
- Linux Flatpak 打包：`docs/flatpak.md`
- 无头渲染：`docs/headless-render.md`　CI 分层：`docs/ci.md`
- CLI：`docs/cli.md`　字体：`docs/fonts.md`　应用图标：`docs/app-icon.md`

**能力与专题**

- 图元覆盖：`docs/dxf-entity-coverage.md`　代理支持：`docs/proxy-support.md`
- 曲线几何：`docs/curve-geometry.md`　ACIS：`docs/kernel-acis.md`
- 三维观察：`docs/view-3d.md`　绘制顺序：`docs/render-order.md`　渲染后端：`docs/render-backends.md`
- 测量：`docs/measure.md`　布局：`docs/layouts.md`　出图：`docs/plot.md`
- 动态块：`docs/dynamic-blocks.md`　注释性缩放：`docs/annotative-scaling.md`
- 性能与预算：`docs/performance.md`
- DWG 测试流程：`docs/testing-dwg.md`　无头 UI 调试：`docs/verify-ui.md`

**参考与决策**

- OpenCADStudio 功能规格参考（非源码）：`docs/ui-requirements/00-INDEX.md`；历史调查：`docs/migration-map.md`
- 决策记录：`docs/adr/`

## 许可与第三方

本仓库以 **AGPL-3.0** 发布（`LICENSE`）。依赖与捆绑资源（acadrust、Slint/wgpu、QCAD 样本、
osifont、mlightcad 字体等）的来源与许可见 `THIRD_PARTY_NOTICES.md` 及 `fonts/SOURCE.md`。
