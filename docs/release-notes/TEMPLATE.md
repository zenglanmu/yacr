# yacr vX.Y.Z

构建来源 / Source: `CI build layers` run `<run-id>`（tag `vX.Y.Z`）；资产由
`.github/workflows/publish-release.yml` 附加。

发布日期 / Released: YYYY-MM-DD

## 本版变化 / What changed

### <主题> / <Topic>

- 一条一个**用户可见**变化，写清结果；实现细节留在 PR 与设计文档。
- 未完成的能力只写一句状态（例如 `Partial` / 未验收），不要重复整段免责声明。

## 资产 / Assets

| 平台 | 文件 | 签名 |
|---|---|---|
| Linux | `yacr-X.Y.Z-linux-x86_64.flatpak` | Flatpak GPG（可选） |
| Windows | `yacr-X.Y.Z-windows-x86_64.zip` | 未签名 / 自签名 |
| macOS | `yacr-X.Y.Z-macos-universal.tar.gz` | ad-hoc，未公证 |
| Android | `app-universal-release.apk` | 仓库自签名 |

每个包附 `.sha256`；`SIGNING.txt` 记录本版签名状态。

## 安装 / Install

- 仅在安装方式变化时展开步骤；默认指向 <https://yacr.pages.dev/#download>。

---

> 证据边界：本页是构建与分发记录，不是运行时、真机、GPU 或像素验收。平台状态以
> `AGENTS.md` 平台表与 `docs/handoff.md` 为准。

<!--
写作约定（不发布）：
- 默认控制在 60 行以内（功能密集的版本最多约 80 行）；版本说明是“这一版改了什么”，不是验收报告。
- 证据边界固定为一句，放在文末；不要在每节重复“NOT VERIFIED / 验收未完成”。
- 大段命令清单、逐条实现细节放到 docs/ 下的专门文档，这里只留入口。
- `## 本版变化` 下的 bullet 会被 scripts/build-landing-changelog.py 发布到落地页
  更新日志；改完必须重跑该脚本。
-->
