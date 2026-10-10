# 应用图标（App icon）

单一来源：**`assets/yacr-icon.svg`**（仓库根 `assets/`，用户提供的 YACR “Y” 蓝图图标，
自包含，无外部引用）。所有平台派生资产都由它生成，不得各自维护副本。

## 派生资产

`python3 scripts/generate-icons.py` 从 SVG 生成全部派生资产；`--check` 逐字节比对已提交
资产，作为漂移守卫（需要 `rsvg-convert` 与 Pillow，属于一次性/本地校验工具，默认提交门禁
不运行）。生成结果（均已提交，构建时无需光栅化工具）：

| 路径 | 用途 |
| --- | --- |
| `assets/yacr-icon.ico` | Windows 多分辨率图标（16–256） |
| `assets/yacr-icon.icns` | macOS 多分辨率图标（16–1024） |
| `assets/icons/yacr-<size>.png` | Linux hicolor、Web manifest、apple-touch |
| `apps/app-android/res/mipmap-*/ic_launcher.png` | Android 传统启动图标（48dp 基准，各密度） |
| `apps/app-android/res/mipmap-*/ic_launcher_foreground.png` | 自适应图标前景（108dp 画布，66dp 安全区） |
| `apps/app-android/res/mipmap-anydpi-v26/ic_launcher.xml` | API 26+ 自适应图标定义 |
| `apps/app-android/res/values/colors.xml` | 自适应图标背景色 `#08151D` |

## 各端接入

- **桌面（Linux / Windows / macOS）**：`crates/cad-ui-slint/ui/app.slint` 的
  `YacrWindow` 设置 `icon: @image-url("../../../assets/yacr-icon.svg")`。图标在编译期嵌入，
  运行时二进制自包含；winit 后端据此设置窗口与任务栏图标（X11 生效；Wayland 通常改从
  `.desktop`/app_id 取图标，见下）。契约测试
  `crates/cad-ui-slint/tests/app_icon_contract.rs`。
- **macOS 包**：`scripts/package-macos-release.sh` 复制
  `Contents/Resources/Yacr.icns` 并在 `Info.plist` 写 `CFBundleIconFile=Yacr`。
- **Linux 包**：`scripts/package-linux-release.sh` 舞台化
  `share/icons/hicolor/scalable/apps/yacr.svg`、`256x256/apps/yacr.png` 与
  `share/applications/yacr.desktop`（`Icon=yacr`）。
- **Windows 包**：`apps/app-windows/build.rs` 用 `winresource` 把
  `assets/yacr-icon.ico` 编进 `yacr.exe` 的 PE 资源段（MSVC 走 `rc.exe`，GNU 走
  `windres`；非 Windows 目标为 no-op）。Explorer/任务栏因此直接显示图标；包内另附
  `yacr.ico` 供安装器/快捷方式使用。`package-windows-release.sh` 用
  `check-pe-imports.py --has-icon` 断言该资源存在，缺失即失败。
- **Android**：`apps/app-android/Cargo.toml` 的 `[package.metadata.android] resources = "res"`
  与 `[package.metadata.android.application] icon/label`；`res/` 由上面的生成资产填充。
- **Web**：`apps/app-web/web/index.html` 链接 SVG favicon、apple-touch 与
  `manifest.webmanifest`；`scripts/build-web.sh` 把 `assets/` 与 manifest 拷进 `web-dist/`。

## 验证状态（诚实记录）

- **已运行（合成/本地）**：`cargo check -p cad-ui-slint`；`cargo test -p cad-ui-slint
  --test app_icon_contract`（2 passed）；`node --test scripts/test-web-icon.mjs`；
  `python3 scripts/test-android-icon.py`；三个 `test-package-*-release.py`；
  `python3 scripts/generate-icons.py --check`。
- **未运行**：真实 GPU/窗口管理器图标显示、Android APK 构建与真机启动图标、macOS `.app`
  图标。Windows `.exe` 内嵌图标经本机 GNU 交叉构建 + `check-pe-imports.py --has-icon`
  验证，MSVC 路径由 windows-release CI 构建时验证，均不证明真机外观。
- **本地已运行（补充）**：`cargo build -p app-windows --target x86_64-pc-windows-gnu` +
  `python3 scripts/check-pe-imports.py --has-icon target/x86_64-pc-windows-gnu/debug/yacr.exe`。

## 平台工具

派生资产需要 `rsvg-convert`（`librsvg2-bin`）与 Pillow；这是**本地生成工具**，非运行时或
构建依赖。Android 启动图标通常也可只提交生成后的 `res/`，CI 不需重跑生成。
