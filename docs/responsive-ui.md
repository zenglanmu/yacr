# 响应式外壳布局与紧凑配置（U01 / U03 / U10）

本文件记录本轮（工作流 `ws/uxpolish`）为审计项 **U01**（单行工具栏 + compact 配置
未消费）、**U03**（面板入口）与 **U10**（硬编码尺寸/英语长文压力）落地的响应式编排。
规范权威是 `CAD_IMPLEMENTATION_SPEC.md` v2.0；面板接线细节见 `docs/panels.md`、
`docs/ui.md`、`docs/diagnostics-ui.md`，国际化契约见 `docs/i18n.md`。

范围：仅 `crates/cad-ui-slint/**` 与本文档。未改 `cad-app`/`cad-db`/宿主 `apps/**`。

## 1. 断点（纯几何，可单测）

断点按**逻辑像素宽度**分类，实现在 `src/responsive.rs::Breakpoint::from_width`：

| 宽度（逻辑像素） | 类 | 参照视口 |
|---|---|---|
| `< 600` | `Phone` | 360×800 竖屏、800×360 横屏 |
| `600 – 1023` | `Tablet` | 800×1280 平板 |
| `>= 1024` | `Desktop` | 1280×800 桌面 |

`ResponsiveMetrics::derive(logical_size, compact_config)` 由宽度与配置派生全部尺寸，
避免在 `.slint` 里逐控件写死像素：

| 字段 | Phone | 非 Phone |
|---|---|---|
| `control_height` | 56px | 40px |
| `touch_target` | **48px**（≥ `MIN_TOUCH_TARGET`） | 32px |
| `side_panel_width` | 0 | 300px（Desktop），Tablet 为 0 |
| `side_panel_collapsed` | 不适用 | Desktop/Tablet 默认 `true` |
| `drawer_height` | 280px | 0 |
| `show_floating_nav` | `false` | Desktop 为 `true` |

**compact/stretch 配置已真正消费**：`compact = 配置 compact || Phone`。同一宽度下
`compact=false` 与 `compact=true` 派生结果不同（单测 `compact_config_is_consumed_not_dead`
断言 `stretched != compact`）；手机永远是 compact，**桌面工具栏不会被压扁**，触控目标
只增不减（`phone_touch_targets_are_at_least_48_logical_pixels`）。

派生值经 `apply_responsive(ui, logical_size, compact)` 写入外壳的几何属性
（`compact-shell`/`phone-shell`/`control-height`/`touch-target`/`side-panel-width`/
`side-panel-collapsed`/`drawer-height`/`show-floating-nav`）。`UiAdapter::new` 在构造时
调用一次，并保留 `UiAdapter::fit_window_to_logical(logical, scale)` 供宿主在 resize 后
重新设置物理尺寸（U07 的 `frame↔画布` 尺寸对齐点）。

## 2. 三种编排（同一组件，按宽度分支）

全部在 `ui/app.slint` 的 `YacrWindow` 内，由 `phone-shell` / `compact-shell` /
`side-panel-width` / `show-floating-nav` 属性选择分支：

- **宽屏 · 查看（Desktop）**：大画布 + 左下浮动导航条（`show-floating-nav`），只含
  `导航` 标签与「适应 / 撤销 / 重做」，不阻塞画布。
- **宽屏 · 工作（Desktop/Tablet）**：`工具与面板` 侧栏，默认折叠
  （`side-panel-collapsed`），顶栏按钮展开；侧栏按组排布「模式 + 历史」「工具」
  「文件 + 后端」，即分组工具 + 可折叠侧栏。
- **手机（Phone）**：顶栏改为 `面板` 抽屉开关 + 诊断；`工具与面板` 抽屉为底部
  分组栏（历史 / 工具 / 文件+后端），所有目标 ≥ 48px。手机**不使用**桌面单行栏，
  而是独立分支 + 独立高度。

## 3. 面板可达性与空状态（U03）

响应式排布里每个已合并面板都有入口，均绑定真实推送模型，空模型显示显式空态：

- 图层（F03）、属性（F05）、批注管理（F09）、诊断抽屉（U08）：见
  `docs/panels.md` / `docs/diagnostics-ui.md`，本轮只重排入口，不改数据契约。
- **布局（F04）**：新增面板，行来自 `bridge::layout_descriptors(&DrawingDatabase)`
  的真实布局表；`UiHandle::set_layout_state(&LayoutPanelState, &[LayoutId])` 推送。
  模型空间是真实哨兵行（index `-1`），非合成布局。不可绘制布局按钮禁用并显示
  `layout.unsupported_marker`；无布局时显示 `layout.empty` 空态，**不造假行**。
- 布局切换回调 `layout-selected(index)` 经 `UiAdapter::set_layout_switch_sink` 交给
  宿主（`LayoutSwitchSink`）；未安装时状态栏显式提示
  `layout.switch_unwired`（"布局切换未接线…"），**不是静默无操作**。本轮未在
  `apps/**` 安装 sink。

## 4. 国际化（U10）

新增 chrome 文案全部经目录，`zh-CN.json` 与 `en.json` 键/占位符一致
（`scripts/check-i18n.py` 由 74 增至 **84** key）：

`layout.panel`、`layout.model_space`、`layout.unsupported_marker`、`layout.empty`、
`layout.switch_unwired`、`shell.tools`、`shell.drawer`、`shell.nav`、
`shell.mode_view`、`shell.mode_work`。

固定尺寸（96px 后端框、28px 状态栏）不再作为默认：状态栏高度改绑
`control-height`；控件最小高度绑 `touch-target`。**英语长文下的实际溢出仍未经
渲染验证**（见 §6）。

## 5. 测试与编译门

- 纯几何/断点/触控目标/compact 消费：`src/responsive.rs` 的 5 个 `#[cfg(test)]`
  单测（`breakpoints_match_the_reference_viewports`、
  `phone_touch_targets_are_at_least_48_logical_pixels`、
  `compact_config_is_consumed_not_dead`、
  `arrangement_flags_are_mutually_exclusive_and_wide_only`、
  `degenerate_viewport_is_a_phone_not_a_panic`）。
- 外壳定义/面板可达性：`src/lib.rs` 新增 `shell_is_responsive_and_consumes_compact_config`、
  `shell_reaches_every_merged_panel_and_layout_list`、`layout_rows_and_active_index_are_pushed_faithfully`。
- **本机无法原生构建 `cad-ui-slint`**（无 fontconfig / pkg-config），因此该 crate 的
  Rust 单测**在此主机未执行**。编译门为：
  `cargo check --workspace --lib --target wasm32-unknown-unknown --locked`，
  以及 `cargo check -p cad-ui-slint --tests --target wasm32-unknown-unknown --locked`
  （验证测试代码可编译）。Slint 渲染未运行。
- 顺带修复 `src/status.rs` 一处**既有**测试编译错误：引用了不存在的常量
  `codes::RESOURCE_FONT_UNRESOLVED`，改为真实常量 `codes::FONT_UNRESOLVED`（否则
  测试目标无法编译，上述测试也无法作为证据）。

## 6. 诚实的缺口（未完成，勿宣称）

- **无设备/浏览器截图验收**：未在真机/浏览器渲染，未测 360×800 / 800×360 /
  800×1280 / 1280×800 的真实溢出、点击命中与视觉层级；窄屏溢出**未经截图确认**。
- **英语长文案溢出未验证**：长标签在窄列的截断/换行未渲染验证。
- **DPR / 安全区（U07）**：`safe_insets` 已并入 `CanvasMetrics`
  （`cad_app::input::inset_canvas_metrics`，退化输入显式拒绝）。宿主接线：Linux
  无便携安全区来源，显式零；Web 由 JS 读 `env(safe-area-inset-*)` 后经
  `web_safe_insets` 传入；Android 提供 `set_surface_insets` 入口但 OS 回调未转发
  （见 `docs/input.md` §5）。真机/浏览器 DPR=1/2/3 与软键盘场景**未验证**。
- **布局切换 sink 已安装**：三个宿主（app-linux / app-web / app-android）均安装
  `LayoutSwitchSink`，经同一 `CommandId::SwitchSpace` 校验命令路径切换（未知布局
  显式拒绝，不改数据库）。
- **宿主未推送面板数据时**：图层/属性/批注/布局面板显示空态，属降级而非假装。
