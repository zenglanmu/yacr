# 代码签名（Windows / Android / macOS / Flatpak）

本文记录当前仓库的签名接线、密钥生成与保管、验证方式与**证据边界**。当前策略是
**仓库自持密钥的自签名**：不依赖公共 CA，不做 Apple 公证，也不使用 SignPath。

> **状态（2026-11-05）**：`build.yml` 的签名步骤已从 SignPath 切换为自签名接线，但
> **真实签名尚未在本环境运行**——没有 Windows/macOS runner，也没有配置 secrets。缺失
> 凭据时签名步骤显式跳过，发布 job 只产出 `*-unsigned` artifact；不会把“跳过”写成
> “已签名”。自签名成功也不等于 SmartScreen 信誉、Gatekeeper 放行、设备安装或像素验收。

## 1. 各自用什么签名

| 载体 | 机制 | 私钥位置 | 产物 | 公共信任 |
|---|---|---|---|---|
| Windows | Authenticode，仓库自签 PFX | GitHub Secrets（PFX + 口令） | `yacr-*.zip` 内两个 `.exe` + `bin/yacr-dev-codesign.cer` | ❌ 需用户手动信任证书 |
| Android release APK | APK Signature Scheme v1–v3，仓库自持 keystore | GitHub Secrets（keystore + 口令） | `app-universal-release.apk` + `SIGNING.txt` + `.sha256` | ⚠️ Android 只校验签名一致性，不要求公共 CA |
| macOS release | `codesign`，默认 ad-hoc（`-`） | 无（ad-hoc 不需要密钥）；可用 `MACOS_SIGN_IDENTITY` 覆盖 | `Yacr.app` + `bin/cad-cli-tools`（tar.gz 内） | ❌ Gatekeeper 仍会拦，未公证 |
| Linux Flatpak | OSTree/GPG commit signature | GitHub Secrets（GPG 私钥） | `*.flatpak` + `.flatpak.gpg` | ❌ 用户需导入公钥 |
| Android debug APK | 本地 debug keystore | 构建机 | `android-apk-debug` | ❌ 仅调试 |

自签名的含义要分清：

- **Windows**：签名结构合法，但证书不在公共信任链里。用户机器上默认显示 “Unknown
  publisher”，SmartScreen 仍可能拦；把随包的 `.cer` 导入 Trusted Root / Trusted
  Publishers 后才变成 `Valid`。
- **Android**：APK 签名只要求“以后每次升级用同一张证书”，**不需要公共 CA**。自签
  keystore 是正式发布的标准做法；证书一旦用于公开发布就不能更换，否则系统视为另一个
  应用。
- **macOS**：ad-hoc 签名让 arm64 二进制可运行、让 bundle 自洽，但**不满足 Gatekeeper**
  的分发要求（Developer ID + 公证）。用户需要右键 Open 或移除 quarantine 属性。
- **Flatpak**：GPG 自持密钥本来就是 Flatpak 的标准信任模型，不需要 CA。

## 2. 一次性生成密钥

以下命令只在本机执行一次。私钥、口令、keystore **不得提交到仓库**；生成的 `.pfx` /
`.keystore` 应放在仓库外或加入 `.gitignore`。

### 2.1 Windows 自签 Authenticode 证书

在 Windows 上用 PowerShell：

```powershell
$cert = New-SelfSignedCertificate `
  -Type CodeSigningCert `
  -Subject "CN=Yacr Development Code Signing" `
  -FriendlyName "Yacr Development Code Signing" `
  -KeyUsage DigitalSignature `
  -KeyAlgorithm RSA -KeyLength 3072 -HashAlgorithm SHA256 `
  -CertStoreLocation Cert:\CurrentUser\My `
  -NotAfter (Get-Date).AddYears(10)

$pwd = Read-Host -AsSecureString "PFX password"
Export-PfxCertificate -Cert $cert -FilePath .\yacr-dev-codesign.pfx -Password $pwd
Export-Certificate   -Cert $cert -FilePath .\yacr-dev-codesign.cer
Get-FileHash .\yacr-dev-codesign.cer -Algorithm SHA256
```

仓库**不提交** `.pfx`；CI 每次从 secret 解码后在 runner 上临时导入，并在签名结束后删除。
公开的 `.cer` 由 CI 从 PFX 导出并放进 zip，用户据此手动信任。

### 2.2 Android release keystore

```bash
keytool -genkeypair \
  -keystore yacr-release.keystore \
  -storetype PKCS12 \
  -alias yacr \
  -keyalg RSA -keysize 4096 \
  -validity 10000 \
  -dname "CN=Yacr Development, O=Yacr, C=CN"
```

`-validity 10000` 约 27 年（到 2053 年左右），覆盖 Google Play 对上传密钥
“有效期至少到 2033-10-22”的要求。**发布后不要重新生成**：Android 的签名证书就是应用
身份。

### 2.3 macOS

默认 ad-hoc，不需要任何密钥。只有将来拿到 Developer ID 时才设置：

```bash
gh variable set MACOS_SIGN_IDENTITY --body "Developer ID Application: <Team Name> (<TEAMID>)"
```

> ad-hoc 与自签证书对 Gatekeeper 的结果相同（都会被拦）。不要为 macOS 单独生成自签
> 证书再存 secrets——收益为零、复杂度更高。真正的分发路径是 Apple Developer Program
> （$99/年）+ Developer ID + 公证，本仓库**尚未接线**。

### 2.4 Flatpak GPG

沿用既有做法：维护者本地生成 GPG key，导出 ASCII-armored 私钥与口令到 secrets。

```bash
gpg --quick-generate-key "Yacr Flatpak <release@example.invalid>" rsa4096 sign 5y
gpg --armor --export-secret-keys <KEY_ID> > flatpak-private.asc
```

## 3. 仓库需要配置的 secrets / variables

| 名称 | 类型 | 用途 |
|---|---|---|
| `WINDOWS_SELF_SIGNED_PFX_BASE64` | Secret | 单行 base64 的 PFX；缺失时 Windows 走 `*-unsigned` |
| `WINDOWS_SELF_SIGNED_PFX_PASSWORD` | Secret | PFX 口令；为空时签名步骤显式失败 |
| `ANDROID_KEYSTORE_BASE64` | Secret | 单行 base64 的 release keystore；缺失时 Android 走 `*-unsigned` |
| `ANDROID_KEYSTORE_PASSWORD` | Secret | keystore 口令 |
| `ANDROID_KEY_ALIAS` | Variable | key alias（不是密钥） |
| `ANDROID_KEY_PASSWORD` | Secret | key 口令（PKCS12 通常与 store 口令相同） |
| `MACOS_SIGN_IDENTITY` | Variable（可选） | 留空 = ad-hoc；填 Developer ID 时用它签名 |
| `FLATPAK_GPG_PRIVATE_KEY` | Secret | ASCII-armored GPG 私钥 |
| `FLATPAK_GPG_PASSPHRASE` | Secret | 私钥口令 |
| `FLATPAK_GPG_KEY_ID` | Variable | `--gpg-sign` 使用的 key id/fingerprint |

示例（在仓库根目录、凭据文件所在机器上）：

```bash
base64 -w0 yacr-dev-codesign.pfx | gh secret set WINDOWS_SELF_SIGNED_PFX_BASE64
gh secret set WINDOWS_SELF_SIGNED_PFX_PASSWORD

base64 -w0 yacr-release.keystore | gh secret set ANDROID_KEYSTORE_BASE64
gh secret set ANDROID_KEYSTORE_PASSWORD
gh secret set ANDROID_KEY_PASSWORD
gh variable set ANDROID_KEY_ALIAS --body yacr
```

不要把 `.pfx`、`.keystore`、口令、GPG 私钥写进 workflow、脚本或文档示例。

## 4. CI 流程

### 4.1 `windows-release`

1. `scripts/package-windows-release.sh` 以 `SKIP_ZIP=1` 只构建/暂存；
2. 配置了 `WINDOWS_SELF_SIGNED_PFX_BASE64` 时：
   - base64 解码 PFX → `Import-PfxCertificate` 到 runner 的 `CurrentUser\My`；
   - `Export-Certificate -Type CERT` 把公开证书写入 `bin/yacr-dev-codesign.cer`；
   - 把该公开证书导入 runner 的 `CurrentUser\Root`（仅本次运行、临时），否则自签链会报
     `UnknownError`，`Get-AuthenticodeSignature` 不会返回 `Valid`；
   - `signtool sign /fd SHA256 /f <pfx> /p <pass> /tr http://timestamp.digicert.com /td SHA256`
     签 `bin/yacr-windows.exe` 与 `bin/cad-cli-tools.exe`；
   - `Get-AuthenticodeSignature` 校验 `Status = Valid` **且**签名证书指纹等于导入证书；
3. `scripts/finalize-windows-release.sh --signed` 写入签名说明并打 zip、算 `.sha256`；
4. 上传 `yacr-windows-release`。

未配置 PFX 时走 `--unsigned`，上传 `yacr-windows-release-unsigned`。

> CI 里的 `Valid` 只说明“签名结构完整，且 runner 刚好导入了这张证书”。它**不是**公共
> 信任，也不是 SmartScreen 信誉。

### 4.2 `android-release`

1. 构建 unsigned release APK；
2. `zipalign -p -f 4` 对齐（**签名前**必须完成，签名后重新对齐会破坏 v2+）；
3. 配置了 `ANDROID_KEYSTORE_BASE64` 时：
   - base64 解码 keystore 到 `$RUNNER_TEMP`；
   - `apksigner sign --ks ... --ks-pass env:... --ks-key-alias ... --key-pass env:...`
     输出 `target/android-signed/app-universal-release.apk`；
   - `apksigner verify --verbose --print-certs` 把证书指纹写入 `SIGNING.txt`；
   - 生成 `.sha256`；
4. 上传 `android-release`。

未配置 keystore 时上传 `android-release-unsigned`（`app-universal-release-unsigned.apk`
+ `.sha256`）。签名后再改名会被视为不同包，禁止重命名已签名 APK。

### 4.3 `macos-release`

`scripts/package-macos-release.sh` 在 staging 完成后、打 tar 之前执行：

```bash
codesign --force --sign "${MACOS_SIGN_IDENTITY:--}" bin/cad-cli-tools
codesign --force --sign "${MACOS_SIGN_IDENTITY:--}" Yacr.app
codesign --verify --strict --verbose=2 bin/cad-cli-tools
codesign --verify --strict --verbose=2 Yacr.app
```

这一步同时修复了原有问题：`lipo -create` 合并 universal 二进制会让每个架构的签名失效，
不重新签名时 Apple Silicon 可能拒绝启动。workflow 还会在打包后解 tar、对 `.app` 与
`cad-cli-tools` 再跑一次 `codesign --verify --strict`。

### 4.4 `linux-release`

Flatpak 的签名对象是 **OSTree commit**，不是外层 `.flatpak` 文件：

```bash
flatpak-builder --gpg-sign="$KEY_ID" --gpg-homedir="$GNUPGHOME" --repo=repo ...
# 或已导出后补签：
flatpak build-sign --gpg-sign="$KEY_ID" --gpg-homedir="$GNUPGHOME" repo io.github.zenglanmu.yacr master
# 单文件 bundle（--gpg-keys 会把公钥嵌入 bundle，安装时自动验证）：
flatpak build-bundle --gpg-keys=public.gpg repo app.flatpak io.github.zenglanmu.yacr
```

`flatpak install --gpg-file=public.gpg ./app.flatpak` 会验证 commit 签名；若不带 key 且
bundle 未嵌入公钥，Flatpak 会**静默信任**，所以发布时必须附公钥或嵌入公钥。

## 5. 验证

```powershell
# Windows：解压 zip 后
Get-AuthenticodeSignature .\bin\yacr-windows.exe | Format-List Status, StatusMessage, SignerCertificate

# 让本机信任这张自签证书（可选；否则 Status 不是 Valid）
Import-Certificate -FilePath .\bin\yacr-dev-codesign.cer `
  -CertStoreLocation Cert:\CurrentUser\TrustedPublisher
Import-Certificate -FilePath .\bin\yacr-dev-codesign.cer `
  -CertStoreLocation Cert:\CurrentUser\Root

signtool verify /pa /v .\bin\yacr-windows.exe   # 仅在证书已受信任时通过
```

```bash
# Android
"$ANDROID_HOME/build-tools/34.0.0/apksigner" verify --verbose --print-certs \
  app-universal-release.apk
keytool -printcert -jarfile app-universal-release.apk
# 与 SIGNING.txt / 首次发布记录的证书 SHA-256 比对，确认身份未变

# macOS
codesign --verify --strict --verbose=2 Yacr.app
codesign -dv --verbose=4 Yacr.app      # ad-hoc 时显示 Signature=adhoc
spctl -a -vv Yacr.app                  # ad-hoc/未公证：预期失败，这不是回归
xattr -dr com.apple.quarantine Yacr.app  # 或 Finder 右键 > 打开

# Flatpak
flatpak install --user --gpg-file=yacr-0.1.2-linux-x86_64.flatpak.gpg \
  yacr-0.1.2-linux-x86_64.flatpak
ostree --repo=target/flatpak/repo show --list-detached-metadata-keys \
  app/io.github.zenglanmu.yacr/x86_64/master
```

## 6. 密钥轮换与丢失

- **Windows**：可以轮换 PFX，但用户必须重新导入新 `.cer`；带时间戳的旧签名在证书过期
  后仍然有效。丢失私钥不影响已发布产物，只影响后续签名。
- **Android**：**不能轮换**。keystore 丢失 = 无法升级已发布应用；只能换包名重新发布，
  或依赖 Google Play App Signing 的上传密钥恢复流程。备份 keystore 与口令到离线介质。
- **macOS**：ad-hoc 无密钥；将来接入 Developer ID 时由 Apple 管理证书。
- **Flatpak**：轮换 GPG key 需要同时发布新公钥；旧 bundle 仍用旧 key 验证。

## 7. 未来升级路径（当前未接线）

以下只是选项记录，**不是已实现能力**：

| 方案 | 能力 | 成本 | 自动化 |
|---|---|---|---|
| SignPath Foundation | 公共信任 Windows Authenticode | $0 | 每次发布**人工审批**；要求项目已发布/OSI/无专有组件；publisher 显示为基金会 |
| Azure Artifact Signing | 公共信任 Authenticode | $9.99/月起 | OIDC + 官方 action，仅 Windows runner；有国家/实体资格限制 |
| Certum Open Source | 公共信任 Authenticode | 约 $58/年起 | 仅个人、非商业 OSS |
| Apple Developer Program | Developer ID + 公证 | $99/年 | 需 macOS 公证流程 |
| Sigstore / cosign | 供应链来源与透明日志 | $0 | GitHub OIDC keyless；**不能**产生 Authenticode 或 APK 平台签名 |
| GitHub artifact attestations | 构建来源证明 | 公共仓库免费 | `actions/attest-build-provenance`；不是代码签名 |

SignPath 的 artifact configuration 模板仍保留在 `.signpath/`，但 CI 不再引用；申请
Foundation 前需重新核对条款（本项目“开发中，无可交付产品”的状态与“已发布”要求冲突）。

## 8. 证据边界

- 自签名成功 = 产物带有结构合法的签名且本地验证命令通过；**不等于**公共信任、
  SmartScreen 信誉、Gatekeeper 放行、设备安装或商店审核通过。
- 没有 secrets 时签名步骤是 **skipped**；job summary 与 artifact 名称必须保留
  `unsigned` 事实，不得写成已签名。
- macOS ad-hoc 签名只证明 bundle 自洽；`spctl` 失败是预期结果。
- CI 的 `Get-AuthenticodeSignature = Valid` 依赖 runner 导入证书，不能作为用户机器上的
  信任证据。
- 密钥生成、备份、轮换与丢失恢复不在本仓库 CI 的证明范围内。
- `docs/handoff.md` 的旧记录、`docs/evidence/**`、`docs/adr/**` 与历史 release notes 中
  的 `dev.yacr.app`、SignPath 结论是历史事实，不随本轮切换回写。
