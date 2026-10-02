# 代码审计与下游 Coding Agent 交接

审计日期：2026-10-01。基线 commit：`c260d8bfc44a108bdcc99b6114ac069b451284ec`。
需求权威：`CAD_IMPLEMENTATION_SPEC.md` v2.0；本报告是审计结果及用户新增需求，不取代原规范。
本轮仅新增本文件，不修改业务实现。审计开始时工作区干净。

## 1. 结论与证据边界

当前是有真实核心算法、事务和部分宿主接线的原型，不是全部空框架，也不是可验收产品。
不能仅按 `pending()` 数量判断完成度：大量缺口表现为忽略字段、未消费输出、缺少宿主接线或错误的成功状态。

- 已有：只读底图 Builder、批注事务、撤销重做、部分几何/测量算法、代理有界 framing、基础表示提供器、CPU 索引、查询、Slint/wgpu 同设备纹理桥、Web 文件入口和静态构建脚本、基础 CLI。
- 未闭环：手机交互、真实文件打开与取消、图层/布局显示、拾取高亮、六类批注工具与显示、无损持久化、恢复保护、字体/外参、完整 3D、ACIS、跨后端初始化与恢复、大图性能。
- 高风险：新图纸可能仍显示旧图；打开/重载会丢未保存批注；JSON 往返丢失语义；能力表与实际绘制不一致。
- UI 审查基于 Slint/Rust/JS/CSS 源码，本轮未运行浏览器、模拟器或真机；布局溢出属于待截图验证风险，不能冒充视觉实测。
- `fixtures/manifest` 的 fixtures 为空，不能声明真实 DWG、天正/探索者、字体或黄金图验收通过。

### 1.1 本轮实际执行

运行 cargo 时需将 `$HOME/.cargo/bin` 加入 PATH。

| 检查 | 结果 |
|---|---|
| `python3 scripts/check-architecture.py` | 通过：23 packages，依赖无环，核心边界完整 |
| 核心 `cargo test --workspace --exclude cad-ui-slint --exclude app-android --exclude app-web --locked --no-fail-fast` | 失败：两个测试目标失败，见 B01/B02；其余执行目标通过 |
| `cargo check --workspace --lib --target wasm32-unknown-unknown --locked` | 失败：app-android 两处编译错误，见 B03 |
| Android target 检查 | 初次缺 SDK 环境；按 docs/build.md 配置 SDK/JDK 后，确认同样两处代码编译错误，见 B03；cad-ui-slint 检查通过 |
| APK 重打包、浏览器、真机、黄金图、性能 | 本轮未运行 |

本地日志（不是可随仓库交付的永久附件）：`/tmp/opencode/yacr-audit-tests.log`、`/tmp/opencode/yacr-audit-wasm.log`、`/tmp/opencode/yacr-audit-android.log`。
下游须重跑并将新证据写入 `docs/validation.md`，不要继续引用旧文档的“全部通过”。

## 2. 原规范功能缺口

状态：**部分**=核心有实现但缺产品闭环；**未实现**=关键路径缺失；**未验收**=缺真实运行/样本证据。所有 F01–F15 均未达到完整产品验收。

| 编号 / 规范 | 当前证据 | 尚缺功能与验收要求 |
|---|---|---|
| F01 本地打开、取消、恢复 | importer 有 DWG 签名/文件大小/取消前后检查；Web File API 已接；Android 仅候选路径 | Android SAF/content URI/授权；原生后台任务/Web Worker；进度与取消；加载替换保护；文档 generation 过期丢弃；合法/损坏/超限样本。入口：`cad-app/src/host.rs::open_bytes`、两宿主 |
| F02 导航 | app 有 pan/zoom/fit/reset；Web 有拖动滚轮 | Android 未安装 ViewInput；双指平移/捏合、鼠标锚点缩放；重置入口；真实画布尺寸/DPI/旋转更新；大坐标正确性 |
| F03 图层 | Session 覆盖、Query layers 已定义/实现 | 无图层面板、搜索/恢复入口；渲染不消费覆盖或 Layer.visible；显隐需不重解析、不改底图、不重新生成全部几何 |
| F04 布局 | importer 读取纸空间与 PaperViewport；app 修改 active_space | bridge 只构建 model_space；无布局切换 UI；矩形裁剪、视口变换比例不正确；纸空间测量策略未闭环 |
| F05 选择/高亮/属性 | CPU GridSpatialIndex 与 SelectionRef、properties 查询已有 | Select 仅设 Selecting 状态；无精确拾取、选择集更新、高亮、实体只读属性面板；需区分 INSERT instance/sub-element |
| F06 测量 | 引擎含距离、长度、角度、共面面积；有局部捕捉接口 | UI 发送 None，无法选点；命令按点数猜算法且无面积入口；无单位显示转换/校准、空间选择、捕捉候选接线、结果保存为批注；返回结构化 MeasurementRecord |
| F07 六类批注 | DB 有文本、引线、矩形、椭圆、自由线、云线类型 | 无可执行工具、编辑器/样式、预览/确认/取消；UI 创建参数缺失；bridge 不读取批注库，既有批注也不可见；需一次确认一事务、多指零误提交 |
| F08 批注编辑/历史 | app create/update/delete + history 路径可用 | 无批注选择/修改/删除 UI；redo 可用状态绑定错误；模式切换未接线；导入绕过权限；变更未分发到 UI/场景/索引 |
| F09 列表/隐藏/交换 | JSON codec、host import/export、Web 下载入口 | 列表 UI/隐藏状态缺失；codec 无损性和 mapping 策略有缺陷；Android 文件导入/导出、原子保存、迁移历史样本、IndexedDB/应用恢复均缺 |
| F10 资源 | MapResolver/ResolverChain/路径策略、样式 resource_keys | 宿主 document.resource_keys 永远空；缺资源包 UI、字体 shaping/图集、SHX/BigFont 支持、CAD TEXT/MTEXT 排版、外参/图片显示、总缓存预算/递归限制接线 |
| F11 诊断/代理 | importer/proxy 输出报告；diagnostics/CLI 有摘要 | UI 只取首条诊断；表示缺失不汇总；能力表错误；代理仅已知 FillOff/UnicodeText，缺线面与状态栈实证支持；缺对象级可浏览缺图报告及真实厂商样本 |
| F12 Web 后端/恢复 | Auto adapter probe、偏好存储重载、桥接纹理存在 | 实际 device 初始化失败回退未闭环；GL 桥仍标 WebGPU；强制失败缺交互决策；重载丢文档/批注；设备重建后场景不恢复；两后端须真实运行 |
| F13 三维观察 | Camera/Projection/StandardView 类型及部分命令 | Switch2d3d/Orbit 显式 pending；GPU 只有 Camera2d；标准视图不进入真实投影；工作平面、近平面、透视缩放和回到 2D 缺失 |
| F14 三维直接几何 | importer 部分 3D polyline/3DFACE/SOLID、mesh 类型/法向算法 | Scene 无拓扑标记，GPU 只有 LineList；缺面着色/深度/透明/镜像绕序/边线偏移、Mesh/Polyface 完整适配、3D 拾取；不是已有 3D 渲染 |
| F15 ACIS | `cad-kernel-adapter::tessellate` 显式 pending | 无可用 SAT/SAB/自有交换数据路径与内核离散；importer ACIS opaque payload 为空；需按 3DSOLID/BODY/REGION/SURFACE 建样本与缺面/边线降级报告 |

### 2.1 横切缺口

1. **增量链（§4/16/18）**：ChangeSet → 依赖 → 表示 → 索引/场景 → GPU → Query/ViewModel 未在宿主接线；scene 不按 mask 区分元数据，GPU 不消费 removed_chunks。漏事件后实际重建路径缺失。
2. **后台调度（§4.10/8.3）**：TaskExecutor/TaskStamp 契约存在，但打开、哈希、normalize、scene build/upload 仍同步；缺背压、每帧预算、Worker 二进制传输与无共享内存路径。
3. **CAD 语义（§3.2/7.1）**：实体样式数据不足；ByLayer/ByBlock、颜色/线型比例/线宽/透明度、OCS、块基点/嵌套/循环、绘制顺序未闭环。HATCH/DIMENSION 等落 opaque，不是已实现。
4. **大图（§8）**：每片段一个 GPU batch/uniform/draw；无视口剔除/实例共享/空间块/LOD 滞回/文字填充缓存；build_scene 绕过 publish/evict，upload 无预算；统计 cpu_ms 固定 0，不能算基准。
5. **生命周期（§9/16.3）**：Persistence/FileAccess/HostLifecycle 只有接口；缺暂停/低内存/进程恢复/Surface 重建/浏览器隐藏暂停/退出保护；Android recovery_enabled 未落实。
6. **扩展/来源（§10/17）**：Representation 注册部分实现；Importer/Tool/CommandHandler/PropertyProvider 等统一注册未完成；Proxy decoder 同优先级冲突与失败状态隔离需补。migration-map 未锁 OpenCADStudio commit/许可/函数与测试。
7. **自动化（§11/19）**：CLI fixed render 无条件 Unsupported，即使有 GPU 也不能执行；缺结构化批注 CRUD 入口；measure 只输出格式化诊断；benchmark 仅表示构建耗时；无 fuzz/property、黄金图/跨后端/真实生命周期验收。
8. **文档一致性**：AGENTS/handoff/requirements/validation 与当前源码接线和构建状态不同步；应基于本轮失败更新，不删除既有实现或把全部模块重新标为空框架。

## 3. UI 设计与交互问题

以下区分源码确认缺失和待运行验证的布局风险；不是对未看到的截图作审美评价。

| 编号 | 问题 / 证据 | 设计修正与验收 |
|---|---|---|
| U01 | `ui/app.slint:32–67` 所有按钮+后端选择器单行排列；compact 配置未消费 | 宽屏查看：大画布+少量浮动导航；宽屏工作：分组工具+可折叠侧栏；手机：底部分组+抽屉。窄屏溢出需截图确认；建议触控目标 ≥48 逻辑像素，不直接压缩桌面栏 |
| U02 | 两宿主 with_demo_document 默认 Work；work-mode 仅开关 enabled，无模式切换入口，mode-label 不显示 | 显式查看/工作切换，共用会话；切换取消未确认预览；UI 与命令权限一致；不把 set_work_mode 当业务授权 |
| U03 | 无图层/属性/批注列表/布局/资源/3D 面板；只有开、适应、历史、测量、批注、导入导出、诊断 | 以 F01–F15 为准补入口，提供分页/搜索/空状态；设置页区分 CAD 后端与 UI renderer，不为每实体创建控件 |
| U04 | 无当前工具提示/选点步骤/确认/取消/返回导航；Measure/CreateAnnotation 按钮发送 None | 建真正 Tool 状态机与 PreviewState；选择测量算法和批注类型；按钮启动工具而不是提交缺参命令 |
| U05 | TouchArea 无多触点身份，Web 总是左键平移；Android 无 ViewInput 接线 | 统一共享输入策略：单指绘制、双指导航、拖动阈值、pointer cancel/capture；多指不得提交批注。Esc/Android 返回先取消工具，再处理退出 |
| U06 | 没有键盘焦点/IME 接线；InputEvent 类型存在但未使用 | 文本框/弹窗获得焦点时暂停画布快捷键；中文组合输入、Esc、软键盘、浏览器缩放测试；不能把拼音组合串当命令 |
| U07 | safe_insets/dpi 配置未接；frame 用全窗口尺寸而 Image 只占工具栏之间区域 | 统一真实画布矩形→逻辑像素→物理像素→世界坐标映射；横竖屏、DPR=1/2/3、安全区、键盘弹出、resize 后绘制与拾取一致 |
| U08 | 单行状态栏只显示首条诊断；HTML host-state 覆盖底部且轮询原始 report。**已部分关闭**：Slint 状态栏改用 `cad-app` 的 `StatusModel`（固定摘要，不含诊断正文，`status_summary_never_embeds_a_diagnostic_message`），诊断抽屉保留全部对象/原因；Web `#host-state` 改为加载/失败专用，`renderer-ready` 后 CSS 隐藏，失败经 `:has(#retry-renderer...)` 重新显示。见 `docs/diagnostics-ui.md`、`docs/ui.md` §5 | 状态栏只显示简要加载/工具/单位/未保存状态；诊断抽屉展示完整对象级原因、实际后端及恢复操作；技术报告不常驻压住提示 |
| U09 | 无加载进度/取消、指纹不匹配映射弹窗、未保存决策、后端强制失败恢复 UI | 建一致 Save/PreserveRecovery/Discard/Cancel 流程；取消不丢文档；失败保留原值并说明具体可恢复动作 |
| U10 | Slint/Rust/JS/HTML 中文硬编码；locale 无效；英译会放大布局压力 | 按 N01 外置双语；英语长文、中文字体与工具提示覆盖测试；固定 96px 后端框、28px 状态栏不要作为所有控件默认尺寸 |
| U11 | Undo/Redo 都绑定 can-undo（`app.slint:38–46`） | 独立 can_undo/can_redo 派生状态，撤销到空后仍能 redo；UI 刷新不能触发重复命令 |
| U12 | Canvas 无原生 HTML 语义，未展示替代可访问入口。**已部分关闭**：`#canvas-host` 标 `role="application"`+目录 `aria-label`+`tabindex="0"`，`#a11y-status` 视觉隐藏 `role="status" aria-live="polite"` 承接播报，`#host-state` 改 `aria-live="off"` 避免重复；`scripts/test-web-a11y.mjs` 断言 DOM/CSS 契约。见 `docs/ui.md` §5 | 标注无障碍边界，测试焦点顺序/键盘可达/对比度/缩放；DOM aria-live 与 Slint 状态避免重复播报，不宣称 Canvas 等价原生语义 |

建议交互验收视口：360×800、横屏 800×360、平板 800×1280、桌面 1280×800；这是新增测试建议，不是已有通过结果。

## 4. Bug 与高风险问题清单

优先级：P0=阻止构建/测试、数据丢失或显示错误文档；P1=正确性/权限/错误能力声明；P2=性能/诊断/健壮性。
证据等级：**实测**=本轮运行失败；**源码确认**=直接可定位逻辑缺陷，复现测试尚待添加；**风险**=需真实平台/样本验证。

### B01 · P0 · 实测：应用契约测试滞后

- 位置：`crates/cad-app/tests/contracts.rs:62–68`；`cad-app/src/lib.rs:368–382`。
- 复现：`cargo test -p cad-app --test contracts --locked`。
- SwitchBackend(None) 已返回 InvalidInput，测试仍断言 NotImplemented。
- 修复：更新契约分别测试合法 Backend、缺参 InvalidInput、权限/过期身份；不要把实现回退为 pending，也不要删测试。

### B02 · P0 · 实测：渲染器初始化状态契约矛盾

- 位置：`cad-render-wgpu/src/lib.rs:128–145,455–461`。
- 复现：`cargo test -p cad-render-wgpu --lib --locked`；new(WebGl2).active_backend() 实际为 Native，测试期待 WebGl2。
- 修复：分清 requested preference、未初始化状态与真实 activated backend；优先将未初始化建模为 None/显式状态，不用偏好充当实际后端；补 native/WebGL2/WebGPU 契约。

### B03 · P0 · 实测：Android 宿主无法编译，且拖累全 workspace Wasm 检查

- 位置：`apps/app-android/src/lib.rs:148,175`。
- E0063：UiConfiguration 缺 logical_size；E0599：CadView 不存在 world_per_px()。
- 修复：初始尺寸正确赋值，再通过真实画布尺寸同步；相机比例读取 viewport.world_per_px()。不要添加假的方法绕过相机权威。
- 验收：原规范的完整 Wasm workspace check、Android UI+host check、APK 打包分别执行。

### B04 · P0 · 源码确认：新图纸沿用 ID，桥接器跳过场景替换

- 位置：`cad-app/src/host.rs:158–181` 固定 DatabaseId(1)；`cad-ui-slint/src/bridge.rs:188–198` 只比较 db.id()；demo 也为 1。
- 影响：demo→真实图纸或 A→B 时，incoming 更新但已有 GPU batches 不重建，可仍显示旧图。
- 修复：场景身份包括 DocumentId/内容身份/generation/revision；每次成功 open 发布新快照，不靠裸 DatabaseId 判断。
- 回归：不同范围的两份合成 DB 同 ID 连续打开，断言来源/顶点/像素均更新；旧任务不得覆盖新图。

### B05 · P0 · 源码确认：打开图纸丢批注且保留旧历史/会话

- 位置：`cad-app/src/host.rs:158–186` 替换 document 为新空 annotations，不调用 prepare_leave，不清历史/selection/overrides/tool。
- 影响：未保存批注直接丢失；旧 UndoRecord 可作用到新图纸，旧 LayerId/SelectionRef 可误指新对象。
- 修复：打开前统一未保存决策与恢复落盘；成功替换时重建与内容身份绑定的 session/history/query，失败/取消保持旧状态。
- 回归：A 创建批注→尝试打开 B→Cancel 保留 A；明确丢弃才替换；B 不能 undo 到 A，且选择/覆盖重置。

### B06 · P0 · 源码确认：Web 切换后端直接 reload，未保存保护失效

- 位置：`apps/app-web/src/lib.rs:220–228`、`cad-ui-slint/src/web.rs:58–65`；启动只重建 demo。
- 修复：先保存/验证恢复快照（文档身份、批注、相机、会话），获得用户决策后重载；失败不 reload；清理唯一恢复副本必须明确丢弃。
- 回归：dirty 批注下切后端/刷新，取消不重载，保留恢复后可恢复全部内容；文件授权失效给重选提示。

### B07 · P0 · 源码确认：Web 序列化即标保存，下载失败仍报成功

- 位置：`apps/app-web/src/lib.rs:36–50,205–210,436–444`。
- export_annotations_json 调用 mark_saved；download_text 各失败分支静默 return，且不能证明用户已持久保存。
- 修复：导出准备与持久写入确认分离，携带 exported_revision；纯 getter 不改 dirty；可用文件 API await 成功后标保存，下载回退须明确其确认限制，保留恢复副本。
- 回归：Blob/URL/DOM/写入失败和用户取消保持 dirty；导出 rev N 后新增 rev N+1，不能把 N+1 标已保存。

### B08 · P1 · 源码确认：批注导入绕过查看模式权限与统一命令分发

- 位置：`cad-app/src/host.rs:246–274` 直接 apply_annotation_changes；Web 直接调用它，未经过 Application::execute/authorize。
- 修复：I/O 留宿主，但 decode 后业务导入使用声明为 Work-only 的统一命令/事务；发布 ChangeSet。
- 回归：Viewer 下直接 host API/JS/CLI 导入均拒绝，revision/history/scene 不改变；Work 一次导入一事务。

### B09 · P0 · 源码确认：JSON 往返丢样式、精度、测量与实例锚点

- 位置：`cad-annotations/src/lib.rs:217–231,264–305,331–385`。
- source 单位被设为 display；logical_width/text_height 解码固定默认；precision 固定 Analytic；MeasurementRecord 的 plane/units/source/precision 未写入；anchor 仅 handle/fallback，instance/sub-element/status 丢失并变 Valid。
- 修复：完整版本化编码及迁移；真实解析枚举，禁止将缓存近似升级成解析精度。时间值需校验，并明确规范要求的时间编码。
- 回归：所有几何/单位转换/样式/ProxyCache 精度/MeasurementRecord/多 INSERT 锚点做全字段相等往返，不只测 text 相等。

### B10 · P1 · 源码确认：指纹映射策略只是放行，未执行其语义

- 位置：`cad-annotations/src/lib.rs:110–120,148–161`。
- ImportUnanchored 未清 anchor；ExplicitCoordinateMapping 未变换点/轴/工作平面/回退坐标，也未重定绑定身份。
- 修复：显式策略作用于全部几何与锚点，必要时标 Unresolved，不能继续 Valid；奇异/非有限映射拒绝；单位/面积变换语义需说明。
- 回归：不匹配文件 + 平移/旋转/比例映射，实际坐标符合预期；无锚策略所有锚解绑。

### B11 · P1 · 源码确认：坏 JSON 被静默吞掉或强制转换

- 位置：`cad-annotations/src/lib.rs:92–109,123–128,175–178,203–205,352–366`。
- 错误批注 decode None 被跳过；UUID 非法变 0；指纹短数组补零/长数组截断/字节取模；schema u64→u32 可截断；未知子字段丢失；bookmark 读写为空；非法 extension JSON 被忽略且可覆盖保留字段。
- 修复：强类型有界验证，拒绝或对象级 partial 明确报告；精确 UUID/hash 长度和字节范围；确定性 schema migration；保留未知字段或拒绝，而非静默丢失。
- 回归：坏 UUID、重复 ID、缺必填、未来 schema 大整数、未知 kind/子字段、非空 bookmark、非法 extension、嵌套/体积超限均有明确结果，不报无损成功。

### B12 · P1 · 源码确认：事务缺少几何/引用/ID 验证及精确 mask

- 位置：`cad-db/src/lib.rs:633–683,728–757`；Builder.finish:524–559。
- annotation 仅检查重复 staged key/删除存在性；key 与 annotation.id 可不同，非有限点/无效样式/测量/空间/锚点无完整校验；所有 Update 都 GEOMETRY。Builder 未校验 INSERT block、Text style、循环/深度、网格索引或数值；同一 staged insert 可覆盖前值。
- 修复：唯一受控写路径统一验证，单写入者/权限由应用能力约束；按 before/after 生成 mask；必要跨库引用在上层受控校验；失败不改变数据/revision/history。
- 回归：NaN/Inf、零轴、负宽度、key/id 不同、缺 block/style/layout、循环块、坏 mesh、重复 staged 创建；元数据/样式更新不触发全几何重建。

### B13 · P1 · 源码确认：命令 schema、payload ID、视口归属不校验

- 位置：`cad-app/src/lib.rs:299–304,444–449,573–601`。
- schema_version 未读取；CreateAnnotation 携带 Delete payload 会按 payload 执行；viewport_mut 不检查 viewport.document 是否等于 command.document；Layer/Space 不验证存在。
- 修复：声明驱动参数/模式/数据库/viewport 校验，先验证后修改。
- 回归：未来 schema、Create+Delete、A command+B viewport、不存在 Layer/Layout 均拒绝且零修改。

### B14 · P1 · 源码确认：连续折线/网格被当 LineList 绘制

- 位置：`cad-scene/src/lib.rs:139–179`；`cad-render-wgpu/src/lib.rs:194–203,425–430`。
- points 直接作为连续顶点上传，LineList 画 (0,1),(2,3)，漏掉 (1,2)；mesh 三角顶点也进入同一路径，不能填面。Point 的重合线段未形成可见标记。
- 修复：RenderBatch 显式管线/拓扑/索引/样式；折线展开相邻段或用兼容 strip；mesh TriangleList + depth/normal；Point 独立标记。
- 回归：三点折线、闭圆、单三角形/四边形、3D face、POINT 固定视口像素与几何断言；两后端一致。

### B15 · P1 · 源码确认：块定义当顶层画，INSERT/文字被跳过

- 位置：`cad-import-acadrust/src/lib.rs:343–355` 将块内实体记 SpaceId::Model；`cad-db/src/lib.rs:294–301` 无 owner 区分；`cad-scene/src/lib.rs:164–169` 跳过 Instance/Text/Image。
- 影响：块定义在原点误显示，实际 INSERT 位置缺失；TEXT/MTEXT 与代理 UnicodeText 无最终绘制；fit bounds 也不展开 insert。
- 修复：明确实体 owner/空间区分顶层和定义；递归有界实例展开/共享，保留 InstancePath、块基点、OCS 与继承样式；文字真实 shaping/图集。不支持时传播 Missing，不空成功。
- 回归：一块两 INSERT + 嵌套/镜像/非均匀变换，顶层无多画、实例独立拾取/锚定、bounds 正确；中文文字有可见输出与替代诊断。

### B16 · P1 · 源码确认：局部原点提前转 f32，破坏大坐标精度

- 位置：`cad-render-wgpu/src/lib.rs:91–96,337–340,391–400`。
- CPU 相对顶点正确，但 origin 保存 f32，再转 f64 与 camera 相减；如原点 1e9+1 可先丢 1 单位。
- 修复：批次原点保留 f64，CPU 做 origin-camera 差值后转 f32；导航只更新 uniforms。
- 回归：1e9 附近毫米/小尺寸对象，与整体平移到原点的固定视口结果一致；不能只测 scene 相对顶点。

### B17 · P1 · 源码确认：画布坐标/DPI/Y 方向不统一

- 位置：`bridge.rs:201–204` 使用全窗口 physical_size，而 Camera 比例是 logical pixel；Slint Image 用 contain；`cad-render-wgpu:385–386` sy 为负；Web 输入又反转屏幕 y。
- 影响：DPR、工具栏高度、contain 留白导致绘制/拾取/fit 比例不一致；Y 轴约定有倒置/拖动方向风险，需非对称图和实际平台验证。
- 修复：只用实际 CAD 内容矩形与明确 world→clip→image→pointer 约定，按 DPI 换算；相机 viewport 尺寸随 resize 更新。
- 回归：非对称 L 形+坐标标记；DPR=1/2/3，旋转/resize；指针世界点、显示像素、平移方向相符。

### B18 · P1 · 源码确认：设备重建不恢复场景，错误吞掉

- 位置：`bridge.rs:188–198,205–217,219–224`。
- Teardown 清 renderer 却不清 document；Setup 创建新 renderer 后相同 db.id 不上传；build/upload/render/Image 错误未汇总，build_scene `?` 又可能因单对象失败放弃整个场景。
- 修复：以 device_generation 失效派生资源，从只读快照重传；局部失败保留对象级诊断与其他对象；错误暴露 UI，重试有界，数据库 revision/history 不变。
- 回归：Surface teardown→setup 相同文档恢复；模拟 upload/单实体失败其余可见；不无限重建。

### B19 · P1 · 源码确认 + 平台风险：后端/能力声明与真实 GPU 不一致

- 位置：Web `start:350` 调 install 默认 WebGpu；`bridge.rs:156`；renderer caps_for:233–255 用偏好推 actual，compute/storage/indirect 无真实 feature/downlevel 检测。
- Auto 仅 probe adapter，未覆盖真实 device/pipeline 初始化失败；强制失败只 console+自动 fallback；webgl2_available 会对呈现 canvas 创建上下文，有锁定 canvas context 类型的风险。
- 修复：从宿主 adapter/device 获取真实 backend/features/limits，传入 chosen；probe 专用临时 canvas，真实初始化成功才 activated；强制失败显式提示并提供选择。
- 回归：无 navigator.gpu、adapter 有但 device 失败、forced GL/GPU、受限 features；实际 adapter/report/UI 一致，GL 不报 compute。

### B20 · P1 · 源码确认：导入和表示缺失却可报告“完整/Verified”

- 位置：importer `run:223–227` 只看 import.* diagnostics，未汇总 entity completeness；`note_capability:584–607` render/pick 复制 semantic，read 恒 Verified；bridge 丢表示 completeness；CLI build-representation 仅统计 Lines 不收集 Partial/Missing。
- 影响：HATCH/ACIS opaque 等 Unverified 无必要诊断也可文档 Complete；TEXT/INSERT 已解析但实际没画，仍宣称 render/pick Verified；proxy.* 部分结果不一定降级总状态。
- 修复：读取/语义/表示/场景/GPU/拾取/测量分别判定，按对象聚合最弱完整性，不以解析成功或代码分支替代样本验收。
- 回归：含 LINE+HATCH+TEXT+无缓存代理，剩余可见，但报告明确缺失与对象来源；未运行支持状态不变 Verified。

### B21 · P1 · 源码确认：代理只保留第一个图形，来源丢失

- 位置：`cad-import-acadrust/src/lib.rs:533–538` into_iter().next()；representation 统一 GeometrySource::Analytic。
- 修复：单 entity 支持多片段，保留 proxy source/precision；普通语义已画时不得重复叠代理。
- 回归：两条 UnicodeText 记录输出都保留；未来线面 decoder 同样适用；缓存几何测量不能报解析精度。

### B22 · P1 · 源码确认：纸空间 viewport 几何和比例错误

- 位置：importer `read_layouts:287–310`。
- clip 仅三个点且第三点是模型 view_center，不是纸面矩形角；比例 view_height/paper_height 的 model_to_paper 方向需纠正；缺 view_center/target/direction/twist 平移旋转。
- 修复：自有明确纸面裁剪与视图矩阵；无法支持的视口状态保守降级，未实现反变换禁用模型测量。
- 回归：已知中心/1:100 的矩形 viewport，四角裁剪、模型与纸面距离正确；复杂裁剪明确 Partial。

### B23 · P1 · 源码确认：非均匀变换、样条与椭圆语义错误

- 位置：`cad-geometry/src/lib.rs:99–155,234–255,354–396,531–550`。
- 非均匀变换 Circle/Arc 仍为原圆类；bulge 直接清零成直线；Ellipse ratio 保持不变且离散 minor 用 cross(major,Z) 没有原法向；Spline 丢源 knots/weights 重新造 uniform knots；intersect_local 用 display_pixels 决定交点几何。
- 修复：正确仿射曲线/局部平面、有理样条与源 knots；不能精确保留时显式近似/Unsupported；交点/测量容差与显示 LOD 分离。
- 回归：圆×(2,1)成椭圆；bulge 非均匀仍弯曲；非均匀 knot/有理圆弧样条；非 XY 椭圆；改变显示 LOD 不改变测量/捕捉语义。

### B24 · P1 · 源码确认：面积工作平面投影掩盖非共面，捕捉接受背后点

- 位置：`cad-measure/src/lib.rs:222–236,171–182,66–113`。
- Plane 分支把所有点 z 置 0，未检查离平面距离，非共面输入可变成有效面积；u/v 各归一化却不保证正交；snap 用无限直线投影，负 t 点可捕捉；Distance3d/Angle 不按 Paper 空间统一拒绝。
- 修复：验证正交工作平面及共面性；ray 参数 t>=0、容差有限正值；所有算法先检查空间约束；中间/最终非有限数拒绝。
- 回归：点偏离明确工作平面、自交、斜基、后方候选、纸空间 3D 距离/角度、极大有限点溢出，均不产生虚假可靠结果。

### B25 · P1 · 源码确认：相机投影非法、标准前后视图基退化

- 位置：app `SwitchProjection:332–337`、`apply_standard_view:513–529`、pan:487–495。
- 正交 scale（世界单位/像素）乘进 FOV 可超 π；Front/Back 的 up=(0,1,0) 与视线平行；pan 不拒绝非有限 delta；reset 未完整重置 eye/up/work-plane。
- 修复：显式合法 FOV 与独立尺度，标准视图正交基，参数有界数值校验；真实 3D bridge 消费 eye/target/up/projection。
- 回归：所有标准视图正交基有效，任意合法 scale 切投影 FOV 有界；NaN/Inf pan 零状态改变；切回 2D 重置一致。

### B26 · P1 · 源码确认：Query 的文档/数据库/revision 跟踪损坏

- 位置：`cad-query/src/lib.rs:70–107,134–145`。
- update 将文档强行记 DocumentId(0)，随后 doc1 查询 Stale；只看 after 不检查 database/before；drawing 和 annotations 共用一份 revision；request revision 被覆盖而非验证；properties 无 freshness；空选择同时产生 Mixed 和 Unset 同 key。
- 修复：按 DocumentId+DatabaseId+generation/revision 分流，检测 follows 且允许明确 rebuild/reset；旧查询丢弃，新文档查询能开始；分页只构建所需行。
- 回归：layers→合法 ChangeSet→layers、双库独立 revision、乱序/漏事件/重复、A→B 查询重建、空选择只有一条 Unset。

### B27 · P2 · 源码确认：scene/GPU 增量与资源预算名存实亡

- 位置：scene `apply_changes:103–125,build:174–179,publish:185–197`；bridge build_scene:36–47；renderer upload:297–346。
- 所有更新（含 Metadata）删场景；没校验 database/revision；draw_order 固定 0；delta 缺稳定批次 ID 对应，removed_chunks 不消费；bridge 仅 build 并积累 combined，绕过预算淘汰；每实体 buffer/uniform/draw。
- 修复：稳定 chunk/batch 映射、按 mask/依赖失效、正确绘制顺序与可见性；有界构建/上传/淘汰恢复，剔除和兼容样式批处理。
- 回归：元数据零几何更新；删除无残影；图层显隐即生效；大图超预算不无限增长、不静默漏图；相机只改参数不全顶点上传。

### B28 · P2 · 源码确认：history 失败丢记录、预算低估、交易 ID 重用

- 位置：`cad-history/src/lib.rs:30–42,87–107,119–152`；host 与 AnnotationService 各有 TX_COUNTER 从 1 开始。
- undo/redo 提前 pop，DB 提交失败不恢复栈；预算只估 text 不含自由线/云线数组，merge 不 enforce_budget，redo 不纳入 used_bytes；undo/redo 复用原 transaction，双计数器也可冲突。
- 修复：提交成功再移栈；预算覆盖 heap/redo/merge，超大单记录有明确策略；新事务 ID 与被撤销原记录分开。
- 回归：无效 patch 的失败撤销保持两栈；大 freehand/merge/redo 内存有界；create/import/undo/redo transaction 标识可区分。

### B29 · P2 · 源码确认：Web 状态轮询与冒烟测试会误判

- 位置：`apps/app-web/web/main.js:75–99,116–120`；`scripts/check-web-ui.mjs:161–184`。
- 轮询等待 backend=Some，但 report 字段是 adapter=Some，250ms 定时永不降频；start_web catch 所有错误当 handoff；截图统计整个 UI，只有工具栏文字也可通过“非空 CAD”；console error 记录但未使 failed=true。
- 修复：结构化状态字段，启动失败严格区分 handoff；隐藏页停止轮询；截图只裁 CAD 区域并断言预期图形/变化；意外 console/page 错误失败。
- 回归：CAD renderer 空白但 UI 可见必须失败；真实启动异常显示错误；所有轮询清理；新增图纸/导航后截图确实变化。

### B30 · P2 · 源码确认：CLI 原子导出/诊断不完整

- 位置：`cad-cli-tools/src/lib.rs:144–148,207–216,253–291`；diagnostics redact_text:54–75。
- notes 用 fs::write 直接覆盖，失败可破坏原文件；proxy-report 漏 import.proxy_*；build-representation 忽略 Missing/Partial；自由文本脱敏不能保证去掉文本内容/带标点文件名。
- 修复：同目录临时写+原子替换/等效事务，标保存绑定输出 revision；共享结构化诊断/敏感字段白名单，保留对象关联与完整性。
- 回归：模拟写失败原文件不变；无缓存代理出现在 CLI；原图文本/文件名含空格标点/路径/身份不进入默认诊断包。

### B31 · P2 · 源码确认 + 样本风险：导入/代理限制与 CAD 转换不完整

- 位置：importer:117–134,159–168,190–195,235–243,644–659；proxy:313–329。
- SHA/bytes.to_vec/解析均同步；normalizing 无取消；max_block_depth 未落实；AcadrustImporter.proxy_limits 被忽略；代理超顶点仍 append 超限输出；厘米码 5 建模为 Meter 且比例 .01，source 元数据不实；INSERT 无块基点/OCS/阵列语义。
- 其他样本风险：SOLID 与 3DFACE 共用 quad_mesh，需核对 SOLID 四角顺序；LWPOLYLINE/2D OCS 直接当 WCS；不可把未核实格式补成猜测算法。
- 修复：分段哈希/任务 cancellation、有效预算、源单位准确；锁定 acadrust API 后基于样本归一化。超限必须显式降级且保持预算，不报 Complete。
- 回归：厘米单位已知线长、倾斜 OCS、非原点块基点/阵列、SOLID 四角、取消 normalize、低 proxy_limits、深层块；跨可信 CAD 输出断言。

## 5. 新增需求 N01：简体中文 + 英文

用户明确要求两种语言；将原规范 §3.5 的“中文默认、保留国际化结构”升级为真实双语交付。

### 5.1 当前状态

- 仅 `crates/cad-ui-slint/i18n/zh-CN.json`；ZH_CN_MESSAGES 只是 include_str，未作为翻译源。
- `UiConfiguration.locale` 没有实际应用；UI/Rust host/app/importer/JS/HTML 文案混合硬编码。
- 无英文资源、语言设置、偏好持久化、缺 key 检测、语言切换或双语布局测试。

### 5.2 下游实施契约

1. 支持稳定 locale：`zh-CN`、`en`（可接受 en-US 输入并规范化）；默认简体中文。未支持 locale 回退中文并有可诊断策略，不显示空 key。
2. 设置页提供“简体中文 / English”，运行时切换可见 UI；偏好由宿主保存并在重启恢复。切换不得丢文档、相机、工具预览、选择、批注或撤销栈；不修改 DWG 文本/用户批注文本。
3. 文案全部外置：按钮、工具步骤、单位、对话框、错误、加载、缺资源/代理提示、后端状态、HTML 启动失败和 JS 文件提示。技术稳定标识 CommandId/diagnostic.code/schema 字段不翻译。
4. 先确认 Slint 1.18.1 的真实翻译 API；选择 Slint 官方机制或统一资源 catalog 写 ADR。不得假造第三方接口。若继续 JSON，增加 `en.json`，两语言使用相同 keys 与占位符，不能只是新建文件却不接线。
5. 领域/核心返回结构化诊断 code+参数，表现层本地化；不要在 cad-domain 引入 Slint 或平台 API。原始技术错误可附于详情，但主用户提示需双语。
6. HTML lang、title、noscript/启动状态与当前语言同步；Rust UI 和 JS 不维护两份易失配的翻译真相，可由构建生成共享 catalog。
7. 数值格式化依据 UnitContext/decimal_places 与语言；未知单位中文“图纸单位”、英文“Drawing units”，不能默认为 mm。角度、长度、面积显示区分，机器 JSON 数值保持 locale 无关。
8. 保证中文可用字体/IME，英文长文案不截掉操作；缺 CAD 字体的诊断与 UI 字体处理分开，资源许可单独核验。

### 5.3 验收清单

- catalog keys/占位符一致，缺失 key 和非法格式令 CI 失败；合法回退行为有单测。
- 中文与英文覆盖全部面板、错误、对话框及 JS 启动/文件流程；开发测试禁止未登记用户文案，可对固定例外建立白名单。
- 两语言在 U01–U12 的手机/宽屏/DPI/键盘弹出场景截图与交互通过；中文组合输入不触发快捷键。
- dirty 批注+活跃工具时切语言，状态/DB revision/history 不改变；重启语言偏好恢复。
- 往返文件里的 DWG 文本、用户批注、数值、ID、schema 不因语言变化；CLI 机器输出 keys 不变。CLI 人类提示可加 locale 参数，但不是完成 GUI 双语的替代品。

## 6. 新增需求 N02：GitHub Actions workflow / runner

### 6.1 解释与范围

“github workflow runner”暂按“可在 GitHub Actions runner 实际执行的构建/测试工作流”处理，不解释为产品内置执行 GitHub workflow 的界面或服务。
已有 `.github/workflows/core.yml`，不是从零缺 CI；当前只有 ubuntu-latest 上纯核心测试/检查、排除 UI/宿主的 Wasm 检查、架构、fmt/clippy。
没有 Web 静态构建/浏览器、Android target/APK、shader 实验、双语校验、产物与 runner 能力报告。
本轮未访问 GitHub 执行记录，也未注册或购买 runner。

默认建议使用 GitHub-hosted Linux runner；**自托管 GPU/Android 真机 runner、标签、机器/费用、仓库权限、部署发布目标必须在实施前确认**。无需为了 CI 增加生产后端/账号/云上传。

### 6.2 必需工作流层次

| Job | 输入/环境 | 必须执行与输出 |
|---|---|---|
| core-quality | 固定 OS image、仓库 Rust 工具链/Cargo.lock | 核心 tests、fmt、clippy、architecture；失败必须红灯；输出测试日志，不靠 continue-on-error 变绿 |
| wasm-check | wasm32-unknown-unknown | 原规范完整 workspace --lib --locked 编译；单独 app-web 检查以便定位宿主误耦合；不能沿用排除 UI/宿主假装两端覆盖 |
| web-build | 锁定 wasm-bindgen-cli 与 crate 版本一致 | scripts/build-web.sh，上传 web-dist 静态产物；检测 .wasm 与 JS 配对、MIME/启动路径，不仅 cargo check |
| web-smoke | 锁定 Node、Playwright 和浏览器版本，静态服务 | 修正 B29 后跑浏览器；基础 WebGL2 路径用可用软件 GPU 做冒烟，注明非真机；截图、JSON、console、服务日志上传；WebGPU 无能力时明确未运行而非通过 |
| android-check | JDK17、SDK/build-tools、NDK 与 cargo-apk 锁定 | UI+host aarch64 target --locked 检查；记录版本与 env；构建脚本补 --locked，不要隐式改锁文件 |
| android-apk | 同上、CI 临时开发签名 | 产出并上传可安装 APK；记录实际 manifest package/minSdk/targetSdk/ABI/签名类型；打包不等于安装运行 |
| i18n-contracts | 双语 catalog/生成器 | key/参数/缺漏/硬编码白名单检查、格式/回退单测，双语 UI 构建 |
| shader-validation | 固定 naga/wgpu 验证工具或可用测试设备 | 所有 WGSL 离线解析/验证；纹理/GPU integration 需真实初始化证据，不能由 shader parser 代替 |

Android emulator 与 GPU/WebGPU/真机黄金图可作为分开的集成 jobs。首次未具备设备时在 job summary 明确 not-run/capability-unavailable；必需 jobs 不得静默 skip。
软件 GPU 冒烟、模拟器交互、真机兼容/性能分别记录；不把软件 GPU FPS 当移动性能验收。

### 6.3 可复现性与安全

- 工作流支持 push、pull_request、workflow_dispatch；并发取消同分支旧任务，设合理 timeout。PR 的必需 jobs 名称稳定，方便仓库管理员配置 branch protection。
- actions 固定可信完整 commit SHA 并标注版本；OS 不用无说明的浮动 latest；工具版本读取或对齐 rust-toolchain.toml、Cargo.lock、build.md。锁定 Node/Playwright，增加锁文件，不依赖个人全局安装。
- cache 按 OS/target/工具链/Cargo.lock 分开；cache miss 也必须可构建，不以陈旧 target/web-dist 为证据。
- 默认 permissions: contents: read；仅确有必要的发布 job 提权，使用 GitHub Environment 审批；不为普通构建授予 write-all。
- fork PR 不注入 signing/deploy secrets，不使用 pull_request_target 直接执行不可信 PR 代码；不让不可信 fork 在持久自托管 GPU/真机上执行。
- CI APK 用明确标识的临时开发签名；生产密钥仅在受控发布 job 通过 Secrets 获取，不入日志/artifact/cache。现有 dev-release.jks 配置不能当生产发布签名。
- 测试样本必须有 fixture hash/来源授权；禁止将用户图纸、私有字体/路径、密钥打包到公共 CI artifact。
- 构建与发布分开：APK/web-dist artifacts 属构建输出；GitHub Releases/Pages 自动部署是可选后续，需要确认目标与发布授权，不擅自上线。
- artifacts 设置 retention 与大小预算；无论成功失败尽量上传脱敏日志/截图/报告；超大 debug APK 与 Wasm 大小变化可记录，不能为体积阈值偷偷删功能。

### 6.4 Runner 验收

1. 从 GitHub-hosted runner 干净 checkout、无个人 SDK/全局 Node 包/签名文件的环境实际跑通必需 jobs。
2. 故意制造测试失败、编译错误、非法 shader、缺翻译 key，对应 job 和总检查必须失败。
3. artifact 的 commit、工具版本、样本 hash、实际后端、签名类型可追踪；APK 安装/浏览器运行若执行，有独立证据。
4. forced WebGPU 不可用、WebGL2 可用、两者都失败各有明确结果；不可用不假报 WebGPU 通过。
5. fork PR 无 secrets、自托管 runner 不执行不可信代码；敏感字段不进入 artifact。
6. `docs/build.md` 增加本地复现命令；`docs/validation.md` 写 job URL/commit/环境/通过或未运行项目；workflow 语法/lint 验证通过。

## 7. 下游执行顺序与完成判据

以下是依赖顺序与优先级，不是工期或排期，也不是允许跳过原规范其它需求。

### A. 恢复可验证基线

- [ ] 修 B01/B02/B03，核心测试、完整 Wasm、Android target 均通过；实际 APK/浏览器未运行则保持明确标注。
- [ ] 将本报告的 B/U/F 编号关联实现/tests；更新 handoff/requirements/validation，不覆盖陌生并发变更。
- [ ] 扩充 N02 core/target/build/i18n jobs，先让缺陷被稳定检测，不以修改断言掩盖产品错误。

### B. 先保护数据与显示正确文档

- [ ] B04/B05/B06/B07：文档身份与 generation、未保存决策、恢复、导出确认；复现 A→B、dirty reload、失败保存。
- [ ] B08–B13：唯一受控业务路径、权限/参数/数值/引用验证、无损 codec 和有效映射；每项先补会失败的回归测试。
- [ ] B14–B22：真实管线拓扑/块与文字/画布映射/后端能力/设备恢复/完整性；不要以 demo 有非空像素宣称真实 DWG 支持。

### C. 建产品交互闭环

- [ ] U01–U12 及 F01–F12：SAF/Web 文件、模式、图层/布局/选择/属性、测量/批注六类工具/管理、资源/诊断。
- [ ] N01 双语与上述 UI 同步完成，catalog 验证进入 CI；不是最后额外补两三个英文按钮。
- [ ] 一次确认一事务，取消零事务，撤销/重做数据/UI/索引/显示一致；程序回填不重复发命令。

### D. 工业几何、3D 与资源预算

- [ ] B23–B31 + F13–F15：数值/变换、源样条/OCS、真实 3D 投影/深度/拾取、ACIS 样本、代理实证解码。
- [ ] 增量依赖链、批处理/实例共享/剔除/LOD、分帧任务与缓存恢复；测量不依赖显示 LOD。
- [ ] 固定 OpenCADStudio commit/许可/采用映射，真实授权 fixture manifest、黄金图/跨后端/生命周期/性能报告。

### 7.1 每个实施单元的输出格式

下游每次交付必须包含：

1. 关闭的 F/U/B/N 编号与对应规范条款；修改代码/测试/文档路径。
2. 修复前失败的最小复现，修复后实际执行命令、环境、结果；静态推断与真机证据分开。
3. DB revision/ChangeSet/history/derived cache/dirty 与权限影响说明。
4. 兼容性、格式/schema migration、恢复策略及未支持范围。
5. 不满足验收的项目保持开放，返回 Unsupported/Partial/NotImplemented 而非空成功。

### 7.2 约束与待确认项

- 保留 Rust + Slint + wgpu + acadrust，不修改 acadrust 源码/不用 Cargo patch；cad-db 唯一权威，业务不调用 GPU draw。
- 不新增原图编辑、云上传、账号、生产后端、任意插件脚本执行等非目标。
- 有框架/依赖版本阻碍时写 ADR，给证据与替代方案，不能删需求或无限“待验证”。
- 需用户/维护者提供：授权 DWG/字体/黄金图与设备矩阵；是否自托管 GPU/真机 runner（及标签/权限）；是否启用发布/部署及正式签名。
- N01 默认 zh-CN+en 可直接实施。N02 默认 GitHub Actions CI runner 工作流可直接设计实现；如“runner”实际指应用内执行 GitHub workflow，必须重新确认范围，不能据本报告擅加产品功能。

## 8. 关键验证命令

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test --workspace --exclude cad-ui-slint --exclude app-android --exclude app-web --locked --no-fail-fast
cargo check --workspace --lib --target wasm32-unknown-unknown --locked
python3 scripts/check-architecture.py
# Android SDK/JDK/NDK 环境按 docs/build.md 配置后：
cargo check --target aarch64-linux-android -p cad-ui-slint -p app-android --locked
scripts/build-android.sh --release
scripts/build-web.sh
# 安装并锁定 Playwright，另开静态服务后：
node scripts/check-web-ui.mjs http://127.0.0.1:8090/ /tmp/opencode/yacr-web-audit.png
```

还需 fmt/clippy、shader/i18n 校验与新增回归测试；上述命令只是现有入口，不表示当前均成功。build-web.sh 自定义 DIST 时会 rm -rf，该路径必须严格验证为预期生成目录，不能指向源码/工作区；CI 下建议固定独立产物目录。
