# Linux Flatpak 打包

Linux 发布包改为 **Flatpak**（app-id `dev.yacr.app`），取代此前的 tar.gz。本文记录 manifest、
打包流程、CI 接线与本地实际验证证据。

## 1. 设计

- **manifest**：`packaging/flatpak/dev.yacr.app.yml`（`org.freedesktop.Platform`/
  `org.freedesktop.Sdk` `26.08`，`command: yacr-linux`）。
- **打包方式：预构建二进制**。`scripts/package-linux-release.sh` 先在宿主上
  `cargo build --release --locked` 出 `yacr-linux` 与 `cad-cli-tools`，连同字体落到
  `target/flatpak/payload`，再由 manifest 以 `type: dir` source 安装进 `/app`。
  **沙箱内不编译**（不使用 Rust SDK 扩展、不 vendored crates）。
- **字体在宿主组装**：`fetch-fonts.sh` 在宿主上把字体写进 `target/flatpak/payload/fonts`，
  manifest 的 `type: dir` 只复制这份**本地**目录进 `/app/fonts`，`flatpak-builder` 沙箱内
  **不联网**。已备好字体目录时用 `FONTS_DIR=<dir>`（须含 `fonts.json`）跳过下载；这也可作为
  离线/受限网络环境的做法（宿主或 CI cache 先下载一次，再交给 flatpak-builder）。
- **不打包宿主共享库**：`org.freedesktop.Platform` 运行时已提供 glibc、fontconfig/freetype、
  GL/Vulkan 加载器与窗口库。把宿主库打进 `/app/lib` 会与运行时的 libc/字体配置世代耦合，
  因此不做。选择 `runtime-version 26.08` 的硬性理由是 **glibc**：它提供 glibc 2.44
  （25.08 只有 2.42），高于受支持构建主机（如 Ubuntu 26.04 的 2.43、CI runner），宿主
  构建的二进制才能在沙箱内加载；虚拟机/CI 具体版本见 §4。
- **finish-args**：`--share=ipc`、`--socket=wayland`、`--socket=fallback-x11`、
  `--device=dri`、`--filesystem=home`。前四条是窗口 + GPU + 共享内存；`--filesystem=home`
  让 `--open PATH` 与 `flatpak run --command=cad-cli-tools … render …` 能读写用户文件
  （图形化打开走 `xdg-desktop-portal` 文件选择器，沙箱原生支持）。

### `/app` 布局

| 路径 | 内容 |
|---|---|
| `/app/bin/yacr-linux` | GUI 宿主 |
| `/app/bin/cad-cli-tools` | 无头 CLI（`render`/`scan`/`measure`/…） |
| `/app/fonts/` | CAD 字体包（二进制同级目录，CLI 与 GUI 自动加载） |
| `/app/share/applications/dev.yacr.app.desktop` | 桌面项（`Exec=yacr-linux %F`） |
| `/app/share/metainfo/dev.yacr.app.metainfo.xml` | AppStream 元数据 |
| `/app/share/icons/hicolor/{scalable,256x256}/apps/dev.yacr.app.{svg,png}` | 图标 |

## 2. 构建与运行

```bash
# 一次性准备
sudo apt-get install -y flatpak flatpak-builder
flatpak remote-add --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo
sudo flatpak install -y --noninteractive flathub \
  org.freedesktop.Platform//26.08 org.freedesktop.Sdk//26.08

# 打包（WITH_FONTS=1 默认；YACR_FLATPAK_SMOKE=1 在沙箱内跑一次 CLI）
YACR_FLATPAK_SMOKE=1 scripts/package-linux-release.sh

# 或：宿主先下载一次字体，再直接交给 flatpak-builder（打包步骤不联网）
scripts/fetch-fonts.sh /tmp/yacr-fonts
FONTS_DIR=/tmp/yacr-fonts scripts/package-linux-release.sh

# 安装与运行
flatpak install --user target/release/dist/yacr-0.1.1-linux-x86_64.flatpak
flatpak run dev.yacr.app
flatpak run --command=cad-cli-tools dev.yacr.app --help
```

若本机没有系统 `flatpak-builder`，脚本也接受 `flatpak install flathub org.flatpak.Builder`
后的 `flatpak run --command=flatpak-builder org.flatpak.Builder`。

## 3. CI

`build.yml` 的 `linux-release`（`workflow_dispatch` / `v*` tag，`ubuntu-latest`）：安装
`flatpak`/`flatpak-builder` 与 Slint 构建依赖 → 装 `org.freedesktop.Platform//26.08` +
`org.freedesktop.Sdk//26.08` → `cargo fetch --locked` → `WITH_FONTS=1
YACR_FLATPAK_SMOKE=1 scripts/package-linux-release.sh` → 上传 `yacr-linux-release`
（`*.flatpak` + `*.flatpak.sha256`）。门禁契约：`scripts/test-package-linux-release.py`
（脚本/manifest/桌面项/元数据的静态 + 变异检查）与 `scripts/check-workflows.py`
（`linux-release` 必须仍含 `flatpak-builder`、`org.freedesktop.Sdk` 等片段）。

## 4. 实际验证（2026-10-10，本机 Ubuntu 26.04 + Wayland + Quadro P620）

`WITH_FONTS=0`（离线，仅已提交字体）路径**实际执行并通过**：

- `flatpak-builder-1.4.8` + `org.freedesktop.Platform/Sdk//26.08`（运行时 glibc 2.44）；
  `scripts/package-linux-release.sh` `EXIT=0`，产出
  `target/release/dist/yacr-0.1.1-linux-x86_64.flatpak`（15,368,080 B）+ sha256。
- `flatpak install --user` 成功；`flatpak info dev.yacr.app` 显示版本 0.1.1 / AGPL-3.0-or-later。
- 沙箱内 `flatpak run --command=cad-cli-tools dev.yacr.app --help` 输出 CLI 用法（exit 0）。
- 沙箱内 `flatpak run dev.yacr.app --headless`（缺 `--output`）打印既定解析错误——证明 GUI
  动态加载成功。
- 沙箱内 CLI 渲染 `fixtures/dxf/qcad-flange/flange.dxf` 出 PNG（22,000 B），证明沙箱内
  Vulkan + 渲染链路可用。
- `YACR_FLATPAK_SMOKE=1`：`flatpak-builder --run … cad-cli-tools --help` 成功（`sandbox launch OK`）。
- 沙箱内 GUI `--headless` 因检测到**真实独显** `NVIDIA Quadro P620` 而按既定不变量拒绝
  （`--headless` 仅接受软件 Vulkan）——这是预期行为，也顺带证明沙箱拿到了真实 GPU 适配器。

**NOT RUN / 限制**：

- `WITH_FONTS=1`（默认、CI 使用）在本机**未跑通**：字体 CDN（jsDelivr /
  raw.githubusercontent）在本环境不可达，`fetch-fonts.sh` **以非零退出**中止（未伪造成功）；
  该字体组装逻辑与其他平台打包共用、非本轮改动，CI 有网络。
- `FONTS_DIR` 路径已实测：用准备的字体目录离线打包 `EXIT=0`，安装后 `/app/fonts` 内含所给
  文件——可作为无网/受限网络环境「宿主先下载、再交给 flatpak-builder」的做法。
- 真实窗口/真实 GPU **像素验收**、真机、真实 DWG 视觉验收均 NOT RUN。
- Flatpak CI job 未在本环境执行（无 runner）；bundle 未签名；未发布到 Flathub。
