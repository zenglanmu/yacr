# AGENTS.md

Rust CAD 查看、测量与批注系统，**开发搭建中**：契约框架 + 合成测试，没有可交付产品。
需求唯一权威是 `CAD_IMPLEMENTATION_SPEC.md`（v2.0）；`docs/handoff.md` 是逐轮交接入口。

## 铁律（违反即返工）

1. 不修改 acadrust 源码，不使用 Cargo patch（`check-architecture.py` 会拒绝）。
2. 未支持/未验证能力必须显式建模（`Unsupported`/`Partial`/诊断），占位与假数据不计完成，
   也**禁止**把失败改成空成功。
3. 业务层只操作数据库/命令/事务；GPU 提交封装在渲染器内。
4. 增量更新：ChangeSet → 依赖失效 → 显示表示重建，不重解析底图。
5. 每项改动同步补契约测试与文档。
6. 证据诚实：未运行就说未运行。README/handoff 中大量“已通过”注明仅为**合成测试**或
   特定环境（模拟器/软件 GPU/无头浏览器），不得推广为真机、真实 GPU、vendor 兼容结论。
7. 不覆盖/删除陌生变更（并发实现常见）；先读源码和最新构建证据。

## 语言约定

代码标识、注释、测试名、commit message 用英文；`docs/`、`AGENTS.md` 用中文。
用户可见文案只能来自 `crates/cad-ui-slint/i18n/{zh-CN,en}.json`（键集必须一致，见
`scripts/check-i18n.py`），不得在 Slint/Rust UI 里硬编码中文。

## 仓库结构

Cargo workspace（`crates/cad-*` + `apps/app-linux`、`apps/app-android`、`apps/app-web`）。分层示意，
**不可反转**；实际依赖以 manifests 与 `scripts/check-architecture.py` 为准：

```
cad-domain
  ↑
cad-db ── cad-dependencies ── cad-history
  ↑
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
apps/app-linux  apps/app-windows  apps/app-macos  apps/app-android  apps/app-web
```

`app-linux` 是 Linux/Windows/macOS 共享的桌面宿主实现（`cfg(any(target_os = "linux",
target_os = "windows", target_os = "macos"))`）；`app-windows` 与 `app-macos` 只是调用其
入口的薄封装（`yacr.exe` / `yacr-macos`）。

`check-architecture.py` 强制的边界：`cad-domain` 不依赖任何 CAD 包；`cad-db` 只依赖
`cad-domain`；`cad-import-acadrust` 是**唯一**依赖 acadrust 的 crate；`cad-render-wgpu`
是唯一依赖 wgpu 的 crate；Slint 只允许出现在 `cad-ui-slint`/`app-*`；平台类型
（`web-sys`/`android-activity`/`jni`/`ndk`）不得进入 `cad-*`。`cad-db` 是唯一权威数据
源；UI 与渲染均为派生。

## 构建与门禁

工具链 1.99.0（`rust-toolchain.toml`）；若 `PATH` 无 cargo，加 `$HOME/.cargo/bin`。
提交前跑快速 debug 编译与静态门禁（CI 同款，见 `docs/ci.md`）；完整测试、Linux 离屏渲染与 release 编译不再属于默认提交门禁：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings
python3 scripts/check-architecture.py
python3 scripts/check-fixture-manifest.py
python3 scripts/check-workflows.py
python3 scripts/check-i18n.py
cargo check --workspace --exclude app-android --exclude app-web --all-targets --locked
cargo check --workspace --lib --target wasm32-unknown-unknown --locked   # 全 workspace，含 UI/宿主
```

Windows：CI `windows-check`（`windows-latest` 原生 MSVC 编译）与按需 `windows-release`
（MSVC release + 全量字体 zip）；本机可用 `scripts/package-windows-release.sh`
（默认 GNU 交叉，`CARGO_XWIN=1` 走 MSVC/cargo-xwin），见 `docs/windows-app.md`。

macOS：CI `macos-check`（`macos-latest` 原生 arm64 编译）与按需 `macos-release`
（universal `lipo` + 自包含 `Yacr.app` + 全量字体 tar.gz）。**Apple SDK/链接器缺失，
Linux 主机无法交叉产出 Mach-O**，故只能在 macOS runner/真机运行
`scripts/package-macos-release.sh`，见 `docs/macos-app.md`。

Linux/Android 发布层：按需 `linux-release`（`ubuntu-latest` 运行
`scripts/package-linux-release.sh`，含全量字体 tar.gz）与 `android-release`（`ubuntu-latest`
自装 **JDK 17** + 固定 SDK/NDK，`scripts/fetch-fonts.sh` 带全量字体后
`scripts/build-android.sh --release`）。二者与 `windows-release`/`macos-release` 同在
`workflow_dispatch`/`v*` tag 触发；Android **打包不等于安装运行**，见 `docs/ci.md`。

- 默认主机编译门禁必须包含 Linux App 和 Slint；编译通过不是运行或渲染验收。Linux 离屏
  `bash scripts/check-linux-app.sh` 与完整测试仅在明确要求时执行；release 编译用于可选发布验证。
  Web 是第二层，
  Android 平台契约继续保留。用户授权解除原生 Slint 编译限制：允许使用
  开发依赖，在 Linux 无窗口环境以 Slint FemtoVG/wgpu + lavapipe 离屏验证（用户已授权
  sudo apt 安装 pkgconf/fontconfig/freetype 开发包）。
  不需要 Android 模拟器，不安装桌面/X11/Wayland，不改系统/LXC 配置；实际执行与编译证据分开记录。
- Rust 单测试：`cargo test -p <crate> <name> --locked`。GPU/CLI 用例串行跑，避免并发软件
  Vulkan 互相干扰：`VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json cargo test -p cad-render-wgpu -p cad-cli-tools --locked -- --test-threads=1`。
- 需要真实 DWG 的测试（如 `cli_contracts` 的 render）设 `YACR_TEST_DWG=<绝对路径>`，否则
  跳过；跳过不是已运行证据。
- 无头 UI 调试循环（测试计划第二层，可选、非 CI 必需）：`bash scripts/verify-ui.sh`
  用 Slint offscreen 平台 + lavapipe 实际运行应用、操作控件、截图、扫 panic/超时并产出
  分层证据包；不安装 X11/Wayland，不等于真实 GPU/窗口/真机，见 `docs/verify-ui.md`。
- Web 门禁：`scripts/build-web.sh`（需 wasm-bindgen-cli **0.2.129**，与 Cargo.lock 严格一致）
  产出 `web-dist/`；`node --test scripts/test-web-host.mjs scripts/test-web-touch.mjs ...`
  是无 wasm/GPU 的模块契约；浏览器脚本见 `docs/testing-dwg.md`、`docs/validation-web.md`。
- 离线 WGSL 校验（无需 GPU）：`cargo test -p cad-render-wgpu --test wgsl_validation --locked`。

## Android（无头模拟器）

宿主是 Rust `cargo-apk`（android-activity + Slint），**不是 Gradle 工程**，不要运行
`./gradlew`。设备需 x86_64 ABI（本机模拟器镜像为 x86_64）。

```bash
cargo check --target aarch64-linux-android -p cad-ui-slint -p app-android --locked  # 仅编译检查
~/android-dev/emulator-start.sh    # 无头 KVM + SwiftShader（幂等）
~/android-dev/wait-boot.sh         # adb 连接 + sys.boot_completed=1
PROFILE=release ~/android-dev/build-apk.sh   # 模拟器用 x86_64
~/android-dev/install-run.sh
~/android-dev/logcat.sh            # 崩溃：~/android-dev/logcat.sh crash
~/android-dev/emulator-stop.sh
```

规则：模拟器只能 `-gpu swiftshader` 且无头（LXC 无 3D GPU），不要 `-gpu host`，不要装
桌面/X11/Wayland，不要改 Proxmox/LXC 配置。安装用 release APK（debug `.so` ~510 MB 会超
ActivityManager attach 超时）。**模拟器 ≠ 真机**，真机能力一律标注未验证。

## 真实 DWG 回归

流程见 `docs/testing-dwg.md`：**原生离屏 wgpu + Mesa lavapipe 为主**（`cargo build -p
cad-cli-tools --release`，`scripts/check-dwg-native.py`），WASM + Playwright 为第二层。
必须分别记录「打开 / 出图 smoke / 视觉验收」三个结论；退出码 0、非空截图、`error=None`
都不能单独证明视觉正确。外部大样本默认放仓库外（默认 `~/sources/cad-test-files/`），
也可用 `scripts/fetch-test-dwg.sh` 拉到 `/tmp/opencode`。**授权明确、可再分发的图纸与参考图
可提交进 `fixtures/` 并记入 `fixtures/manifest`**，须在 `provenance` 记录来源 URL、下载
日期、许可与 SHA-256；未授权或来源不明的用户/厂商数据仍不得提交。每轮用新输出目录，
保留失败/超时记录，不用旧证据冒充本轮通过。

## 交接点

用 `rg 'pending\(' crates apps` 查找显式占位（文件路径与标识对应实施单元）。替换 `pending`
必须补验收测试，禁止改成空成功。已定义但仍需设计审查或未闭环的事项集中在
`docs/handoff.md`「已定义，但尚需设计审查」与各 `docs/*.md` 的“未完成”小节。

## 文档索引

`docs/architecture.md`（边界与不变量）、`docs/build.md`（各平台构建）、`docs/ci.md`（CI
分层与 NOT RUN）、`docs/validation*.md`（实际执行证据）、`docs/compatibility.md`（能力表）、
`docs/handoff.md`（逐轮交接）、`docs/adr/`、`docs/testing-dwg.md`、`fixtures/manifest`。
