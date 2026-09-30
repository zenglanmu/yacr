# yacr — Rust 工业 CAD 查看、测量与批注系统

依据 `CAD_IMPLEMENTATION_SPEC.md` v2.0 实现。核心为数据库驱动（`cad-db`：对象、
事务、revision、ChangeSet），UI 使用 Slint，CAD 绘制使用 wgpu，DWG 解析使用未修改的
acadrust 0.5.5。

首要交付物：Android APK 与浏览器静态产物。核心无平台依赖，可单独测试与用于 CLI。

- 架构：`docs/architecture.md`
- 兼容性与能力表：`docs/compatibility.md`
- 渲染后端：`docs/render-backends.md`
- 代理支持：`docs/proxy-support.md`
- 性能与预算：`docs/performance.md`
- OpenCADStudio 迁移映射：`docs/migration-map.md`
- 决策记录：`docs/adr/`

## 状态

本仓库以“诚实的能力表”为原则：未实现或未验证的能力在代码与文档中均标记为
`Unsupported` / `Unverified`，不会以占位实现冒充完成。
