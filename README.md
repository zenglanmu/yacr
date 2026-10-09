# yacr — Rust CAD 查看、测量与批注系统（开发中）

> **当前处于开发搭建阶段，尚不可用。** 本仓库是依据 `CAD_IMPLEMENTATION_SPEC.md` v2.0
> 推进的契约框架：能力以合成测试覆盖为主，多数宿主与真实数据路径尚未接线或未验证，
> 没有可交付的产品。请勿把它当作已完成或生产可用的 CAD 系统，具体边界见下文
> 「当前状态 / 明确未完成」。

依据 `CAD_IMPLEMENTATION_SPEC.md` v2.0 搭建。目标架构为数据库驱动（`cad-db`：对象、
事务、revision、ChangeSet），UI 计划使用 Slint，CAD 绘制计划使用 wgpu，DWG 解析计划使用
未修改的 acadrust 0.6.3。核心设计为无平台依赖，可单独测试并用于 CLI；这些是架构目标，
不代表功能已完备。

* 架构：`docs/architecture.md`
* 构建（Linux App 主验收，含 Android APK）：`docs/build.md`
* Linux 宿主入口与限制：`docs/linux-app.md`
* Windows 宿主与打包：`docs/windows-app.md`
* macOS 宿主与打包：`docs/macos-app.md`
* UI 导航/禁用、DXF 与旧式 SHX 字号修复：`docs/ui-dxf-text-fixes.md`
* 验证与运行证据：`docs/validation.md`
* DWG 测试流程（lavapipe 主测试 + 无头浏览器复验）：[docs/testing-dwg.md](docs/testing-dwg.md)
* 真实图纸回归结果：[docs/validation-dwg.md](docs/validation-dwg.md)
* 兼容性/能力表：`docs/compatibility.md`
* 字体来源：`docs/fonts.md`
* 渲染后端：`docs/render-backends.md`
* 代理支持：`docs/proxy-support.md`
* 性能与预算：`docs/performance.md`
* OpenCADStudio 功能规格参考：`docs/ui-requirements/00-INDEX.md`（非源码参考）；历史调查：`docs/migration-map.md`
* 决策记录：`docs/adr/`

## 当前状态（开发搭建中）

以下模块已有契约级实现，并以合成测试覆盖，但整体仍处搭建/验证阶段，不代表可用的
成品：领域类型、数据库与事务、几何引擎、代理回放器、acadrust 导入（含实体/图层透明
度）、显示表示、空间索引、场景批处理（拓扑/法向/深度/绘制顺序/透明）、测量、批注
（含版本化 JSON）、依赖失效、撤销/重做、查询层、应用命令层、wgpu 2D/3D 渲染器、
Slint UI 与 Linux/Android/Web 宿主骨架。

Linux App 仍是优先宿主；2026-10-04 起默认提交门禁与 CI `linux-app` 改为 debug 编译，
包含测试代码但不执行完整测试、Linux 离屏渲染或 release 验证。
`bash scripts/check-linux-app.sh` 保留为可选运行验收工具；编译通过不代表渲染或真机验证。

本轮已实际运行（2026-10-02）：

- Android：x86_64 release APK 在无头 **模拟器**（KVM + SwiftShader）安装、启动、
  渲染，画布平移/“适应”经像素 diff 验证（`docs/validation-android.md`）。
- Web：`web-dist/` wasm 产物在无头 **Chromium**（Chrome for Testing 153）以 WebGL2
  运行，加载/导航/双语切换通过（`docs/validation-web.md`）。
- 3D/纸空间：Slint 桥按空间与视图模式分派 `render`/`render_3d`（`docs/view-3d.md`）。
- 透明：acadrust `Transparency` → 场景 → 透明管线，含软件 Vulkan（lavapipe）合成测试。
- 曲线（B23）：真实 NURBS（源 knots/weights）、椭圆 OCS 法向、非均匀仿射真椭圆、
  bulge、解析交点（`docs/curve-geometry.md`）。
- ACIS（F15）：acadrust SAT/SAB → 中性 B-rep → 平面（含孔）/球/柱/环面/锥面子集离散
  （含截头圆锥）；合成 SAT 夹具入 `fixtures/manifest`；其余显式 `Unsupported`
  （`docs/kernel-acis.md`）。
- 动态块可见性（§3.2）：读取命名状态并在内存库中切换活动状态（GEOMETRY 增量，
  `docs/dynamic-blocks.md`）；注释性缩放：TEXT/MTEXT 按活动比例缩放并应用按比例覆盖
  （`docs/annotative-scaling.md`）。
- 布局/测量（F04/F06）：4 角纸空间视口与修正比例，纸面/视口模型测量经已验证逆变换。
- 捕捉/填充：端点/中点/圆心/象限/垂足/局部交点捕捉；HATCH 多环含孔洞实心填充。

**明确未完成**（不得视为已交付）：

- 跨后端对照与性能基准尚未建立；已提交 QCAD `flange` DXF 与上游参考图
  （`fixtures/dxf/qcad-flange/`），但它作为 `Partial` 样本只覆盖模型空间几何，不构成
  大型真实 DWG 兼容性或黄金图矩阵验收。
- Android 未在真机运行；surface 尺寸/安全区、SAF、量测/批注拾取未接线。
- Web 仅验证 WebGL2 软件路径；WebGPU、真实 GPU 与移动/桌面浏览器矩阵未验证。
- Linux 已有桌面/离屏共用宿主，但窗口系统与真实 GPU 未验收；文件选择器、恢复决策、后台
  导入、Trim 点选等尚未闭环。Windows 已有交叉编译宿主与含字体 zip（`docs/windows-app.md`），
  但真实 Windows/GPU/文件对话框未验收。macOS 已接入同一共享宿主并配 CI 打包
  （`docs/macos-app.md`），但只能在 macOS 上构建，真实 Mac/Metal GPU/文件对话框未验收；
  iOS 仍仅平台抽象。
- ACIS 仅有合成样本子集（平面/球/柱/环面/锥面），无授权真实
  3DSOLID/BODY/REGION/SURFACE 样本；带环球面/非圆椭圆/样条面未实现。
  复杂文字整形、动态块参数/夹点求值、复杂视口裁剪、代理无缓存几何均按样本标记为
  未支持或未验证；注释性缩放仅 TEXT/MTEXT，宿主比例切换 UI 未接线。
- 未支持项在 `docs/compatibility.md` 中逐项标注；未验证即未验证，不冒充完成。
