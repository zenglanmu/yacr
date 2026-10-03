# 文件与文档相关 UI 功能需求清单

本文档反推自 `src/ui/wrap_bar.rs`、`src/app/view/mod.rs`、`src/app/view/modal.rs`、`src/app/document.rs`、`src/app/history.rs`、`src/app/recent.rs`、`src/app/startup.rs`、`src/app/update/file.rs`、`src/app/update/dialog.rs`、`src/app/update/mod.rs`、`src/app/commands/fileops.rs`、`src/app/commands/display.rs`、`src/ui/ribbon/mod.rs`、`src/ui/ribbon/widgets.rs`、`src/ui/statusbar/mod.rs`、`src/ui/window/recovery.rs`、`src/ui/window/update_notice.rs`、`src/ui/window/open_progress.rs`、`src/ui/window/missing_fonts.rs`、`src/io/mod.rs`、`src/io/recovery.rs`、`src/app/shortcuts.rs`、`src/app/alias.rs`、`src/modules/view/file_tabs.rs`、`src/modules/view/layout_tabs.rs`、`src/modules/insert/landxml.rs` 等。文档标签栏、文件命令、对话框、最近文件、启动页、恢复、更新提示、撤销/重做均在此覆盖。

---

## 一、文档标签栏（Document Tab Bar）

文档标签栏位于 Ribbon 下方（`show_file_tabs` 为真时），每个文档一个标签，可换行、拖拽重排、右键弹出菜单。右侧有新建标签的 `+` 按钮。权威布局入口 `src/app/view/mod.rs::doc_tab_bar()`。

### 文档标签（Document Tab）
- **功能简介**：每个打开的图纸是一个标签，显示文件名（未保存显示 `DrawingN`），点击切换到该文档。
- **UI 入口**：`主窗口 → Ribbon 下方 → 文档标签栏 → 标签（点击）`。
- **样式**：标签容器高固定 28px，标题字号 12px，内边距 `[5, 14]`；活动标签背景 `primary.weak.color`、边框 `primary.base.color` 宽 1；悬停标签背景 `background.weak.color`；非活动标签文字为 `background.base.text` 42%~72% 透明。活动标签文字色 `primary.weak.text`。标题过长用 `text_util::elide(name, 24)`。
- **触发命令**：点击发 `Message::TabSwitch(idx)`。
- **实现位置**：`src/app/view/mod.rs::doc_tab_bar()`；标签文本 `src/app/document.rs::DocumentTab::tab_display_name()`（有 `current_path` 取文件名，`is_start` 取 `Start`，否则 `tab_title`）。
- **备注**：`is_start` 的 Start 标签固定在 index 0，无关闭按钮，不可拖拽，不可关闭。

### 未保存标记（Dirty Dot）
- **功能简介**：文档有未保存修改时，标签标题前显示一个警告色小圆点。
- **UI 入口**：`文档标签栏 → 标签标题左侧圆点`。
- **样式**：`icons::themed_warning(DIRTY_DOT, 14.0)` 图标 + 标题行 `row`，间距 5，垂直居中。
- **触发命令**：无（由 `DocumentTab::dirty` 驱动）。
- **实现位置**：`src/app/view/mod.rs::doc_tab_bar()`（`if tab.dirty` 分支）。

### 关闭按钮（Close Button）
- **功能简介**：关闭该文档标签；若有未保存修改会先弹出保存确认。
- **UI 入口**：`文档标签栏 → 标签右侧 “×” 按钮`。
- **样式**：`text("×")` 字号 12，内边距 `[5, 9]`，悬停/按下背景 `warning.weak.color`、文字色 `warning.weak.text`；非悬停文字为 72% 透明。
- **触发命令**：点击发 `Message::TabClose(tab.id)`（用稳定 id 而非索引）。
- **实现位置**：`src/app/view/mod.rs::doc_tab_bar()`；处理 `src/app/update/mod.rs::update` 的 `Message::TabClose` → `src/app/update/command.rs::on_tab_close()`。
- **备注**：Start 标签无关闭按钮（`row![title_btn]`）。

### 新建标签按钮（+ New Tab）
- **功能简介**：新建一个空白图纸标签。
- **UI 入口**：`文档标签栏 → 标签列表末尾 “+” 按钮`。
- **样式**：`text("+")` 字号 14，高 28，内边距 `[4, 8]`，边框圆角 3，底色 `background.base.color`，悬停 `background.weak.color`，左内边距 2。
- **触发命令**：点击发 `Message::TabNew`；命令行 `NEW`（快捷键 `CTRL+N`）。
- **实现位置**：`src/app/view/mod.rs::doc_tab_bar()`；处理 `src/app/update/mod.rs::update` 的 `Message::TabNew`（新建 `DocumentTab::new_drawing`，迁移 Ortho/OSNAP，应用显示默认值）。

### 标签悬停高亮（Tab Hover）
- **功能简介**：鼠标移入标签时高亮该标签，移出恢复。
- **UI 入口**：`文档标签栏 → 标签（鼠标悬停）`。
- **样式**：悬停背景 `background.weak.color`，文字 `background.weak.text`。
- **触发命令**：`Message::DocTabHover(Some(idx))` / `Message::DocTabHover(None)`。
- **实现位置**：`src/app/view/mod.rs::doc_tab_bar()` 的 `mouse_area(...).on_enter/.on_exit`；处理 `src/app/update/mod.rs` 的 `Message::DocTabHover`。

### 标签完整路径提示（Path Tooltip）
- **功能简介**：已在磁盘保存的文档，鼠标悬停标签时在下方显示其完整文件路径。
- **UI 入口**：`文档标签栏 → 标签（悬停，有路径时）`。
- **样式**：`tooltip` 位于 `Position::Bottom`，内容 `bordered_box`、字号 11、内边距 `[4, 8]`，间距 4。
- **触发命令**：无（悬停）。
- **实现位置**：`src/app/view/mod.rs::doc_tab_bar()`（`if let Some(path) = &tab.current_path` 分支）。

### 标签拖拽重排（Drag-to-Reorder Tabs）
- **功能简介**：用鼠标拖动文档标签改变其顺序；拖到目标标签左/右半侧决定插入位置。
- **UI 入口**：`文档标签栏 → 拖动标签标题（非关闭按钮区域）`。
- **样式**：拖动中光标 `Grabbing`，悬停 `Grab`；落点指示为主题主色竖线（宽 2、高 `bounds.height-4`）；起拖阈值 4px（`START_DISTANCE_SQUARED = 16.0`）。
- **触发命令**：释放后发 `Message::TabReorder { from, to, after }`。
- **实现位置**：`src/ui/wrap_bar.rs::ReorderTab`（`document()`、`update()`、`draw()`、`drop_target()`）；处理 `src/app/update/mod.rs::update` 的 `Message::TabReorder`。
- **备注**：Start 标签不可作为拖拽源或目标；`drop_target` 通过 `PosReport` 记录的 `DOC_TAB:{to}` 边界判定；关闭按钮在 `ReorderTab` 外层之外，避免误触。

### 标签栏换行与滚动（Tab Bar Wrapping）
- **功能简介**：标签过多时自动换行，使用 `Row::wrap` + 垂直间距 2；标签行整体隐藏由 `FILETAB` 控制。
- **UI 入口**：`文档标签栏`（窗口变窄/标签变多时）。
- **样式**：`container` 底色 `background.base.color`、边框 `background.neutral.color` 宽 1，宽 `Fill`，内边距 `[2, 2]`；`Row.wrap().vertical_spacing(2.0)`。
- **触发命令**：`FILETAB` 切换显示/隐藏。
- **实现位置**：`src/app/view/mod.rs::doc_tab_bar()`；显隐判断 `src/app/view/mod.rs::view_main()`（`if self.show_file_tabs`）；命令 `src/app/commands/display.rs::dispatch_display` 的 `"FILETAB"`；开关状态 `src/app/mod.rs` 字段 `show_file_tabs`、`src/ui/ribbon/mod.rs::set_file_tabs`。

### 文档标签右键菜单（Document Tab Context Menu）
- **功能简介**：右键文档标签弹出操作菜单；菜单项因是否已有文件路径、是否存在其它图纸而启用/禁用。
- **UI 入口**：`文档标签栏 → 右键标签 → 上下文菜单`。
- **样式**：菜单宽 `MENU_W = 210.0`，`bordered_box`、内边距 `[4, 0]`；每行 `container(text(label).size(12)).padding([4, 12])`，禁用项文字 42% 透明；行用 `mouse_area` + `Pointer`。菜单由 `iced_aw::ContextMenu` 负责展开/定位/边界钳制/关闭。
- **触发命令**：见下属各菜单项。
- **实现位置**：`src/app/view/mod.rs::doc_tab_context_menu()`；挂载 `src/app/view/mod.rs::doc_tab_bar()`（`ContextMenu::new(tab_target, ...))`。
- **备注**：菜单全部项（源码顺序）：
  - `Save All` → `Message::DocTabSaveAll`
  - `Close All` → `Message::DocTabCloseAll`
  - `Close All Other Drawings` → `Message::DocTabCloseOthers(tab_idx)`（仅当存在其它图纸）
  - `Copy Full File Path` → `Message::DocTabCopyFullPath(tab_idx)`（仅桌面；仅有路径时启用）
  - `Open File Location` → `Message::DocTabOpenFileLocation(tab_idx)`（仅桌面；仅有路径时启用）

#### Save All（保存全部）
- **功能简介**：保存所有已具文件路径的图纸。
- **UI 入口**：`文档标签栏 → 右键标签 → Save All`。
- **样式**：菜单行（见上）。
- **触发命令**：`SAVEALL`。
- **实现位置**：`src/app/commands/fileops.rs::dispatch_fileops` 的 `"SAVEALL"`；菜单消息处理 `src/app/update/mod.rs::update` 的 `Message::DocTabSaveAll`（`dispatch_command("SAVEALL")`）。
- **备注**：只读会话报错 “Read-only session (--read-only): saving is disabled.”；无路径的标签跳过并提示需 `SAVEAS`；已有保存任务在跑的标签跳过；web 版提示逐个保存。

#### Close All（关闭全部）
- **功能简介**：关闭所有非 Start 标签，逐个走未保存确认。
- **UI 入口**：`文档标签栏 → 右键标签 → Close All`。
- **样式**：菜单行。
- **触发命令**：无独立命令（消息驱动）。
- **实现位置**：处理 `Message::DocTabCloseAll` → `src/app/update/command.rs::begin_tab_close_queue()`。

#### Close All Other Drawings（关闭其它图纸）
- **功能简介**：保留当前标签，关闭其余所有非 Start 标签。
- **UI 入口**：`文档标签栏 → 右键标签 → Close All Other Drawings`（有其它图纸时可用）。
- **样式**：菜单行（无其它图纸时禁用，42% 透明）。
- **触发命令**：无独立命令。
- **实现位置**：处理 `Message::DocTabCloseOthers(idx)` → 先 `TabSwitch` 再 `begin_tab_close_queue()`。

#### Copy Full File Path（复制完整路径）
- **功能简介**：把当前文档的绝对路径复制到剪贴板，并在命令行回显。
- **UI 入口**：`文档标签栏 → 右键标签 → Copy Full File Path`（桌面专有，需已保存）。
- **样式**：菜单行；未保存时禁用。
- **触发命令**：无独立命令。
- **实现位置**：处理 `Message::DocTabCopyFullPath(idx)`（`path.canonicalize()` 后 `iced::clipboard::write`）。
- **备注**：未保存报错 “Save the drawing before copying its file path.”；web 报 “Full file paths are unavailable in the web application.”。

#### Open File Location（打开文件位置）
- **功能简介**：在系统文件管理器中定位并高亮该文档。
- **UI 入口**：`文档标签栏 → 右键标签 → Open File Location`（桌面专有，需已保存）。
- **样式**：菜单行；未保存时禁用。
- **触发命令**：无独立命令。
- **实现位置**：处理 `Message::DocTabOpenFileLocation(idx)` → `crate::sys::reveal_in_file_manager`。

---

## 二、文件命令与快速访问工具栏

### 快速访问工具栏（Quick Access Toolbar）
- **功能简介**：Ribbon 顶部左侧的一排文件命令图标，含新建、打开、保存、另存为、打印。
- **UI 入口**：`Ribbon 顶部 → 快速访问条（最左）`。
- **样式**：`quick_access_btn`（SVG 图标 16px、按钮宽 `TOP_HIST_W`、高 24、关闭色主题化、`button::subtle`），带位置 `Bottom`、延迟 400ms 的 tooltip。
- **触发命令/按钮**（源码顺序）：
  - New（`DOC_NEW` 图标）→ `NEW`
  - Open（`FOLDER_OPEN`）→ `OPEN`
  - Save（`SAVE`）→ `SAVE`
  - Save As（`FILE_EXPORT`）→ `SAVEAS`
  - Print（`PRINT`）→ `PRINT`
- **实现位置**：`src/ui/ribbon/mod.rs::Ribbon::view()`（`quick_access_btn` 调用）；`src/ui/ribbon/widgets.rs::quick_access_btn()`。
- **备注**：每个按钮发 `Message::Command(cmd)`，走通用命令分发。

### New（新建）
- **功能简介**：新建空白图纸标签。
- **UI 入口**：`Ribbon → 快速访问 → New`；`命令行 → NEW`；`CTRL+N`；`启动页 → New Drawing`；`文档标签栏 → “+”`。
- **样式**：快速访问小图标按钮（见上）；启动页为描边按钮（`outline_btn`）。
- **触发命令**：`NEW`；快捷键 `CTRL+N`。
- **实现位置**：`src/app/commands/fileops.rs::dispatch_fileops` 的 `"NEW"` → `Message::TabNew`；快捷键表 `src/app/shortcuts.rs`。
- **备注**：在 Start 页按 New 会新建并可执行（命令类在 Start 页允许）。

### Open（打开）
- **功能简介**：打开 CAD 文件（DWG/DXF/BAK/SV$ 等），桌面弹出原生文件选择器，web 走浏览器选择。
- **UI 入口**：`Ribbon → 快速访问 → Open`；`命令行 → OPEN`；`CTRL+O`；`启动页 → Open File`。
- **样式**：快速访问小图标按钮；启动页 `outline_btn`。
- **触发命令**：`OPEN`；快捷键 `CTRL+O`。
- **实现位置**：`src/app/commands/fileops.rs` 的 `"OPEN"` → `Message::OpenFile`；处理 `src/app/update/file.rs::on_open_file()`；文件选择器 `src/io/mod.rs::pick_open_path()`（标题 “Open CAD file”）。
- **备注**：桌面路径经 `Message::OpenPathPicked` 转入后台加载（见「打开进度」）；web 走 `pick_and_load_web`。文件过滤器见「打开对话框」条目。

### Save / QSAVE（保存）
- **功能简介**：保存当前图纸；已有路径直接写回，无路径则进入另存流程。
- **UI 入口**：`Ribbon → 快速访问 → Save`；`命令行 → SAVE`/`QSAVE`；`CTRL+S`。
- **样式**：快速访问小图标按钮。
- **触发命令**：`SAVE`、`QSAVE`；快捷键 `CTRL+S`。
- **实现位置**：`src/app/commands/fileops.rs` 的 `"SAVE" | "QSAVE"` → `Message::SaveFile`；处理 `src/app/update/file.rs::on_save_file()`。
- **备注**：只读会话直接报错禁止保存；`recovery_save_as_required` 为真的图纸强制转 Save As（打开时有修复，见「修复/恢复」）；后台保存任务重复时提示 “Save already running for this drawing.”。

### Save As / SAVEAS（另存为）
- **功能简介**：选择格式与版本，另存为新文件。
- **UI 入口**：`Ribbon → 快速访问 → Save As`；`命令行 → SAVEAS`；`CTRL+SHIFT+S`；`文档标签右键 → （经 Save All/流程）`。
- **样式**：快速访问小图标按钮；随后打开 SaveDialog 模态框（见「另存为对话框」）。
- **触发命令**：`SAVEAS`；快捷键 `CTRL+SHIFT+S`。
- **实现位置**：`src/app/commands/fileops.rs` 的 `"SAVEAS"` → `Message::SaveAs`；处理 `src/app/update/mod.rs` 的 `Message::SaveAs` → `src/app/update/dialog.rs::open_save_dialog_window()`。
- **备注**：只读会话禁止。

### Print（打印）
- **功能简介**：打开打印对话框（打印或导出 PDF）。
- **UI 入口**：`Ribbon → 快速访问 → Print`；`命令行 → PRINT`/`PLOT`；`CTRL+P`。
- **样式**：快速访问小图标按钮。
- **触发命令**：`PRINT`、`PLOT`；快捷键 `CTRL+P`。
- **实现位置**：`src/app/commands/display.rs` 的 `"PLOT" | "PRINT"` → `Message::PlotDialogOpen`。

### 命令行文件命令总表（Command-line File Commands）
以下命令通过命令输入框（自动补全来自 `inventory::submit!` 注册与动态别名）或工具栏触发；实现集中在 `src/app/commands/fileops.rs::dispatch_fileops`：

| 命令 | 行为 | 实现 |
|---|---|---|
| `NEW` | 新建标签 | `Message::TabNew` |
| `OPEN` | 打开文件 | `Message::OpenFile` |
| `SAVE` / `QSAVE` | 保存当前图纸 | `Message::SaveFile` |
| `SAVEALL` | 保存所有有路径的图纸 | 同步保存循环 |
| `SAVEAS` | 另存为 | `Message::SaveAs` |
| `CLOSE` | 关闭当前标签（走未保存确认） | `Message::TabClose(tab.id)` |
| `EXIT` / `QUIT` | 退出应用（走未保存确认） | `Message::WindowCloseRequested` 或 `exit_app()` |
| `UNDO` / `UNDO <n>` | 撤销 1 步 / n 步 | `Message::Undo` / `Message::UndoMany(n)` |
| `REDO` | 重做 | `Message::Redo` |
| `OOPS` | 恢复最近一次 ERASE 删除的对象 | `restore_erased_entities` |
| `CLEAR` / `CLR` | 清空当前场景 | `Message::ClearScene` |
| `ARCHIVE` / `ETRANSMIT` | 打包图纸与引用文件到 `<name>_archive` 目录 | 直接文件复制 |

- **备注**：命令别名表 `assets/ocad.pgp`（默认 `U → UNDO`）；用户可在 ALIASEDIT 里增删。插件脚本禁止调用 `QUIT`/`EXIT`/`CLOSE`/`NEW`/`QNEW`/`OPEN`/`SAVE`/`QSAVE`/`SAVEAS`/`SAVEALL`/`RECOVER`/`SCRIPT` 等（`src/app/plugin_host.rs::script_command_allowed`）。

### 文件命令快捷键（File Shortcuts）
- **功能简介**：默认键盘快捷键映射到文件命令，可在快捷键编辑器（SHORTCUTS）中改。
- **UI 入口**：`快捷键编辑器` 或直接按键。
- **样式**：无独立控件。
- **触发命令**：`CTRL+N`→NEW、`CTRL+O`→OPEN、`CTRL+P`→PLOT、`CTRL+Q`→QUIT、`CTRL+S`→SAVE、`CTRL+SHIFT+S`→SAVEAS、`CTRL+Z`→UNDO、`CTRL+SHIFT+Z`/`CTRL+Y`→REDO。
- **实现位置**：`src/app/shortcuts.rs::default_bindings()`；快捷键消息映射 `src/app/shortcuts.rs`（`"UNDO" => ...`、`"REDO" => Message::Redo`）。

### FILETAB 开关（显示/隐藏文档标签栏）
- **功能简介**：切换顶部文档标签栏的显示。
- **UI 入口**：`Ribbon → View 选项卡 → File Tabs 按钮`；`命令行 → FILETAB`。
- **样式**：Ribbon 大/小按钮（`src/modules/view/file_tabs.rs` 提供图标 `file_tabs.svg`，`ModuleEvent::Command("FILETAB")`）。
- **触发命令**：`FILETAB`。
- **实现位置**：`src/modules/view/file_tabs.rs::tool()`；命令 `src/app/commands/display.rs::dispatch_display` 的 `"FILETAB"`；处理 `src/app/update/mod.rs`（`show_file_tabs ^= true`，`ribbon.set_file_tabs`）。

---

## 三、打开 / 保存对话框

### 打开对话框（Open File Dialog）
- **功能简介**：桌面原生文件选择器，用于选择要打开的 CAD 文件；web 版由浏览器提供文件选择。
- **UI 入口**：`Open` 命令/按钮 → 原生选择器。
- **样式**：系统原生对话框；标题本地化 “Open CAD file”。
- **触发命令**：`OPEN`。
- **实现位置**：`src/io/mod.rs::pick_open_path()`（桌面）。
- **备注**：过滤器全部项：
  - CAD Files：`dwg, dxf, bak, sv$, DWG, DXF, BAK`
  - DWG Files：`dwg, DWG`
  - DXF Files：`dxf, DXF`
  - Backup / Autosave：`bak, sv$, BAK`
  - All Files：`*`
  - 选择后同时返回文件大小（用于进度显示 “47.3 MB”）。

### 拖放打开（Drag & Drop Open）
- **功能简介**：把文件从系统拖入窗口即打开，接受与 Open 对话框一致的格式。
- **UI 入口**：`主窗口 → 拖入文件`。
- **样式**：无专门控件；不支持类型在命令行报错。
- **触发命令**：`Message::FileDropped(path)`。
- **实现位置**：处理 `src/app/update/mod.rs` 的 `Message::FileDropped`。
- **备注**：仅接受 `dwg|dxf|bak|sv$`；若已有打开/修复占位则排队 `pending_opens`；已在某标签打开的文件则切到该标签；否则走 `OpenRecent`。

### 另存为对话框（Save Drawing As / SaveDialog）
- **功能简介**：选择保存格式（DWG/DXF 与版本）和文件名，然后进入原生保存对话框（桌面）或浏览器下载（web）。
- **UI 入口**：`Ribbon → Save As / 命令行 SAVEAS → SaveDialog 模态框`。
- **样式**：模态框 `ModalKind::SaveDialog`，标题 “Save Drawing As”；表单标题字号 14；字段标签字号 11（`dialog_muted_text_style`）；格式用 `pick_list`；按钮 `Save as...`（primary）、`Cancel`（secondary）；内边距 `[14, 16]`。
- **触发命令**：`SAVEAS`。
- **实现位置**：`src/app/view/modal.rs::save_as_dialog_window()`；打开逻辑 `src/app/update/dialog.rs::open_save_dialog_window()`；确认 `src/app/update/file.rs::on_save_dialog_confirm()`。
- **备注**：桌面版隐藏文件名输入（由 OS 对话框收集）；web 版显示 “File name:” 输入框（占位 `drawing.dwg`）与 “Format:” 下拉。格式下拉来自 `SAVE_FORMAT_OPTIONS`。

### 保存版本目标下拉（Save Format / Version）
- **功能简介**：选择另存的目标类型与版本（R14 至 2018）。
- **UI 入口**：`SaveDialog 模态框 → Format 下拉（pick_list）`。
- **样式**：`pick_list`，宽度随模态框（`Fill` 或 `Shrink`）。
- **触发命令**：选择发 `Message::SaveDialogFormatChanged(fmt)`。
- **实现位置**：选项常量 `src/io/mod.rs::SAVE_FORMAT_OPTIONS`；解析 `src/io/mod.rs::parse_save_format()`、`format_for_version()`。
- **备注**：全部选项：`DWG 2018`、`DWG 2013`、`DWG 2010`、`DWG 2007`、`DWG 2004`、`DWG 2000`、`DWG R14`、`DXF 2018`、`DXF 2013`、`DXF 2010`、`DXF 2007`、`DXF 2004`、`DXF 2000`、`DXF R14`。默认 `DWG 2018`（`DEFAULT_SAVE_FORMAT`）。改版本会自动把文件名扩展名同步为新格式（`Message::SaveDialogFormatChanged`）。另存为新/未保存图纸使用 Options 中的 `default_save_format`；已打开文件默认沿用其原始类型与版本。

### 文件名输入（File Name，web 专有）
- **功能简介**：web 版无原生保存对话框，在模态框内直接输入文件名。
- **UI 入口**：`SaveDialog 模态框 → File name 输入框`（仅 web）。
- **样式**：`text_input`，占位 `drawing.dwg`，字号 13，内边距 `[5, 8]`。
- **触发命令**：输入发 `Message::SaveDialogFilenameChanged`。
- **实现位置**：`src/app/view/modal.rs::save_as_dialog_window()`（`#[cfg(target_arch = "wasm32")]` 分支）。

### 未保存更改确认对话框（Unsaved Changes）
- **功能简介**：关闭脏图纸或退出时询问保存/丢弃/取消。
- **UI 入口**：`关闭标签 / Close All / QUIT / 窗口关闭` 触发 → `ModalKind::Unsaved`。
- **样式**：`automatic_flow`；主体文件名字号 14，提示字号 12（muted）；按钮 `Save`（primary）、`Discard`（danger）、`Cancel`（secondary），水平排列间距 8；内边距 `[24, 28]`，居中。
- **触发命令**：无独立命令。
- **实现位置**：`src/app/view/modal.rs::unsaved_changes_dialog_window()`；消息 `Message::UnsavedDialogSave/Discard/Cancel`；处理 `src/app/update/dialog.rs::on_unsaved_dialog_save/discard()`。
- **备注**：`PendingClose` 区分 `Tab(id)` 与 `Quit`；Save 会在需要时进入 Save As；Discard 删除该标签的 `.sv$` 自动保存副本。

### 文件被占用对话框（File In Use，桌面专有）
- **功能简介**：保存失败（文件被其它程序占用）时提示重试、另存或取消。
- **UI 入口**：保存失败 → `ModalKind::FileInUse`。
- **样式**：标题字号 14；说明、路径、详情字号 11~13；按钮 `Retry`（primary）、`Save As`、`Cancel`（均 secondary）；内边距 `[18, 20]`。
- **触发命令**：无独立命令。
- **实现位置**：`src/app/view/modal.rs::file_in_use_dialog_window()`；处理 `Message::SaveFileInUseRetry/SaveAs/Cancel`（`src/app/update/file.rs::on_save_file_in_use_retry/save_as`）。

### 外部修改冲突对话框（External Change，桌面专有）
- **功能简介**：磁盘文件被其它应用改动后再保存时，提示重载/另存/覆盖/取消。
- **UI 入口**：检测到外部改动 → `ModalKind::ExternalChange`。
- **样式**：标题字号 14；说明/路径字号 11~13；按钮 `Reload from Disk`（primary）、`Save As`、`Overwrite`（danger）、`Cancel`；内边距 `[18, 20]`。
- **触发命令**：无独立命令。
- **实现位置**：`src/app/view/modal.rs::external_change_dialog_window()`；处理 `Message::ExternalChangeReload/SaveAs/Overwrite/Cancel`。

### AEC 对象丢失警告（AEC Drop Warning）
- **功能简介**：另存到不兼容版本/DXF 会丢弃 AEC/Civil 等原样透传对象时，提示改用源版本或坚持保存。
- **UI 入口**：`SaveDialog 确认` 检测到将丢失对象 → `ModalKind::AecDropWarning`。
- **样式**：`automatic_flow`；说明字号 13（含丢失数量、目标格式、源版本）；按钮 `Save in source version`（primary）、`Save anyway`（warning）、`Back`（secondary）；内边距 `[24, 28]`，居中。
- **触发命令**：无独立命令。
- **实现位置**：`src/app/view/modal.rs::aec_drop_dialog_window()`；计数 `src/io/mod.rs::dropped_on_save_count()`；处理 `Message::AecDropSameVersion/Proceed/Back`（`src/app/update/file.rs::on_aec_drop_same_version/proceed`）。

### 覆盖确认（Overwrite，由 OS 对话框提供）
- **功能简介**：桌面另存时若目标文件已存在，由系统保存对话框提供覆盖确认。
- **UI 入口**：`SaveDialog 确认 → 原生保存对话框`。
- **样式**：系统原生。
- **触发命令**：`Message::SaveDialogPathPicked`。
- **实现位置**：`src/app/update/file.rs::on_save_dialog_confirm()`（`dlg.save_file()`）、`on_save_dialog_path_picked()`。
- **备注**：web 版无此步骤（直接下载）；恢复图纸另存默认文件名加 `_recovered` 后缀。

### 只读模式保存限制（Read-only Session）
- **功能简介**：以 `--read-only` 启动时，所有保存操作被禁止并在命令行报错。
- **UI 入口**：`SAVE`/`QSAVE`/`SAVEAS`/`SAVEALL`/关闭前保存。
- **样式**：命令行错误行。
- **触发命令**：`SAVE`、`SAVEAS`、`SAVEALL`。
- **实现位置**：字段 `src/app/mod.rs` 的 `read_only: bool`（由 CLI `--read-only` 设置）；检查 `src/app/update/file.rs::on_save_file()`、`src/app/commands/fileops.rs` 的 `"SAVEALL"`、`src/app/update/mod.rs` 的 `Message::SaveAs`。
- **备注**：只读模式还抑制缺失字体下载提示（`on_file_opened` 中 `!self.read_only`）。

---

## 四、导出 / 导入与文件格式

### 打开/保存支持的核心格式
- **功能简介**：DWG 与 DXF（多版本）为原生读写格式；BAK/SV$ 作为备份/自动保存文件也可打开。
- **UI 入口**：`Open` / `SaveDialog`。
- **样式**：见对话框条目。
- **触发命令**：`OPEN`、`SAVE`、`SAVEAS`。
- **实现位置**：`src/io/mod.rs`（加载 `load_file_with_progress`、读取嗅探 `sniff_dwg_or_dxf`、保存格式常量）；版本枚举 `codec::DxfVersion`（AC1014= R14 至 AC1032= 2018）。
- **备注**：DXF 版本映射见 `parse_save_format()`；`.bak` 通过文件头 `AC10` 判断 DWG/DXF。

### STL 导出（STLOUT）
- **功能简介**：把图纸的三维网格导出为 STL 文件。
- **UI 入口**：`命令行 → STLOUT`（别名 `EXPORTSTL`）。
- **样式**：原生保存对话框；过滤器 “STL Files” `stl` + “All Files”；默认文件名 `export.stl`，标题 “Export STL”。
- **触发命令**：`STLOUT`、`EXPORTSTL`。
- **实现位置**：`src/app/commands/display.rs` 的 `"STLOUT" | "EXPORTSTL"` → `Message::StlExport`；处理 `src/app/update/mod.rs` 的 `Message::StlExport/StlExportPath/StlExportFinished`。
- **备注**：无三维网格时报 “STLOUT: no 3D mesh data in this drawing.”；成功回显 “STLOUT: exported to ...”。

### STEP 导出（STEPOUT）
- **功能简介**：把三维网格导出为 STEP AP203 文件。
- **UI 入口**：`命令行 → STEPOUT`（别名 `EXPORTSTEP`、`STPOUT`）。
- **样式**：原生保存对话框；过滤器 “STEP Files” `step, stp` + “All Files”；默认 `export.step`，标题 “Export STEP AP203”。
- **触发命令**：`STEPOUT`、`EXPORTSTEP`、`STPOUT`。
- **实现位置**：`src/app/commands/display.rs` 的 `"STEPOUT" | ...` → `Message::StepExport`；处理 `src/app/update/mod.rs` 的 `Message::StepExport/StepExportPath/StepExportFinished`。

### OBJ 导入（IMPORTOBJ）
- **功能简介**：导入 Wavefront OBJ 网格并作为三维实体网格放入当前图纸。
- **UI 入口**：`命令行 → IMPORTOBJ`（别名 `OBJIMPORT`）。
- **样式**：原生打开对话框；过滤器 “Wavefront OBJ” `obj, OBJ` + “All Files”；标题 “Import OBJ Mesh”。
- **触发命令**：`IMPORTOBJ`、`OBJIMPORT`。
- **实现位置**：`src/app/commands/display.rs` 的 `"IMPORTOBJ" | "OBJIMPORT"` → `Message::ObjImport`；处理 `src/app/update/mod.rs` 的 `Message::ObjImport/ObjImportPath/ObjImportFinished`。
- **备注**：成功后回显 `IMPORTOBJ: imported "<stem>" as mesh.`；目标标签已关则提示；导入走一次 `push_undo_snapshot(i, "IMPORTOBJ")`。

### LandXML 导入（LANDXMLIMPORT）
- **功能简介**：从 LandXML 文件导入 `<CgPoint>` 测量点。
- **UI 入口**：`Ribbon → Insert 选项卡 → LandXML Import 工具`；`命令行 → LANDXMLIMPORT`。
- **样式**：Ribbon 工具（`src/modules/insert/landxml.rs`，图标 `landxml.svg`）；命令行用 `ValuePromptCommand` 提示 “LANDXMLIMPORT  path to the .xml file:”。
- **触发命令**：`LANDXMLIMPORT`（可带路径 `LANDXMLIMPORT <path>`）。
- **实现位置**：`src/modules/insert/landxml.rs::tool()`；命令 `src/app/commands/display.rs` 的 `"LANDXMLIMPORT"` 与 `cmd.starts_with("LANDXMLIMPORT ")`。
- **备注**：无 `<CgPoint>` 时报 “no <CgPoint> survey points found.”；成功按导入点数回显并建议 `ZOOM EXTENTS`。

### 数据提取导出 CSV（DATAEXTRACTION）
- **功能简介**：把数据提取结果保存为 CSV。
- **UI 入口**：`数据提取（DATAEXTRACTION）结果 → 保存`。
- **样式**：原生保存对话框；过滤器 “CSV” `csv` + “All Files”；默认 `extraction.csv`。
- **触发命令**：`DATAEXTRACTION`。
- **实现位置**：处理 `src/app/update/mod.rs` 的 `Message::DataExtractionSaveResult`；过滤器常量见 `src/app/annotation_data.rs`（`add_filter(t!("CSV"), &["csv"])`）。
- **备注**：成功回显 “DATAEXTRACTION  {rows} rows → "<name>"”。

### PDF 导出 / 打印（PLOT / PRINT / EXPORTPDF）
- **功能简介**：通过打印对话框导出 PDF（含 CTB/STB 打印样式）。
- **UI 入口**：`Ribbon → 快速访问 → Print`；`PLOT`/`PRINT`；`EXPORT`/`EXPORTPDF` 直出 PDF。
- **样式**：打印对话框（`src/ui/window/plot.rs`）；PDF 保存对话框过滤器 “PDF Files” `pdf`。
- **触发命令**：`PLOT`、`PRINT`、`EXPORT`、`EXPORTPDF`。
- **实现位置**：`src/app/commands/display.rs` 的 `"PLOT" | "PRINT"` → `Message::PlotDialogOpen`；PDF 导出 `src/io/pdf_export.rs`（web 报 “PDF export is not available in the web version.”）。
- **备注**：CTB 打印样式文件加载 `src/io/plot_style.rs`（过滤器 “Plot Style Tables”/“CTB Files” `ctb`，见 `src/io/mod.rs`）。

### 图纸关联文件类型（File Association）
- **功能简介**：把 OpenCADStudio 注册为 DWG 等文件的默认打开程序。
- **UI 入口**：`启动提示 → 关联确认对话框`（AssocPrompt）。
- **样式**：启动模态框（`ModalKind::AssocPrompt`）。
- **触发命令**：无独立命令。
- **实现位置**：`src/app/startup.rs::queue_startup_prompts()`；处理 `Message::AssocPromptYes/No`；注册逻辑 `src/io/file_association.rs::register_progid`（“OpenCADStudio.DWG”）。
- **备注**：设置项 `file_assoc_enabled` 控制；`default_assoc_prompted` 记忆是否已询问。

---

## 五、最近文件与启动页（Start tab）

启动页是固定于 index 0 的 Start 标签，没有图纸时替换视口区域显示欢迎页；其左栏为最近文件面板。视图入口 `src/app/view/mod.rs::start_page_view()`。

### 启动页概览（Start Page / Welcome）
- **功能简介**：启动页显示主标题、操作按钮、赞助商及四个可收缩侧栏（最近文件、教程视频、讨论、支持者）。
- **UI 入口**：`文档标签栏 → Start 标签`（默认启动即为此页）。
- **样式**：整页底色 `background.base.color`，内边距 `{top:16,right:8,bottom:16,left:8}`；主标题 `Open CAD Studio` 字号 40、主色。窄屏时按序折叠侧栏（先 Tutorials，再 Discussions、Supporters，最后 Recent Documents），剩余为 Compact 模式（顶部出现分段标签栏）。
- **触发命令**：无独立命令。
- **实现位置**：`src/app/view/mod.rs::start_page_view()` / `start_page_content()`；`StartLayout` 枚举与 `panel_visible` 折叠逻辑。
- **备注**：英文文案固定（源码注释：公共欢迎页跨语言一致）。

### 新建图纸按钮（New Drawing）
- **功能简介**：从启动页新建空白图纸。
- **UI 入口**：`Start 页 → New Drawing（描边按钮）`。
- **样式**：`outline_btn`，内边距 `[10, 22]`，字号 14，圆角 `START_ACTION_RADIUS = 6.0`；悬停背景 `background.strong`，默认 `background.weak`。
- **触发命令**：`Message::TabNew`（等同 `NEW`）。
- **实现位置**：`src/app/view/mod.rs::start_page_content()`。

### 打开文件按钮（Open File）
- **功能简介**：从启动页打开已有图纸。
- **UI 入口**：`Start 页 → Open File（描边按钮）`。
- **样式**：`outline_btn`。
- **触发命令**：`Message::OpenFile`（等同 `OPEN`）。
- **实现位置**：`src/app/view/mod.rs::start_page_content()`。

### 捐赠按钮（Donate）
- **功能简介**：主行动号召按钮，触发 DONATE 命令/打开捐赠页。
- **UI 入口**：`Start 页 → Donate（danger 按钮，带心形图标）`。
- **样式**：`button::danger`，圆角 6，内边距 `[10, 22]`，心形图标 `HEART` 14px + 文字。
- **触发命令**：`Message::RibbonToolClick { tool_id: "DONATE", event: Command("DONATE") }`。
- **实现位置**：`src/app/view/mod.rs::start_page_content()`。

### 次级操作按钮组（Feedback / Options / Plugins / OCS Web）
- **功能简介**：启动页第二排按钮。
- **UI 入口**：`Start 页 → 第二排按钮`。
- **样式**：`WrapFlow` 间距 12、行高 44；前三个为 `outline_btn`，web 版链接按钮为 `button::primary`。
- **按钮清单**：
  - `Send Feedback` → `RibbonToolClick { tool_id:"REPORT", Command("REPORT") }`
  - `Options`（`action.options`）→ `Message::OptionsOpen`
  - `Plugins`（`action.plugins`）→ `Message::PluginManagerOpen`
  - 桌面：“OCS Web” → `Command("WEBVERSION")`（primary）
  - web：“OCS Desktop” → `Message::OpenUrl(.../releases/latest)`（primary）
- **实现位置**：`src/app/view/mod.rs::start_page_content()`。

### Reddit 链接（r/OpenCADStudio）
- **功能简介**：打开 OpenCADStudio 的 Reddit 社区页面。
- **UI 入口**：`Start 页 → r/OpenCADStudio（带 reddit 图标）`。
- **样式**：描边按钮，边框橙色 `rgb(255,69,0)`，图标 20px，内边距 `[10, 22]`。
- **触发命令**：`Message::OpenUrl("https://www.reddit.com/r/OpenCADStudio/")`。
- **实现位置**：`src/app/view/mod.rs::start_page_content()`。

### 赞助商卡片（Sponsors）
- **功能简介**：显示赞助商 logo，点击打开其网站。
- **UI 入口**：`Start 页 → 中部 Sponsors`。
- **样式**：标题字号 15，logo SVG 高 120、`ContentFit::Contain`，最小宽 300，`Pointer` 光标。
- **触发命令**：`Message::OpenUrl("https://open-aec.com/")`。
- **实现位置**：`src/app/view/mod.rs::start_page_content()`。

### 最近文件面板（Recent Documents）
- **功能简介**：列出最近打开的图纸，点击重新打开，右侧 ✕ 可移除；底部可设置保留数量。
- **UI 入口**：`Start 页 → Recent Documents 栏（左栏，宽 280）`。
- **样式**：面板底色 `background.weak.color`，边框 `background.neutral.color` 宽 1、圆角 8，内边距 20；标题 `Recent Documents` 字号 15；每行左侧 46×34 DWG 缩略图、文件名（`elide 28`）字号 12、目录（`elide 38`）字号 10 muted；移除按钮为 `CLOSE` 图标 11px。
- **触发命令**：点击行 → `Message::OpenRecent(path)`；点 ✕ → `Message::RecentRemove(path)`。
- **实现位置**：`src/app/view/mod.rs::recent_files_panel()`；列表管理 `src/app/recent.rs`；缩略图 `src/io/thumbnail.rs`。
- **备注**：空列表显示 “no-recent-files” 提示；web 版目录行显示 “browser-storage”（浏览器不暴露源目录）；行的 `open_btn` 宽 `Fill`，悬停背景 `background.strong`。

### 最近文件数量设置（Keep recent files）
- **功能简介**：设置保留多少个最近文件，含减/输入框/加。
- **UI 入口**：`Start 页 → Recent Documents 栏底部 → “Keep recent files” 行`。
- **样式**：标签字号 11 muted（宽 Fill）；`[-]` 与 `[+]` 小按钮（边框圆角 4，悬停 `background.strong`）；中间 `text_input` 宽 46、字号 13、内边距 `[2,6]`；末尾 `/ {RECENT_MAX}` 字号 11 muted；步进 `STEP = 5`。
- **触发命令**：输入发 `Message::RecentLimitInput`；Enter 发 `Message::SetRecentLimit(shown)`；加减发 `SetRecentLimit(shown ± 5)`。
- **实现位置**：`src/app/view/mod.rs::recent_files_panel()`；处理 `src/app/update/mod.rs` 的 `Message::SetRecentLimit`/`RecentLimitInput`；clamp 常量 `src/app/recent.rs`（`RECENT_MIN=5`、`RECENT_MAX=100`、`RECENT_DEFAULT=20`）。
- **备注**：只保留数字字符；`set_recent_limit` 会 clamp 到 [5,100] 并裁剪超限条目、驱逐 web 缓存副本。

### 最近文件缩略图后台加载（Recent Thumbnails）
- **功能简介**：为最近文件异步解码 DWG 预览缩略图，避免启动卡顿。
- **UI 入口**：`最近文件面板`（自动）。
- **样式**：固定 46×34 缩略图框，无图时用空 `Space` 保持行对齐。
- **触发命令**：内部 `Message::RecentThumbsLoaded`。
- **实现位置**：`src/app/recent.rs::refresh_recent_thumbs()`（桌面后台线程 `io::thumbnail::read_handle`；web 用 `web_recent::read_thumbnail`）；`src/io/thumbnail.rs`。

### 教程视频栏（Tutorials）
- **功能简介**：显示官方播放列表中的视频缩略图与标题，点击在浏览器中打开。
- **UI 入口**：`Start 页 → Tutorials 栏（右起第二栏，宽 280）`。
- **样式**：面板底色 `background.weak`、圆角 8、内边距 16；标题字号 15；每张卡片含 16:9 缩略图（圆角 6、`Contain`）与标题字号 12 muted；可滚动；底部 `Open Playlist` 按钮（danger 色，圆角 6）。
- **触发命令**：点卡片 → `Message::OpenUrl(videos::watch_url(&v.id))`；点播放列表 → `Message::OpenUrl(videos::PLAYLIST_URL)`。
- **实现位置**：`src/app/view/mod.rs::start_page_content()`（videos_panel）；数据 `crate::videos`。
- **备注**：列表为空时显示 loading/online 提示。

### 讨论栏（Discussions）
- **功能简介**：显示 GitHub Discussions 列表，置顶加 `pinned` 标签；点击打开对应讨论。
- **UI 入口**：`Start 页 → Discussions 栏`。
- **样式**：面板底色 `background.weak`、圆角 8、内边距 16；卡片 `background.base` 42% 透明、边框圆角 6，含标题字号 12、元信息（`#number`、`pinned`、`@author`）字号 10；可滚动；底部 `Open Discussions` 按钮（primary 色）。
- **触发命令**：点卡片 → `Message::OpenUrl(discussion.url)`；点按钮 → `Message::OpenUrl(DISCUSSIONS_URL)`。
- **实现位置**：`src/app/view/mod.rs::start_page_content()`（discussions_panel）；数据 `crate::discussions`。

### 支持者栏（Supporters）
- **功能简介**：列出 Patreon 赞助者及其金额，底部提供赞助按钮。
- **UI 入口**：`Start 页 → Supporters 栏（最右，宽 280）`。
- **样式**：面板底色 `background.weak`、圆角 8、内边距 20；标题字号 15；每行 `名称`（宽 Fill）+ `金额`（`$x.xx`）字号 12 muted；底部 `Support on Patreon` 按钮（danger 色，心形图标 13px，圆角 6）。
- **触发命令**：点按钮 → `Message::OpenUrl("https://patreon.com/HakanSeven12")`。
- **实现位置**：`src/app/view/mod.rs::start_page_content()`（supporters）；数据 `crate::patreon`。
- **备注**：Patreon 金额规范化为 USD 分。

### 启动页分段标签栏（Compact Section Tabs）
- **功能简介**：窄窗口（Compact 布局）时，把各栏收成分段标签，一次显示一栏。
- **UI 入口**：`Start 页（窄屏）→ 顶部标签栏`。
- **样式**：`tab_btn` 字号 14，内边距 `[8, 18]`，圆角 6；活动项背景 `primary.weak`、文字 `primary.weak.text`、边框 `primary.base`；悬停 `background.strong`。
- **分段**：`Recent Files`、`Videos`、`Welcome`、`Discussions`、`Supporters`。
- **触发命令**：`Message::StartSectionSelect(section)`。
- **实现位置**：`src/app/view/mod.rs::start_page_content()`（`StartLayout::Compact` 分支）；枚举 `src/app/mod.rs::StartSection`；持久化 `src/app/update/mod.rs` 的 `Message::StartSectionSelect`（写入 `StartConfig.section`）。

---

## 六、撤销 / 重做（Undo / Redo）

### Undo 按钮（快速访问区）
- **功能简介**：撤销一步；有可撤销步数时按钮可用，标注可撤销步数。
- **UI 入口**：`Ribbon 顶部 → 快速访问条 → Undo（撤销图标）`。
- **样式**：`render_history_control`，主按钮宽 `TOP_HIST_W` 高 24；可撤销时有主题色，否则禁用灰；tooltip 位置 `Right`、延迟 400ms，内容 “Undo\n{n} steps available”。
- **触发命令**：点击发 `Message::Undo`；快捷键 `CTRL+Z`；命令行 `UNDO`。
- **实现位置**：`src/ui/ribbon/widgets.rs::render_history_control()`；`src/ui/ribbon/mod.rs::Ribbon::view()`；处理 `src/app/update/mod.rs` 的 `Message::Undo` → `src/app/history.rs::undo_active_tab()`。
- **备注**：命令进行中时 Ctrl+Z 优先让命令自身回退一步（`CadCommand::on_undo_step`），否则才撤销文档。

### Redo 按钮（快速访问区）
- **功能简介**：重做一步。
- **UI 入口**：`Ribbon 顶部 → 快速访问条 → Redo（重做图标）`。
- **样式**：同 Undo；tooltip “Redo\n{n} steps available”。
- **触发命令**：点击发 `Message::Redo`；快捷键 `CTRL+SHIFT+Z`、`CTRL+Y`；命令行 `REDO`。
- **实现位置**：`src/ui/ribbon/widgets.rs::render_history_control()`；处理 `src/app/update/mod.rs` 的 `Message::Redo` → `src/app/history.rs::redo_active_tab()`。

### 撤销/重做历史下拉（Undo/Redo History Dropdown）
- **功能简介**：点击按钮右侧 ▾ 展开历史列表，可直接跳到某一步（一次撤销/重做 n 步）。
- **UI 入口**：`Ribbon → 快速访问 → Undo/Redo 右侧 ▾ 按钮`。
- **样式**：下拉面板宽 170，`popup_panel_style`；每行 `text(label).size(11)`、内边距 `[5,10]`、宽 Fill；布局锚点由 `dd_anchor` 计算并 `position_ribbon_dropdown` 定位，外层 `dropdown_backdrop`。
- **触发命令**：展开发 `Message::ToggleRibbonDropdown(UNDO_HISTORY_ID|REDO_HISTORY_ID)`；点第 n 行发 `Message::UndoMany(n)` / `Message::RedoMany(n)`。
- **实现位置**：`src/ui/ribbon/mod.rs::Ribbon::dropdown_overlay()`（`UNDO_HISTORY_ID`/`REDO_HISTORY_ID` 分支）；标签构造 `src/app/history.rs::history_dropdown_labels()`（栈逆序取 `label`）；ID 常量 `src/ui/ribbon/widgets.rs::UNDO_HISTORY_ID/REDO_HISTORY_ID`。
- **备注**：下拉只在有标签时渲染；标签为每次编辑操作的 `label`（命令名，如 `LINE`、`MOVE`、`OOPS`）。

### UNDO 命令（含步数）
- **功能简介**：命令行撤销一步或一次多步。
- **UI 入口**：`命令行 → UNDO` 或 `UNDO <n>`。
- **样式**：命令行输出/错误行。
- **触发命令**：`UNDO`；别名 `U`（`assets/ocad.pgp`）。`UNDO <n>` 参数超出可用步数时按可用步数执行；非法参数报 “Usage: UNDO [number of steps]”；`UNDO 0` 无操作。
- **实现位置**：`src/app/commands/fileops.rs` 的 `"UNDO"` 与 `cmd.starts_with("UNDO ")` → `Message::Undo` / `Message::UndoMany(n)`；`src/app/history.rs::undo_steps()`。
- **备注**：无内容时报 “Nothing to undo.”；成功回显 `Undo: {label}`。

### REDO 命令
- **功能简介**：命令行重做。
- **UI 入口**：`命令行 → REDO`。
- **样式**：命令行输出行。
- **触发命令**：`REDO`。
- **实现位置**：`src/app/commands/fileops.rs` 的 `"REDO"` → `Message::Redo`；`src/app/history.rs::redo_steps()`。
- **备注**：无内容时报 “Nothing to redo.”；成功回显 `Redo: {label}`。

### OOPS 命令（恢复删除对象）
- **功能简介**：恢复最近一次 ERASE 删除的对象，且不撤销其后的其它工作。
- **UI 入口**：`命令行 → OOPS`。
- **样式**：命令行输出行。
- **触发命令**：`OOPS`。
- **实现位置**：`src/app/commands/fileops.rs` 的 `"OOPS"` → `Scene::restore_erased_entities` + `push_undo_snapshot`/`commit_undo_delta`。
- **备注**：无缓存报 “OOPS: nothing to restore.”；成功回显 “OOPS: restored {n} object(s).”。命令别名表常量 `src/app/plugin_host.rs` 中的文件命令列表亦包含相关项。

### 撤销历史的数据结构（History State）
- **功能简介**：每个文档维护独立 undo/redo 栈，条目为实体增量 + 可选结构快照，含对象可见性快照。
- **UI 入口**：无直接控件（由 Undo/Redo 按钮与下拉呈现）。
- **样式**：无。
- **触发命令**：内部。
- **实现位置**：`src/app/document.rs::HistorySnapshot`（`Delta`/`ObjectVisibility`、`label()`、`estimated_bytes()`）、`src/app/document.rs` 的 `HistoryState`；`src/app/history.rs` 全部 `begin_undo`/`commit_undo_delta`/`begin_*_undo`/`push_undo_entry`/`trim_history`。
- **备注**：`edit_revision` 单调递增，供后台保存判断新旧；历史限制默认 256 条 / 512 MiB，可用环境变量 `OCS_HISTORY_MAX_ENTRIES`、`OCS_HISTORY_MAX_MB` 覆盖（0 表示无限）。

---

## 七、布局标签（Layout Tabs，状态栏）

布局标签位于状态栏左区（`show_layout_tabs` 为真时），显示 Model 与各图纸布局，以及块编辑（BEDIT）标签。视图入口 `src/ui/statusbar/mod.rs::StatusBar::view()`、`space_tab()`。

### 布局标签（Model / Paper Layout Tab）
- **功能简介**：点击切换 Model/布局；纸张布局可重排、重命名、删除。
- **UI 入口**：`状态栏 → 左区 → 布局标签（点击切换）`。
- **样式**：文字字号 12，内边距 `[4, 10]`；活动标签背景 `primary.weak.color`、文字 `primary.weak.text`、边框 `primary.base` 宽 1、圆角 2；非活动文字 72% 透明；Start 页时禁用（42% 透明并带提示 “Open or create a drawing to switch layouts.”）。
- **触发命令**：点击发 `Message::LayoutSwitch(name)`。
- **实现位置**：`src/ui/statusbar/mod.rs::space_tab()`；数据来自 `src/app/view/mod.rs` 传入的 `layout_names`。
- **备注**：`layouts[0]` 恒为 “Model”；纸张标签用 `PosReport::owned("SB_LAYOUT_TAB:{name}")` 记录边界，供拖拽重排。

### 布局标签重排（Layout Drag-to-Reorder）
- **功能简介**：拖动纸张布局标签改变顺序。
- **UI 入口**：`状态栏 → 拖动纸张布局标签`。
- **样式**：同文档标签拖拽（主题主色竖线落点指示器）。
- **触发命令**：释放发 `Message::LayoutReorder { from, to, after }`。
- **实现位置**：`src/ui/wrap_bar.rs::ReorderTab::layout()`；处理 `src/app/update/mod.rs` 的 `Message::LayoutReorder`（`Scene::set_layout_tab_order`）。
- **备注**：仅纸张布局可重排（`reorderable_layouts` 为 `layouts[1..]`）；Model 与块标签不可。

### 布局标签右键菜单（Layout Context Menu）
- **功能简介**：右键纸张布局标签可重命名或删除。
- **UI 入口**：`状态栏 → 右键纸张布局标签 → 上下文菜单`。
- **样式**：菜单宽 160，`bordered_box`、内边距 `[4,0]`；每项字号 12、内边距 `[4,12]`。
- **菜单项**：
  - `Rename` → `Message::LayoutRenameStart(name)`
  - `Delete` → `Message::LayoutDelete(name)`
- **实现位置**：`src/ui/statusbar/mod.rs::layout_tab_context_menu()`；挂载 `space_tab()`。
- **备注**：Model 标签无菜单（`has_context_menu = reorderable_layouts.contains(&label)`）。

### 布局内联重命名（Inline Layout Rename）
- **功能简介**：重命名布局时标签就地变为文本输入框，可提交或取消。
- **UI 入口**：`布局标签右键 → Rename → 标签变输入框`。
- **样式**：`text_input` 宽 90、字号 12、内边距 `[3,6]`；旁附 ✕ 取消按钮（`CLOSE` 图标 10px）。
- **触发命令**：输入发 `Message::LayoutRenameEdit`；Enter 发 `Message::LayoutRenameCommit`；✕ 发 `Message::LayoutRenameCancel`。
- **实现位置**：`src/ui/statusbar/mod.rs::space_tab()`（`rename_edit` 分支）；输入框 Id `LAYOUT_RENAME_INPUT_ID`；处理 `src/app/update/mod.rs` 的 `Message::LayoutRename*`。

### 新建布局按钮（+ Add Layout）
- **功能简介**：新增一个纸张布局。
- **UI 入口**：`状态栏 → 布局标签末尾 “+” 按钮`（Start 页禁用并带提示）。
- **样式**：`text("+")` 字号 12、内边距 `[4,8]`、`button::subtle`。
- **触发命令**：点击发 `Message::LayoutCreate`。
- **实现位置**：`src/ui/statusbar/mod.rs::StatusBar::view()`（add_btn）。

### 布局列表汉堡菜单（Model and Layout List）
- **功能简介**：状态栏最左的汉堡按钮，下拉列出 Model 与全部布局，便于直接选择（标签条换行时尤其有用）。
- **UI 入口**：`状态栏 → 最左 汉堡按钮（MENU 图标）`。
- **样式**：`button::subtle`、内边距 `[4,8]`、图标 16px；Start 页禁用；下拉 `statusbar_menu::layout_entries` 宽 200。
- **触发命令**：`Message::StatusMenuTooltipHidden(true)`（展开）。
- **实现位置**：`src/ui/statusbar/mod.rs::StatusBar::view()`（menu_btn）；`src/ui/statusbar/statusbar_menu.rs::layout_entries()`。

### 布局标签开关（Layout Tabs Toggle，LAYOUTTAB）
- **功能简介**：显示/隐藏状态栏布局标签条。
- **UI 入口**：`Ribbon → View 选项卡 → Layout Tabs 按钮`；`命令行 → LAYOUTTAB`。
- **样式**：Ribbon 工具（图标 `layout_tabs.svg`）。
- **触发命令**：`LAYOUTTAB`。
- **实现位置**：`src/modules/view/layout_tabs.rs::tool()`；状态读取/写入 `src/ui/ribbon/widgets.rs`（`"LAYOUTTAB" => state.show_layout_tabs`）、`src/ui/ribbon/mod.rs::set_layout_tabs()`；切换处理 `src/app/update/mod.rs`（`show_layout_tabs ^= true`）。
- **备注**：状态栏传给 `space_tab` 的 `show_layout_tabs` 决定是否渲染标签。

### 布局标签滚动（Layout Tabs Scroll）
- **功能简介**：横向滚动布局标签条（窄窗口时）。
- **UI 入口**：`状态栏 → 布局标签条`。
- **样式**：滚动容器 Id `LAYOUT_TABS_SCROLL_ID`。
- **触发命令**：`Message::ScrollLayoutTabs(dx)`。
- **实现位置**：`src/ui/statusbar/mod.rs::LAYOUT_TABS_SCROLL_ID`；处理 `src/app/update/mod.rs` 的 `Message::ScrollLayoutTabs`（`scroll_by`）。

---

## 八、打开进度、修复/恢复、缺失字体、更新提示

### 打开进度覆盖层（Open Progress）
- **功能简介**：加载 CAD 文件时显示的模态进度卡片，含文件名/大小、当前阶段、百分比进度条和取消按钮。
- **UI 入口**：`Open/OpenRecent/拖放 → 加载中自动显示`。
- **样式**：卡片宽 `CARD_WIDTH = 420.0`，进度条轨宽 `BAR_TRACK_WIDTH = 380.0`、高 `BAR_TRACK_HEIGHT = 6.0`（圆角 3，填充主色、轨 `background.strong`）；标题 “Opening file” 字号 15；文件名+大小字号 13（文字 82% 透明）；阶段+百分比字号 12（主色）；取消按钮 danger 色；卡片底色 `background.weak`、边框圆角 6；外层半透明深色背板（`background.strong` 72% 透明）挡住点击。
- **阶段文案**（`phase_label`）：`Reading file…`、`Parsing entities…`、`Loading references…`、`Building scene caches…`、`Finalizing…`、`Working…`。
- **触发命令**：取消发 `Message::OpenCancel`（清除 `opening` 并报 “Open cancelled: ...”，随后继续处理排队打开）。
- **实现位置**：`src/ui/window/open_progress.rs::view()`；阶段常量 `src/app/mod.rs`（`OPEN_PHASE_*`）；视图挂载 `src/app/view/mod.rs::view_main()`（`open_progress_layer`）；处理 `src/app/update/mod.rs` 的 `Message::OpenCancel`。

### 修复报告对话框（Recovery Report）
- **功能简介**：文件打开时检出并对可修复错误做了修复（或引用失败）后，展示修复统计与详情。
- **UI 入口**：`Open 检出问题 → ModalKind::Recovery`。
- **样式**：标题字号 20（Recovered 用 `warning.base`，Failed 用 `danger.base`）；文件名字号 13；描述字号 11 muted；四个指标卡（已扫描实体、发现问题、已移除实体、已检查引用）标签字号 10 muted / 数值字号 18，卡片底色 `background.weak`、边框圆角 5、内边距 `[10,12]`；详情区为可滚动 `bordered_box`。
- **按钮**：`Save a copy...`（`Message::RecoverySaveAs`，仅恢复成功且 `save_as_required` 且有标签；primary）、`Show log`（`Message::RecoveryShowLog`，secondary；有日志或 web 时显示）、`Close`（`Message::RecoveryClose`，secondary）。
- **实现位置**：`src/ui/window/recovery.rs::view_window()`；报告结构 `src/io/recovery.rs::RecoveryReport`；视图挂载 `src/app/view/modal.rs`（`ModalKind::Recovery`）；处理 `src/app/update/mod.rs` 的 `Message::Recovery*`；生成 `src/app/update/file.rs::on_file_opened()`。
- **备注**：修复过的图纸被标记 `dirty` 且 `recovery_save_as_required`，直接 Save 会强制转 Save As；报告会持久化（`report.persist()`）并写日志（`log_path`）。

### 恢复尝试提示（Recovery Prompt）
- **功能简介**：严格读取失败但可能可恢复时，先询问用户是否尝试恢复读取。
- **UI 入口**：`Open 严格读取失败 → ModalKind::RecoveryPrompt`。
- **样式**：标题 “recovery-prompt” 字号 20；文件名字号 13；描述字号 11 muted；错误详情为可滚动 `bordered_box`（字号 10）。
- **按钮**：`Decline`（`Message::RecoveryDecline`，secondary）、`Attempt`（`Message::RecoveryAttempt`，primary）。
- **实现位置**：`src/ui/window/recovery.rs::view_prompt()`；视图挂载 `src/app/view/modal.rs`；处理 `src/app/update/mod.rs` 的 `Message::RecoveryAttempt/Decline`。
- **备注**：`RecoveryAttempt` 会用 `recover_path_with_phase` 重新读取（web 用 `recover_web_bytes`）；文件在提示期间被改动则重新读取最新内容。`RecoveryDecline` 清除打开状态并报 “Recovery cancelled: ...”。

### 缺失字体提示（Missing Fonts）
- **功能简介**：图纸引用了本机没有的 `.shx` 字体时，提示从社区仓库或自定义源下载，否则继续用替代字体。
- **UI 入口**：`Open 成功且检出缺失字体（check_missing_fonts 开启且非只读）→ ModalKind::MissingFonts`。
- **样式**：标题 “Missing fonts” 字号 20；说明字号 11 muted；缺失字体列表为可滚动 `bordered_box`（每项字号 12）；字体源输入框占位 “https://server/fonts or a GitHub raw folder”、字号 11、内边距 `[4,8]`；源说明标签字号 10 muted。
- **按钮**：`Skip`（`Message::MissingFontsDismiss`，secondary；下载中禁用）、下载按钮（`Message::MissingFontsDownload`，primary；文案在 `Download available` 与 `Downloading...` 间切换）。
- **触发命令**：无独立命令；`Font source` 输入发 `Message::MissingFontsSourceChanged`。
- **实现位置**：`src/ui/window/missing_fonts.rs::view_window()`；检测 `crate::io::font_repo::missing_shx_fonts`；视图挂载 `src/app/view/modal.rs`；处理 `src/app/update/mod.rs` 的 `Message::MissingFonts*`。
- **备注**：下载源跨会话记忆（`font_source_url`）；本次会话内已跳过的字体会抑制重复提示（`suppressed_missing_fonts`）；只读/MCP 会话不弹此框、不下载（仅在控制状态报告）。

### 更新提示对话框（Update Notice）
- **功能简介**：发现 GitHub 上有更新版本时提示，展示版本对比、发布说明，并提供前往发布页/稍后。
- **UI 入口**：`启动时更新检查发现新版本 → ModalKind::UpdateNotice`。
- **样式**：标题 “New Release Available” 字号 20 主色；副标题字号 11 muted；两张版本卡（`Installed v{当前}` 左、`Latest v{新}` 右，后者边框/标签用主色），卡片数值字号 20、标签字号 10；中间装饰箭头图标 20px；发布说明 “What's new” 用轻量 markdown 渲染（`##`/`###` 标题、`-`/`*` 列表、去 `**`/`` ` ``），正文字号 11~13，装在可滚动 `bordered_box` 中。
- **按钮**：`Later`（`Message::UpdateNoticeClose`，secondary）、`Open Release Page`（`Message::UpdateNoticeOpenRelease`，primary）。
- **触发命令**：无独立命令。
- **实现位置**：`src/ui/window/update_notice.rs::view_window()`；视图挂载 `src/app/view/modal.rs`（`ModalKind::UpdateNotice`）；检查任务 `src/io/update_check.rs::check_for_update()`（启动时 `Message::UpdateCheckResult`）；处理 `src/app/update/mod.rs` 的 `Message::UpdateCheckResult` 与 `Message::UpdateNoticeClose/OpenRelease`。
- **备注**：通过 `pending_startup_modals` 排队（不会抢在关联/捐赠/GPU 等启动模态框之前）；打开发布页用 `update_check::RELEASES_PAGE`。

### 启动模态框排队（Startup Modals Queue）
- **功能简介**：启动时按序排队若干提示（关联默认程序、捐赠、GPU 警告、更新提示、恢复等），一次只显示一个；有打开/修复进行时延后。
- **UI 入口**：`启动 → 依次显示`。
- **样式**：取决于各模态框。
- **触发命令**：内部 `show_next_startup_modal()`。
- **实现位置**：`src/app/startup.rs::queue_startup_prompts()` / `show_next_startup_modal()` / `mark_startup_modal_shown()`。
- **备注**：捐赠提示每版本一次（`donation_prompt_version`）；GPU 警告按 verdict 记忆（`gpu_warning_silenced`）；`show_next_startup_modal` 在 `active_modal`、`opening`、`pending_opens` 皆空时才弹下一个。

### GPU 警告对话框（GPU Warning，文件/渲染上下文）
- **功能简介**：检测到软件光栅化或无可用渲染器时提示（与文件操作相关的画面可用性说明）。
- **UI 入口**：`启动后检测到 GPU 问题 → ModalKind::GpuWarning`。
- **样式**：标题字号 13；说明与平台建议字号 13；按钮 `OK`/`Don't show again`（`Message::GpuWarningSilence`）。
- **实现位置**：`src/app/view/modal.rs::gpu_warning_window()`；逻辑 `src/app/startup.rs::refresh_gpu_status()`。
- **备注**：软件渲染时命令行另有一条警告（含适配器名）；状态栏保留一个常驻 pill。

### 自动保存与恢复副本（Autosave / .sv$）
- **功能简介**：按 SAVETIME 间隔把所有脏图纸写入 `.sv$` 恢复副本（桌面），供崩溃后恢复；干净保存/退出时清除。
- **UI 入口**：无独立按钮（`SAVETIME` 设置间隔）。
- **样式**：无。
- **触发命令**：`SAVETIME`（分钟；0 关闭）。
- **实现位置**：`src/app/update/file.rs::on_autosave()`、`autosave_target()`、`cleanup_autosaves()`、`exit_app()`；订阅 `src/app/view/mod.rs`（`autosave` Subscription）；设置 `src/app/settings.rs`（`savetime_min`）；命令 `src/app/commands/styleprops.rs` 的 `"SAVETIME"`。
- **备注**：有路径的图纸副本写在其旁 `<name>.ocs-autosave.sv$`，无路径的写入临时目录 `OpenCADStudio_<name>_<id>.sv$`；自动保存不改动原文件也不清除 dirty；关闭标签/放弃更改时删除对应副本。

### 文件编辑租约与外部改动（Edit Lease / External Change，桌面专有）
- **功能简介**：打开文件时获取编辑租约（防止其它编辑器同时写入），并跟踪磁盘指纹以检测外部改动。
- **UI 入口**：冲突时经对话框（FileInUse / ExternalChange）与命令行提示体现。
- **样式**：见对应对话框。
- **触发命令**：无独立命令。
- **实现位置**：`src/app/update/file.rs::install_native_edit_guard()`、`refresh_native_edit_guard_after_save()`；锁定实现 `src/io/edit_lock.rs`；标签字段 `src/app/document.rs`（`edit_lease`、`edit_lock_conflict`、`disk_fingerprint`）。
- **备注**：打开时被其它编辑器锁定会以只读方式打开并提示 “Opened read-only against other editors: ...”；平台不支持租约时提示但仍做外部改动检查。

---

## 九、其它文件/文档相关

### WBLOCK 写块（文件输出）
- **功能简介**：把选中对象或指定块写入新的 DWG/DXF 文件。
- **UI 入口**：`命令行 → WBLOCK`（别名 `W`）。
- **样式**：原生保存对话框；成功后回显。
- **触发命令**：`WBLOCK`（别名 `W`，见 `assets/ocad.pgp`）。
- **实现位置**：`src/app/update/file.rs::on_wblock_save_result_some()`；提取逻辑 `src/modules/insert/wblock.rs`。

### ARCHIVE / ETRANSMIT 打包
- **功能简介**：把图纸及其引用文件（外部参照 + 光栅图像）复制到旁边的 `<name>_archive` 文件夹。
- **UI 入口**：`命令行 → ARCHIVE` 或 `ETRANSMIT`。
- **样式**：命令行输出/错误行。
- **触发命令**：`ARCHIVE`、`ETRANSMIT`。
- **实现位置**：`src/app/commands/fileops.rs` 的 `"ARCHIVE" | "ETRANSMIT"`。
- **备注**：无文件路径时报 “ARCHIVE: save the drawing first (it has no file path yet).”；成功回显 “packaged {n} file(s) into ...”。

### SCRIPT 运行命令脚本
- **功能简介**：运行 `.scr` 命令脚本，逐行作为命令输入。
- **UI 入口**：`命令行 → SCRIPT`（或带路径 `SCRIPT <path>`）；`--script` 启动参数。
- **样式**：命令行 `ValuePromptCommand` 提示 “SCRIPT  path to the .scr file:”。
- **触发命令**：`SCRIPT`、`SCR`。
- **实现位置**：`src/app/commands/fileops.rs` 的 `"SCRIPT" | "SCR"` 与 `cmd.starts_with("SCRIPT ")`。
- **备注**：跳过 `#`、`;` 开头的行；空行对活动命令提交 Enter；插件脚本禁用文件/退出类命令（见「命令行文件命令总表」备注）。

### 图元/属性编辑器等关联对话框（文件内容编辑入口）
- **功能简介**：与文档内容相关的多个模态框（属性编辑器、块属性编辑器、超级链接等）在文件打开后的文档上下文中弹出。
- **UI 入口**：`内容命令触发（如 ATTEDIT、HYPERLINK）`。
- **样式**：各模态框（`ModalKind::AttributeEditor`、`ModalKind::Hyperlink` 等）。
- **触发命令**：`ATTEDIT` / `HYPERLINK` 等。
- **实现位置**：`src/app/view/modal.rs::modal_content()`（各 `ModalKind` 分支）；模态框标题 `src/app/view/modal.rs::modal_title()`。
- **备注**：ModalKind 标题映射见 `modal_title()`（如 `UpdateNotice → "update-available"`、`Recovery → "recovery-report"`、`MissingFonts → "Missing fonts"`、`RecoveryPrompt → "recovery-prompt"`）。

### 文档标签稳定 ID（Document Identity）
- **功能简介**：每个标签有单调递增的稳定 `id`，后台任务与关闭队列据此定位标签，避免索引漂移。
- **UI 入口**：无直接控件。
- **样式**：无。
- **触发命令**：内部（`TabClose`、`TabReorder`、保存任务等）。
- **实现位置**：`src/app/document.rs::DocumentTab`（字段 `id`，`NEXT_DOCUMENT_TAB_ID`）；`on_tab_close`、`continue_tab_close_queue` 以 id 解析。
- **备注**：后台保存等异步工作必须用 id 而非瞬时向量索引。

### Web 版最近文件缓存（Browser-private Recent Copies）
- **功能简介**：web 版无文件系统，打开的图纸与缩略图存入源私有存储（OPFS），支持再次打开最近文件。
- **UI 入口**：`Start 页 → Recent Documents`（web）。
- **样式**：同最近文件面板；目录行显示 “browser-storage”。
- **触发命令**：点击 → `Message::OpenRecent(path)` → `io::open_recent_web`。
- **实现位置**：`src/io/web_recent.rs`（`store_open`、`read`、`read_thumbnail`、`remove`）；`src/io/mod.rs::open_recent_web`；处理 `src/app/update/mod.rs` 的 `Message::OpenRecent`（web 分支）。
- **备注**：删除最近项或超限驱逐时会异步移除对应缓存副本（`recent.rs::remove_cached_copies`）。

---

（文档结束）
