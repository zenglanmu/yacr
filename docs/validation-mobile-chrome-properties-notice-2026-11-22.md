# 验证记录：紧凑布局无顶栏 / 属性面板自适应与移动抽屉 / 格式提示可持久关闭（2026-11-22）

范围与实现说明见 `docs/handoff-archive/2026-11.md` 同条目与 `docs/compatibility.md`。
本文件只记录**实际运行过的命令与结果**；没有运行的项目明确写 **NOT RUN**。

## 1. Rust（新增 `formatNoticeEnabled` 设置）

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-host --locked` | 452 passed / 0 failed（含 7 个 settings 键计数断言、root preference 默认/持久化/坏叶子回落、clear 恢复默认） |
| `cargo test -p cad-ui-core --locked settings` | 25 passed / 0 failed（含 `CadSettings` 默认快照 `format_notice_enabled == true`） |
| `cargo test -p app-linux --locked settings_tests` | 2 passed / 0 failed（含完整扁平 camelCase JSON 形状与 32 字段计数） |
| `cargo test -p cad-desktop-shell --locked setting` | 1 passed（`setting_commands`，8 个被过滤器排除） |
| `cargo check -p app-linux --all-targets --locked` | exit 0 |
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy -p cad-host -p cad-ui-core -p cad-desktop-shell --all-targets --locked -- -D warnings` | exit 0 |

## 2. 前端（`apps/desktop-ui`）

| 命令 | 结果 |
|---|---|
| `npm test` | 1945 passed / 0 failed（tsc --noEmit ×3 项目 + node --test；含 `mobile-layout`、`navigation-dock`、`theme`、`palette-window`、`properties`、`mobile-actions`、`shell-shortcuts`、`format-provenance-notice`、`format-notice-options`、`options`） |
| `npm run build` | exit 0（仅有既有 chunk-size 警告） |
| `python3 scripts/check-file-size.py` | OK：2619 个代码文件，上限 1000 行 |
| `python3 scripts/check-i18n.py` | OK：3124 keys / 2 catalogs，键与占位符一致，无硬编码 CJK |
| `python3 scripts/check-architecture.py` | OK：44 packages，无环，核心边界完整 |

## 3. NOT RUN

- `cargo test --workspace --locked`、`cargo check --workspace --lib --target wasm32-unknown-unknown`：
  本轮未跑；只验证了改动直接涉及的 crate / target。
- 真实宿主运行（`bash scripts/run-react-linux.sh` / `yacr`）：**NOT RUN**。
- 截图、像素、真实 GPU、触摸手感、真机（Android/平板）：**NOT VERIFIED**。
- Web smoke（Chromium + SwiftShader，`scripts/web-ui-smoke.cjs`）与 Android 宿主布局日志：**NOT RUN**。
  紧凑 chrome 的 `.mobile-topbar` / `.mobile-topbar__menu` 选择器被保留，这两个 harness 的定位契约未改，
  但“保留选择器”不是已运行证据。
- 安全区（`env(safe-area-inset-top)` 在 edge-to-edge + 刘海设备上的实际值）：**NOT VERIFIED**；
  若某台设备把该内缩报成 0，导航卡/按钮/方位球仍会落在切口下——这是宿主/WebView 事实，
  与底部 `--yacr-safe-bottom` 同理。

## 4. 证据边界

本轮所有结论都是**契约测试、类型检查与构建**证据。它证明的是：

- 紧凑布局不再渲染 52px 顶栏；菜单按钮是 fixed 44px 锚点，安全区参与定位；抽屉层为按钮保留
  60px 带，且上限公式不会越出视口；
- ViewCube 在紧凑布局为右上，`right = var(--navrail-w) + 18px`，导航卡与按钮/球共用
  `env(safe-area-inset-top)`；
- 属性面板宽度公式 / 拖拽上限 / preset 范围由纯函数钉住；移动端通过 `MobilePropertiesSheet`
  挂载同一个 `PropertiesDock`，`PROPERTIES`/`QUICKPROPERTIES`/`Ctrl+1`/抽屉入口可达，
  其它 docked palette 仍拒绝；
- 格式提示的“不再提示”只在宿主读回 `formatNoticeEnabled === false` 后关闭；Options 开关与按钮
  写同一个 key；Web/X11 该设置是内存态。

它**不**证明任何像素布局、真实触摸、真实设备或真实 GPU 行为。
