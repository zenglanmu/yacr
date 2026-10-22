# yacr — Rust CAD 系统（开发中）

[English](README.md) | [中文](README.zh-CN.md)

> **项目仍处于开发搭建阶段，尚无可交付的产品。** yacr 是一个用 Rust 从零实现的 CAD 系统，
> 目标是打开 DWG/DXF 图纸。桌面 UI 是 **React + Tauri 原生宿主**（Linux 优先；2026-10-16 从 Slint
> 迁移）。Windows/macOS 桌面宿主与 Android/Web UI 当前是**显式 NOT IMPLEMENTED 占位**，不是可运行
> 产品。当前能力以**契约/合成测试 + 合成夹具上的有界真实 GPU 宿主运行**为证据；尚未完成任何已验收的
> DWG 兼容性、真实图纸、vendor、golden 图像或性能验收。请勿把它当作可用或生产级的 CAD 软件。
>
> **范围变更**：用户批注（annotation）已于 2026-10-05 整体移除，本项目聚焦**查看 + 测量**；
> 2026-10-16 桌面 UI 由 Slint 迁移为 React/Tauri，`cad-ui-slint` 已删除。

## 项目架构

数据流（UI 与 CLI 共用）：

```
UI / CLI → Application 与权限 → Command / DrawingTransaction → DB transaction → ChangeSet
         → 查询 / 历史 / 依赖失效 → Representation → SceneDelta → Renderer → RenderHost
```

图纸数据与业务逻辑**只**操作数据库、事务和命令；渲染是数据库的**派生**结果。`cad-db` 是唯一权威
数据源，UI 与渲染均由它派生；同一套核心被桌面宿主与无头 CLI 复用，因此行为可独立测试。crate 分层
（自底向上；由 `scripts/check-architecture.py` 强制）：

```
cad-domain                                   领域模型：强类型 ID、f64 几何、单位、容差
  ↑
cad-db ─ cad-dependencies ─ cad-history      数据库/事务、依赖失效、撤销/恢复
  ↑
cad-geometry ─ cad-kernel-adapter ─ cad-representation   几何、ACIS/SAT 隔离、显示表示
  ↑
cad-spatial ─ cad-scene ─ cad-render-wgpu    空间索引、场景缓存、wgpu 渲染器
  ↑
cad-proxy ─ cad-import-acadrust ─ cad-resources   代理图形、DWG/DXF 导入、资源解析
  ↑
cad-measure ─ cad-query ─ cad-app            测量、分页查询、应用/会话/命令
  ↑
cad-engine ─ cad-platform ─ cad-diagnostics ─ cad-cli-tools   渲染会话、平台契约、诊断、CLI
  ↑
cad-host                                     平台无关宿主编排（不依赖任何 UI 工具包）
  ↑
cad-ui-core                                  wire protocol、逻辑布局、CAD 会话、shell、i18n
  ↑
apps/app-linux（React + Tauri）   apps/app-{windows,macos,android,web}（NOT IMPLEMENTED 占位）
```

**必须保持的边界**（详见 `docs/architecture.md`、`docs/core-invariants.md`）：

- **renderer-only wgpu**——只有 `cad-render-wgpu` 依赖 wgpu。
- **importer-only acadrust**——只有 `cad-import-acadrust` 依赖 acadrust（**未修改的 0.6.3**）；
  **禁止 Cargo patch**。
- `cad-host` **不依赖任何 UI 工具包**；平台 I/O 只存在于 `apps/app-*`，经 `cad-platform` 契约
  （`PlatformFiles` / `ConfigStore` / `SurfaceSource` / `FontLoader` / …，事件式、无 futures、无 `Send`）。
- `cad-ui-core` 承载平台无关 UI 逻辑（wire protocol、逻辑布局、CAD 会话、宿主 admission/error 状态、
  i18n 目录），必须保持 UI 工具包与 GPU 无关。
- `cad-engine` 是 UI 无关的渲染会话协调层；`cad-app` 不依赖它，它也不依赖 `cad-ui-core` / `cad-host`
  或任何 UI 工具包。
- GPU/内核/UI 工具包/平台类型不得进入 `cad-domain` / `cad-db`；无整幅回读；单 presenter per drawable。

**桌面宿主组合**（Linux，`apps/app-linux`，二进制 `yacr`）：GTK `GtkOverlay` 内是**原生 GTK/wgpu CAD
分区**（Rust + renderer-only wgpu），Tauri 2 `WebView` 作为透明 sibling 呈现 React（Vite）chrome，
二者 **co-visible**。宿主支持 **X11/XWayland**（CAD 分区获得自己的原生 X child 窗口）与**原生
Wayland**（宿主自建 app-owned `wl_surface` + `wl_subsurface`，经 `apps/app-linux/native/` 私有 C shim）。
后端在 GTK 启动前由 `apps/app-linux/src/backend.rs` 显式决议，**无静默回退**。见 ADR 0010、ADR 0011。

## 特色

- **一套核心，多消费者**：共享 `cad-*` 核心 + 平台无关的 `cad-ui-core` 层被桌面宿主与无头 CLI 复用，
  二者共用同一应用命令路径与同一数据库。
- **数据库为权威，渲染为派生**：修改经事务与 `ChangeSet`；依赖失效只重建受影响的显示表示，不重新解析底图。
- **依赖隔离由构造保证**（见上「边界」），并由 CI 强制。
- **诚实的建模**：未支持/未验证能力显式建模（`Unsupported` / `Partial` / 诊断）；占位与假数据不计完成，
  失败绝不改成空成功。
- **i18n 唯一权威**：所有用户可见文案来自 `crates/cad-ui-core/i18n/{zh-CN,en}.json`（键集必须一致）；
  代码里硬编码 CJK 会被拒绝。
- **桌面宿主做真实原生渲染**：React chrome 合成在原生 GTK/wgpu CAD 分区之上，同时支持 X11/XWayland 与
  原生 Wayland，后端决议显式、无回退。
- **能力集**（共享核心 / 无头 CLI 的能力集，有契约/合成测试覆盖——当前 React/Tauri GUI 只接线
  OPEN、FIT 与原生导航）。**保真度不一**：不少图元只是近似表示或仍有缺口，逐项状态以
  `docs/compatibility.md`、`docs/dxf-entity-coverage.md` 为准：
  - **打开与解析**：本地 DWG / DXF（ASCII 与二进制），共用同一语义/数据库转换；异步导入带进度与取消。
  - **查看**：模型空间与受支持布局切换；图层显隐、搜索、恢复；图元选择、高亮与基本属性。
  - **导航**：平移、缩放、适应图纸、重置；**2D 与 3D 观察**（轨道旋转、标准视图、正交/透视）。
  - **测量**：距离、折线长度、角度、多边形面积，单位/精度/几何来源可追踪。
  - **图元显示**：覆盖全部可绘制 DXF 图元（LINE/POLYLINE/CIRCLE/ARC/ELLIPSE/SPLINE/INSERT/TEXT/
    MTEXT/HATCH/DIMENSION/LEADER/MULTILEADER/MLINE/TABLE/…）。TEXT/MTEXT 用真实字体整形为线段
    （TTF/OTF/WOFF 与 SHX）；HATCH 支持多环实心填充与渐变；DIMENSION 对线性/对齐/半径/直径/角度/
    坐标/弧长合成显示几何（部分子类为近似）。
  - **光栅图像**：RASTERIMAGE 端到端纹理显示（相对路径解析 + PNG/JPEG 解码 + GPU 纹理、每键去重、
    UV 方向修正、裁剪多边形）。
  - **ACIS 实体（子集）**：3DSOLID / BODY / REGION / SURFACE 解析为中性 B-rep，仅平面（含孔）/球/
    柱/环面/锥面离散；其余显式标记未支持。
  - **代理图元**：天正、探索者等自定义图元仅按公开的**代理图形缓存记录**显示，无缓存几何不显示。
  - **出图**：模型/布局光栅 PNG（CLI 另有 SVG/PDF 纯矢量路径）；无矢量打印样式表（CTB）。
  - **大图保护**：场景批次/顶点硬预算，超限**显式失败**而不是 OOM。

## 源码编译与运行

工具链 **1.99.0**（`rust-toolchain.toml`），`Cargo.lock` 已锁定；若 `PATH` 无 cargo，加上 `$HOME/.cargo/bin`。

### Linux 桌面宿主（首选，唯一可运行 GUI）

系统前置（脚本不自动安装）：

```bash
sudo apt-get install -y pkgconf libgtk-3-dev libwebkit2gtk-4.1-dev libwayland-dev \
  libfontconfig-dev libfreetype-dev mesa-vulkan-drivers
pkg-config --exists webkit2gtk-4.1 gtk+-3.0 wayland-client   # 应通过
```

推荐入口（不强制 `GDK_BACKEND`；构建 Vite 前端并以 `tauri/custom-protocol` 运行）：

```bash
bash scripts/run-react-linux.sh --open /absolute/drawing.dwg
```

直接运行（需先构建前端）：

```bash
(cd apps/desktop-ui && npm ci && npm run build)
cargo run --release --locked -p app-linux --bin yacr \
  --features tauri/custom-protocol -- --open /absolute/drawing.dwg
```

原生 Wayland（真实原生，非 XWayland）与显式 X11 替代：

```bash
env -u DISPLAY -u XAUTHORITY GDK_BACKEND=wayland WAYLAND_DISPLAY=wayland-0 \
    XDG_RUNTIME_DIR=/run/user/1000 ./target/release/yacr --open /absolute/drawing.dwg
# 显式 X11/XWayland：GDK_BACKEND=x11 DISPLAY=:0 ./target/release/yacr --open ...
```

宿主参数：`--open <path>`、`--fonts-dir <path>`、`--images-dir <path>`、`--probe`。配置目录为
`~/.config/yacr/`（由旧的 `~/.config/yacr-react/` 迁移，**不自动迁移**）。宿主需要真实显示；
**无显示的 SSH/无头不支持**。完整参数、后端策略与限制见 `docs/react-linux-app.md`。

### 无头 CLI

```bash
cargo build -p cad-cli-tools --release --locked
./target/release/cad-cli-tools render /absolute/drawing.dwg --png /tmp/opencode/out.png
./target/release/cad-cli-tools --help
```

成功时 stdout 只输出一份 JSON 结果文档，失败为非零退出码 + 结构化错误。见 `docs/cli.md`。

### 本地 Flatpak bundle（Linux）

```bash
bash scripts/package-linux-release.sh
# → target/release/dist/yacr-<version>-linux-<arch>.flatpak  (+ .sha256)
flatpak install --user target/release/dist/yacr-<version>-linux-<arch>.flatpak
flatpak run dev.yacr.app --open /absolute/drawing.dwg
```

它构建单文件 Flatpak bundle（`bin/yacr` + committed `fonts/`，runtime `org.gnome.Platform//51`）。
**不是**发布通道：未上 Flathub，CI 的 `linux-release` job 仍硬禁用。见 `docs/flatpak.md`。

### 全量门禁（CI 同款；发版前运行）

```bash
cargo fmt --all -- --check
cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings
python3 scripts/check-architecture.py
python3 scripts/check-fixture-manifest.py
python3 scripts/check-workflows.py
python3 scripts/check-i18n.py
cargo check --workspace --all-targets --locked
cargo check --workspace --lib --target wasm32-unknown-unknown --locked
cargo test --workspace --locked -- --test-threads=1   # GPU/CLI 用例串行跑
(cd apps/desktop-ui && npm ci && npm test && npm run build)
```

日常开发只验证改动范围（改动 crate 的编译/测试/fmt/clippy）；上述全量门禁用于明确要求与发版验收。
未运行一律标注 `NOT RUN`，绝不用旧证据冒充当前结果。

### 其他平台

Windows/macOS/Android/Web 本轮 **NOT IMPLEMENTED**；其构建/打包/部署脚本非零失败（已退役），CI 发布
job 已硬禁用。当前真实状态见 `docs/build.md` 与 `docs/ci.md`——**不要**把已退役脚本当作可用流程。

## 现有进度

**编译通过不等于运行验收。** 各端实际运行到什么程度如下（`docs/validation*.md` 中已退役 Slint 宿主的
证据**不**迁移为 React/Tauri 宿主的结论）：

| 平台 | 宿主 | 当前实际状态 |
|---|---|---|
| **Linux 桌面（首选，当前）** | `apps/app-linux`（`yacr`） | React/Tauri chrome + 原生 GTK/wgpu CAD 分区。**仅 OPEN 与 FIT + 原生导航接线**，其余控件一律显式禁用并标注原因。已做有界本地验收：React 迁移 P6 gate **ACCEPT-WITH-FIXES**（合成夹具上的硬件 Vulkan 真实窗口 smoke）、原生 Wayland W3 **ACCEPT-WITH-LIMITATIONS**（GNOME/Mutter + Quadro P620、`scale=1`）——见 `docs/validation-react-linux-migration-2026-10-16.md` 与 `docs/validation-native-wayland-2026-10-16.md`。**仅有界本地 Linux 验收**，**不是**完整 CAD 产品/跨平台/vendor/golden/性能验收。 |
| **Windows 桌面** | `apps/app-windows`（`yacr-windows`） | **NOT IMPLEMENTED** 失败桩（无 Tauri 依赖；打印诊断并非零退出）。CI 只编译（`windows-check`）；无可运行 GUI。 |
| **macOS 桌面** | `apps/app-macos`（`yacr-macos`） | **NOT IMPLEMENTED** 失败桩。CI 只编译（`macos-check`）；无可运行 GUI。 |
| **Android** | `apps/app-android` | **NOT IMPLEMENTED** 占位（UI 已退役；原生入口打印诊断后 abort）。 |
| **Web** | `apps/app-web` | **NOT IMPLEMENTED** 占位（UI 已退役；wasm 入口返回错误）。 |
| **无头 CLI** | `cad-cli-tools` | 保留：经共享应用路径无头打开/出图。 |

近期（2026-10-17/18）在真实使用下打磨了渲染与拾取路径：CPU 拾取接入场景空间索引（ADR 0012，S4；
`cad-spatial` 每 chunk R-tree，实测权衡如实记录）、帧 pacing / 增量提交修复、每帧 CPU plan 与 encode
跳过的优化、GPU 帧时间测量、块级视锥剔除、设备 limits 协商。当前本地 Flatpak bundle 与自动证据采集
脚本（`scripts/collect-evidence.sh`）也出自同期。逐轮证据见 `docs/handoff.md` 与 `docs/validation*.md`。

### 诚实边界

- **没有已验收的 DWG 兼容性、实体或平台。** `fixtures/manifest` 含合成夹具与一个开源 QCAD `flange`
  样本（`Partial`）；构建证据不等于兼容性。
- **验证证据的边界**：静态门禁与契约/合成测试，以及有界可执行证据（前端测试、`--locked` 编译/测试、
  合成夹具上的硬件 Vulkan 真实窗口 smoke、以及一次有界原生 Wayland 运行）。真实 GPU 像素矩阵、真机、
  精确视觉 golden、性能与真实图纸验收**未覆盖**，不得推广；软件渲染结果须显式标注，绝不冒充真实 GPU 结论。
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

**总览**——需求权威：`CAD_IMPLEMENTATION_SPEC.md`（v2.0）；逐轮交接：`docs/handoff.md`；架构边界与
不变量：`docs/architecture.md`、`docs/core-invariants.md`；兼容性与能力矩阵：`docs/compatibility.md`；
验证与运行证据：`docs/validation.md`、`docs/validation-dwg.md`。

**构建与平台**——各平台构建：`docs/build.md`；Linux 桌面宿主（当前）：`docs/react-linux-app.md`；
Linux Flatpak（本地 bundle，未发布）：`docs/flatpak.md`；决策记录：`docs/adr/`（ADR 0010 = React 宿主
取代 Slint、ADR 0011 = 原生 Wayland 子表面、ADR 0012 = 场景空间索引与交互）；无头渲染：
`docs/headless-render.md`；CI 分层：`docs/ci.md`；CLI：`docs/cli.md`；字体：`docs/fonts.md`；
应用图标：`docs/app-icon.md`。已退役（历史，含横幅）：`docs/linux-app.md`、`docs/windows-app.md`、
`docs/macos-app.md`、`docs/verify-ui.md`。

**能力与专题**——图元覆盖：`docs/dxf-entity-coverage.md`；代理支持：`docs/proxy-support.md`；曲线几何：
`docs/curve-geometry.md`；ACIS：`docs/kernel-acis.md`；三维观察：`docs/view-3d.md`；绘制顺序：
`docs/render-order.md`；渲染后端：`docs/render-backends.md`；测量：`docs/measure.md`；布局：
`docs/layouts.md`；出图：`docs/plot.md`；动态块：`docs/dynamic-blocks.md`；注释性缩放：
`docs/annotative-scaling.md`；命令行与键位：`docs/shortcuts.md`；绘图/编辑子集：`docs/drawing-edit.md`；
性能与预算：`docs/performance.md`；DWG 测试流程：`docs/testing-dwg.md`。

## 许可与第三方

本仓库以 **AGPL-3.0** 发布（`LICENSE`）。依赖与捆绑资源（acadrust、wgpu/Tauri/GTK、QCAD 样本、
osifont、mlightcad 字体等）的来源与许可见 `THIRD_PARTY_NOTICES.md` 及 `fonts/SOURCE.md`。
