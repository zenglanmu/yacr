# Flatpak Linux 打包重启验证（2026-10-17）

**目的**：记录 2026-10-16 Slint 退役后被删除的 Flatpak 打包流水线在本轮的**重新启用**、其**
前置条件**与**本机实际执行**结果。对应交付：
`packaging/flatpak/dev.yacr.app.{yml,desktop,metainfo.xml}` 与重写的
`scripts/package-linux-release.sh`；说明见 `docs/flatpak.md`。

**这不是**发布验收，**不是**视觉/精度/vendor 结论，**不是**跨平台结论：单机、单适配器、
单会话（GNOME/Mutter + Quadro P620 + `scale=1`）、同 UID。`target/` 产物是**本地** artifact。

## 环境与授权边界

| 项 | 值（实测） |
|---|---|
| 机器/会话 | 本机 GNOME/Mutter 桌面会话，**Wayland**，同一 UID；**无** `sudo`/root、**无**虚拟/离屏显示、**无**屏录 |
| 适配器 | `Quadro P620`（Vulkan，`cad_status` 报 `"backend":"vulkan","adapter":"Quadro P620"`） |
| `flatpak` | **1.16.6** |
| `flatpak-builder` | **1.4.8** |
| 运行时 / SDK | `org.gnome.Platform//51`、`org.gnome.Sdk//51`（均 **user scope**） |
| 运行时 glibc | **2.44**（读取运行时内 `libc.so.6` 版本串） |
| 二进制 glibc 需求 | 最高 **`GLIBC_2.43`**（`objdump -T target/release/yacr`，取 `GLIBC_2.*` 最大者） |
| 运行时 webkit/gtk | `libwebkit2gtk-4.1.so.0`、`libgtk-3.so.0` 均在 `org.gnome.Platform//51` 内 |
| 构建记录 | `cargo 1.99.0`、`node v24.22.0` / `npm 11.19.1`、`vite 8.3.4`（打包日志） |
| 镜像 | 运行时/依赖经本机 **`flathub-mirror`** 远端取得；直连 CDN 实测约 **38 KB/s** |

**镜像 ≠ 替代信任根**：安装摘要的签名为 **subkey `562702E9E3ED7EE8`**（属 Flathub Repo Signing
Key `4184DD4D907A7CAE`），以官方 key 校验通过；镜像只影响**传输**，不影响信任。

**为什么是 GNOME 51 而不是 50**：二进制最多需要 `GLIBC_2.43`，GNOME 51 运行时提供 **2.44**；
GNOME 50 运行时为 glibc **2.42**（低于该需求）。**算术推论**——本轮**未**装 GNOME 50 单跑加载失败，
所以「GNOME 50 无法加载」在本文件里是**推论，不是实测**。

## 产物

| 项 | 值（实测） |
|---|---|
| 命令 | `scripts/package-linux-release.sh`（exit **0**） |
| bundle | `target/release/dist/yacr-0.1.2-linux-x86_64.flatpak` |
| 大小 | **6,724,496 B** |
| sha256 | `cd453e230aac16f64583c25f34552c561ce0761f7338a9c4d31fbde484661e90` |
| 版本来源 | `apps/app-linux/tauri.conf.json` 的 `version` = 0.1.2 |
| 嵌入前端 | `dist/assets/index-Cu0GkZB-.css`、`dist/assets/index-2p1kGFsX.js`（构建日志 “embedded assets verified: 2 file(s)”） |
| 安装 | `flatpak install --user …` 成功；`flatpak info dev.yacr.app` → 0.1.2 / AGPL-3.0 / runtime `org.gnome.Platform//51` / branch `master`（同 app-id/branch 覆盖旧 **0.1.1 Slint** 包） |
| MIME | `xdg-mime query default image/vnd.dwg` → `dev.yacr.app.desktop` |

**嵌入校验的修正（本轮）**：第一版守卫 grep `http://localhost:1420` 判“是否嵌入前端”，是**假阳性**
（嵌入的 `tauri.conf.json` 在 dev 与 release 都带 `build.devUrl`，该串恒存在），会挡下正确构建。
现改为检查 `ui/dist/assets` 的 **hashed 文件名**是否出现在二进制内；`--features
tauri/custom-protocol` 仍是必需条件。修复后构建通过。

## 沙箱内运行结果

命令形状（`flatpak run` 默认继承宿主环境，逐帧探针经 `YACR_PROBE_CAD_LOG=1` 打开；路径参数以日志
`startup drawing:` / `documentLabel` 为准）：

```bash
flatpak run dev.yacr.app --open <repo>/fixtures/debug-default-fonts.dwg
flatpak run dev.yacr.app --open <repo>/fixtures/complex-test.dwg
flatpak run --file-forwarding dev.yacr.app @@ <home 路径> @@
flatpak run --file-forwarding dev.yacr.app @@ /tmp/opencode/fwd-outside.dwg @@
```

| 运行 | 结果（实测） |
|---|---|
| `debug-default-fonts.dwg` | `tauri://localhost` Finished；Vulkan `Quadro P620`；`entityCount=10656`；`pulled=active`；自然 exit 0 |
| `complex-test.dwg` | **523 帧 `pulled=active`**；`entityCount=44591`；`device-limits … granted_max_buffer_size=4292870144 … degraded=false`；自然 exit 0；**零 `is invalid`、零 panic** |
| `--file-forwarding` + **home** 路径 | `documentLabel=/home/…/fixtures/dwg/synthetic-four-lines.dwg`（**原样**传入），4 实体 |
| `--file-forwarding` + **`/tmp`** 路径 | `documentLabel=/run/user/1000/doc/9f5d78c3/fwd-outside.dwg`（document portal 转发），4 实体 |

**同级文件边界（实测）**：document portal 只转发被打开的文件，**图纸的同级文件在该模式下不可见**；
本仓的外部参照（图像）按图纸目录解析，故 manifest 保留 `--filesystem=home`，否则相关图纸会**静默**
少渲染。

## Flathub 状态

- **未提交 Flathub**（用户 2026-10-17 明确决定）：没有提交、没有审核、没有 CI 发布；bundle 只在本地
  `target/` 下。
- `.github/workflows/build.yml` 的 `linux-release` job 本轮**未改**，仍是 `if: false` 硬禁用
  （`name: linux-release (RETIRED — packaging NOT IMPLEMENTED)`）；因此**没有**可下载的发布产物。

## 未测 / 边界（不得省略）

- **NOT RUN**：沙箱内 **X11**（`--socket=fallback-x11`）——只跑了 Wayland。
- **NOT RUN**：沙箱内 `xdg-desktop-portal` **文件选择器**；沙箱窗口的**视觉/截图像素**判读；
  原生 Wayland 子表面的**视觉**判读。
- **NOT RUN**：本机之外的宿主（其它发行版/桌面/合成器/缩放比例）；其它 **GPU/vendor**；
  Flatpak on 非 x86_64；GNOME 50 运行时的加载失败（推论）。
- 运行时未能覆盖的仍由宿主提供：glibc/内核、窗口系统、Vulkan 驱动、`xdg-desktop-portal`、
  `fc-match`；bundle **未签名**。
- 口径提醒：本文件**不**复用旧 Slint 包（`yacr-0.1.1-linux-x86_64.flatpak`，15,368,080 B）的任何
  结论；那是另一条流水线、另一个宿主。旧流水线事实见 `docs/linux-app.md` 历史小节与
  `docs/validation.md`。
