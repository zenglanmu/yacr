# 代码签名（Windows / Android / Flatpak）

本文记录当前仓库的签名接线、所需凭据、验证方式与**证据边界**。签名成功只代表产物带有对应
平台的签名，不代表窗口、GPU、设备、像素或性能验收。

> **状态**：签名步骤已接线到 `.github/workflows/build.yml`，但 **SignPath 与 GPG 的真实
> 签名请求尚未在本环境运行**。没有配置 secrets/vars 时，SignPath 步骤显式跳过，发布 job 只
> 产出 `*-unsigned` artifact；不会把“跳过”写成“已签名”。

## 1. 各自用什么签名

| 载体 | 机制 | 谁持有密钥 | 产物 |
|---|---|---|---|
| Windows | Authenticode（SignPath） | SignPath（HSM） | `yacr-*.zip` 内的两个 `.exe` |
| Android release APK | APK Signature Scheme v1–v4（SignPath） | SignPath（HSM） | `app-universal-release.apk` + `.idsig` |
| Linux Flatpak | OSTree/GPG commit signature | 仓库维护者（GitHub Secrets 导入） | `*.flatpak` + `.flatpak.gpg` |
| Android debug APK | 本地 debug keystore | 构建机 | `android-apk-debug` |

**SignPath 不支持 Flatpak/OSTree/ELF**。Flatpak 必须用 GPG；不要尝试用 SignPath 的代签
服务替代。

## 2. Windows / Android：SignPath

### 2.1 账号前置条件

SignPath Foundation 面向符合条件的开源项目，但**不是自动通过**：

- OSI 认可许可证、无商业双许可/专有组件、项目已发布且可验证；
- Authors / Reviewers / Approvers 角色，MFA，每次发布人工审批；
- 项目主页需要有 “Code signing policy” 段落；
- 仓库当前仍标注“开发中，无可交付产品”，这与 Foundation 的“已发布形式”要求存在
  **资格风险**；不要假定一定会被接受。

申请入口：<https://signpath.org/apply>。接受后完成：

1. 添加 GitHub.com connector 并安装 SignPath GitHub App；
2. 创建 project，以及 **Windows 与 Android 各一个 signing policy**（Authenticode 与 APK 使用
   不同证书；都启用人工审批与 trusted build verification）；
3. 创建两个 artifact configuration，slug 分别填入仓库 variables；
4. 创建 CI user（Submitter 权限），生成 API token。

> **Android 覆盖范围需确认**：SignPath 文档把 APK 列入 Open Source Code Signing，但公开的
> Foundation 条款偏 Windows；在 onboarding 时必须向 SignPath 确认 Foundation 是否提供长期
> Android 签名密钥、轮换与恢复策略。若 Foundation 不覆盖 APK，Android 需退回自签 keystore +
> GitHub Secrets（或 SignPath 的付费方案），不要假定已经可用。

### 2.2 仓库需要配置的 secrets / variables

| 名称 | 类型 | 用途 |
|---|---|---|
| `SIGNPATH_API_TOKEN` | Secret | CI user token；缺失时签名步骤跳过 |
| `SIGNPATH_ORGANIZATION_ID` | Variable | 组织 GUID |
| `SIGNPATH_PROJECT_SLUG` | Variable | 项目 slug |
| `SIGNPATH_WINDOWS_SIGNING_POLICY_SLUG` | Variable | Windows Authenticode 签名策略 slug |
| `SIGNPATH_ANDROID_SIGNING_POLICY_SLUG` | Variable | Android APK 签名策略 slug（与 Windows 分开，证书类型不同） |
| `SIGNPATH_WINDOWS_ARTIFACT_CONFIGURATION_SLUG` | Variable | Windows PE 配置 |
| `SIGNPATH_ANDROID_ARTIFACT_CONFIGURATION_SLUG` | Variable | APK 配置 |

`token` 只放 Secret；slug/GUID 不是密钥，放 Variable。不要把证书私钥或 Android keystore
提交到仓库。

### 2.3 Artifact configuration

仓库内保留可审查副本：

- `.signpath/windows-pe.xml`：`<zip-file>` 下两个 `<pe-file>`（`bin/yacr-windows.exe`、
  `bin/cad-cli-tools.exe`），各自 `<authenticode-sign />`。
- `.signpath/android-apk.xml`：`<zip-file>` 下 `<apk-file path="app-universal-release.apk">`
  + `<create-apk-signature />`；v4 会额外生成 `.idsig`。

SignPath UI 中的配置才是权威；仓库副本用于审查和 onboarding。Foundation 可能要求在
`<pe-file>` 上增加 `product-name` / `product-version` 等 PE 元数据限制——请在 SignPath
里根据实际 PE 版本资源填写，不要在仓库里猜值。

### 2.4 CI 流程

`windows-release`：

1. `scripts/package-windows-release.sh` 以 `SKIP_ZIP=1` 只构建/暂存，不打包；
2. 暂存目录上传为 `yacr-windows-stage-unsigned`；
3. `signpath/github-action-submit-signing-request@v3` 用 **`github-artifact-id`**（不是
   artifact name）提交签名，`output-artifact-directory` 下载已签 exe；
4. 复制签好的 exe 回暂存目录，`Get-AuthenticodeSignature` 验证 `Valid`；
5. `scripts/finalize-windows-release.sh --signed` 写入签名说明、重新打 zip、重算 `.sha256`；
6. 上传 `yacr-windows-release`。无 token 时走 `--unsigned`，上传
   `yacr-windows-release-unsigned`。

`android-release`：

1. 构建 unsigned release APK；
2. `zipalign -p -f 4` 对齐为 `app-universal-release.apk`（**签名前**必须对齐）；
3. 上传为 `android-release-input`，提交 SignPath；
4. `apksigner verify --print-certs` 验证；
5. 上传 `android-release`（APK + `.idsig` + `.sha256`）。无 token 时上传
   `android-release-unsigned`。

SignPath 人工审批默认等待 600 秒不够，workflow 设为 3600 秒，并把
`signing-request-web-url` 写入 job summary。

### 2.5 Android 应用身份警告

SignPath Foundation 用**它托管的 Android 密钥**签名。APK 的签名证书就是应用身份：一旦用
该证书发布，后续升级必须继续用同一证书；换回自己的 key 会被系统视为另一个应用。本项目
当前未发布，已按维护者决定接受该身份绑定；若以后要迁回自签 keystore，必须在首次公开发布
**之前**决定。

### 2.6 验证

```bash
# Windows（在 Windows 上；解压 zip 后）
signtool verify /pa /v bin/yacr-windows.exe
signtool verify /pa /v bin/cad-cli-tools.exe
# 或 PowerShell
Get-AuthenticodeSignature bin/yacr-windows.exe | Format-List

# Android
"$ANDROID_HOME/build-tools/34.0.0/apksigner" verify --verbose --print-certs \
  app-universal-release.apk
```

## 3. Linux Flatpak：GPG

### 3.1 机制

Flatpak 安装时验证的是 **OSTree commit 签名**，不是外层 `.flatpak` 文件的 detached
signature。`flatpak build-bundle --gpg-sign` 当前只用于 OCI 路径，不能用来签普通
`.flatpak`。正确做法是在导出/构建阶段签 commit：

```bash
flatpak-builder --gpg-sign="$KEY_ID" --gpg-homedir="$GNUPGHOME" --repo=repo ...
# 或已导出后补签：
flatpak build-sign --gpg-sign="$KEY_ID" --gpg-homedir="$GNUPGHOME" repo io.github.zenglanmu.yacr master
# 单文件 bundle（--gpg-keys 会把公钥嵌入 bundle，安装时自动验证）：
flatpak build-bundle --gpg-keys=public.gpg repo app.flatpak io.github.zenglanmu.yacr
```

`flatpak install --gpg-file=public.gpg ./app.flatpak` 会验证 commit 签名；若不带 key 且
bundle 未嵌入公钥，Flatpak 会**静默信任**，所以发布时必须附公钥或嵌入公钥。

### 3.2 CI 凭据

| 名称 | 类型 | 用途 |
|---|---|---|
| `FLATPAK_GPG_PRIVATE_KEY` | Secret | ASCII-armored 私钥 |
| `FLATPAK_GPG_PASSPHRASE` | Secret | 私钥口令（通过 `gpg-preset-passphrase` 预置） |
| `FLATPAK_GPG_KEY_ID` | Variable | `--gpg-sign` 使用的 key id/fingerprint |

`linux-release` 在构建前把私钥导入 `$RUNNER_TEMP/gnupg`，预置口令，设置
`FLATPAK_GPG_HOMEDIR`；`scripts/package-linux-release.sh` 检测到 `FLATPAK_GPG_KEY_ID` 时
给 `flatpak-builder` 加 `--gpg-sign` / `--gpg-homedir`，并导出
`yacr-<version>-linux-<arch>.flatpak.gpg`。未配置 key 时保持未签名，并在 summary 明确写
`UNSIGNED`。

### 3.3 验证

```bash
# 公钥已嵌入 bundle 时：
flatpak install --user yacr-0.1.2-linux-x86_64.flatpak

# 或显式提供发布的公钥（也会真正安装到 user scope）：
flatpak install --user --gpg-file=yacr-0.1.2-linux-x86_64.flatpak.gpg \
  yacr-0.1.2-linux-x86_64.flatpak

# 查看 commit 的 detached metadata（GPG 签名键为 `ostree.gpgsigs`）
ostree --repo=target/flatpak/repo show --list-detached-metadata-keys \
  app/io.github.zenglanmu.yacr/x86_64/master
```

不要把 `flatpak install` 成功当成窗口/GPU/像素验收。

## 4. 证据边界

- SignPath 步骤成功 = SignPath 返回了签名产物且本地验证命令通过；不等于 SmartScreen
  信誉、Android 设备安装、Vulkan/GPU 或像素验收。
- Foundation 证书 / GPG key 的审批、轮换、丢失恢复不在本仓库 CI 的证明范围内。
- 没有 secrets/vars 时，workflow 的签名步骤是 **skipped**；job summary 与 artifact 名称
  必须保留 `unsigned`/`skipped` 事实，不得写成已签名。
- `docs/handoff.md` 的旧记录、`docs/evidence/**`、`docs/adr/**` 与历史 release notes 中
  的 `dev.yacr.app` 是历史事实，不随本次改名回写。
