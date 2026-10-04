# Linux 真实 DWG 打开后无响应排查（2026-10-04）

## 最新一轮：加载响应与预算截断处理（2026-10-04）

用户继续反馈 release 卡死、只显示一团小点/局部。本轮发现两个不同问题，不能混作一个：

1. 窗口启动后打开仍在 UI 线程读取/导入，随后在零延迟 timer 上同步 CPU 准备，并在
   `BeforeRendering` 一次复制/上传全部 GPU 批次。旧短回归只统计出图后的响应，漏了加载。
2. 默认帧预算为 800 万顶点，而该样本派生场景约 5025 万顶点。旧 planner 到预算就停，
   每次重画仍从同一前缀开始，剩余图元一直不绘制；桥接还没有用这个结果区分“局部帧”与
   “当前场景完成”。非空截图/像素改变不能证明完整提交。

### 修复边界

- Linux 桌面读取文件使用后台线程，导入走既有 `HostController` worker/命令契约。
  从读取到导入终态保留单一 slot，不因重复 Open 启动更多 parser。开始和发布之间的
  底图快照/批注修订变化使结果取消；不覆盖期间的新编辑，显式未保存决策由原宿主负责。
  `ImportJob` 放弃活任务仅请求取消并 detach，不在事件循环 join 不可中断的解析。
- native CPU 准备使用 `LatestTask`：一条 worker、一个最新待执行输入、一个完成结果。
  输入是不可变数据库/字体/批注 Arc 和层、空间、overlay 快照；旧 generation 输出不发布，
  worker panic 显式报错。相机心跳不改变内容键，旧同 ID 数据库不能冒充新输入。
  保留底图、批注、highlight 各自缓存；失败不发布空成功。WASM CPU 准备仍同步，未冒充
  浏览器 worker。退出/teardown 不等待活任务，也不继续 timer 链。
- GPU 资源仍完全由 renderer 创建/提交。增加借用批次切片的 staged upload：每片最多
  256 批次、约 262,144 顶点（单个超大片段不可抢占），每回调最多 4 片、目标约 8ms；
  不复制整份 SceneDelta。上传完成才原子替换 GPU 场景，旧版本/旧 device 的 staged
  结果不能提交。上述时间是调度软预算，不是单个 mesh/驱动调用的硬时限。
- 桥接启用显式 progressive rendering，CLI 原有单帧模式默认不变。先确定整场景的
  opaque 与透明 back-to-front 全局顺序，再按**原有**每帧预算分页；后页保留颜色和深度，
  不重复混合已提交页。相机、尺寸、场景/设备替换重启累积，单批次超预算显式失败，
  不能称空完成。没有抬高顶点预算、丢弃 hatch/虚线或使用 LOD 偷换几何。
- `current_drawing_presented()` 要求当前输入准备、原子上传、全部可见批次提交并实际渲染；
  旧 demo 帧/预算前缀不能通过就绪门。UI 通过双语目录区分准备、上传、分帧绘制，提交
  完成之前不显示“已打开”。“完成”指已准备场景的所有可见批次，不保证导入支持了全部原图。
- 精确拾取将 ray/options 的重复验证移至外层，compound 子项复用已验证参数；保留
  closest/tie、Unsupported 和子元素语义。没有引入未经验证的空间裁剪。

### 本轮实际证据（不是旧截图重用）

同一仓库外 DWG、真实 Wayland 桌面/default Slint GPU，窗口启动后经注入文件选择结果打开。
计时期间没有并行编译/其他 GPU 回归。测试不是实际 portal 文件选择，也不是长期手工验收。

| 桌面回归 | 当前场景全页完成 | 加载阶段最大事件间隔 | 点击 | 全程最大间隔 |
| --- | --- | --- | --- | --- |
| debug | 27.99 秒 | 391ms | 1.92 秒 | 1.97 秒 |
| release | 19.97 秒 | 309ms | 1.49 秒 | 1.51 秒 |

两次都记录 **97,013 / 97,013 可见批次**提交完成、30 个后续回调、27 CAD frames、滚轮相机
变化和 24,905 个 CAD 像素改变。最终 screenshot 阅读显示纵向多组平面图、右侧图形/表格，
不再只见局部一团。全图 fit 涵盖所有模型范围，因此单组平面图仍较小，可继续放大。
**未取得参考 CAD 截图逐项对照**，不能称全图视觉、所有原始图元/字体/比例验收通过。

证据留在 `/tmp/opencode/yacr-responsive-visual-{debug,release}-final/` 与同名前缀 `.log`，
含用户图纸截图/坐标，**不得提交仓库**。Slint 选中 vendor 未单独记录，桌面结论不推广
为厂商兼容；另两个小型 renderer GPU 合成契约实际报告 NVIDIA RTX 3070 Laptop/Vulkan
并通过：有限预算的多页像素与完整单帧一致（含透明 mesh），以及超大批次显式失败。

合成契约已跑：background 3/3、import tasks 14/14（含批注编辑 guard 契约）、UI bridge
20/20、spatial 27/27、上述 GPU 契约 2/2；不是完整 workspace 测试。主机/wasm 编译、
全树严格 clippy、fmt 与静态门禁已通过。最终 guard 修改后再次通过 tasks、严格 clippy
与 wasm 编译，并重建 `target/debug/yacr-linux`、`target/release/yacr-linux`。桌面计时回归
发生在最后的 guard/取消按钮状态小改动之前，未在最终二进制上再次进行手工操作验收。

仍未验收：真实 picker 点击、大图加载取消的全时序/资源释放、连续打开第二张大图、
长期交互/内存上限、超大单 mesh 的硬时限、完整参考图视觉、Web/Android 运行。
debug 点击接近 2 秒阈值，不宣称所有交互流畅。最终 CPU/GPU worker reviewer 在本 harness
不可用，因此没有独立专家审查通过的证据。

## 前轮历史：手工 release 问题未闭环

用户反馈 debug 可打开、release 仍卡死，因此前轮短时 `--open` 回归不是解决问题的充分证据。
本轮增加窗口先启动再调用打开入口（注入文件选择结果，不等于真实 portal 操作）、鼠标移动和点击选择。

- 基线 release 窗口启动后打开，事件循环间隔 **16.997 秒**，2 秒响应契约失败；当时
  “ready=3.74 秒”仅由帧数判断，可能是旧 demo 帧，**不得引用为新图就绪证据**。
  当前测试在打开开始即计入导入/场景准备的回调间隔，并明确标记帧数门槛不是新场景 freshness 验收。
- CPU-only 真图测量（无 GPU，不等于桌面出图）：导入 3.20 秒、fit 0.328 秒、场景
  17.59 秒，其中展开 14.24 秒；44,591 个数据库实体、96,521 个批次、50,251,439 顶点。
- 增加显式 `LineSegments` 独立端点对；不透明虚线可一次生成多个段，分为最多 65,536
  顶点且局部相对坐标受限的片段，避免每条 dash 拷贝路径和样式。透明图元仍按原次序逐 run
  生成。变换、布局裁剪、场景、highlight、vector export 和 CLI 统计均识别新类型。
  原 provider 默认仍为逐 run API。无细节降级/LOD/隐藏图元捷径；超出 packed 展开上限时
  显式 `Partial` 连续线回退，而不是截断后称成功。
- 虚线弧长建立一次索引，避免每个 dash 从第一个顶点重扫。CPU 场景测量约 **14.0 秒**，
  97,012 个批次、50,251,439 顶点；顶点数未减少不能单独证明视觉一致。
- 点击前收集约 723,587 个 pick item，旧路径深拷贝所有几何。改为只读借用并复用同一
  precise-pick 循环，保留 nearest/tie、实例身份和 Unsupported 报告。debug 收集从
  1.85 秒降至 0.20 秒；整次点击从约 4.29 秒降至 **2.05 秒，仍失败**。
- 优化后 release `--open` 桌面短回归：首次帧 **17.68 秒**、点击 **1.58 秒**、30 callbacks、
  最大间隔 1.60 秒、4 CAD frames、11,529 个改变像素，契约通过。此结果**排除初始同步加载**，
  不等于手工问题或所有交互已修复，也不证明整图视觉正确。无强制软件 ICD，选中 vendor 未单独记录。

本轮仍未实现异步 scene 准备、保守空间 broad phase 或增量 GPU 上传；不能把“减少耗时”
说成“加载期间响应”。下一步应围绕不可变输入快照、单 worker+latest pending、取消/过期结果拒绝、
prepared→presented freshness、GPU 上传背压设计；现有 ImportJob Drop 会 join，不能直接
把取消按钮接上就宣称无阻塞。未运行真 portal 点击、长期手工使用、完整视觉验收。

本轮临时生产计时输出已移除，保留显式 ignored CPU/桌面探针；未提交原图或截图，未 commit/push。

### 本轮最终门禁与失败保留

- 加强加载 heartbeat 后的最终 release post-start 回归**失败**：打开开始后首个 callback
  间隔 **3.04 秒**，在同步导入阶段即超过 2 秒 sentinel，测试随即退出，**没有测到此轮
  后续 CPU/GPU 阶段**。不能把它与基线 17 秒直接比较为优化收益。
- 合成单元契约：geometry 61/61、representation 142/142、scene 68/68、spatial 27/27
  通过；另跑 cad-app picking 定向契约。新增测试覆盖 packed 可见段/角点/重复点/无连接线、
  无效参数和展开上限、来源/样式、vector export、borrowed nearest tie 与 Unsupported。
  这是合成契约，不是完整视觉、真机或 vendor 验收。
- 主机 workspace all-targets debug check（含 Linux/Slint）、完整 workspace wasm lib check、
  定向严格 clippy、fmt、架构/fixture/workflow/i18n 与 diff 检查通过。
- 全 workspace 严格 clippy 仍被未修改的 `crates/cad-app/tests/select_all.rs:152`
  `err_expect` 拦截；没有修改无关测试或降低 warning 门禁。完整 workspace 测试、
  Linux 软件 GPU 离屏、Web 浏览器、Android 与完整视觉验收均 **NOT RUN**。
- 本轮 debug/release `yacr-linux` 二进制均已重新构建；编译产物不是运行或视觉验收证据。

## 范围与结论

本轮环境为 **Wayland 桌面窗口**，不是无头平台；`DISPLAY=:0`、
`WAYLAND_DISPLAY=wayland-0`。`vulkaninfo --summary` 检测到 NVIDIA GeForce RTX 3070
Laptop GPU（驱动 580.178.04）与软件 llvmpipe。本轮桌面回归使用 Slint 默认设备选择，
不设置 lavapipe ICD，也不安装或修改桌面、驱动与系统配置；未单独采集 Slint 选中设备的
vendor 标识，因此不将本轮结果推广为 NVIDIA 驱动兼容性结论。

输入为用户提供的仓库外 DWG，SHA-256：
`0d77832ee8eeb4d4303133b62b84f5b9769c4171f56dfc373c19fad0ec3633eb`。
不提交原图、截图、完整路径或文本内容；这不是可再分发 fixture。

**已建立的证据**：debug/release 桌面宿主打开该图后出图，出图后的事件循环持续运行；通过
Slint 窗口事件派发滚轮后，相机和 CAD 区域像素改变。
**尚未建立的证据**：加载过程中持续响应、整图视觉正确、所有图元都已出图、长期交互
稳定性、其他图纸/平台/vendor 兼容性。首次 CPU 准备和 GPU 上传仍同步占用事件循环，
不能将本次修复称为无卡顿异步加载。

## 原因与修复边界

1. 初始 debug 运行约 30 秒时 RSS 接近 5.8 GB，主线程持续消耗 CPU。GDB 栈落在
   `DefaultRepresentationProvider::build_inner` 的 `entity.clone()`：复合实体每个子图元
   都先深拷贝包含全部兄弟图元的父实体，产生平方级重复复制。
   改为保留只读实体身份、按引用递归子几何；子图元顺序、来源、完整性和诊断不变。
2. 修复复制后，GDB 在模型空间场景构建末尾读取到 **20,842,742 个批次**。每个虚线
   run/填充线分别分配 GPU buffer/uniform/bind group，造成大量 CPU/GPU 资源分配。
   宿主场景改用 `SceneCache::build_compact`，将相邻、同样式的不透明折线转为真正的
   line-list，再局部合批；折线之间不产生连接线。来源保留为批次的 `SelectionRef` 列表，
   不将 GPU 索引替代数据库或实例身份。
3. 合批不跨 mesh/未支持图元边界，不跨样式边界，不合并透明片段（避免合成一个深度键）。
   每次合并受 65,536 顶点和 8,192 世界单位局部坐标范围约束，避免无限大批次与远距离
   重基址的严重精度损失。单条超长折线的独立分块仍未实现；这是有界合并，不是完整空间
   分块/LOD。旧 `SceneCache::build` 契约保留，宿主的 model/paper 构建采用紧凑路径。
4. 没有修改 acadrust、重新解析底图、改变测量/数据库几何、提高 GPU 预算、隐藏填充/
   虚线或将失败改成空成功。现有 renderer 的帧预算仍可能限制大图出图；桌面 bridge
   尚未完整向 UI 传播 `FrameStats::over_budget`。像素变化不能证明整图完整显示。

## 回归与证据

新增 6 项合成契约：

- representation：20,000 子线段顺序与父实体选择身份；嵌套复合实体的不支持诊断。
- scene：折线完整线段与无跨折线连接；40,000 段的批次上限与重复来源压缩；样式/透明
  边界；大坐标与未支持图元边界。

合成测试实际执行：`cad-representation --lib` **139/139**，`cad-scene --lib` **66/66**。
`cad-app --lib` **293 passed / 2 failed**：
`layer_override_does_not_touch_the_database`（unknown layer 3）与
`restore_layers_clears_overrides_without_touching_the_database`（unknown layer 0）。
本轮未触及这两个测试及其图层命令逻辑，不将该运行记录为全部通过。

新增默认忽略的桌面回归（必须独立进程运行；需要桌面和外部输入）：

```bash
export YACR_TEST_DWG=/absolute/path/to/external.dwg
cargo test -p app-linux --lib desktop_dwg_event_loop_and_navigation_remain_responsive \
  --locked -- --ignored --nocapture
cargo test -p app-linux --lib desktop_dwg_event_loop_and_navigation_remain_responsive \
  --release --locked -- --ignored --nocapture
```

该测试运行生产 `LinuxApp` 与真实 Slint 桌面事件循环，不安装 offscreen 平台。首个 CAD
帧之后记录 20 个定时回调，间隔须低于 2 秒；第 10 个回调截图并派发滚轮，第 20 个回调
验证相机、CAD 帧数和 CAD 区域像素。截图读回耗时从事件延迟检查中排除，但滚轮同步
处理和随后的 CAD 重绘不排除。像素仅保留在测试内存中。

- 最终 debug：实际通过；首帧后首次定时回调距构造开始 24.54 秒，总耗时 27.15 秒，
  20 回调，最大事件间隔 421.84 ms，3 CAD frames，CAD 改变像素 11,529。
- 最终 release：实际通过；首帧后首次定时回调距构造开始 26.84 秒，总耗时 30.25 秒，
  20 回调，最大事件间隔 757.41 ms，3 CAD frames，CAD 改变像素 11,529。
- 上述是本环境短时 smoke，不是建议交互性能预算的完整验收；定时回调阈值 2 秒是
  卡死回归哨兵，不代表交互已达到流畅标准。debug 同期有 release 编译，release 同期
  有静态编译门禁，因此不比较两种 profile 的性能。较早的 debug 同期编译运行耗时
  47.15 秒，也保留为首次加载仍显著阻塞的证据。
- 默认完整 workspace 测试、Linux lavapipe 离屏验收、Android、Web 浏览器、参考图视觉
  验收与长期压力测试：**NOT RUN**。wasm 编译门禁与桌面运行结论分别记录。

## 构建门禁

fmt、架构、fixture manifest、workflow、i18n 已实际通过。全主机严格 clippy 在
`crates/cad-app/tests/select_all.rs:152` 的 `.err().expect(...)` 被 `err_expect` 拒绝；
本轮未修改该文件，未将失败门禁记为通过。
定向严格 clippy（representation/scene/app-linux 全目标）、主机全 workspace 全目标
debug 编译（排除 Android/Web App，含 Linux/Slint）、wasm 全 workspace lib 编译和最终
`yacr-linux` debug/release 二进制构建均实际通过。首次 release 构建在 120 秒编译时限
中止，取消时限后重跑成功；中止不计为构建通过。全树严格 clippy 的上述失败仍保留。

## 未完成

- 导入、CPU 场景准备和 GPU 上传仍同步执行；需要独立设计任务取消、过期结果拒绝、
  背压和分帧上传，不能用移到另一个零延时 UI timer 伪装后台工作。
- 本图展开后仍有数千万顶点；需要可解释的空间分块/显示 LOD、内存与帧预算诊断，
  不改变权威几何或测量精度。
- 单条超长折线分块、长期桌面操作与参考图视觉验收未闭环。
