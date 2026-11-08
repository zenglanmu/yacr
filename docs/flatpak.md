# Linux Flatpak 打包（React/Tauri 宿主）

> **状态（2026-10-17）：打包流水线已重启，产物仅本地**。`packaging/flatpak/**` 与
> `scripts/package-linux-release.sh` 曾于 2026-10-16 随早期桌面 UI 退役删除（commit `7d83572`），
> 本轮为 **React + Tauri** 宿主 `apps/app-linux`（bin `yacr`）**重新引入**。产出的 Flatpak bundle
> **不提交 Flathub、不发布**；没有 Flathub 提交、审核或 CI 发布流水线。逐条运行证据见
> `docs/validation-flatpak-linux-2026-10-17.md`；早期桌面 UI 流水线的历史事实见 `docs/linux-app.md`
> （历史小节，**不是**当前分发路径）。

本文记录 manifest、打包流程、前置条件与**本机实际执行**结果。所有数字均为**实测**，除显式标注者。

## 1. 设计

- **app-id / 运行时**：`io.github.zenglanmu.yacr`，`runtime: org.gnome.Platform//51`，
  `sdk: org.gnome.Sdk//51`，`command: yacr`。运行时由构建用的 SDK 同代提供（见 §2 前置条件）。
- **打包方式：预构建二进制**。`scripts/package-linux-release.sh` 先构建 React 前端
  （`apps/desktop-ui`：`npm ci` + `npm run build`），再以
  `cargo build --release --locked -p app-linux --bin yacr --features tauri/custom-protocol` 构建
  宿主，然后把 `bin/yacr` 与字体包落到 `target/flatpak/payload`；manifest 以
  `type: dir` source 安装进 `/app`。**沙箱内不编译、不联网下载字体**。
- **字体包**：默认 `WITH_FONTS=1` 在 **host 侧**调用 `scripts/fetch-fonts.sh`，把
  `mlightcad/cad-data` 全量目录与提交的 `fonts/`（osifont + Noto Sans CJK SC）合并进 payload
  （需要网络）；`FONTS_DIR=<dir>` 复用预置目录（CI 缓存/离线），`WITH_FONTS=0` 只放提交包
  （离线）。下载失败是硬失败，**不产出“看起来有字体”的空包**。同一个暂存目录还通过
  `YACR_FONT_PACKAGE_DIR` 交给 Vite，嵌入二进制里的 `dist/fonts`（X11/XWayland 的
  `dom-canvas` 页面从 `/fonts/` 懒加载）与 `/app/fonts` 因此是**同一份**全量包。
- **UI 不单独发运**：`ui/dist` 在构建时**嵌入**二进制；`--features tauri/custom-protocol` 是**必需**
  的（否则二进制指向 Vite dev server，无法独立运行）。脚本按 `ui/dist/assets` 里的 **hashed 文件名**
  验证嵌入；**不**用 `http://localhost:1420` 当判据（嵌入的 `tauri.conf.json` 在 dev 与 release 都带
  `build.devUrl`，该串恒存在，第一版守卫是**假阳性**，本轮修正）。
- **bundle 内只有 GUI 宿主**：`/app/bin/yacr` + `/app/fonts/` + 桌面项/AppStream/图标。
  旧 UI 包里的 `cad-cli-tools` **不再**随包分发。
- **不打包宿主共享库**：运行时提供 glibc、GTK3、WebKitGTK 4.1、fontconfig/freetype、GL/Vulkan 加载器
  与 Wayland/X11 客户端库；把宿主库塞进 `/app/lib` 会与运行时代际耦合。
- **finish-args**：`--share=ipc`、`--socket=wayland`、`--socket=fallback-x11`、`--device=dri`、
  `--filesystem=home`。前四条是窗口 + GPU + 共享内存；`--filesystem=home` 的理由是**图纸的同级文件**：
  图像等外部参照按**图纸自身目录**解析，而 document portal 只转发被打开的那一个文件，
  纯 portal 沙箱会隐藏同级文件并**静默**少渲染——故保留家目录读访问。

### `/app` 布局

| 路径 | 内容 |
|---|---|
| `/app/bin/yacr` | GUI 宿主（React chrome + 原生 CAD 分区） |
| `/app/fonts/` | CAD 字体包（二进制解析 `<exe dir>/fonts`，无需 `--fonts-dir`） |
| `/app/share/applications/io.github.zenglanmu.yacr.desktop` | 桌面项（`Exec=yacr %F`） |
| `/app/share/metainfo/io.github.zenglanmu.yacr.metainfo.xml` | AppStream 元数据（AGPL-3.0 / CC0-1.0） |
| `/app/share/icons/hicolor/{scalable,256x256}/apps/io.github.zenglanmu.yacr.{svg,png}` | 图标 |

### `Exec` 必须是 `%F`

宿主 **拒绝**任何含 `://` 的参数（`LaunchOptions::parse_args` 显式诊断），因此 `Exec=yacr %U`
会让文件管理器打开失败（收到 `file://…` URI）。`%F` 传普通路径，裸位置参数按 `--open <path>` 处理。

## 2. 前置条件（实测）

| 项 | 值（本机实测） |
|---|---|
| `flatpak` | **1.16.6** |
| `flatpak-builder` | **1.4.8** |
| 运行时 | `org.gnome.Platform//51`（user scope） |
| SDK | `org.gnome.Sdk//51`（user scope） |
| 运行时 glibc | **2.44**（`org.gnome.Platform//51`） |
| 二进制 glibc 需求 | 最高符号 **`GLIBC_2.43`**（`objdump -T target/release/yacr`） |
| 运行时 webkit/gtk | `libwebkit2gtk-4.1.so.0`、`libgtk-3.so.0`（均在本运行时内） |

- **为什么是 GNOME 51 而不是 50**：二进制要求至多 `GLIBC_2.43`，GNOME 51 运行时给 **2.44**；
  GNOME 50 运行时为 glibc 2.42（低于该要求）⇒ 无法加载该二进制。**算术推论**：本轮**未**安装
  GNOME 50 单独跑加载失败（标为未测），只实测了 51 能加载并运行。
- **镜像说明（实测）**：运行时/依赖经本机 **`flathub-mirror`** 远端取得（直连 CDN 约 **38 KB/s**）；
  脚本的 `FLATPAK_REMOTE_DEPS` / `FLATPAK_REMOTE_BUILD` 默认即 `flathub-mirror`，可用别处配置的远端覆盖。
  摘要签名以官方 key 校验通过（subkey `562702E9E3ED7EE8`，属 Flathub Repo Signing Key
  `4184DD4D907A7CAE`）⇒ 这是**传输镜像**，不是替代信任根。
- **字体下载（打包期，host 侧）**：默认 `WITH_FONTS=1` 访问 `FONT_BASE_URL`（默认 jsDelivr）与
  `FONT_FALLBACK_BASE_URL`（默认 raw.githubusercontent.com）。下载按文件缓存在 `FONT_CACHE_DIR`
  （默认 `${XDG_CACHE_HOME:-$HOME/.cache}/yacr/fonts`），重复打包复用；`FORCE=1` 刷新缓存。
  离线或受限网络用 `FONTS_DIR=<已组装目录>` 或 `WITH_FONTS=0`（只放提交包），二者都不联网。

## 3. 构建 / 安装 / 运行

```bash
# 一次性准备（user scope；缺 runtime/SDK 时脚本硬失败并给出安装提示）
# 用本机配置的远端（脚本默认 flathub-mirror；有官方 flathub 远端时也可用 flathub）
flatpak install --user <remote> org.gnome.Platform//51 org.gnome.Sdk//51

# 打包（前端 + release 宿主 + 字体全量组装 + flatpak-builder + build-bundle）
scripts/package-linux-release.sh

# 离线 / 预置字体包
WITH_FONTS=0 scripts/package-linux-release.sh            # 只放提交的 fonts/
FONTS_DIR=/path/to/fonts scripts/package-linux-release.sh  # 复用已组装目录，打包步骤免联网

# 安装与运行
flatpak install --user target/release/dist/yacr-0.1.2-linux-x86_64.flatpak
flatpak run io.github.zenglanmu.yacr --open /absolute/drawing.dwg
```

配置 `FLATPAK_GPG_KEY_ID` / `FLATPAK_GPG_PRIVATE_KEY` / `FLATPAK_GPG_PASSPHRASE` 后，
`package-linux-release.sh` 会签 OSTree commit，并把公钥嵌入 bundle；此时上面的 `flatpak install`
会验证 commit 签名，同时产物旁会多一个 `*.flatpak.gpg` 供显式 `--gpg-file=` 使用。未配置时
bundle 是未签名的，CI summary 会明确写 `UNSIGNED`。

- 脚本接受 `FLATPAK_REMOTE_DEPS` / `FLATPAK_REMOTE_BUILD`（默认 `flathub-mirror`）；缺少
  `cargo`/`node`/`npm`/`flatpak`/`flatpak-builder`/`python3` 或 runtime/SDK 时**非零失败**，不产出包。
- 字体：默认 `WITH_FONTS=1` 在 host 侧跑 `scripts/fetch-fonts.sh`（需网络，体量随全量目录增长）；
  `WITH_FONTS=0` 只放提交的 `osifont` + `Noto Sans CJK SC`；`FONTS_DIR=<dir>` 用已组装目录
  （目录必须含 `fonts.json`），此时打包步骤不联网。非法 `WITH_FONTS` 值直接失败。
- 若本机没有系统 `flatpak-builder`，可用 `flatpak run --command=flatpak-builder org.flatpak.Builder`
  的包装脚本（脚本只要求 `flatpak-builder` 在 `PATH` 上）。
- 脚本自检包括：前端产物存在、**hashed asset 已嵌入二进制**、payload 的 `fonts/fonts.json` 存在；
  任一项失败即中止，不产出半成品。

## 4. 实测结果（2026-10-17，本机 GNOME/Mutter + Quadro P620 + `scale=1`）

> **历史 app-id 说明**：下表是 2026-10-17 的实测记录，当时 app-id 仍为 `dev.yacr.app`。
> 当前 app-id 已统一为 `io.github.zenglanmu.yacr`；历史命令/输出不改写，避免伪造证据。
> 该轮 payload 字体为提交包；当前默认 `WITH_FONTS=1` 会额外打入 mlightcad 全量目录，bundle 体积
> 会明显大于下表，下表数字**不可**当作当前产物体积。

| 项 | 结果（实测） |
|---|---|
| `scripts/package-linux-release.sh` | **exit 0** |
| 产物 | `target/release/dist/yacr-0.1.2-linux-x86_64.flatpak`，**6,724,496 B** |
| sha256 | `cd453e230aac16f64583c25f34552c561ce0761f7338a9c4d31fbde484661e90` |
| `flatpak info dev.yacr.app` | 0.1.2 / AGPL-3.0 / runtime `org.gnome.Platform//51` / branch `master` |
| 安装 | `flatpak install --user` 成功，同 app-id/branch **覆盖**旧 0.1.1 版本的包 |
| MIME 关联 | `xdg-mime query default image/vnd.dwg` → `dev.yacr.app.desktop` |

沙箱内运行（同 UID、无 root/sudo、无屏录、Wayland 会话；命令形状如下，路径参数以日志里的
`startup drawing:` / `documentLabel` 为准，逐帧探针由宿主环境继承 `YACR_PROBE_CAD_LOG=1` 打开）：

| 运行 | 结果（实测） |
|---|---|
| `flatpak run dev.yacr.app --open <debug-default-fonts.dwg>` | `tauri://localhost` Finished；Vulkan `Quadro P620`；`entityCount=10656`；`pulled=active`；exit 0 |
| `flatpak run dev.yacr.app --open <complex-test.dwg>` | 523 帧 `pulled=active`；`entityCount=44591`；`device-limits … granted_max_buffer_size=4292870144 … degraded=false`；exit 0；**无 panic、无 `is invalid`** |
| `flatpak run --file-forwarding dev.yacr.app @@ <home path> @@` | 路径**原样**传入（**home** 可见），4 实体 |
| `flatpak run --file-forwarding dev.yacr.app @@ <tmp path> @@` | 变为 `/run/user/1000/doc/9f5d78c3/fwd-outside.dwg`（document portal），4 实体；该模式下**同级文件不可见** ⇒ 保留 `--filesystem=home` |

## 5. 未测 / 边界（不得省略）

- **未发布 Flathub**（用户 2026-10-17 明确决定）：bundle 未提交、未审核、无 Flathub 发布。
  2026-11-01 起 CI `linux-release` 已重新启用（仅 `v*` tag / 手工 dispatch），会上传
  `.flatpak` workflow artifact；**不是** Flathub 发布，也不代表本文件的本地运行证据已在 CI 复现。
- **NOT RUN**：沙箱内 **X11**（`--socket=fallback-x11`）；沙箱内 `xdg-desktop-portal` 文件选择器；
  沙箱窗口的**视觉/截图像素**判读；原生 Wayland 子表面的视觉判读。
- **NOT RUN**：本机之外的宿主（其它发行版/桌面/合成器）；其它 **GPU/vendor**；
  Flatpak on 非 x86_64；GNOME 50 运行时的加载失败（只做算术推论）。
- 本文件不构成视觉正确性、精度、性能或 vendor 兼容结论；单机、单适配器、`scale=1`。

## 6. 已知崩溃：NVIDIA + Wayland + 运行时 WebKitGTK 2.54.1 显式同步

- **现象**：真机 5K/`scale=2` NVIDIA Wayland 宿主机上，启动时或窗口移动到 2×（5K）输出时，
  GDK 打印 `Gdk-Message: ... Error 71 (Protocol error) dispatching to Wayland display`
  （中文环境为“协议错误”），客户端被杀、应用退出。
- **原因**：`org.gnome.Platform//51` 当前携带 **WebKitGTK 2.54.1**（2026-11-05 沙箱内
  `webkit_get_major_version = 2.54.1` 实测）。其 DMA-BUF 渲染器启用了 Wayland 显式同步
  （`wp_linux_drm_syncobj`），但提交时没有设置 acquire point；Mutter 以
  `wl_display.error(..., 4, no_acquire_point)` 拒绝该提交。宿主 WebKitGTK **2.52.6** 不复现。
  上游：WebKit bug 280210。
- **立即可用的命令**（优先保留 DMA-BUF/加速）：
  - 首选：`flatpak run --env=__NV_DISABLE_EXPLICIT_SYNC=1 io.github.zenglanmu.yacr --open <dwg>`
  - 回退：`flatpak run --env=WEBKIT_DISABLE_DMABUF_RENDERER=1 io.github.zenglanmu.yacr --open <dwg>`
- **应用内护栏**：`apps/app-linux/src/nvidia_quirk.rs` 在 GTK/WebKit/EGL 初始化前，仅当
  `resolved == Wayland`、检测到 NVIDIA 驱动（读 `/proc/driver/nvidia/version`，沙箱内可见）且用户
  未设置四个覆盖变量（`WEBKIT_DISABLE_DMABUF_RENDERER`、`__NV_DISABLE_EXPLICIT_SYNC`、
  `WEBKIT_DISABLE_COMPOSITING_MODE`、`WEBKIT_DMABUF_RENDERER_FORCE_SHM`）之一时，设置
  `__NV_DISABLE_EXPLICIT_SYNC=1` 并打印稳定探针行
  `[probe] nvidia-quirk: __NV_DISABLE_EXPLICIT_SYNC=1 reason=wayland+nvidia+webkit-explicit-sync (upstream WebKitGTK bug 280210)`；
  有覆盖时只打印 `[probe] nvidia-quirk: not applied (webkit/nvidia env override present)`。原生与
  Flatpak 运行都生效。
- **证据诚实**：崩溃来自真实 5K/scale-2 NVIDIA 宿主报告；在无头 Mutter + 虚拟显示器（scale 1→2）
  下**未复现**。因此该 workaround 在失败的真机配置上仍是 **NOT VERIFIED**，直到按上面的命令重测；
  本文件不把它写成“已修复”。
