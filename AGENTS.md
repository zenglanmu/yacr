# AGENTS.md

Rust 工业 CAD 查看、测量与批注系统。规范：`CAD_IMPLEMENTATION_SPEC.md`（v2.0）。
规范是需求的唯一权威；本文件只描述仓库结构与构建方式。

## 仓库结构

Cargo workspace，核心无平台依赖。下面是分层示意（不是逐 crate 依赖边）；
实际依赖以 Cargo manifests 和 `scripts/check-architecture.py` 为准，**不可反转**：

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

工具链见 `rust-toolchain.toml`（1.98.1）。当前是契约框架，没有可运行 APK/Web UI。
Android JDK/SDK/NDK/打包器组合尚待锁定并验证，不得凭 metadata 宣称兼容。

```bash
# 纯核心测试（宿主无 fontconfig 开发头，故排除 Slint/宿主 crate）
cargo test --workspace --exclude cad-ui-slint --exclude app-android --exclude app-web --locked
cargo check --workspace --lib --target wasm32-unknown-unknown --locked
python3 scripts/check-architecture.py
# Android 核心路径与 APK
cargo check --target aarch64-linux-android -p cad-ui-slint -p app-android --locked
scripts/build-android.sh   # 产出 target/<profile>/apk/yacr.apk
```

## 约束（来自规范）

1. 不修改 acadrust 源码，不使用 Cargo patch。
2. 未支持/待验证能力必须显式建模，占位与假数据不计入完成。
3. 业务层只操作数据库/命令/事务，GPU 提交封装在渲染器内。
4. 增量更新：ChangeSet → 依赖失效 → 显示表示重建，不重解析底图。
5. 每项功能同步补充契约测试与文档。
6. 先读 `docs/handoff.md` 和 `docs/requirements.md`；`pending("模块.操作")`
   是可搜索交接点，替换时必须增加验收测试，禁止改成空成功。

## 文档

`docs/architecture.md`、`docs/compatibility.md`、`docs/render-backends.md`、
`docs/proxy-support.md`、`docs/performance.md`、`docs/migration-map.md`、
`docs/adr/`、`fixtures/manifest`。
