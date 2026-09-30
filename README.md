# yacr — Rust 工业 CAD 查看、测量与批注系统

依据 `CAD_IMPLEMENTATION_SPEC.md` v2.0 实现。架构为数据库驱动（`cad-db`：对象、
事务、revision、ChangeSet），UI 使用 Slint，CAD 绘制使用 wgpu，DWG 解析使用未修改的
acadrust 0.5.5。核心无平台依赖，可单独测试并用于 CLI。

* 架构：`docs/architecture.md`
* 构建（含 Android APK）：`docs/build.md`
* 验证与运行证据：`docs/validation.md`
* 兼容性/能力表：`docs/compatibility.md`
* 渲染后端：`docs/render-backends.md`
* 代理支持：`docs/proxy-support.md`
* 性能与预算：`docs/performance.md`
* OpenCADStudio 迁移映射：`docs/migration-map.md`
* 决策记录：`docs/adr/`

## 当前状态

已实现并以合成契约测试覆盖：领域类型、数据库与事务、几何引擎、代理回放器、
acadrust 导入、显示表示、空间索引、场景批处理、测量、批注（含版本化 JSON）、
依赖失效、撤销/重做、查询层、应用命令层、wgpu 2D 渲染器、Slint UI 与 Android 宿主。

构建产物：`target/debug/apk/yacr.apk`（arm64-v8a，debug 签名）。

**明确未完成**（不得视为已交付）：

- 无真实 DWG 样本、黄金图、跨后端对照或性能基准（`fixtures/manifest` 为空）。
- Android 未在真机/模拟器运行；Web 无可运行产物（无 Wasm 导出/JS 宿主）。
- 桌面/iOS 仅平台抽象，不提供宿主。
- 3D 观察渲染、ACIS 曲面离散、SAF 文件选择器、字体 shaping、WebGL2 路径未实现。
- 未支持项在 `docs/compatibility.md` 中逐项标注；未验证即未验证，不冒充完成。
