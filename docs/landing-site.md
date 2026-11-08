# 宣传页部署（Landing site）

本文件描述 `apps/landing/` 静态宣传页的部署路径。页面本身**没有构建步骤**，也不属于 Cargo
workspace；它只是 `index.html` / `styles.css` / `main.js` / `downloads.json` / `assets/`。

- 线上地址：<https://yacr.pages.dev/>（Cloudflare Pages 项目 `yacr`）
- 安装包镜像：Cloudflare R2 桶 `yacr-downloads`，通过 r2.dev 公共域名分发
- 对象前缀：`releases/<tag>/`（例如 `releases/v0.1.5/yacr-0.1.5-linux-x86_64.flatpak`）

> 部署/上传成功只是**分发**证据，不是运行时、GPU、真机或像素验收。页面与下载清单也不构成
> DWG/DXF 兼容性承诺。

## 所需 secrets

GitHub Actions 仓库 secrets：

| secret | 权限 |
|---|---|
| `CLOUDFLARE_API_TOKEN` | Account 级 `Cloudflare Pages: Edit` + `Workers R2 Storage Write` |
| `CLOUDFLARE_ACCOUNT_ID` | Cloudflare account id |

token 只在脚本内部使用，脚本不会打印 token。缺少任一 secret 时脚本在第一步非零退出。

## 本地预览

```bash
python3 scripts/serve-web.py --directory apps/landing --port 8091
```

`downloads.json` 在仓库里的默认形态是 `cdnBase: null`，页面回退到 `githubBase`（GitHub
Release 下载链接）。本地预览无需 Cloudflare 凭据。

## 本地/手动镜像与部署

```bash
# 1) 把已发布的 release 资产镜像到 R2，并重写 apps/landing/downloads.json
#    需要 CLOUDFLARE_API_TOKEN / CLOUDFLARE_ACCOUNT_ID，且 gh 已认证
RELEASE_TAG=v0.1.5 bash scripts/upload-r2-downloads.sh

# 2) 发布页面（需要 Cloudflare Pages: Edit）
CLOUDFLARE_API_TOKEN=... CLOUDFLARE_ACCOUNT_ID=... bash scripts/deploy-landing-pages.sh
```

`upload-r2-downloads.sh` 会：

1. `gh release download` 下载 Linux Flatpak（含 `.sha256`、`.flatpak.gpg`）、Windows zip（含
   `.sha256`）、macOS tar.gz（含 `.sha256`）、Android APK（含 `.sha256`）与 `SIGNING.txt`；
2. 对每个 `*.sha256` 运行 `sha256sum -c`；
3. 幂等创建 `yacr-downloads` 桶并启用 r2.dev 托管域名；
4. 用 wrangler v4 `r2 object put ... --remote` 上传（**必须** `--remote`，v4 默认写本地模拟）；
5. 对 4 个主包做 `curl -fsI` HEAD；
6. 全部成功后用临时文件 + `mv` 原子重写 `apps/landing/downloads.json`。

`deploy-landing-pages.sh` 会先校验静态目录（文件非空、`index.html` 引用 `styles.css`/`main.js`、
`downloads.json` 可解析、`assets/` 至少一个非空文件、可用时 `node --check main.js`），再幂等创建
Pages 项目并 `wrangler pages deploy`。它不管理自定义域名。

## CI

`.github/workflows/site.yml`，**只有 `workflow_dispatch`**，没有 `v*` tag 触发：

- 原因：`publish-release.yml` 是唯一 GitHub Release 发布入口，且本身是手动的。tag push 可能发生在
  release 资产可下载之前，tag 触发会与手动发布竞态（失败或发布旧页面）。
- 操作顺序：先等 release 发布完成、资产可下载，再手动 dispatch 本 workflow。
- inputs：
  - `release_tag`（默认 `v0.1.5`）：要镜像到 R2 的 release tag；
  - `upload_downloads`（默认 `true`）：关闭时跳过 R2，直接部署仓库里已提交的
    `downloads.json` 回退清单（适合只改文案/布局）。
- 步骤顺序：先 R2（会重写 `downloads.json`），再 Pages（上传重写后的清单）。
- `concurrency: landing-production`、`cancel-in-progress: false`、`timeout-minutes: 45`；
  任何一步失败即 job 失败，没有 `continue-on-error`。
- 本 workflow 不发布 GitHub Release；发布仍只走 `publish-release.yml`。

## 诚实边界

- **r2.dev 是测试/低成本镜像**：有速率限制，不承诺生产 SLA，也不适合当作唯一 CDN。国内访问
  r2.dev 与 `pages.dev` 均可能不稳定；需要稳定分发时应另接自有域名/CDN（本仓库不自动化域名绑定）。
- `downloads.json` 的 `cdnBase: null` 是合法默认值：页面走 GitHub 回退链接；此时
  `deploy-landing-pages.sh` 会打印 warning 但允许部署。
- 上传/部署成功不等于安装包在真机、真实 GPU 上运行通过；Windows/macOS/Android 的运行时证据边界
  仍以 `AGENTS.md` 平台表与 `docs/handoff.md` 为准。
- `yacr.pages.dev` 子域名是全局唯一的；若已被占用，Pages 可能给项目分配带随机后缀的域名，
  或创建直接失败。此时需改 `CF_PAGES_PROJECT`（以及 `site.yml` 里的值）并同步页面的
  canonical/OG URL，不要静默复用别人的项目。

## 门禁

`python3 scripts/test-landing-site.py` 是纯标准库契约测试，校验页面文件/下载清单 schema、两个
部署脚本的 fail-closed 片段，以及 `site.yml` 保持手动触发、不发布 GitHub Release。它已接入
`core-quality`（`scripts/check-workflows.py` 同步固定该命令），页面缺失或清单漂移会让门禁失败。
