# 插件系统 UI、UI 基础设施与全局界面框架功能需求清单

本文档反推自 `src/ui/icons.rs`、`src/ui/style/fusion_theme.rs`、`src/ui/style/common.rs`、`src/ui/theme_accessibility_tests.rs`、`src/ui/window/plugin_manager.rs`、`src/ui/window/mod.rs`、`src/ui/mod.rs`、`src/plugin/`（`mod.rs`、`external.rs`、`registry.rs`、`marketplace.rs`、`host.rs`、`v4_support.rs`）、`src/app/plugin_host.rs`、`crates/ocs_plugin_api/src/ribbon.rs`，以及全局框架相关的 `src/ui/window/update_notice.rs`、`src/ui/window/about.rs`、`src/ui/window/options.rs`（仅主题/插件入口）。插件在 Ribbon 中的呈现入口是 `src/ui/ribbon/mod.rs::Ribbon::set_modules` 与 `src/plugin/registry.rs::ribbon_modules_enabled`。

---

## 一、插件管理器窗口（Plugins / Plugin Manager）

窗口为模态框（`ModalKind::PluginManager`），桌面尺寸 940×600（`src/app/view/modal.rs::modal_content` 的 `sized_flow(ex, 940, 600, …)`），整体背景为 `background.base.color`。布局为左右两栏：左侧目录栏宽 410（`container(catalog_pane).width(Length::Fixed(410.0))`），右侧为详情（README）面板。

### 插件管理器窗口本体（Plugins）
- **功能简介**：集中浏览、安装、启用/禁用、更新与卸载插件；查看每个插件的详情与 GitHub README。
- **UI 入口**：`开始页（Start）→ Plugins 按钮`；或 `命令输入框 → PLUGINS / PLUGINMANAGER`；或 `Options → Applications → Plugins…`。窗口标题 “Plugins”（`crate::tr!("modal", "plugin-manager")`），副标题 “Browse, install, and manage add-ons. Select one to view its README.”。
- **样式**：标题字号 20；副标题字号 12 使用 `muted_style`（`background.base.text` 68% 透明）；窗口内边距 18；卡片圆角 6、选中卡片边框主色 1.5px + `primary.weak` 18% 背景，未选中边框 `background.strong` 1.0px（`card_style`）；滚动内容右缘预留 `SCROLLBAR_GUTTER = 16.0`。
- **触发命令**：`PLUGINS`、`PLUGINMANAGER`（`inventory` 注册于 `src/ui/window/plugin_manager.rs`；分发 `src/app/commands/view.rs::"PLUGINS" | "PLUGINMANAGER"`；`start_allowed` 允许在开始页执行）。
- **实现位置**：`src/ui/window/plugin_manager.rs::view_window`、`MarketView`；宿主状态 `src/app/mod.rs`（`ModalKind::PluginManager`）；消息处理 `src/app/update/mod.rs`（`PluginManagerOpen` 等）。
- **备注**：每次打开时重新 `discover()` 磁盘插件，并抓取目录（registry）、各仓库 release 列表与选中插件的 README；`selected_plugin_repo` 缺省取第一个已安装/目录/手动仓库。Web 构建改为显示桌面下载提示（见下）。

### 搜索框（Search plugins…）
- **功能简介**：按名称、id、描述、仓库或命令前缀（`command_prefixes`）过滤插件目录与已安装列表。
- **UI 入口**：`Plugins 窗口 → 左栏顶部 → Search plugins… 输入框`。
- **样式**：`text_input`，字号 13，内边距 `[7, 10]`，宽度 Fill；占位符 “Search plugins…”。
- **触发命令**：`Message::PluginSearchInput(String)`。
- **实现位置**：`src/ui/window/plugin_manager.rs::view_window`（`search`）、`external_matches_search`、`matches_search`。
- **备注**：搜索为大小写不敏感子串匹配；无匹配时分别显示 “No installed plugins match your search.” / “No available plugins match your search.”。

### 已安装插件区（Installed）
- **功能简介**：列出本机 `plugins` 目录（及随包内置）发现的插件包，每包一张卡片，含启用/禁用、更新、卸载。
- **UI 入口**：`Plugins 窗口 → 左栏 → “Installed” 标题下方`（仅当存在已安装插件时显示标题）。
- **样式**：小节标题“Installed”字号 13 主色（`primary_style`）；每张卡片由 `external_card` 生成，圆角 6、内边距 `[10, 12]`、卡片间距 10；未安装时显示 “No plugins installed yet.”（字号 13，muted）。
- **触发命令**：无（数据来自 `crate::plugin::external::discover()`）。
- **实现位置**：`src/ui/window/plugin_manager.rs::view_window`（`externals` 循环）、`external_card`。
- **备注**：每卡显示名称、版本徽章（`badge`，字号 11，`primary.weak` 背景圆角 4）、API 版本徽章、状态徽章、id（字号 11 muted）、描述（字号 12 muted）、命令前缀行（“Commands:  %{cmds}”）、错误详情（Danger 色）。

#### 插件状态徽章（Status badge）
- **功能简介**：以彩色药丸显示插件当前装载状态，避免用户困惑为何插件未生效。
- **UI 入口**：`Plugins 窗口 → Installed 卡片 → 右上角状态药丸`。
- **样式**：`status_badge`，字号 11，内边距 `[2, 8]`，圆角 4；Muted=`background.weak`、Success=`success.weak`、Danger=`danger.weak`、Warning=`warning.weak`。
- **触发命令**：无（`StatusKind` 由状态推导）。
- **实现位置**：`src/ui/window/plugin_manager.rs::status_badge`、`external_card`。
- **备注**：状态取值（按判定顺序）：`Disabled`（已加载但被禁用，Muted）、`Loaded`（Success）、`API incompatible`（Danger）、`Incompatible`（acadrust/rustc 不匹配，Danger）、`Load failed`（Danger）、`No library`（Warning，缺本机库文件）、`Restart to load`（Warning，需重启加载）。

#### 启用 / 禁用按钮（Enable / Disable）
- **功能简介**：对已加载插件切换启用状态；禁用会立即移除其 Ribbon 选项卡与命令分发，并持久化到设置，跨启动生效。
- **UI 入口**：`Plugins 窗口 → Installed 卡片 → 右下角 “Enable” / “Disable” 按钮`（标签显示点击后将执行的动作）。
- **样式**：`toggle_button`，字号 12，内边距 `[3, 12]`；禁用态显示 “Enable” 用 `button::success`，启用态显示 “Disable” 用 `button::danger`。
- **触发命令**：`Message::SetPluginEnabled(id, bool)`。
- **实现位置**：`src/ui/window/plugin_manager.rs::toggle_button`；状态变更 `src/app/update/mod.rs::Message::SetPluginEnabled`（维护 `disabled_plugins`、`rebuild_ribbon_modules`、`persist_settings_if_changed`）；持久化字段 `src/app/settings.rs::UserSettings::disabled_plugins`。
- **备注**：仅 `loaded` 的插件显示该按钮；切换后立刻重建 Ribbon 选项卡与命令行补全池，无需重启。

#### 更新按钮（Update to <tag>）
- **功能简介**：当仓库存在比已装版本更高的可兼容 release 时，一键升级到该版本。
- **UI 入口**：`Plugins 窗口 → Installed 卡片 → 右下角 “Update to %{tag}”`。
- **样式**：`pill_button`，字号 12，内边距 `[4, 12]`，`button::primary`。
- **触发命令**：`Message::PluginUpdate(repo, tag)`。
- **实现位置**：`src/ui/window/plugin_manager.rs::external_card`（`newest_update` 计算标签）、`pill_button`；处理 `src/app/update/mod.rs::Message::PluginUpdate`（转 `install_task`）。
- **备注**：`newest_update` 用 semver 比较（`trim_version_prefix` 去 `v` 前缀），且仅接受 `host_accepts_plugin_version`、`acadrust_compatible`、`rustc_compatible` 的 release；无更新则不显示按钮。

#### 卸载按钮（Uninstall）
- **功能简介**：删除插件包目录；当前会话仍加载（库常驻），下次启动起效。
- **UI 入口**：`Plugins 窗口 → Installed 卡片 → 右下角 “Uninstall”`（内置 `bundled` 插件不显示该按钮，改显示 “Bundled with Open CAD Studio” muted 文本）。
- **样式**：`pill_button`，`button::danger`。
- **触发命令**：`Message::PluginUninstall(id)`。
- **实现位置**：`src/ui/window/plugin_manager.rs::external_card`；处理 `src/app/update/mod.rs::Message::PluginUninstall`（先 `remove_plugin` 停止 runner，再 `external::uninstall`）；实现 `src/plugin/external.rs::uninstall`。
- **备注**：卸载前先停止插件进程以便 Windows 释放 DLL；失败时 `marketplace_status` 显示 “Uninstall failed: …” 或 “plugin '{id}' did not stop in time”。

#### 点击卡片信息区（打开 README）
- **功能简介**：点击卡片名称/描述区域选中该插件，并在右侧载入其 README。
- **UI 入口**：`Plugins 窗口 → Installed/目录卡片 → 信息区（可点击）`。
- **样式**：无边框文本按钮（`button::text`），宽度 Fill，内边距 0。
- **触发命令**：`Message::PluginReadmeSelect(repo)`（仅当能解析出仓库时；否则信息区为不可点击静态文本）。
- **实现位置**：`src/ui/window/plugin_manager.rs::external_card`；`repository_for_external`。

### 可用插件市场区（Available plugins）
- **功能简介**：显示精选目录（registry）与用户手动关联仓库中可安装的插件，选择 release 后安装。
- **UI 入口**：`Plugins 窗口 → 左栏 → “Available plugins” 标题下方`。
- **样式**：小节标题“Available plugins”字号 13 主色；卡片同 `card_style`；已安装或无匹配的条目不重复显示（`repository_is_installed` 去重、精选与手动仓库互相抑制）。
- **触发命令**：无（数据来自 `crate::plugin::marketplace::fetch_registry` 与 `fetch_release_info`）。
- **实现位置**：`src/ui/window/plugin_manager.rs::marketplace_section`、`market_card`、`install_controls`。
- **备注**：无可见项且无错误时显示 “No additional plugins are available.” 或 “No available plugins match your search.”。

#### Release 选择器（pick_list）
- **功能简介**：在某个仓库的可用 release 列表中选择要安装/升级的版本。
- **UI 入口**：`Plugins 窗口 → Available plugins 卡片 → release 下拉`。
- **样式**：`pick_list`，字号 12；无 release 时显示 “no releases”（字号 11 muted）。
- **触发命令**：`Message::PluginReleaseSelect(repo, tag)`。
- **实现位置**：`src/ui/window/plugin_manager.rs::install_controls`。

#### 安装按钮 / 兼容性药丸（Install / Incompatible / Unavailable）
- **功能简介**：安装所选 release；若该 release 与宿主 API/ABI 不兼容则显示不可安装提示。
- **UI 入口**：`Plugins 窗口 → Available plugins 卡片 → release 下拉右侧`。
- **样式**：“Install” 为 `pill_button`（`button::success`）；不兼容为 Danger 药丸 “Incompatible”；未选/无可用为 Muted 药丸 “Unavailable”。
- **触发命令**：`Message::PluginInstall(repo)`（仅当选中的 release 满足 `host_accepts_plugin_version` 且 acadrust/rustc 兼容）。
- **实现位置**：`src/ui/window/plugin_manager.rs::install_controls`；处理 `src/app/update/mod.rs::Message::PluginInstall`；下载安装 `src/plugin/marketplace.rs::install`。
- **备注**：安装写入 `plugins/<id>/`（库文件 + `plugin.toml` + `.source_repo`）；成功后状态栏提示 “Installed '{id}'. Restart to load it.” 并刷新已安装列表。

#### 移除仓库按钮（✕，Remove repository）
- **功能简介**：从目录中移除一个手动添加的仓库。
- **UI 入口**：`Plugins 窗口 → Available plugins → 用户添加仓库卡片 → 右端 ✕ 图标按钮`。
- **样式**：`pill_icon_button`（`icons::CLOSE` 图标 11px），内边距 `[5, 9]`，`button::danger`。
- **触发命令**：`Message::PluginRepoRemove(repo)`。
- **实现位置**：`src/ui/window/plugin_manager.rs::install_controls`、`pill_icon_button`；处理 `src/app/update/mod.rs::Message::PluginRepoRemove`。
- **备注**：仅手动添加的仓库（`removable=true`）显示；精选 registry 条目不显示。

### 添加仓库卡片（Add from GitHub）
- **功能简介**：手动关联一个公开 GitHub 仓库，自动检测其兼容 release 与 README。
- **UI 入口**：`Plugins 窗口 → Available plugins 区底部 → “Add from GitHub” 卡片`。
- **样式**：卡片内标题字号 14，说明字号 11 muted；输入框占位 “GitHub URL or owner/repository”，字号 13，宽度 Fill，内边距 0；卡片内边距 `[10, 12]`。
- **触发命令**：输入 `Message::PluginRepoInput(String)`；提交 `Message::PluginRepoAdd`（回车或按钮）。
- **实现位置**：`src/ui/window/plugin_manager.rs::add_repository_card`；处理 `src/app/update/mod.rs::Message::PluginRepoAdd`（`normalize_repository` 归一化）。
- **备注**：非法输入提示 “Enter a GitHub URL or repository in owner/repo format.”；重复（已在目录或已安装）提示 “{repo} is already in the catalog.” / “{repo} is already installed.”；成功提示 “Fetching releases for {repo}…” 然后 “Repository added. N installable release(s) found.”。该卡片下方有 `marketplace_status` 状态行（字号 11 muted）。

### 注册目录加载 / 错误通知（registry notice）
- **功能简介**：展示精选目录抓取中的进度或抓取失败的用户友好错误，并提供重试、查看/复制诊断。
- **UI 入口**：`Plugins 窗口 → Available plugins 标题下方`（错误或加载中时才出现）。
- **样式**：错误卡为 `warning.weak` 16% 背景 + `warning.weak` 边框 1px、圆角 6、内边距 `[10, 12]`；标题字号 13，说明字号 11 muted；加载卡为 `container::bordered_box`。诊断详情为 `bordered_box` 内字号 10 全文。
- **触发命令**：`Message::PluginRegistryRetry`、`PluginRegistryErrorDetailsToggle`、`PluginRegistryCopyDiagnostics`。
- **实现位置**：`src/ui/window/plugin_manager.rs::registry_notice`、`registry_error_message`；处理 `src/app/update/mod.rs` 对应分支。
- **备注**：错误分三类文案——证书无法验证（“Unable to verify the server certificate”）、请求超时（“Plugin registry request timed out”）、其它（“Unable to load the plugin registry”）；按钮 “Retry”/“Hide details”/“Show details”/“Copy details”。加载卡文案 “Loading plugin catalog…” + “Connecting securely using your system certificate settings.”。“Copy details” 复制含版本、OS、架构、REGISTRY_URL 与错误的诊断块。

### 详情 / README 面板（Plugin details）
- **功能简介**：显示选中插件的 GitHub README（Markdown），并可跳转 GitHub。
- **UI 入口**：`Plugins 窗口 → 右栏`（未选中时显示 “Plugin details / Select a plugin to read its GitHub README.”）。
- **样式**：`container::bordered_box`，内边距 `[12, 14]`；头部标题字号 16，仓库名 `owner/repo` 字号 11 主色；`markdown::view` 文本字号 13；内容为可滚动，右缘 `SCROLLBAR_GUTTER`。
- **触发命令**：`Message::OpenUrl(...)`（“View on GitHub” 按钮与 README 内链接，`resolve_readme_link` 解析相对/锚点/裸链接）。
- **实现位置**：`src/ui/window/plugin_manager.rs::readme_panel`、`resolve_readme_link`；抓取 `src/plugin/marketplace.rs::fetch_readme`。
- **备注**：加载中显示 “Loading README… / Fetching the default branch from GitHub.”；失败显示 “README could not be loaded” + 错误 + “Retry”（`PluginReadmeSelect` 二次点击即重试）；未见内容提示 “Select the plugin again to load its README.”。

### Web 构建的插件提示（view_web_notice）
- **功能简介**：浏览器版无法运行原生插件，改为引导下载桌面版。
- **UI 入口**：`Web 构建 → Plugins 模态框`（`ModalOptions::NOTICE`）。
- **样式**：齿轮图标（`icons::GEAR` 26px）置于 `primary.weak` 圆角 12、44×44 图标容器；标题字号 20；正文字号 13 muted；下载按钮 `button::primary` 白字、内边距 `[9, 18]`；整体居中，最小宽 380。
- **触发命令**：`Message::OpenUrl(DESKTOP_DOWNLOAD_URL)`。
- **实现位置**：`src/ui/window/plugin_manager.rs::view_web_notice`、`DESKTOP_DOWNLOAD_URL`；分派 `src/app/view/modal.rs`（wasm 分支）。

### Options 中的插件入口与路径
- **功能简介**：在选项窗口查看插件安装目录并直接打开插件管理器。
- **UI 入口**：`Options → Applications → “Installed plugins and their sources” 行 → “Plugins…” 按钮`；`Options → Folders → “Plugins” 行 → “Open folder”`。
- **样式**：说明文字字号 12；按钮 “Plugins…” 字号 11，内边距 `[4, 10]`，`button::secondary`；“Open folder” 同风格。
- **触发命令**：`Message::PluginManagerOpen`、`Message::OpenFolder(path)`。
- **实现位置**：`src/ui/window/options.rs`（`folders.plugins`、`Folders`、`folder_row`）；路径由 `src/app/view/modal.rs` 传入 `crate::plugin::external::plugins_dir()`；`src/plugin/external.rs::plugins_dir`。

---

## 二、插件在 UI 中的呈现

### 插件 Ribbon 选项卡
- **功能简介**：每个已启用的已加载插件贡献一个独立的 Ribbon 选项卡，其标题与分组由插件声明。
- **UI 入口**：`Ribbon 选项卡栏 → <插件标题>`（位于核心模块选项卡之后）。
- **样式**：选项卡按钮字号 12；选中态用 `background.weakest` 背景 + 主色下划线（与核心选项卡同渲染路径 `src/ui/ribbon/mod.rs` 选项卡构建），布局标签页 `layout` 模块不显示选项卡。
- **触发命令**：`Message::RibbonSelectTab(i)`。
- **实现位置**：`src/ui/ribbon/mod.rs::Ribbon::new`（`all_ribbon_modules()`）、`set_modules`；`src/plugin/registry.rs::all_ribbon_modules/ribbon_modules_enabled`；选项卡渲染 `src/ui/ribbon/mod.rs`（`self.modules.iter().enumerate()`）。
- **备注**：顺序为“核心模块 + 已加载外部插件”，外部插件内部按 `manifest.ribbon_order`（小者在前）再按 id 排序（`src/plugin/external.rs::discover_from_roots`）。插件标题经 `i18n::ribbon_module_title(id, fallback)`，非内置 id 用插件自报标题。禁用插件通过 `disabled` 集合过滤。

### 插件工具按钮（ToolDef → RibbonItem）
- **功能简介**：插件在自身选项卡分组中声明按钮，点击后触发其 `ModuleEvent`（命令/视觉样式/图层面板/文件对话框等）。
- **UI 入口**：`Ribbon → <插件选项卡> → <插件分组> → 插件按钮`。
- **样式**：由 `RibbonItem` 形态决定（`Tool`/`LabeledTool`/`LargeTool`/`Dropdown`/`LabeledDropdown`/`LargeDropdown`/`ToolGrid`/`LayerComboGroup`/`PropertiesGroup`/`StyleComboGroup`，同核心 Ribbon 尺寸常量 `ROW_H`、`LARGE_W`、`SMALL_ICON` 等）。
- **触发命令**：`Message::RibbonToolClick { tool_id, event }`；下拉项同路径，事件为 `ModuleEvent`。
- **实现位置**：`src/ui/ribbon/widgets.rs::render_large` / `render_group`（渲染 `RibbonItem`）；`src/ui/ribbon/mod.rs` 事件分发；插件模块来源 `ocs_plugin_api::ribbon::CadModule::ribbon_groups`，宿主经 `PluginManager::ribbon_modules` 提供。
- **备注**：插件图标经 `make_icon(IconKind, size)`：`Glyph` 用文本字形（`text(s).size(size*0.7)`），`Svg` 用 `icons::semantic`（多色语义图标，随主题重着色）。

### 插件命令出现在命令行补全
- **功能简介**：插件声明的命令（每个 ribbon 工具的 id + manifest `command_prefixes`）合并进命令行自动补全池，可键入而非仅点按钮。
- **UI 入口**：`命令输入框`（输入前缀时提示插件命令）。
- **样式**：命令行补全下拉（同核心命令渲染）。
- **触发命令**：输入命令名；分发 `src/plugin/registry.rs::try_dispatch`。
- **实现位置**：`src/plugin/registry.rs::plugin_command_names`；写入 `CommandLine.dynamic_commands` 于 `src/app/update/file.rs::rebuild_ribbon_modules`。
- **备注**：启用/禁用、启动加载、设置重载时都会刷新该池。

### 插件交互式命令（命令行提示与对象拾取）
- **功能简介**：插件命令可进入交互步骤，向宿主提供提示串并要求对象拾取；宿主把它们当作常规命令展示。
- **UI 入口**：`命令行提示` 与 `视口拾取`（激活插件命令时）。
- **样式**：复用核心命令提示与拾取高亮。
- **触发命令**：`Message::RibbonToolClick`/命令行 → `PluginProcessInteractiveAdapter`；点/回车/Escape/对象拾取事件。
- **实现位置**：`src/app/plugin_host.rs::PluginProcessInteractiveAdapter`（`CadCommand` 实现：`prompt`、`on_point`、`on_enter`、`on_escape`、`needs_entity_pick`、`on_entity_pick`）；启动 `src/plugin/registry.rs::try_dispatch` 的 `result.started`。
- **备注**：`Drop` 时发送 `InteractiveEvent::Cancel` 释放等待中的插件步骤。

### 插件运行错误上报到命令行
- **功能简介**：插件 panic、进程死亡或分发错误会以错误行出现在命令行历史，而不是静默失败或崩溃宿主。
- **UI 入口**：`命令历史（Command History）→ 错误行（Danger 色）`。
- **样式**：命令行错误条目用 `danger.base.color`（见无障碍节）。
- **触发命令**：无（由 `guard`/`try_dispatch` 自动产生）。
- **实现位置**：`src/plugin/mod.rs::guard`、`drain_errors`；`src/plugin/registry.rs::try_dispatch` 的 `dead_plugins`/`errors`；`src/app/mod.rs::push_plugin_error`；`HostApi` 的 `push_info`/`push_output`/`push_error`（`src/app/plugin_host.rs`）。

### 插件生命周期面向用户的行为
- **功能简介**：插件为进程外 cdylib，启动时发现并按 API/ABI 门控加载；安装/升级需重启到加载；禁用即时生效。
- **UI 入口**：状态体现在 `Plugins 窗口`的状态徽章与状态行。
- **样式**：见各状态徽章。
- **触发命令**：无。
- **实现位置**：`src/plugin/external.rs::load_at_startup`、`discover`、`uninstall`、`remove_plugin`、`shutdown_plugins`；`src/plugin/registry.rs`；门控 `ocs_plugin_api::manifest::host_accepts_plugin_version`、`version_info::uses_acadrust_gate`。

---

## 三、插件 API 提供的 UI 元素词汇（ocs_plugin_api::ribbon）

插件通过实现 `CadModule`（`id`/`title`/`ribbon_groups`）声明其 UI。以下为可用的全部 UI 数据类型与形态。

### CadModule（模块 = 一个 Ribbon 选项卡）
- **功能简介**：插件声明一个选项卡及其分组；`title` 为选项卡标题，`id` 用于本地化/禁用识别。
- **UI 入口**：`Ribbon → <title> 选项卡`。
- **样式**：选项卡按钮（同核心）。
- **实现位置**：`crates/ocs_plugin_api/src/ribbon.rs::CadModule`。
- **备注**：有 `owned` 子模块（`pub mod owned;`）用于宿主渲染已拥有数据。

### RibbonGroup（面板/分组）
- **功能简介**：将一组工具在选项卡内并排列为一个带标题的面板。
- **UI 入口**：`Ribbon → <插件选项卡> → <group.title> 面板`。
- **样式**：面板标题可点击展开飞出的行为由宿主统一处理（核心与插件一致）。
- **实现位置**：`ocs_plugin_api::ribbon::RibbonGroup { title, tools }`。

### ToolDef（按钮定义）
- **功能简介**：单个工具按钮：唯一命令 id、标签、图标、点击时发出的事件。
- **UI 入口**：按钮本体（形态由包裹的 `RibbonItem` 决定）。
- **样式**：`label` 为图标下/旁文字，`icon` 决定图形。
- **实现位置**：`ocs_plugin_api::ribbon::ToolDef { id, label, icon, event }`。

### IconKind（图标来源）
- **功能简介**：插件可选用 Uni​​code 字形或嵌入 SVG 作为按钮图标。
- **UI 入口**：按钮图标。
- **样式**：`Glyph(&'static str)` → 文本字形（`size*0.7`）；`Svg(&'static [u8])` → 语义多色图标（`icons::semantic`，随主题重着色）。
- **实现位置**：`ocs_plugin_api::ribbon::IconKind`；渲染 `src/ui/ribbon/widgets.rs::make_icon`。

### 各 RibbonItem 形态（插件能做出的界面元素类型）
- **`Tool(ToolDef)`** — 1 行小按钮，仅图标无标签。
- **`LabeledTool(ToolDef)`** — 1 行按钮，图标 + 文字标签。
- **`LargeTool(ToolDef)`** — 3 行大按钮，图标 + 下方标签，占满面板高度。
- **`Dropdown { id, icon, items, default }`** — 1 行下拉，仅图标 + 右侧 ▾；`items` 为 `(id, label, IconKind)` 列表，`default` 为当前项 id。
- **`LabeledDropdown { id, label, icon, items, default }`** — 1 行带固定标签的下拉 + 菜单箭头。
- **`LargeDropdown { id, label, icon, items, default }`** — 3 行下拉，图标 + 标签 + ▾。
- **`ToolGrid { columns }`** — 显式多列、每列小图标按钮网格。
- **`LayerComboGroup { row2, row3 }`** — 图层组合下拉 + 两行小按钮（row2 作用于选中对象图层 off/freeze/lock/make-current；row3 全图层 on/thaw/unlock/match）。
- **`PropertiesGroup { match_prop }`** — Match Properties 大按钮 + 颜色/线型/线宽组合。
- **`StyleComboGroup { style_key, combo_id, manager_cmd, rows }`** — 样式选择组合框（文本/标注/多重引线/表格样式）+ 0~2 行小工具；`manager_cmd` 打开样式管理器。
- **实现位置**：`ocs_plugin_api::ribbon::RibbonItem`、`StyleKey`（`TextStyle`/`DimStyle`/`MLeaderStyle`/`TableStyle`）。
- **备注**：`impl From<ToolDef> for RibbonItem` 让 `ToolDef` 可直接作为最简小按钮加入分组。

### ModuleEvent（点击发出的宿主事件）
- **`Command(String)`** — 触发具名 CAD 命令（如 `LINE`、`CIRCLE`）。
- **`OpenFileDialog`** — 打开操作系统文件对话框。
- **`ClearModels`** — 清空场景中所有已加载模型。
- **`SetVisualStyle(String)`** — 切换活动视口视觉样式；名称为 `WIREFRAME2D`、`WIREFRAME3D`、`HIDDENLINE`、`FLATSHADED`、`GOURAUDSHADED`、`FLATSHADEDWITHEDGES`、`GOURAUDSHADEDWITHEDGES`（大小写不敏感）。
- **`ToggleLayers`** — 切换图层管理器面板。
- **`PluginFileDialog { command, title, filter_name, extensions }`** — 宿主打开文件选择器，选中后把 `"<command> <path>"` 回派给插件（保留原始大小写，绕过命令行）；取消则无动作。
- **实现位置**：`ocs_plugin_api::ribbon::ModuleEvent`。

### Forum：插件面板/菜单入口的能力边界
- **功能简介**：插件只能通过 Ribbon 选项卡/分组/按钮与 `ModuleEvent` 扩展界面；不能自定义独立窗口、停靠面板或任意停靠区（据源码推断——`CadModule` 仅暴露 `ribbon_groups`）。
- **UI 入口**：无独立窗口。
- **实现位置**：`ocs_plugin_api::ribbon::CadModule`（仅选项卡 + 分组）。
- **备注**：`ModuleEvent::ToggleLayers` 可打开核心图层面板；`OpenFileDialog`/`PluginFileDialog` 借用宿主对话框。

---

## 四、图标系统

### 图标资源类别
- **功能简介**：应用图标分为多类静态 SVG，编译期用 `include_bytes!` 嵌入，运行时按类别取用。
- **UI 入口**：全局 UI（Ribbon、状态栏、菜单、窗口）。
- **样式**：见下各类。
- **实现位置**：`src/ui/icons.rs`。
- **备注**：类别清单：
  - 三角箭头/导航（`ui/`）：`tri_down/up/right/left.svg`、`home.svg`、`undo.svg`、`redo.svg`。
  - 对象捕捉标记（`osnap/`）：endpoint、midpoint、center、node、quadrant、intersection、extension、insertion、perpendicular、tangent、nearest、apparent、parallel、grid、mtp 共 15 个。
  - 图层状态（`layers/`）：`layon/layoff/layfrz/laythw/laylck/layulk.svg`。
  - 单色 chrome 字形（`ui/`）：`CHECK`、`CLOSE`、`PLUS`、`MINUS`、`TRASH`、`NODE_GRAPH`、`COPY`、`MENU`、`MOVE`、`RESIZE`、`PIN`、`SPLIT_V`、`SPLIT_H`、`GRID`、`SNAP`、`DOC_NEW`、`DOC`、`FOLDER_OPEN`、`SAVE`、`FILE_EXPORT`、`PRINT`、`HEART`、`GEAR`（仅 wasm）、`DOT`、`DIRTY_DOT`、`ARROW_LONG_RIGHT`。均为黑底透明，调用处用 `tinted`/`themed*` 重着色。
  - 状态栏开关（`status/`）：ortho、polar、osnap、otrack、dyn、lwt、transparency、isolate、quickprops、filter、selcycle、cleanscreen（issue #216）。
  - 导航：`pan.svg`、`zoom_in.svg`。

### 主题化 chrome 图标（themed 家族）
- **功能简介**：单色界面图标按活动主题的文字/主色/语义色着色，保证跨亮暗主题可读。
- **UI 入口**：按钮、菜单、工具栏、勾选单元格。
- **样式**：`themed` 用 `background.base.text`；`themed_secondary` 用文字 72% 透明；`themed_disabled` 用 42% 透明；`themed_primary`/`themed_primary_weak_text`/`themed_success`/`themed_success_text`/`themed_warning`/`themed_danger`/`themed_danger_text` 分别用对应语义色对。
- **实现位置**：`src/ui/icons.rs::themed/themed_secondary/themed_disabled/themed_primary/themed_primary_weak_text/themed_success/themed_success_text/themed_warning/themed_danger/themed_danger_text`。
- **备注**：内部经 `themed_handle` 线程本地 `(地址, 长度)` 缓存 `svg::Handle`（warm ≈9ns、cold ≈26ns/次），避免每帧重建。

### 语义多色工具图标（semantic 家族）
- **功能简介**：工具 SVG 用一小组语义色源色，绘制时映射到活动主题的扩展调色板，保留多色外观并适配主题。
- **UI 入口**：Ribbon 工具图标（含插件 `IconKind::Svg`）。
- **样式**：源色映射——青色系→`primary`、绿→`success`、黄→`warning`、红→`danger`、灰阶→文字/次要/弱次要、`#1a1a1a`→背景。
- **实现位置**：`src/ui/icons.rs::semantic/semantic_disabled/semantic_handle/recolor_semantic_svg/semantic_color`。
- **备注**：缓存键含 6 色调色板指纹（`SemanticCacheKey`），上限 `SEMANTIC_CACHE_LIMIT = 2048`。

### 箭头/撤销重做/勾选辅助图标
- **功能简介**：下拉箭头随展开翻转、撤销/重做随可用性变灰、勾选单元格固定宽度着色。
- **UI 入口**：Ribbon 下拉 ▾、命令/菜单、下拉勾选列。
- **样式**：`themed_arrow_down/up/toggle(open)`、`themed_arrow_right/left`、`themed_primary_weak_arrow_down`、`themed_secondary_arrow_down`、`themed_disabled_arrow_down/right`、`themed_home`、`themed_undo(size, enabled)`、`themed_redo(size, enabled)`；`themed_check_cell(active)` 固定宽 14，勾号 11×11。
- **实现位置**：`src/ui/icons.rs`。
- **备注**：`themed_check_cell` 的勾色用 `accessible_accent` + WCAG 3.0 回退逻辑，保证在所有主题上对 `weak`/`strong` 背景都 ≥3.0:1。

### OSNAP / 图层快捷图标按状态取用
- **功能简介**：按捕捉类型或图层状态返回对应 SVG 字节。
- **UI 入口**：捕捉菜单、图层管理器/图层下拉。
- **样式**：`osnap(SnapType)`、`mtp_icon()`、`layer_visible(bool)`、`layer_freeze(bool)`、`layer_lock(bool)`。
- **实现位置**：`src/ui/icons.rs::osnap/mtp_icon/layer_visible/layer_freeze/layer_lock`。
- **备注**：3D 捕捉模式暂无专用图标，复用最接近的 2D 形状（如 `Vertex`→node、`FaceCenter`→center）（据源码注释）。

---

## 五、主题系统（Fusion Black / Fusion White）

### Fusion Black 主题
- **功能简介**：近黑界面 chrome 配白色画布，使图形成为屏幕上唯一明亮之物；蓝色主色在两半背景上均可读。
- **UI 入口**：`Options → 主题下拉 → Fusion Black`。
- **样式**：`Theme::custom("Fusion Black", …)`——背景 `#1A1A1A`、文字 `#F2F2F2`、主色 `#0696D7`、成功 `#4CAF50`、警告 `#FFB300`、危险 `#E53935`；模型空间画布 `FUSION_BLACK_CANVAS = [250, 250, 250]`（刻意非纯白）。
- **触发命令**：`Message::OptionsThemeChanged("Fusion Black")`。
- **实现位置**：`src/ui/style/fusion_theme.rs::fusion_black`、`FUSION_BLACK`、`FUSION_BLACK_CANVAS`。

### Fusion White 主题
- **功能简介**：Fusion Black 的反相——浅色 chrome、近黑文字，主色加深以保持浅色表面上的对比。
- **UI 入口**：`Options → 主题下拉 → Fusion White`。
- **样式**：背景 `#FAFAFA`、文字 `#1A1A1A`、主色 `#0277BD`、成功 `#2E7D32`、警告 `#E65100`、危险 `#C62828`；画布 `FUSION_WHITE_CANVAS = [255, 255, 255]`。
- **触发命令**：`Message::OptionsThemeChanged("Fusion White")`。
- **实现位置**：`src/ui/style/fusion_theme.rs::fusion_white`、`FUSION_WHITE`、`FUSION_WHITE_CANVAS`。

### 主题清单与画布色解析
- **功能简介**：主题选择器列出 iced 内置主题 + Fusion 两主题；画布色按主题名解析。
- **UI 入口**：`Options → 主题下拉`（含 “Custom” 项）。
- **样式**：`pick_list` 宽度 `sizing.width`，标签字号 12、宽 150。
- **触发命令**：`Message::OptionsThemeChanged`。
- **实现位置**：`src/app/config.rs::all_themes/builtin_theme/theme_canvas_background`；`src/ui/window/options.rs`（主题区）；`src/ui/style/fusion_theme.rs::fusion_themes/fusion_canvas`。
- **备注**：Fusion 为 `Theme::Custom`，不在 `iced::Theme::ALL` 中，凡枚举主题处必须走 `all_themes()`。Fusion 画布刻意打破“画布跟随 chrome”规则（黑 chrome + 白画布）。主题名逐字持久化于 `settings.json` 的 `theme.name`。

### 模型空间外观联动
- **功能简介**：可选择画布背景/网格/默认线色自动适配主题，或锁定经典深灰，或自定义。
- **UI 入口**：`Options → Model Space Appearance → Canvas mode`。
- **样式**：`pick_list` 宽度 Fill，标签字号 12 宽 140；自定义模式额外显示 Model/Paper/Desk 背景取色与 `#RRGGBB` 输入框，以及 Grid opacity 滑块（5~100）。
- **触发命令**：`Message::ModelSpaceModeChanged`、`RestoreModelSpaceDisplayDefaults` 等。
- **实现位置**：`src/ui/window/options.rs`；`src/app/config.rs::ModelSpaceMode`。
- **备注**：MatchTheme 说明“Canvas background, grid, and default line colors automatically adapt to the active theme.”；ClassicDark 锁定 `#212830`。

---

## 六、无障碍（Accessibility）

### WCAG AA 对比度要求
- **功能简介**：所有主题的 UI 域（核心表面、Ribbon、状态栏、命令行、属性/停靠面板、模态与动作按钮、下拉/勾选、视口开关、积木面板）都需满足 WCAG 2.1 对比度阈值，测试对 22 个内置主题逐一断言。
- **UI 入口**：全局（不可见约束，影响所有文本/控件）。
- **样式**：正文/标签 ≥4.5:1；次要文本（0.72/0.68 alpha 合成后）≥3.0:1；UI 组件与勾号 ≥3.0:1（WCAG 1.4.11）；章节标题 ≥4.0:1；错误指示 ≥2.0:1。
- **实现位置**：`src/ui/theme_accessibility_tests.rs`（`test_theme_core_surfaces_contrast`、`test_ribbon_contrast`、`test_statusbar_contrast`、`test_command_line_contrast`、`test_properties_and_dock_contrast`、`test_modals_and_action_buttons_contrast`、`test_dropdowns_and_selection_overlays`、`test_viewport_controls_toggle_buttons_contrast`、`test_block_palette_contrast`）。
- **备注**：Fusion 两主题另有 `fusion_themes_meet_wcag_aa_on_every_surface`（base/weak/strong/weakest ≥4.5:1）与 `fusion_accent_is_visible_on_the_canvas`（主色对画布 ≥3.0:1）测试，因为它们是手写而非生成调色板。

### 对比度工具函数
- **功能简介**：提供 WCAG 亮度/对比度计算与“不够对比则回退”的取色辅助，供全局样式使用。
- **UI 入口**：无（内部基础函数）。
- **样式**：`wcag_contrast` 返回 1.0–21.0；`accessible_accent` 在 ≥3.0:1 时用 accent 否则回退；`accessible_accent_threshold` 可传自定义阈值（如 4.5）；`canvas_is_light` 判断画布是否浅色。
- **实现位置**：`src/ui/style/common.rs::to_linear/wcag_luminance/wcag_contrast/accessible_accent_threshold/accessible_accent/canvas_is_light`。
- **备注**：`muted_style`/`muted_text_style` 统一为 `background.base.text` 68% 透明（单点定义，主题变则全局变）。`btn_s` 故意未合并（`style_manager` 与 `plotstyle` 行为不同）。

### CJK 文本宽度处理
- **功能简介**：下拉/弹出宽度估算把东亚宽/全角字符按双倍计，避免本地化中文等标签截断。
- **UI 入口**：各类下拉弹出面板宽度计算。
- **样式**：`dropdown_popup_width(labels, font_size, chrome, min)` 按每字符宽度单位（宽字符 2）求和，再乘 `font_size*0.68` 加 chrome，取不小于 `min`。
- **实现位置**：`src/ui/style/common.rs::is_wide/dropdown_popup_width`。
- **备注**：`is_wide` 覆盖 Hangul Jamo、CJK 部首/假名/统一表意文字、谚文音节、CJK 兼容、全角 ASCII/符号、CJK 扩展 B–F（近似，非穷尽）。

---

## 七、全局窗口与界面框架

`src/ui/window/` 下窗口清单（`src/ui/window/mod.rs` 全部 `pub mod`；多数作为 `ModalKind` 在画布内以模态渲染，见 `src/app/view/modal.rs`）。已在他文详述者在此仅列名并交叉引用。

### 窗口模块总览
- **功能简介**：所有独立对话框/窗口的实现模块集中于此，由宿主模态系统按 `ModalKind` 呈现。
- **UI 入口**：各自命令、菜单或按钮触发（见下）。
- **样式**：统一由 `crate::ui::modal::modal` 包裹，标题由 `ModalKind` 映射（`crate::tr!("modal", …)`）。
- **实现位置**：`src/ui/window/mod.rs`。
- **备注**：清单（模块名 → 用途）：

| 模块 | 用途 | 交叉引用 |
| --- | --- | --- |
| `about` | 关于对话框（版本/平台/架构/构建信息、Copy Info） | 见下「关于对话框」 |
| `block_definition` | 块定义 | Drawing/Block 文 |
| `block_palette` | 积木（块）面板 | 积木面板文 |
| `browser` | 内部浏览器 | — |
| `layout_manager` | 布局管理器 | Layout 文 |
| `layer_state_manager` | 图层状态管理器 | 图层文 |
| `drawing_units` | 图形单位 | Draw 文 |
| `geometric_tolerance` | 几何公差 | 标注文 |
| `drafting_settings` | 草图设置 | 状态栏文 |
| `auto_constrain_settings` | 自动约束设置 | 参数化文 |
| `layer_translator` | 图层转换器 | Draw 文 |
| `plot` / `print_all` | 打印 / 批量打印 | 打印文 |
| `plugin_manager` | 插件管理器 | 本文第一节 |
| `shortcuts` | 快捷键 | 快捷键文 |
| `layers` | 图层特性管理器（停靠面板/窗口） | Draw 文 |
| `update_notice` | 新版本提示 | 见下 |
| `open_progress` | 打开进度 | 文件文 |
| `missing_fonts` | 缺失字体提示 | 文件文 |
| `recovery` | 图形恢复 | 文件文 |
| `options` | 选项（主题、语言、光标、文件夹等） | 本文第五节与 Options 文 |
| `attribute_editor` | 属性编辑器 | 块文 |
| `annotation_data` | 标注数据 | 标注文 |
| `alias_editor` | 命令别名编辑器 | 命令行文 |
| `find_replace` | 查找替换 | 编辑文 |
| `named_parameters` | 命名参数 | 参数化文 |
| `pc_manager` | 点云管理器 | 点云文 |
| `pdf_dialogs` | PDF 对话框 | PDF 文 |
| `xref_attach` / `xref_help` / `xref_manager` | 外部参照附加/帮助/管理器 | 外部参照文 |
| `wblock` | 写块 | 块文 |

### UI 模块总览（`src/ui/mod.rs`）
- **功能简介**：UI 层顶层模块，除 `window` 外还有 Ribbon、状态栏、命令行、属性、停靠、模态、弹出、侧边工具栏、包装栏、宽菜单、节点图等。
- **样式**：全局行高基准 `ROW_H = 26.0`（单一来源，缩放 Ribbon/图层管理器/特性面板行高）。
- **实现位置**：`src/ui/mod.rs`（`ROW_H` 与模块列表；重导出 `CommandLine`/`PropertiesPanel`/`Ribbon`/`StatusBar`/`LayerPanel`）。
- **备注**：`theme_accessibility_tests` 以 `#[cfg(test)] mod` 挂在此模块下。

### 关于对话框（About）
- **功能简介**：展示应用标识、完整版本、平台、架构、构建/提交日期/构建配置，并可复制信息。
- **UI 入口**：`命令输入框 → ABOUT`；或开始页相关入口。
- **样式**：背景 `background.base.color`，内边距 16；hero 含 72×72 应用 logo、标题 “Open CAD Studio” 字号 28 主色、副标题字号 11 muted、版本字号 13；信息卡圆角 8、边框 `background.neutral`、背景 `background.weakest`、内边距 `[10, 12]`、高 62（标签字号 10 muted + 值字号 14）；按钮 “Copy Info”。
- **触发命令**：`ABOUT`；`Message::AboutCopyInfo`。
- **实现位置**：`src/ui/window/about.rs::view_window/info_card/platform_name/architecture_name/build_label`；模态分派 `src/app/view/modal.rs`（`automatic_flow(ex, crate::ui::window::about::view_window)`）。
- **备注**：`OCS_FULL_VERSION` 为完整构建标识；`build_label` 去掉前导 `+` 后缀或显示 “Release”。

### 新版本提示对话框（Update Notice）
- **功能简介**：检测到 GitHub 有新版本时提示，展示已装/最新版本对比与发布说明，并可打开发布页或稍后。
- **UI 入口**：应用启动检查新版本后自动弹出（`ModalKind::UpdateNotice`，尺寸 560×460）。
- **样式**：背景 `background.base.color`，内边距底部/左右 20；标题 “New Release Available” 字号 20 主色、副标题字号 11 muted；两张版本卡（Installed 普通、Latest 高亮）圆角 6、内边距 `[14, 12]`、标签字号 10、值字号 20，中间装饰性长箭头（`ARROW_LONG_RIGHT` 20px）；“What's new” 标题字号 11 muted，发布说明轻量 Markdown 渲染在 `bordered_box` 内、可滚动；按钮 “Later”（secondary）与 “Open Release Page”（primary），字号 11、内边距 `[6, 16]`。
- **触发命令**：`Message::UpdateNoticeClose`、`Message::UpdateNoticeOpenRelease`。
- **实现位置**：`src/ui/window/update_notice.rs::view_window/version_card/render_notes_line/strip_inline_md`；模态分派 `src/app/view/modal.rs`。
- **备注**：发布说明识别 `##`/`###` 标题、`-`/`*` 项目符号（用 `icons::DOT` 5px）、`**粗体**` 与行内代码（剥标记按统一色调渲染，因 iced text 无行内样式）；空正文显示 “No release notes provided.”。

### 模态框架与窗口标题
- **功能简介**：所有 `ModalKind` 对话框以画布内覆盖层方式统一呈现，兼顾客桌面单一主窗口与 Web 构建。
- **UI 入口**：任一打开模态的命令/按钮。
- **样式**：`crate::ui::modal::modal(underlay, title, content, CloseModal, offset, options)`；`ModalOptions::STANDARD` 或 Web 插件模态的 `NOTICE`；标题经 `crate::tr!("modal", "plugin-manager")` 等映射；关闭用 ✕（`Message::CloseModal`）。
- **触发命令**：`Message::CloseModal`。
- **实现位置**：`src/app/view/mod.rs`（模态组合，含 PluginManager 的 `NOTICE` 特例）、`src/app/view/modal.rs::modal_content/modal_title`；`src/ui/modal.rs`；`ModalKind` 枚举 `src/app/mod.rs`。
- **备注**：同一时刻至多一个模态；各对话框数据存在自带字段中。Web 构建下 Plugins 模态改用 `NOTICE` 选项并渲染 `view_web_notice`。

### 图标/主题作为「界面基础属性」
- **功能简介**：图标与主题不是独立功能入口，而是所有界面元素共享的基础属性：图标按主题着色、按资源类别取用；主题决定 chrome/画布/语义色/对比度。
- **UI 入口**：全局。
- **样式**：图标经 `icons::themed*`/`icons::semantic` 系列随 `Theme.palette()` 解析；主题经 `Theme`/`all_themes`/`fusion_canvas` 全局生效；`ROW_H` 为几何基准。
- **实现位置**：`src/ui/icons.rs`、`src/ui/style/fusion_theme.rs`、`src/ui/style/common.rs`、`src/ui/mod.rs`。
- **备注**：主题名逐字持久化，改名会使用户选择失效（`FUSION_BLACK`/`FUSION_WHITE` 注释）。

---

（文档结束）
