# Ribbon「Manage」选项卡功能需求清单

本文档反推自 `src/modules/manage/`（`mod.rs`、`user_interface.rs`、`cui_import.rs`、`cui_export.rs`、`find_nonpurgeable.rs`、`purge.rs`、`overkill.rs`、`audit.rs`、`options.rs`、`about.rs`）及其所辖对话框/窗口：`src/ui/window/options.rs`、`src/ui/window/options/spacemouse.rs`、`src/ui/window/plugin_manager.rs`、`src/ui/window/alias_editor.rs`、`src/ui/window/shortcuts.rs`、`src/ui/window/about.rs`、`src/ui/window/update_notice.rs`、`src/ui/window/missing_fonts.rs`、`src/ui/window/recovery.rs`、`src/ui/window/drawing_units.rs`、`src/ui/window/drafting_settings.rs`；辅以 `src/modules/draw/modify/overkill.rs` 与命令分发 `src/app/commands/view.rs`、`src/app/commands/styleprops.rs`、`src/app/commands/mod.rs`。

Ribbon 面板与工具的权威布局入口是 `src/modules/manage/mod.rs::ManageModule::ribbon_groups()`，模块 id 为 `"manage"`、标题为 `"Manage"`。面板组顺序为：Customization、Cleanup、Application。

---

## 一、Customization 面板（Ribbon → Manage 选项卡 → Customization 面板）

面板布局（`src/modules/manage/mod.rs`）：`LargeTool(CUI)`、`LargeTool(TOOLPALETTES)`，随后两个小 `Tool`（`CUIIMPORT`、`CUIEXPORT`），最后是一个 `Dropdown(ALIASEDIT_DROPDOWN)`。

### 用户界面（User Interface）
- **功能简介**：打开自定义用户界面入口；在当前实现中打开键盘快捷键编辑器（键盘绑定即 CUI 数据）。
- **UI 入口**：`Ribbon → Manage 选项卡 → Customization 面板 → 用户界面（LargeTool，标签“User Interface”）`。
- **样式**：`LargeTool`（大按钮，图标 `assets/icons/user_interface.svg`，标签两行 “User / Interface”）。
- **触发命令**：`CUI`。
- **实现位置**：`src/modules/manage/user_interface.rs::tool()`（`id: "CUI"`，`ModuleEvent::Command("CUI")`）；分发 `src/app/commands/view.rs` 的 `"CUI"` 分支 → `Message::ShortcutsPanelOpen`。
- **备注**：源码注释明确 “CUI — keyboard shortcut / key-binding editor”；别名表另有 `ALIASEDIT`。窗口见 `src/ui/window/shortcuts.rs`。

### 工具选项板（Tool Palettes）
- **功能简介**：打开工具选项板（Tool Palettes）功能入口。
- **UI 入口**：`Ribbon → Manage 选项卡 → Customization 面板 → 工具选项板（LargeTool，标签“Tool Palettes”）`。
- **样式**：`LargeTool`（图标 `assets/icons/tool_palettes.svg`，标签两行 “Tool / Palettes”）。
- **触发命令**：`TOOLPALETTES`。
- **实现位置**：`src/modules/manage/mod.rs::ribbon_groups()` 内联 `ToolDef { id: "TOOLPALETTES", .. }`；分发 `src/app/commands/display.rs` 的 `"TOOLPALETTES"` 分支。
- **备注**：当前为占位实现，命令行输出 “TOOLPALETTES: Tool Palettes not yet implemented.”（`src/app/commands/display.rs`）。另见 `src/modules/view/tool_palettes.rs::tool()`。

### 导入 CUI（Import）
- **功能简介**：从“KEY COMMAND”文本文件导入键盘快捷键自定义项。
- **UI 入口**：`Ribbon → Manage 选项卡 → Customization 面板 → 导入（Tool，标签“Import”）`。
- **样式**：`Tool`（1 行小图标，`assets/icons/cui_import.svg`）。
- **触发命令**：`CUIIMPORT`（也接受 `CUILOAD`）。
- **实现位置**：`src/modules/manage/cui_import.rs::tool()`（`id: "CUIIMPORT"`）；分发 `src/app/commands/view.rs` 的 `"CUIIMPORT" | "CUILOAD"` 与 `cmd if cmd.starts_with("CUIIMPORT ") || cmd.starts_with("CUILOAD ")` 分支。
- **文件格式与行为**：纯文本，每行 `KEY COMMAND`；以 `#` 开头或空行跳过；键经 `crate::app::shortcuts::normalize_key` 规范化，命令转大写后写入 `shortcut_bindings`，随即持久化设置。裸命令先以 `ValuePromptCommand` 提示 “CUIIMPORT  shortcuts file to load:”。成功输出 “CUIIMPORT: loaded {n} shortcut(s) from \"{path}\".”，失败输出 “CUIIMPORT: cannot read \"{path}\": {e}”。

### 导出 CUI（Export）
- **功能简介**：将当前键盘快捷键自定义项导出为“KEY COMMAND”文本文件。
- **UI 入口**：`Ribbon → Manage 选项卡 → Customization 面板 → 导出（Tool，标签“Export”）`。
- **样式**：`Tool`（小图标，`assets/icons/cui_export.svg`）。
- **触发命令**：`CUIEXPORT`。
- **实现位置**：`src/modules/manage/cui_export.rs::tool()`（`id: "CUIEXPORT"`）；分发 `src/app/commands/view.rs` 的 `"CUIEXPORT"` 与 `cmd if cmd.starts_with("CUIEXPORT ")` 分支。
- **文件格式与行为**：按字母序列出 `shortcut_bindings`，每行 `KEY COMMAND\n` 写入指定路径。裸命令提示 “CUIEXPORT  file to save shortcuts to:”。成功输出 “CUIEXPORT: wrote {count} shortcut(s) to \"{path}\".”，失败输出 “CUIEXPORT: cannot write \"{path}\": {e}”。

### 编辑别名下拉（Edit Aliases / Load Partial CUI）
- **功能简介**：打开命令别名编辑器，或加载局部 CUI（快捷键）文件。
- **UI 入口**：`Ribbon → Manage 选项卡 → Customization 面板 → 编辑别名（Dropdown，默认 ALIASEDIT，标签“Edit Aliases”）`。
- **样式**：`Dropdown`（1 行小图标 + ▾，图标 `assets/icons/edit_aliases.svg`）。
- **触发命令**：默认 `ALIASEDIT`。下拉子项（`src/modules/manage/mod.rs` 的 `items`，全部）：
  - `ALIASEDIT` — Edit Aliases（编辑别名，图标 `edit_aliases.svg`）
  - `CUILOAD` — Load Partial CUI（加载局部 CUI，图标 `cui_import.svg`）
- **实现位置**：`src/modules/manage/mod.rs::ribbon_groups()`（`RibbonItem::Dropdown { id: "ALIASEDIT_DROPDOWN", .. }`）；`ALIASEDIT` 分发 `src/app/commands/view.rs` → `Message::AliasEditorOpen`；`CUILOAD` 与 `CUIIMPORT` 同分支，进入 ValuePrompt。
- **备注**：`ALIASEDIT` 打开 `ocad.pgp` 别名表编辑器（`src/ui/window/alias_editor.rs`）；`CUILOAD` 与导入 CUI 共用快捷键文件解析路径，仅提示文本不同。

---

## 二、Cleanup 面板（Ribbon → Manage 选项卡 → Cleanup 面板）

面板布局（`src/modules/manage/mod.rs`）：`LargeTool(FINDNONPURGEABLE)`，随后三个小 `Tool`：`PURGE`、`OVERKILL`、`AUDIT`。

### 查找不可清理项（Find Non-Purgeable Items）
- **功能简介**：只读扫描当前图形，列出被对象引用、因而无法 PURGE 的命名对象（图层、线型、文字样式、块）。
- **UI 入口**：`Ribbon → Manage 选项卡 → Cleanup 面板 → 查找不可清理项（LargeTool，标签两行 “Find Non- / Purgeable Items”）`。
- **样式**：`LargeTool`（图标 `assets/icons/find_nonpurgeable.svg`）。
- **触发命令**：`FINDNONPURGEABLE`。
- **实现位置**：`src/modules/manage/find_nonpurgeable.rs::tool()`（`id: "FINDNONPURGEABLE"`）；分发 `src/app/commands/styleprops.rs` 的 `"FINDNONPURGEABLE"` 分支。
- **备注**：输出到命令行：“FINDNONPURGEABLE: named objects in use (not purgeable):”，并分行列出 `Layers:`、`Linetypes:`（排除 ByLayer/ByBlock）、`Text styles:`（Text/MText 样式）、`Blocks:`（Insert 块名），无项显示 `(none)`，集合按字母排序。不修改图形。

### 清理（Purge）
- **功能简介**：删除图形中未使用的命名定义（图层、文字样式、线型、块），反复迭代直到一轮无删除为止。
- **UI 入口**：`Ribbon → Manage 选项卡 → Cleanup 面板 → 清理（Tool，标签“Purge”）`。
- **样式**：`Tool`（小图标，`assets/icons/purge.svg`）。
- **触发命令**：`PURGE`（另接受 `PURGE <sub>`）。
- **实现位置**：`src/modules/manage/purge.rs::tool()`（`id: "PURGE"`）；分发 `src/app/commands/styleprops.rs` 的 `"PURGE"` 与 `cmd if cmd.starts_with("PURGE ")` 分支。
- **备注**：裸命令提示 “PURGE  [All / Blocks / LAyers / LineTypes / STyles]:”，关键字 All/BLocks/LAYers/LINETYPES/STyles。删除块会释放仅被其成员引用的嵌套块/图层/线型/样式，故实现循环到不动点（源码注释）。子选项非法时输出 “PURGE: unknown option …”。删除前压入一次撤销快照（`push_undo_snapshot(i, "PURGE")`）。

### 消除重复（Overkill）
- **功能简介**：删除重叠或重复对象、合并部分重叠与首尾相接对象、优化多段线直线段，可控制忽略的特性与容差。
- **UI 入口**：`Ribbon → Manage 选项卡 → Cleanup 面板 → 消除重复（Tool，标签“Overkill”）`。
- **样式**：`Tool`（小图标，`assets/icons/overkill.svg`）。
- **触发命令**：`OVERKILL`（`-OVERKILL`）。
- **实现位置**：工具 `src/modules/manage/overkill.rs::tool()`（`id: "OVERKILL"`）；命令类 `src/modules/draw/modify/overkill.rs::OverkillCommand`、`Settings`、`normalized()`、`optimize()`；分发 `src/app/commands/styleprops.rs` 的 `"OVERKILL" | "-OVERKILL"` 与 `OVERKILL_APPLY` 分支。
- **备注**：两步——先收集对象（若已有选择则跳过），再进入选项步 `Step::Options`，提示 “Tolerance=… Ignore=… Optimize=… Overlap=… End-to-end=…”。选项关键字：Done(`D`)、Ignore(`I`)、Tolerance(`O`)、Optimize polylines(`P`)、Combine overlap(`T`)、Combine end-to-end(`E`)、Associativity(`A`)。Ignore 子选项 None/All/Color/LAyer/Ltype/ltScale/LWeight/Thickness/TRansparency/plotSTyle/Material（对应位掩码 1/2/4/8/16/32/64/128/256，All=511）。容差为 ≥0 的有限数，默认 `1e-6`。Yes/No 步用 Y/N。设置记忆在进程内（`remembered()`，`Mutex<Settings>`）。完成后触发 `OVERKILL_APPLY`，在所选实体上迭代到不动点，保留关联对象（`reactors` 非空）与锁定图层上的对象；`optimize` 仅对无 bulge/宽度、无闭合约束的 LwPolyline 折叠共线顶点。

### 核查（Audit）
- **功能简介**：只读扫描图形数据库完整性，报告引用未定义图层或未定义块定义的对象；仅报告不自动修复。
- **UI 入口**：`Ribbon → Manage 选项卡 → Cleanup 面板 → 核查（Tool，标签“Audit”）`。
- **样式**：`Tool`（小图标，`assets/icons/audit.svg`）。
- **触发命令**：`AUDIT`。
- **实现位置**：`src/modules/manage/audit.rs::tool()`（`id: "AUDIT"`）；分发 `src/app/commands/styleprops.rs` 的 `"AUDIT"` 分支。
- **备注**：输出 “AUDIT: scanned {total} object(s).”；无问题输出 “AUDIT: no issues found.”，否则以错误行列出未定义图层与未定义块名。因不自动修复，永远不会使图形变差（源码注释）。

---

## 三、Application 面板（Ribbon → Manage 选项卡 → Application 面板）

面板布局（`src/modules/manage/mod.rs`）：`LargeTool(OPTIONS)`、`LargeTool(ABOUT)`。源码注释指出 Options 按钮此前虽已存在但未加入任何组，仅能靠输入 `OPTIONS` 或从起始页进入。

### 选项（Options）
- **功能简介**：打开应用程序首选项对话框，集中配置语言、主题、显示、选择、草图、3D 建模、文件位置与用户首选项。
- **UI 入口**：`Ribbon → Manage 选项卡 → Application 面板 → 选项（LargeTool，标签“Options”）`；或命令 `OPTIONS`；或起始页按钮。
- **样式**：`LargeTool`（图标 `assets/icons/options_tool.svg`）。
- **触发命令**：`OPTIONS`（`OP`）。
- **实现位置**：`src/modules/manage/options.rs::tool()`（`id: "OPTIONS"`）；分发 `src/app/commands/view.rs` 的 `"OPTIONS" | "OP"` 分支 → `Message::OptionsOpen`；对话框 `src/ui/window/options.rs::view_window`。
- **备注**：`start_allowed` 允许该命令从起始页运行（`src/app/commands/mod.rs`）。对话框各页见第四节。

### 关于（About）
- **功能简介**：显示版本、平台、架构、构建信息、提交日期与构建配置，并可复制信息。
- **UI 入口**：`Ribbon → Manage 选项卡 → Application 面板 → 关于（LargeTool，标签“About”）`。
- **样式**：`LargeTool`（图标 `assets/icons/about.svg`）。
- **触发命令**：`ABOUT`。
- **实现位置**：`src/modules/manage/about.rs::tool()`（`id: "ABOUT"`）；分发 `src/app/commands/view.rs` 的 `"ABOUT"` 分支 → `Message::AboutOpen`；窗口 `src/ui/window/about.rs::view_window`。
- **备注**：窗口内容见第五节「About 窗口」。

---

## 四、Options 对话框（Ribbon → Manage → Application → Options / `OPTIONS`）

对话框布局：左侧竖排标签栏（`TAB_RAIL_WIDTH = 178.0`，8 个页面），右侧内容区（可滚动）+ 底部按钮行。模态固有尺寸 `DIALOG_WIDTH = 880.0`、`DIALOG_HEIGHT = 620.0`。对话框实现于 `src/ui/window/options.rs::view_window`；页面枚举 `OptionsTab`（`General/Files/OpenAndSave/Display/Drafting/Modeling/Selection/UserPreferences`，持久化）。

样式：标签按钮 `button(text(label).size(12.5))`，选中用 `button::primary`，未选用 `button::text`；底部 `OK`（primary，`Message::OptionsOk`）、`Apply`（`dirty` 时可用，`button::secondary`，否则禁用为 `button::text`）、`Close`（`button::text`，`Message::OptionsClose`）。关闭若有未提交改动会询问（`Message::OptionsCloseDiscard`/`OptionsCloseKeep`）。逐页功能如下。

### 4.1 General（常规）页
- **功能简介**：设置界面语言、插件/快捷键/别名入口、绘图打印入口与新布局页面设置选项。
- **UI 入口**：`Options 对话框 → 左侧 General 标签`。
- **样式**：分组标题字号 15；行标签字号 12，宽 150；右侧为 `pick_list` 或 `button::secondary` 小按钮（`padding [4,10]`）。
- **触发命令**：`Message::OptionsTabChanged(OptionsTab::General)`；语言 `Message::LanguageChanged`；`Message::PageSetupOnNewLayoutChanged`。
- **实现位置**：`src/ui/window/options.rs::view_window`（`general` 列）。
- **备注**：全部控件：
  - **Language 分组**：语言下拉（`Language::ALL`，显示 `value.label()`）。
  - **Applications 分组**：`Plugins…` 按钮（`Message::PluginManagerOpen`）；`Keyboard Shortcuts…` 按钮（`Message::ShortcutsPanelOpen`）；`Command Aliases…` 按钮（`Message::AliasEditorOpen`）。
  - **Plotting 分组**：`Plot and Page Setup…` 按钮（`Message::PlotDialogOpen`）；复选框 “Show the page setup for new layouts”。

### 4.2 Files（文件）页
- **功能简介**：显示应用自身文件的固定位置，并提供“打开文件夹”按钮。
- **UI 入口**：`Options 对话框 → 左侧 Files 标签`。
- **样式**：说明文字字号 11；每个 `folder_row` 为标签（12）+ 路径（11）+ 右侧 `Open folder` 小按钮；无路径时显示 “Not available” 且按钮禁用。
- **触发命令**：`Message::OpenFolder(path)`。
- **实现位置**：`src/ui/window/options.rs::view_window`（`files` 列、`folder_row` 闭包）。
- **备注**：四行：Configuration、Plot styles、Plugins、Autosave files。源码注释说明本应用没有支持文件搜索路径，故只显示真实位置。

### 4.3 Open and Save（打开和保存）页
- **功能简介**：设置默认保存格式、文件关联、约束标记显示，以及自动保存与备份。
- **UI 入口**：`Options 对话框 → 左侧 Open and Save 标签`。
- **样式**：`Open and Save` 标题 15、`File Safety` 标题 15；标签 12；滑块 + 数值文本；复选框字号 15。
- **触发命令**：`Message::DefaultSaveFormatChanged`、`FileAssocChanged`、`ShowConstraintValuesChanged`、`SaveTimeChanged`、`BackupOnSaveChanged`。
- **实现位置**：`src/ui/window/options.rs::view_window`（`open_and_save` 列）。
- **备注**：全部控件：
  - **Open and Save**：`Default save format` 下拉（`crate::io::SAVE_FORMAT_OPTIONS`）。
  - 复选框 “Open .dwg and .dxf files with Open CAD Studio”（并安装应用与文件类型图标）。
  - 复选框 “Show values and parameter names on constraint markers”（默认开）。
  - **File Safety**：`Automatic save` 滑块 0..=120（SAVETIME，0 显示 “Off”，否则 “{} min”）；复选框 “Keep a .bak copy when overwriting a drawing (ISAVEBAK)”。

### 4.4 Display（显示）页
- **功能简介**：配置主题颜色、模型空间外观、十字光标、线宽显示、命令窗口提示与历史淡出。
- **UI 入口**：`Options 对话框 → 左侧 Display 标签`。
- **样式**：各分组标题 15，行标签 12；色块 `28×22`、圆角 3；色值输入框宽 130/150；`Restore Defaults` 小按钮。
- **触发命令**：`OptionsThemeChanged`、`OptionsThemeColorChanged`、`RestoreModelSpaceDisplayDefaults`、`ModelSpaceModeChanged`、`ModelSpaceBgChanged`、`PaperSpaceBgChanged`、`DeskSpaceBgChanged`、`GridOpacityChanged`、`CursorSizeChanged`、`CursorTypeChanged`、`CrosshairColorChanged`、`LineweightDisplayScaleChanged`、`TextFillChanged`、`ClipromptLinesChanged`、`CommandLineFadeChanged`，以及 `BgPickerOpen/Cancel/Submit`。
- **实现位置**：`src/ui/window/options.rs::view_window`（`display` 列、`bg_swatch_picker`）。
- **备注**：全部区块与控件：
  - **Theme 分组**：主题下拉（`all_themes()` + “Custom”）；6 个主题色：color-background、color-text、color-primary、color-success、color-warning、color-danger（各含色块 + `#RRGGBB` 输入）。
  - **Model Space Appearance 分组**：`Restore Defaults` 按钮；`Canvas mode` 下拉（`ModelSpaceMode::ALL`）；仅 Custom 模式显示 “Model background” 色块 + `#RRGGBB`；恒显 “Paper background”“Desk surround” 色块 + 十六进制输入；`Grid opacity` 滑块 5..=100（%）；随模式变化的说明文字（MatchTheme / ClassicDark / Custom）。
  - **Crosshair 分组**：`Crosshair size` 滑块 1..=100（%）；`Cursor type` 下拉（`CursorType::ALL`）；`Crosshair color` 色块 + `#RRGGBB or blank` 输入（留空自动对比）。
  - **Lineweight 分组**：`Model display scale` 滑块 25..=200（%，不影响打印）；复选框 “Fill TrueType glyphs (TEXTFILL)”。
  - **Command Line 分组**：`Prompt lines` 滑块 0..=50（CLIPROMPTLINES）；`History fade time` 滑块 0..=60000、步 250（COMMANDLINEFADETIME，0 显示 “Off”，否则秒）。

### 4.5 Selection（选择）页
- **功能简介**：设置拾取框、选择模式、视觉效果、预览与夹点设置。
- **UI 入口**：`Options 对话框 → 左侧 Selection 标签`。
- **样式**：分组标题 15、行标签 12/宽 140；滑块 + 数值；颜色为 8 项 ACI 下拉（`0: Theme Default` … `7: White/Black`）；`Restore Defaults` 小按钮。
- **触发命令**：`PickBoxChanged`、`ShiftToAddToggled`、`PickDragRectToggled`、`SelectionCyclingChanged`、`RestoreSelectionVisualDefaults`、`SelectionAreaToggled`、`SelectionOpacityChanged`、`SelectionWindowColorChanged`、`SelectionCrossingColorChanged`、`SelectionEffectToggled`、`SelectionHighlightColorChanged`、`SelectionPreviewIdleToggled`、`SelectionPreviewCommandToggled`、`GripSizeChanged`、`GripObjectLimitChanged`、`GripColorChanged`、`GripHotChanged`、`GripHoverChanged`。
- **实现位置**：`src/ui/window/options.rs::view_window`（`selection` 列）。
- **备注**：全部区块：
  - **Selection**：`Pick box size` 滑块 0..=50（PICKBOX，同时控制选择框与点击孔径）。
  - **Selection Modes**：复选框 “Use Shift to add to selection (PICKADD)”（勾选为 PICKADD 的反）；“Press and drag draws a rectangle instead of a lasso (PICKDRAG)”；“Clicking overlapping objects opens a picker”（选择循环）。
  - **Visual Effect Settings 分组**：`Restore Defaults` 按钮；复选框 “Indicate selection area with transparent fill (SELECTIONAREA)”；`Selection area opacity` 滑块 0..=100（SELECTIONAREAOPACITY）；`Window selection color`、`Crossing selection color` 下拉；复选框 “Show selection effect (SELECTIONEFFECT)”；`Selection highlight color` 下拉。
  - **Preview 分组**：两个复选框，`SELECTIONPREVIEW` 位掩码——空闲悬停预览（bit 1）与命令中选择预览（bit 2）。
  - **Grip Settings 分组**：`Grip size` 滑块 1..=25（px）；`Object limit for grips` 滑块 0..=1000（GRIPOBJLIMIT，0 显示 “Unlimited”）；`Unselected grip color`、`Selected/hot grip color`、`Hover grip color` 下拉。

### 4.6 Drafting（草图）页
- **功能简介**：设置草图旋转与极轴追踪增量，并提供进入草图设置的入口。
- **UI 入口**：`Options 对话框 → 左侧 Drafting 标签`。
- **样式**：分组标题 15；文本输入宽 110；下拉全宽；`Drafting Settings…` 小按钮。
- **触发命令**：`Message::SnapAngleInputChanged`、`PolarIncrementChanged`、`Message::ToggleSnapPopup`。
- **实现位置**：`src/ui/window/options.rs::view_window`（`drafting` 列）。
- **备注**：
  - **Drafting 分组**：`Drafting rotation` 文本输入（默认 “0” 度，SNAPANG，旋转当前 UCS 中的十字光标与捕捉栅格）；`Polar tracking increment` 下拉（90/45/30/22.5/18/15/10/5/1 度，经 `format_snap_angle`）。
  - **Snap and Grid 分组**：`Drafting Settings…` 按钮打开草图设置（`Message::ToggleSnapPopup`，即状态栏捕捉弹窗/`DSETTINGS`）。

> 源码注释说明捕捉模式、栅格间距与对象捕捉保留在 Drafting Settings 对话框；此处仅放此前无归属的旋转角与极轴增量。

### 4.7 3D Modeling（三维建模）页
- **功能简介**：开关导航立方体与 UCS 图标；对当前图形设置显示分辨率与实体历史。
- **UI 入口**：`Options 对话框 → 左侧 3D Modeling 标签`。
- **样式**：`3D Modeling`/`Display Tools`/`Display Resolution`/`Solid History` 标题 15；`applies to the current drawing` 提示字号 11；滑块 + 数值。
- **触发命令**：`ShowViewCubeChanged`、`ShowUcsIconChanged`、`UcsIconAtOriginChanged`、`IsolinesChanged`/`IsolinesReleased`、`DispSilhChanged`、`SurfaceUChanged`、`SurfaceVChanged`、`SurfaceTypeChanged`、`SolidHistChanged`、`ShowHistChanged`。
- **实现位置**：`src/ui/window/options.rs::view_window`（`modeling` 列）。
- **备注**：
  - **Display Tools**：复选框 “Show the navigation cube (NAVVCUBE)”、“Show the UCS icon (UCSICON)”、“Place the UCS icon at the origin (UCSICON ORigin)”。
  - 仅当 `drawing_prefs.available`（有打开图形）时显示：
    - **Display Resolution**（applies to the current drawing）：`Isolines per surface` 滑块 0..=64（ISOLINES，释放时重镶嵌）；复选框 “Show silhouette edges on solids (DISPSILH)”；`Surface density U`/`V` 滑块 0..=200（SURFU/SURFV）；`Surface type` 下拉——Quadratic B-spline (5)、Cubic B-spline (6)、Bezier (8)。
    - **Solid History**：复选框 “Record the history of composite solids (SOLIDHIST)”；`Show solid history` 下拉——Never(0)/As set per solid(1)/Always(2)（SHOWHIST）。

### 4.8 User Preferences（用户首选项）页
- **功能简介**：SpaceMouse 设置、缩放行为、文字与标注、注释、右键定制、块编辑双击行为与绘图单位入口。
- **UI 入口**：`Options 对话框 → 左侧 User Preferences 标签`。
- **样式**：分组标题 15；行标签 12（宽 150）；滑块 + 数值；`Drawing Units…` 小按钮。
- **触发命令**：`SpaceMouse*` 系列（见 4.8.1）、`ZoomWheelReversedChanged`、`ZoomFactorChanged`、`TextEditModeChanged`、`DimContinueModeChanged`、`QdimSnapPriorityChanged`、`AnnoAutoScaleChanged`、`RightClickModeChanged`、`RightClickHoldMsChanged`、`DoubleClickBlockRefeditChanged`、`DoubleClickBlockAtteditChanged`、`OpenDrawingUnits`。
- **实现位置**：`src/ui/window/options.rs::view_window`（`user_prefs` 列）+ `src/ui/window/options/spacemouse.rs::view`。
- **备注**：全部区块：
  - **User Preferences**：内嵌 SpaceMouse 设置块（`spacemouse` 元素，见 4.8.1）。
  - **Zoom**：复选框 “Reverse mouse wheel zoom (ZOOMWHEEL)”；`Zoom factor` 滑块 3..=100（ZOOMFACTOR）。
  - **Text and Dimensions**：复选框 “TEXTEDIT edits one object and ends (TEXTEDITMODE)”；“Continued dimensions inherit the base dimension's layer and style (DIMCONTINUEMODE)”；`QDIM origin priority` 下拉——Endpoints(0)/Intersections(1)。
  - **Annotation**：`Add scales automatically` 下拉——Off(0)/Skip objects on layers that are off, frozen or locked(1)/Skip objects on layers that are off or frozen(2)/Skip objects on locked layers(3)/All annotative objects(4)（ANNOAUTOSCALE，符号为状态栏开关）。
  - **Right-click Customization**：`Right-click in drawing area` 下拉（`RightClickMode::ALL`）；`Hold duration` 滑块 100..=1000、步 50（ms，SHORTCUTMENUDURATION）。
  - **Block Edit**：复选框 “Double-click to edit block in-place (REFEDIT)”；“Double-click attributed blocks to edit attributes (ATTEDIT)”。
  - **Drawing Units**：`Drawing Units…` 按钮（`Message::OpenDrawingUnits`）。

#### 4.8.1 SpaceMouse 设置（内嵌于 User Preferences）
- **功能简介**：启用/禁用 3Dconnexion SpaceMouse，选择导航模式、反向平移与平移速度，显示连接状态与详情，并可打开驱动设置。
- **UI 入口**：`Options 对话框 → User Preferences 标签 → SpaceMouse 分组`。
- **样式**：标题行含主题化图标（22px）+ “SpaceMouse” 15；复选框 15；状态/导航/速度行标签宽 150；导航下拉宽 220；`Connection details` 为文本按钮（带折叠箭头 `themed_arrow_toggle`）。
- **触发命令**：`Message::SpaceMouseEnabled`、`SpaceMouseMode`、`SpaceMousePanReversed`、`SpaceMousePanSpeed`、`SpaceMousePause`、`SpaceMouseDriverSettings`、`SpaceMouseDetails`。
- **实现位置**：`src/ui/window/options/spacemouse.rs::view`（`ICON` = `assets/icons/ui/spacemouse.svg`）。
- **备注**：仅 Windows（`cfg!(windows)`）支持启用与模式选择；非 Windows 时复选框与下拉只读。状态文本：禁用 “Disabled”、暂停 “Paused”，否则为 `status.label()`。非 Full3D 且受支持时显示 “Reverse pan direction” 复选框与 `Pan speed` 滑块 10..=300、步 10（%）。暂停时显示 “Resume SpaceMouse” 按钮。按钮在 Windows 为 “Open 3Dconnexion settings…”，否则 “SpaceMouse information…”。详情文本区分 Unavailable/Unsupported/Disconnected/其它；说明导航模式跨图形与会话保存，对话框打开时导航暂停。

---

## 五、Options 对话框所辖/关联窗口

### 5.1 About 窗口
- **功能简介**：展示应用标识与构建身份（版本、平台、架构、构建、提交日期、构建配置），并可一键复制信息。
- **UI 入口**：`Ribbon → Manage → Application → About`；命令 `ABOUT`；起始页 About。
- **样式**：hero 卡片（图标 72×72 + “Open CAD Studio” 28 + 副标题 11 + 版本 13）；信息卡 62px 高、圆角 8、弱背景；`Copy Info` 为主按钮。
- **触发命令**：`ABOUT`（`Message::AboutOpen`）；`Message::AboutCopyInfo`。
- **实现位置**：`src/ui/window/about.rs::view_window`、`info_card`、`platform_name`、`architecture_name`、`build_label`；复制处理 `src/app/update/mod.rs` 的 `Message::AboutCopyInfo`。
- **备注**：卡片两组各三张——Version/Platform/Arch 与 Build/Commit date/Profile；构建标签取自 `OCS_BUILD_METADATA`（去前导 `+`，干净构建显示 “Release”）。复制内容含版本、修订、提交日期、配置、特性、OS、架构。

### 5.2 插件管理器（Plugin Manager）
- **功能简介**：列出本构建内置/已装插件并启用/禁用，浏览 GitHub 市场清单、搜索、从仓库发布安装/更新/卸载、查看 README。
- **UI 入口**：`Options 对话框 → General → Plugins…`；命令 `PLUGINS`/`PLUGINMANAGER`。
- **样式**：标题 20 + 副标题 12；两栏布局（左目录固定宽 410，右 README 面板）；卡片圆角 6、选中主色 1.5px 边框；标签徽章、彩色状态徽章（Muted/Success/Danger/Warning）；搜索框 `Search plugins…`。
- **触发命令**：`PLUGINS`（`PLUGINMANAGER`）。
- **实现位置**：`src/ui/window/plugin_manager.rs::view_window`、`external_card`、`market_card`、`marketplace_section`、`install_controls`、`readme_panel`、`registry_notice`、`status_badge`、`toggle_button`；分发 `src/app/commands/view.rs` 的 `"PLUGINS" | "PLUGINMANAGER"`。
- **备注**：全部功能点：
  - **Installed 区**：每个外部插件卡片显示名称、`v{version}`、`API {version}`、状态徽章（Loaded/Disabled/API incompatible/Incompatible/Load failed/No library/Restart to load）、id、描述、命令前缀、加载错误详情；有更新时显示 `Update to {tag}` 主按钮；已加载插件显示 Enable/Disable 切换；非内置插件显示 `Uninstall`；内置显示 “Bundled with Open CAD Studio”。
  - **Available plugins 区**：来自精选 registry 与用户添加仓库的卡片，含名称/描述、Release 下拉、Install（可安装时 success，否则 “Incompatible”/“Unavailable” 徽章）、用户仓库可 `✕` 移除；`Add from GitHub` 表单（URL 或 owner/repository + `Add repository`）。
  - **注册表状态**：加载中提示 “Loading plugin catalog…”；错误提示按证书/超时/其它定制文案，带 `Retry`、`Show details`/`Hide details`、`Copy details`。
  - **README 面板**：无选中时提示选择插件；加载中/失败/成功（markdown 渲染，链接经 `resolve_readme_link` 解析）；顶部 `View on GitHub` 按钮。
  - 网页版（wasm32）显示 “Plugins are available in the desktop app” 通知与 `Download desktop app` 按钮（`view_web_notice`）。

### 5.3 命令别名编辑器（Alias Editor / ALIASEDIT）
- **功能简介**：编辑命令行别名表（`ocad.pgp`），支持新增、重映射、删除、恢复默认，带重复/未知命令校验。
- **UI 入口**：`Ribbon → Manage → Customization → 编辑别名下拉 → Edit Aliases`；`Options → General → Command Aliases…`；命令 `ALIASEDIT`。
- **样式**：标题 “Command Aliases” 15 + 提示 11；表头 Alias（宽 120）/Command（余宽）+ 右侧 62px 删除列；无效输入用 danger 边框/底色；草稿行有 ✓/✕，已提交行有垃圾桶；右侧保留 16px `GUTTER` 滚动条车道。
- **触发命令**：`ALIASEDIT`。
- **实现位置**：`src/ui/window/alias_editor.rs::view_window`（`AliasField`、`danger_input_style`）；分发 `src/app/commands/view.rs` 的 `"ALIASEDIT"` → `Message::AliasEditorOpen`。
- **备注**：功能点：
  - `+ Add alias` 新增草稿行（`AliasEditorAdd`），可 `接受`（`AliasEditorDraftAccept`）或 `Cancel add (Esc)`/`AliasEditorDraftCancel`；Esc 取消待定行。
  - 每行 Alias/Command 文本输入（`AliasEditorInput`）；重复别名与未知命令即时红色标记。
  - 删除行 `AliasEditorRemove`。
  - `Reset to default`（`AliasEditorResetAsk`）→ 就地确认 “Are you sure you want to reset?…” + `Yes, reset`（`AliasEditorResetConfirm`）/`No`（`AliasEditorResetDeny`）。
  - `Apply`（`AliasEditorApply`，提交并保持打开）/`Apply && Exit`（`AliasEditorApplyExit`）。
  - 顶部显示 “Number of aliases: {n}”；冲突横幅列出 “Alias already used for command: {alias} → {command}” 与 “Unknown command: {command}”。
  - 关闭未提交时叠加遮罩确认：“Unsaved changes will be discarded.” + `Discard && close`（`AliasEditorCloseDiscard`）/`Keep editing`（`AliasEditorCloseKeep`）。

### 5.4 键盘快捷键编辑器（Shortcuts / CUI）
- **功能简介**：编辑键盘快捷键表，通过按键捕获新增绑定，带重复键/未知命令校验与恢复默认。
- **UI 入口**：`Ribbon → Manage → Customization → User Interface`；`Options → General → Keyboard Shortcuts…`；命令 `CUI`/`SHORTCUTS`。
- **样式**：标题 “Keyboard Shortcuts” 15 + 提示 11；表头 Key（宽 180）/Command + 30px 删除列；捕获中的键单元为 `button::primary`，重复键整格 `button::danger`，普通为 `button::secondary`；未知命令输入框 danger 样式。
- **触发命令**：`CUI`；`SHORTCUTS`（含 `SHORTCUTS SET <key> <command>`、`CLEAR/DELETE/REMOVE <key>`、`LIST`）。
- **实现位置**：`src/ui/window/shortcuts.rs::view_window`（`ShortcutField`、`danger_input_style`）；分发 `src/app/commands/view.rs` 的 `"CUI"` 与 `SHORTCUTS` 分支。
- **备注**：功能点：
  - `+ Add`（`ShortcutEditorAdd`）新增草稿行；点击 Key 单元进入捕获（`ShortcutCaptureStart`），显示 “Press a key combination...”，再次点击取消（`ShortcutCaptureClear`）；未捕获显示 “Click, then press keys...”。
  - Command 文本输入（`ShortcutEditorInput`）；草稿行 ✓（`ShortcutEditorDraftAccept`，仅合法时可点）/✕（`ShortcutCaptureCancel`）；已提交行垃圾桶（`ShortcutEditorRemove`）。
  - `Reset to default`（`ShortcutEditorResetAsk`）就地确认＋ `Yes, reset`（`ShortcutEditorResetConfirm`）/`No`（`ShortcutEditorResetDeny`）。
  - `Apply`（`ShortcutEditorApply`）/`Apply && Exit`（`ShortcutEditorApplyExit`）。
  - 显示 “Number of shortcuts: {n}”；冲突横幅 “Shortcut already used for command: {key} → {command}” 与 “Unknown command: {command}”。
  - 关闭未提交时遮罩确认（`ShortcutEditorCloseDiscard`/`Keep editing`）。
  - 手动键输入可通过命令行 `SHORTCUTS SET`。

### 5.5 缺失字体提示（Missing Fonts）
- **功能简介**：图形打开后若缺少 `.shx` 字体，列出缺失字体并允许从仓库或自定义源下载，或跳过。
- **UI 入口**：`打开图形后自动弹出的模态窗口`（缺少 SHX 字体时）。
- **样式**：标题 “Missing fonts” 20 + 说明 11；字体列表为带边框可滚动容器（字号 12）；源输入框字号 11；`Skip`（secondary）与 `Download available`/`Downloading...`（primary）按钮。
- **触发命令**：`Message::MissingFontsDownload`、`MissingFontsDismiss`、`MissingFontsSourceChanged`。
- **实现位置**：`src/ui/window/missing_fonts.rs::view_window`；下载处理 `src/app/update/mod.rs` 的 `Message::MissingFontsDownload`。
- **备注**：默认留空使用社区仓库，也可指向公司字体服务器/GitHub raw 文件夹；不可再分发的字体会继续使用替代字体（说明文字）。

### 5.6 图形修复/恢复窗口（Recovery）
- **功能简介**：图形打开失败或带修复打开时，展示恢复报告（扫描/问题/移除/引用统计与诊断）并提供保存副本、查看日志、关闭。
- **UI 入口**：`打开图形触发修复时弹出的模态窗口`；提示窗口在打开失败时询问是否尝试恢复。
- **样式**：标题 20（成功为 warning 色，失败为 danger 色）；指标卡为弱背景、圆角 5、数值 18/标签 10；详情为带边框可滚动区；动作按钮 `[6,14]` 内边距。
- **触发命令**：`Message::RecoverySaveAs`、`RecoveryShowLog`、`RecoveryClose`；提示窗口 `RecoveryDecline`、`RecoveryAttempt`。
- **实现位置**：`src/ui/window/recovery.rs::view_window`、`view_prompt`、`metric`、`status_style`。
- **备注**：`view_window` 显示四指标——entities-checked、issues-found、entities-removed、references-checked；条件行含 referenced-entities-removed、references-unavailable、错误、最多 100 条诊断 `[kind] message`、日志路径/写失败/网页下载就绪。仅 `recovered && save_as_required && allow_save_copy` 时显示 `save-copy` 按钮；有日志或 wasm 时显示 `show-log`；恒有 `close`。`view_prompt` 显示文件名、错误文本与 `decline`/`attempt` 按钮。

### 5.7 更新提示窗口（Update Notice）
- **功能简介**：发现 GitHub 上有更新版本时提示，并展示发行说明。
- **UI 入口**：`检测到新版本时自动弹出的模态窗口`。
- **样式**：标题 “New Release Available” 20 + 副标题 11（居中）；两张版本卡（Installed 普通、Latest 主色高亮）中间装饰箭头；发行说明为轻量 Markdown；`Later`（secondary）与 `Open Release Page`（primary）。
- **触发命令**：`Message::UpdateNoticeClose`、`UpdateNoticeOpenRelease`。
- **实现位置**：`src/ui/window/update_notice.rs::view_window`、`version_card`、`render_notes_line`、`strip_inline_md`。
- **备注**：发行说明支持 `## ` 标题（主色 13）、`### ` 标题（12）、`- `/`* ` 项目符号（DOT 图标 + 11）、内联 `**`/`` ` `` 标记被剥离；空正文显示 “No release notes provided.”；长正文可滚动。

### 5.8 绘图单位对话框（Drawing Units）
- **功能简介**：设置长度/角度格式与精度、角度基线与方向、插入比例单元，并实时预览样例输出；仅写入设置不改动几何。
- **UI 入口**：`Options → User Preferences → Drawing Units…`；命令 `DWGUNITS`（另有状态栏单位弹窗）。
- **样式**：分组容器带浅边框、圆角 4、内边距 8；行标签字号 11/宽 78；`pick_list` 字号 12；`OK`（primary）`Cancel`（`Message::CloseModal`）。
- **触发命令**：`Message::DrawingUnitsApply`、`DrawingUnitsField(...)`。
- **实现位置**：`src/ui/window/drawing_units.rs::view_window`、`State`、`Choice`、`Field`、`precision_row`、`drop_row`、`sample_length`、`sample_angle`。
- **备注**：全部区块：
  - **Length**：`Type` 下拉（`units::linear_formats()`）；`Precision` 下拉 0..=8（架构/分数格式显示分数分母样例）。
  - **Angle**：`Type` 下拉（`units::angular_formats()`）；`Precision` 0..=8；`Zero at` 度输入（ANGBASE）；复选框 “Positive angles run clockwise”（ANGDIR）。
  - **Insertion scale**：`Unit` 下拉（`units::all()`，INSUNITS）；说明 “Scales blocks and drawings inserted from elsewhere. Unitless inserts them unscaled.”。
  - **Sample output**：以当前待定设置经同一格式化器显示长度（1.5 与 33.5）与角度（π/4）样例。
  - 工作副本在 OK 前不写入图形（`State` 注释）。

### 5.9 草图设置对话框（Drafting Settings）
- **功能简介**：集中配置捕捉与栅格、极轴追踪、对象捕捉、三维对象捕捉、动态输入、快速特性和选择循环。
- **UI 入口**：`Options → Drafting → Drafting Settings…`（`Message::ToggleSnapPopup`，即状态栏捕捉弹窗）；命令 `DSETTINGS`；状态栏捕捉按钮右键。
- **样式**：真实标签条（7 个不换行、溢出裁剪的标签按钮），活动标签用 `primary.strong` 背景，未活动用弱背景，上方圆角、下方直角，下方 1px 分隔线；分组容器浅边框圆角 4；底部 `OK`（primary）/`Apply`（dirty 时 secondary）/`Close`。
- **触发命令**：`Message::DraftingSettingsTabChanged`、`DraftingSettingsOk`、`DraftingSettingsApply`、`DraftingSettingsClose`，以及各开关消息（见下）。
- **实现位置**：`src/ui/window/drafting_settings.rs::view_window`、`DraftingSettingsState`、`DraftingSettingsTab`、`group`、`toggle`、`parse_snap_spacing`、`parse_grid_major`。
- **备注**：关闭有未提交改动时 `discard_guard`（`DraftingSettingsCloseDiscard`/`CloseKeep`）。各标签内容：
  - **Snap and Grid**：左列 “Snap On (F9)”（`DraftingSettingsToggleSnap`）；Snap spacing 组（Snap X/Y 间距输入 + “Equal X and Y spacing”）；Snap type 组（“Enable isometric drafting” + IsoPlane 按钮 L/T/R + 提示 “F5 cycles Left, Top, and Right.” + “Rotation: {angle}°” 与 `Reset rotation`）。右列 “Grid On (F7)”（`ToggleGrid`）；Grid spacing 组（Grid X/Y 间距、`Major line every:` 2..=100）；Grid behavior 组（“Adaptive grid”、“Display grid beyond Limits”）。
  - **Polar Tracking**：“Polar Tracking On (F10)”、“Ortho mode (F8)”；Polar Angle Settings 组显示增量角与说明（`TogglePolar`/`ToggleOrtho`）。
  - **Object Snap**：“Object Snap On (F3)”、“Object Snap Tracking On (F11)”（`ToggleOsnap`/`ToggleOtrack`）；`Select All`/`Clear All` 按钮（`DraftingSettingsSnapSelectAll`/`SnapClearAll`）；Object Snap modes 组按 `ALL_SNAP_MODES` 两列复选框（`ToggleSnapMode`）。
  - **3D Object Snap**：“3D Object Snap On (F4)”（`Toggle3dOsnap`）+ 3D Object Snap modes 组（`ALL_3D_SNAP_MODES`，`ToggleSnapMode3d`）。
  - **Dynamic Input**：“Enable Pointer Input (F12)”（`ToggleDynInput`）；Pointer Input 组（坐标输入近十字光标、启用尺寸输入字段，均为恒选复选框）；Dynamic Prompts 组（在十字光标附近显示命令提示与输入）。
  - **Quick Properties**：“Display Quick Properties palette on selection”（`ToggleQuickProps`）；Palette Location 组（Cursor-dependent position 选中 / Static quadrant location 未选）。
  - **Selection Cycling**：“Allow selection cycling”（`ToggleSelCycling`）；Display Selection Cycling List Box 组（重叠时显示循环徽章、点击时显示候选列表框，均为恒选复选框）。

---

## 六、覆盖范围说明

- **Customization 面板**：User Interface(CUI)、Tool Palettes、Import/Export CUI、Edit Aliases 下拉（ALIASEDIT/CUILOAD）均已覆盖。
- **Cleanup 面板**：Find Non-Purgeable Items、Purge、Overkill、Audit 均已覆盖。
- **Application 面板**：Options、About 已覆盖，Options 对话框 8 页逐项列出（含 SpaceMouse）。
- **所辖/关联窗口**：About、Plugin Manager、Alias Editor、Shortcuts、Missing Fonts、Recovery、Update Notice、Drawing Units、Drafting Settings 均已覆盖。
- 上述命令均经 `start_allowed` 门控（`src/app/commands/mod.rs`），可从起始页运行 `ABOUT`、`CUI`、`ALIASEDIT`、`CUILOAD`、`CUIIMPORT`、`OPTIONS`/`OP` 等。

（文档结束）
