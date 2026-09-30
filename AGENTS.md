# AGENTS.md

Rust 工业 CAD 查看、测量与批注系统。规范：`CAD_IMPLEMENTATION_SPEC.md`（v2.0）。
规范是需求的唯一权威；本文件只描述仓库结构与构建方式。

## 仓库结构

Cargo workspace，核心无平台依赖。依赖方向（**不可反转**）：

```
cad-domain
  ↑
cad-db ── cad-dependencies ── cad-history
  ↑            ↑
cad-geometry ─ cad-kernel-adapter ─ cad-representation
  ↑
cad-spatial ─ cad-scene ─ cad-render-wgpu
  ↑
cad-proxy ─ cad-import-acadrust ─ cad-resources
  ↑
cad-measure ─ cad-annotations ─ cad-query ─ cad-app
  ↑
cad-ui-slint ─ cad-platform ─ cad-diagnostics ─ cad-cli-tools
  ↑
apps/app-android  apps/app-web
```

- `cad-domain` 不得依赖 Slint / wgpu / Android / `web-sys` / 文件系统。
- `cad-import-acadrust` 是**唯一**依赖 acadrust 的 crate。
- `cad-db` 是唯一权威数据源；UI 与渲染均为派生。

## 构建

工具链见 `rust-toolchain.toml`（1.98.1）。Android 需要 JDK 17、Android SDK
build-tools 34、NDK 27.0.12077973 与 `cargo-apk 0.10.0`。

```bash
cargo test --workspace --exclude app-android --exclude app-web   # 纯核心测试
cargo check --target aarch64-linux-android -p cad-ui-slint       # Android 编译路径
cargo apk build -p app-android --release                         # 产出 APK
scripts/build-web.sh                                             # 浏览器产物
```

## 约束（来自规范）

1. 不修改 acadrust 源码，不使用 Cargo patch。
2. 未支持/待验证能力必须显式建模，占位与假数据不计入完成。
3. 业务层只操作数据库/命令/事务，GPU 提交封装在渲染器内。
4. 增量更新：ChangeSet → 依赖失效 → 显示表示重建，不重解析底图。
5. 每项功能同步补充契约测试与文档。

## 文档

`docs/architecture.md`、`docs/compatibility.md`、`docs/render-backends.md`、
`docs/proxy-support.md`、`docs/performance.md`、`docs/migration-map.md`、
`docs/adr/`、`fixtures/manifest`。
