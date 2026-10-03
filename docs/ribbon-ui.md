# Ribbon 与命令栏布局（2026-10-02）

继续使用 Slint UI + 同设备 wgpu 离屏合成，不迁移到 DOM 工具界面。
Web 的 DOM 仅提供文件选择、语言/启动恢复与原生多点触控适配。

## 文件边界

- `ui/app.slint`：组合根、保持现有 adapter 属性/回调协议。
- `ui/ribbon.slint`：常用 / 视图 / 管理分组与展开状态。
- `ui/button.slint`：深色按钮，大触控目标，不压缩桌面工具来适应手机。
- `ui/command-bar.slint`：持续可见的命令行、可展开状态/操作行。
- `ui/canvas.slint`：Image 与 Slint 单指针/滚轮事件。
- `ui/panels.slint`：真实图层、布局、属性、批注和诊断数据的滚动面板。
- `src/command_line.rs`：小型命令词路由；不重复实现 CAD 业务逻辑。
- `web/host/touch.js`：CAD 区域内双指/拖动识别，控件输入仍属于 Slint。
- `browser/shell.rs`：CSS 像素手势转换成应用 Pan/Zoom 命令，不直接更改渲染相机。

## 行为

桌面上方显示 Ribbon，箭头可收起为标签行；底部始终保留命令行，箭头展开/收起
状态和操作行。手机默认不显示 Ribbon、宿主设置或面板，仅保留底部命令行。
输入 `TOOLS` 或展开底部操作行再点工具按钮，显示手机 Ribbon 与宿主语言设置。
手机按钮至少 48 CSS px 高，320px 宽度单独验收；管理页文件操作纵向排列。

`OPEN` 打开真实 DWG 文件选择器，`FIT` / `ZOOM EXTENTS` 适应图纸，`UNDO` / `REDO`
使用真实可用性；`TOOLS` / `RIBBON` 切换 Ribbon，`PANELS` 切换数据面板，
`DIAGNOSTICS` 打开诊断。不是 AutoCAD 完整命令解析器。

绘制/编辑 Ribbon（直线、圆、移动、修剪）与命令词 `LINE` / `CIRCLE` / `MOVE` /
`TRIM`（别名 `L`/`C`/`M`/`TR`）已是真实工具，不再是禁用占位：
`cad-app::draw_tool` 提供纯捕获状态机（`DrawToolKind`/`DrawTool`/`DrawPreview`/
`DrawIntent`），shell 的 `begin-draw-tool` 启动捕获，画布取点进入状态机并推入
状态行；确认恰好发出一次绘制命令，取消零事务。`docs/drawing-edit.md` §4 的
`pending("ui.command.editing")` 与 `pending("ui.ribbon.draw_modify")` 占位已删除。
未知命令显示 `command.unknown` 的明确文案，绝不是空成功。

工具为 **Work-only**：Viewer 下按钮禁用，命令层仍会拒绝（`PermissionDenied` →
目录文案 `draw.error.read_only`），不靠隐藏按钮。`MOVE` 需要非空选择，否则显式
提示 `draw.error.selection_required` 且不启动；`TRIM` 按“先目标后边界”两步取点。
多指手势由既有 `cad-app::input::InputPolicy` 处理：第二个手指落下即
`ToolCancelled`，且绘制提交必须显式确认，因此多指**不可能**提交。预览经
`CadView::set_draw_preview` 走既有预览叠加层（LINE/MOVE/TRIM 为橡皮筋，
CIRCLE 为整圆），提交几何由命令层负责，叠加层从不写库。

**仍未支持的图元**：`TRIM` 只支持 LINE 被 LINE/LWPOLYLINE 直线段裁剪；目标/边界
含圆弧、SPLINE、INSERT 实例、Opaque 等返回显式 `Unsupported` 或 `Partial` 诊断且
不改库（`docs/drawing-edit.md` §3）；`CreateLine`/`CreateCircle` 仅创建直线与圆。
圆弧/样条/多段线绘制尚未支持。

**宿主接线**：Web 与 Android 已安装绘制命令和预览 sink。`MoveEntities`/`TrimEntity`
需要会话的 `SelectionRef`，shell 只有选择计数而非 refs，因此由宿主实现两个 sink：
`UiAdapter::set_draw_command_sink(DrawCommandSink)`（确认时按 `docs/drawing-edit.md`
§2 映射为 `CommandId::CreateLine/CreateCircle/MoveEntities/TrimEntity` 并走共享
`UiCommandSink`/事务/历史路径；未安装时状态行显式提示 `draw.error.unwired`），
以及 `UiAdapter::set_draw_preview_sink(DrawPreviewSink)`（把预览转发到
`CadView::set_draw_preview`；未安装则仅无实时叠加，状态行仍跟踪捕获）。
`DrawCommandSink` 的 `commit` 契约与逐条映射记录在 `draw.rs` 文档注释。
绘制命令失败须向 shell 返回真实错误，保留捕获参数，不得清空工具冒充成功。
两宿主的状态漏斗通过 `CadView::sync_drawing` 发布当前数据库 Arc，使创建/编辑及
撤销重做进入显示表示重建；导航/叠加更新复用原 Arc，不重新导入。主指针捕获不再
同时派发导航/选择，防止 MOVE 取空白锚点时丢失选择。
Web 的原生触控适配在第二触点落下或 touchcancel 时调用 `touch_cancel_draw`，立即清空
未确认捕获；成功换图纸时两宿主均调用 `UiHandle::cancel_draw_capture`，旧点不能进入
新图纸。TRIM 的世界拾取容差由相机与逻辑像素策略派生，不再固定为 0.5 图纸单位。

Web 在 CAD 区域内单指拖动平移、双指捏合缩放并跟随中心平移；**手指数变化只重新
建立基准、不发出导航增量**（第二个手指落下或抬起时，基线在变化点重置，之后的移动
才相对新基线发出增量，因此质心跳变不会被当成平移/缩放），取消不选取，拖动/捏合
不误发点击，轻触才进入既有 pick 通道。工具与命令控件的触屏事件不被 CAD 手势拦截。
缩放围绕当前 CAD 相机中心，尚非任意触点锚定缩放；3D 专用多指轨道映射与真实
iOS/Android 手势仍待验收。本轮为 `web/host/touch.js` 的手指数契约补了显式基线与
单测（见“验收”），**未新增浏览器/设备证据**。

## 独立 CAD 引擎：下一阶段，不冒充已完成

领域/数据库/表示/场景/渲染仍独立于 Slint。CPU controller 已在 `cad-app/render_scene`，
UI 只派生命令与视图；参见 `bridge-runtime.md`。当前 `app-web.wasm` 仍包含 Slint，
**不是**对外独立 CAD 显示 wasm。下一阶段应增加无 Slint 依赖的 wasm facade，明确
文档句柄、导入/释放、资源、设备提供、视图、错误与异步任务协议，并由 Slint 宿主
复用它；不得把宿主导出改名当作核心提取完成。这一阶段由后续助手实施。

## 验收

`node --test scripts/test-web-touch.mjs` 覆盖手势计算、输入边界、取消、tap/drag/pinch
冲突与手指数转换：包含第二个手指落下不发出质心跳变增量、手指数变化后移动相对新基线、
第三个手指（采样上限后手指数不变）不移动基线，以及轻触选取/拖动不选取。`scripts/check-web-ribbon.mjs` 在真实无头 Chromium 的桌面、
390px DPR3、320px DPR2 验证初始布局、两条栏展开/收起、Slint Open 文件选择器、
4 实体 DWG 导入及 CDP 双指/拖动导致真实应用相机/帧变化。
截图路径与报告：`/tmp/opencode/yacr-ribbon-validation/`。
SwiftShader 在连续动态截图时出现 compositor capture 停滞；布局动作以状态契约
验收，默认页面截图用独立 capture，并单独检查展开布局。不是硬件 GPU 性能证据。

最终验收：核心串行 875 passed / 0 failed / 1 ignored；JS 契约 11 passed；
app-web wasm 严格 clippy、UI wasm 测试编译、Android aarch64 编译、架构/i18n
检查通过。真实无头 WebGL2 的 ribbon、mobile、desktop 三套脚本串行通过；
桌面导航像素改变，语言偏好与空批注 sidecar 往返保留，移动端旋转保留 4 实体文档。
报告另见 `/tmp/opencode/yacr-ribbon-mobile/report.json` 和
`/tmp/opencode/yacr-ribbon-desktop.json`。同环境并发浏览器测试不稳定，须串行；
本次未验收真机/WebGPU，未将这些结果作为生产性能声明。
发布构建使用同源完整字体目录（99 个 CAD 字体 + catalog），不是 WITH_FONTS=0 的
测试目录；源自此前自托管构建的缓存，许可证/再分发责任仍见 `docs/fonts.md`。
