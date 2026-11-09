# ADR 0032：应用内更新检测（落地页版本清单）

- 状态：**Accepted（2026-11-09）**。验证边界：Rust 共享层/宿主目标与前端逻辑测试可在 Linux 运行；
  Windows/macOS/Android/Web 的真实网络、CSP 与外部浏览器打开一律 **NOT VERIFIED**。
- 日期：2026-11-09。
- 相关：[ADR-0024](0024-dom-canvas-browser-carrier.md)（`dom-canvas` 载体）、
  [ADR-0010](0010-react-linux-host.md)（React 宿主）、`docs/landing-site.md`（版本清单与
  `changelog.json`）、`scripts/build-landing-changelog.py`。
- 影响：`cad-ui-core`（一个纯命令）、`cad-desktop-shell`（外部打开 seam 与 shell 命令）、四个
  Tauri 宿主的 `connect-src`、`apps/desktop-ui`（更新检查与提示）。**不**改变：`cad-db` 是唯一
  文档权威；更新提示不是 CAD 文档状态；不引入 HTTP 客户端、updater 插件或第二套文档会话。

---

## 1. 问题

1. 已发布的安装包没有更新通道：用户不知道 <https://yacr.pages.dev/> 上已经有了新版本。落地页
   只有 `downloads.json`（唯一机器可读版本号），没有任何应用侧消费者。
2. 项目是“一棵 React 树，多种载体”，更新检测必须按载体建模（铁律 8/9），不能把 Linux 的
   实现当作通用实现，也不能在 `dom-canvas` 里实例化第二份状态。
3. 仓库没有 TLS 可用的 Rust HTTP 客户端（`reqwest` 只是 tauri 的传递依赖且无 TLS backend），
   也没有任何 `tauri-plugin-updater`/`opener`。为一个版本号把 TLS/证书栈放进共享层是错误方向。
4. 更新提示不能伪装成 CAD 文档对话框：`cad_status.popup` 是模态门控，会阻塞 CAD 输入。

## 2. 决策

### D1 网络读取只在 WebView 侧发生一次

- 唯一请求：`GET https://yacr.pages.dev/downloads.json`（`cache: no-store`，8 s 超时，≤64 KB）。
- 实现在 `apps/desktop-ui/src/update/source.ts`；所有载体（`native-below` 与 `dom-canvas`）
  共用同一份解析与比较。它不读取 `cad-db`，不建立第二份文档状态。
- 版本字段校验：`^v?\d+\.\d+\.\d+$`。清单是**不可信文本**：严格 schema、尺寸/超时上限、
  不作为 HTML 渲染。
- 为此：四个 Tauri `tauri.conf.json` 与 `apps/app-web/web/index.html` 的 `connect-src` 增加
  `https://yacr.pages.dev`；落地页 `_headers` 的 `/downloads.json` 增加
  `Access-Control-Allow-Origin: *`（WebView/浏览器是跨源读取）。

### D2 运行版本由载体回答：共享纯命令 `cad_app_version`

- `cad-ui-core::app_version()` = `env!("CARGO_PKG_VERSION")`，与 About 对话框同源；在
  `registry.rs` 注册为 `shared, pure` 行。
- `native-below` 由 Tauri 包装器回答（原生二进制版本），`dom-canvas` 由 wasm dispatch 回答
  （wasm 产物版本）；无会话时 `without_session` 也回答，避免假 `NO_SESSION`。
- 路由沿用 `invokeRoute`（按 `hostProfile.surfaceMode`），**不按** `__YACR_RUNTIME__`。
- 不使用构建期 `__YACR_APP_VERSION__`：UI bundle 与原生/wasm 产物可能混搭，会让页面撒谎。

### D3 外部打开是 shell 能力，不是 CAD 命令：`cad_open_release_page`

- 命令**不接受 URL 参数**；目标固定为 `cad-desktop-shell::release::RELEASE_PAGE_URL`
  （`https://yacr.pages.dev/#download`）。清单无法指定任意 scheme/host。
- `ShellReleaseOpener` 是平台 seam（与 `ShellTelemetry`/`ShellPopupHost` 同构），实现只在
  `apps/app-*`：
  - Linux `xdg-open`；Windows `cmd /C start`；macOS `open`（均为 std 进程，spawn 后即 ack）。
  - Android 暂不实现：`release_opener = None`，命令返回稳定
    `unsupported-runtime: cad_open_release_page` 拒绝；不是静默成功。
  - Web（无 Tauri bridge）由 `web-adapter` 执行 `window.open(..., '_blank', 'noopener,noreferrer')`；
    被拦截时返回 `update.popup_blocked`。
- 命令进入 `SHELL_ONLY_COMMANDS`：在有 bridge 的 `dom-canvas`（X11/XWayland）也走 Tauri，
  而不是 wasm。

### D4 提示是 chrome 本地状态，不是宿主 popup

- 状态在 `apps/desktop-ui/src/update/store.ts`（`useSyncExternalStore`）；不调用
  `cad_open_popup`，不设置 `cad_status.popup` 模态门控。
- 只有“确认有更新版本”才显示提示；`unsupported` / `unavailable` 不显示任何提示，也**绝不**
  显示“已是最新”。
- 打开失败时保留提示，展示可选中链接 + 复制按钮（复用 `clipboard.ts`）。

### D5 打扰策略

- 每次启动最多检查一次；成功读取后 24 h 节流（`yacr.update.lastCheckAt`）。离线启动不写节流，
  下次启动重试。
- “稍后”只关闭本次会话；[“跳过此版本”] 持久化 `yacr.update.skippedVersion`；更高版本会再次提示。
- 持久化用 `localStorage`（与 `yacr.locale` 同模式，try/catch），**不**放进
  `CadSettingKey`（18 键 CAD 设置表面）或 `preference.json`。

## 3. 数据契约

- 落地页清单：`downloads.json.release`（如 `"v0.1.6"`）是唯一“最新版本”来源；应用比较时允许
  可选 `v` 前缀。
- 命令：
  - `cad_app_version` → JSON 字符串 `"0.1.6"`（无 `v`）。
  - `cad_open_release_page` → `{"accepted": true}`；拒绝以稳定标记开头
    （`unsupported-runtime: …` / `update.open_url_failed: …` / `update.popup_blocked: …`）。
- 前端公开 API：`UpdateStatus`（`idle/checking/up-to-date/available/skipped/unsupported/unavailable`）、
  `startUpdateCheck` / `dismissUpdate` / `skipUpdateVersion` / `openReleasePage`。
- 跨语言漂移：`test/update-config-contract.test.ts` 断言 TS 的 `RELEASE_PAGE_URL` 与 Rust 常量一致。

## 4. 失败闭合

- 未知版本格式、网络失败、超时、超大响应、宿主命令拒绝：都映射为不可用/不支持并保持沉默，
  不产生“已是最新”结论。
- 打不开浏览器：显式失败 + 复制链接，不假装跳转成功。
- Android 没有 opener：稳定拒绝，不用 `window.open` 假装。

## 5. 拒绝的替代方案

| 方案 | 拒绝原因 |
|---|---|
| 每个载体加 Rust HTTP 客户端 | TLS/根证书依赖进共享层，四平台各一套，wasm 仍需 JS fetch ⇒ 两个 fetcher/parser。 |
| `tauri-plugin-http` | 插件 + ACL scope + `@tauri-apps/*` 导入越过 `invoke.ts` 单点，收益为零。 |
| `tauri-plugin-updater` | 完整签名安装/重启流水线，改变发布流程，超出“提示更新”的范围。 |
| `window.open` 作为原生打开方式 | WKWebView/WebView2 行为不一致且不可验证；必须走宿主命令。 |
| 新增宿主 `PopupKind` | 触碰模态门控，阻塞 CAD 输入，需要 native+wasm 双端弹窗处理。 |
| 构建期注入版本号 | 第二个版本权威，混搭产物会撒谎。 |
| GitHub Releases API | 速率限制、不是用户指定的来源、第二套契约。 |

## 6. 证据边界

- 已运行（Linux）：`cad-ui-core`/`cad-desktop-shell`/`app-web` 契约测试、`app-web` wasm 编译、
  `app-linux` 宿主编译、前端 `tsc` + `node --test`。
- **NOT VERIFIED**：Windows/macOS/Android/Web 的真实网络、CSP 生效、`localStorage`、外部浏览器
  打开；任何打包产物对 `yacr.pages.dev` 的 CORS/边缘缓存行为；真实机器上的提示视觉。
- 本 ADR 不涉及 GPU/像素/性能结论。
