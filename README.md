# yacr — yet another CAD app written in Rust

依据 `CAD_IMPLEMENTATION_SPEC.md` v2.0 实现。核心为数据库驱动（`cad-db`：对象、
事务、revision、ChangeSet），UI 使用 Slint，CAD 绘制使用 wgpu，DWG 解析使用未修改的
acadrust 0.5.5。当前交付为**项目框架与公共契约**，不是可用 CAD 应用；
框架验收不包含 Slint/wgpu 实际运行；相关接入另有进行中的源码变更，
导入、测量和平台宿主等占位操作返回结构化 `NotImplemented`。

产品目标：本地 DWG 查看、二维/三维观察、图层与布局、测量、独立批注。
首要平台为 Android 与 Web（WebGPU + WebGL2），不修改 DWG、不上传图纸、
不修改 acadrust、不引入 Iced。APK 和浏览器可运行产物尚未交付。

## 开始开发

安装 Rust/rustup；工具链固定为 `rust-toolchain.toml` 中的 1.98.1。

```bash
cargo check --workspace --locked
cargo test --workspace --locked
# 只验证无 UI 核心（Slint 接入进行中时可用）
cargo test --workspace --exclude cad-ui-slint --exclude app-android --exclude app-web --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo check --workspace --lib --target wasm32-unknown-unknown --locked
cargo run -p cad-cli-tools -- --help
python3 scripts/check-architecture.py
```

CLI 非帮助操作当前以非零状态退出，不伪装扫描或渲染成功。
Android 路径需另安装 `rustup target add aarch64-linux-android`；详见
`docs/build.md`。不能将 Rust 目标编译成功等同于真机、Slint 或 GPU 验收。

## 目录与交接

- `crates/`：21 个职责明确的核心/适配/UI/CLI 包。
- `apps/`：Android、Web 组合根位置（显式占位）。
- `docs/handoff.md`：后续 agent 接手入口与接口风险。
- `docs/requirements.md`：F01–F15 及架构约束到模块/测试的追踪。
- `fixtures/manifest`：合法样本登记；当前没有真实 DWG 样本。

业务只提交数据库事务；显示缓存可丢弃并重建。文档/会话/视口分离，
标识携带实例路径；CPU 几何使用 f64。所有未知能力独立记录，不以一个支持布尔值替代。

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

本次检查的快照、后续并发变更和未验收项目见 `docs/validation.md`。
参考代码尚未抽取；OpenCADStudio commit、代码与字体许可证须在采用前核查。
主项目许可证方向为 MIT OR Apache-2.0；依赖、字体、图纸授权不能由此推断。
