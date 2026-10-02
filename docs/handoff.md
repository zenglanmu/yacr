# 后续 agent 接手入口

1. 阅读规范、AGENTS.md、requirements.md、architecture.md 与最近 Git diff。
   此次恢复中发现并发实现与提交，结束时可能仍有其他实现工作；本文描述的是框架交接，
   请以当前源码和最新构建证据确认状态，不覆盖陌生变更。
2. 运行 README 的 check/test/architecture 命令；不要删除陌生的已有实现。
3. 用 `rg 'pending\(' crates apps` 查找显式占位；文件路径与字符串标识对应实施单元。
4. Android 的 Slint + wgpu 组合已验证到打包层（ADR 0002、docs/validation.md）；
   本轮进一步在无头模拟器（KVM + SwiftShader，x86_64）实际安装、启动、渲染并验证
   画布平移/适应（docs/validation-android.md）。**真机仍未运行**。Web 已可运行：
   wasm 静态产物在无头 Chromium 以 WebGL2 通过加载/导航/双语（docs/validation-web.md）；
   **WebGPU 与真实 GPU 未验证**。
5. 逐项实现 Builder/事务/历史 → 导入/基础语义 → 显示/索引/场景 → 应用/工具 → 宿主闭环；
   不把这个依赖顺序当作排期，也不绕过优先 GPU/UI 验证。
6. 每次移除占位同步补齐规范测试、能力表、构建说明、Git 提交。

## 本轮状态（compact，2026-10-02）

四个并行 workstream 已合入 main 并验证：

- **Android 运行闭环**：x86_64 release APK 在无头模拟器安装/启动/渲染；修复画布输入
  未接线（现在单指拖动平移、滚轮/捏合缩放）、初始状态文案、后端日志；像素 diff 证明
  平移生效。证据 `docs/validation-android.md`（含截图/日志）。
- **Web 运行闭环**：修复桥固定 WebGPU、Slint 缺 `renderer-femtovg-wgpu`、wasm 轮询
  误判设备丢失、WebGL2 MSAA present 失败；`web-dist/` 在无头 Chromium 以 WebGL2 通过
  加载/导航/双语，语言偏好持久化；B29 冒烟脚本修复。证据 `docs/validation-web.md`。
- **3D / 纸空间宿主接线**：bridge 按 `SpaceSelection` 与视图模式分派 `render`/`render_3d`，
  UI 提供 2D/3D、投影、标准视图、拖动轨道与布局选择；退化/不支持视图显式诊断。
  宿主编排者已补 `CadView::sync_session`（空间+相机+模式）。`docs/view-3d.md`。
- **透明度与代理**：acadrust `Transparency`（ByLayer/ByObject/ByBlock）经
  `DisplayFragment.alpha` 进入透明管线；代理保留全部片段与真实来源/精度。软件 Vulkan
  透明合成测试通过。`docs/render-order.md`、`docs/proxy-support.md`。

核心测试 **573 passed / 0 failed**；Wasm、Android target、fmt、clippy(0)、架构、
`check-i18n.py`（116 keys）、`check-fixture-manifest.py`、`check-workflows.py` 全通过；
`cad-render-wgpu` 在 lavapipe 下 47 passed（含透明合成）。

**集成轮 2（核心 CAD，Linux 构建）**：曲线升级为真实 NURBS + 椭圆 OCS 法向 +
仿射真椭圆 + 解析交点（`docs/curve-geometry.md`）；F15 ACIS 打通
acadrust SAT/SAB → 中性 B-rep → 平面/球/柱/环面子集离散（`docs/kernel-acis.md`，
合成夹具入 manifest）；纸空间 4 角视口/正确比例 + 空间感知测量（`docs/layouts.md`、
`docs/measure.md`）；对象捕捉六类 + HATCH 多环含孔洞填充。核心测试 **667 passed /
0 failed**。

**集成轮 3（核心 CAD，Linux 构建）**：实体颜色（ByObject/ByLayer/ByBlock，ACI/RGB）与
线宽（mm）进入渲染（`docs/entity-style.md`，线宽显式不绘制）；MTEXT 格式 run 解析与
整形（`docs/mtext.md`，堆叠分数/颜色/装饰为显式 Partial）；导入 4 角纸空间视口/完整变换/
INSERT/OCS/SOLID（闭合 round-2 `docs/layouts.md` §3.1）；网格面 `SubElementId` 拾取 +
选择高亮叠加（`docs/picking-3d.md`）。核心测试 **735 passed / 0 failed**，软件 Vulkan
**50 passed**；fmt/clippy/架构/i18n/fixtures/wasm `--lib` 全通过（仍只构建 Linux 核心，
未构建 Android/Web）。仍开放：ACIS 真实授权样本与锥面/带环球面、LINETYPE 虚线、渐变
HATCH、动态块求值、注释性缩放、打印/出图、异步可取消导入与进度、性能基准与预算实测。

**集成轮 4（核心 CAD，Linux 构建）**：LINETYPE 虚线端到端（`docs/entity-style.md`）；
纸空间布局出图到 PNG + `PLOTSETTINGS` 导入（`docs/plot.md`）；性能预算
（`cpu_bytes`/`queued_tasks`/`upload_bytes_per_frame`）真实计费 + 可复现 benchmark
（`docs/performance.md`）；渐变 HATCH 逐顶点颜色烘焙（`docs/hatch-gradient.md`）；
异步可取消导入（进度 + 过期结果丢弃，F01）（`docs/import-async.md`）。集成测试
**824 passed / 0 failed**（lavapipe），fmt/clippy/架构/i18n/fixtures/wasm `--lib` 全通过。
仍开放：ACIS 真实授权样本与锥面/带环球面、动态块求值、注释性缩放、矢量出图
（PDF/HPGL/CTB）、复杂/嵌入形状线型、宿主异步导入面板接线、手机内存/FPS 实测。

仍开放（受环境/外部依赖限制）：**F15 真实 ACIS 离散**（需内核 + SAT/SAB 解析器 + 授权样本）；
**真机**（模拟器 SwiftShader 不等同真机）；**WebGPU / 真实 GPU / 移动与桌面浏览器矩阵**；
Android surface 尺寸/安全区（U07）、SAF、量测/批注拾取与面板状态推送；自托管 GPU/Android
runner；**授权 DWG/字体/黄金图**（`fixtures/manifest` 仍无授权样本）；MultiLeader；
复杂文字整形；自动保存/崩溃恢复保留策略。详见各功能 `docs/*.md` 的"未完成"。

## 已定义，但尚需设计审查

- RenderTarget/HostTexture 当前仅为自有数字 token；共享 Device/Queue 的拥有者、
  token 生命周期与安全访问方式须在平台合成 ADR 中固定，不可凭 token 猜测 GPU 对象。
- AnnotationId(u128) 只表达 UUID 位宽；生成/碰撞防护由宿主服务和数据库校验完成。
  JSON UUID 编码、时间格式、extension 原始 JSON 校验尚未实现。
- FingerprintPolicy 的显式策略不意味着导入已经能自动建立锚点；fingerprint 计算需分段/异步。
- CommandPayload 的类型校验、防止 ID 与 payload 不匹配、事务权限与撤销能力仍需接线。
  当前 Application 提前检查模式与文档身份，但 Work 模式执行仍返回 NotImplemented。
- RepresentationProvider 的字体/样式上下文目前只提供最小配置，需要扩充自有只读 views。
- DB 中已有基础实现应保留并复查几何边界/块实例/验证完整性，不代表工业语义已验收。
- 数据库/几何已有基础真实实现与合成测试，代理也已有有界 framing/记录回放代码。
  这些是在恢复过程中保留的新增实现；需审查实际支持范围，不能据此宣称真实 DWG/工业兼容。
- 扩展点包含 Importer/Tool/CommandHandler 等；注册/优先级/冲突策略需统一实现。
- PropertyProvider、字体 shaping、缓存容量、Worker 二进制传输、Journal codec、schema migrations
  都需要完整接线和验收，不允许以现有结构体宣称闭环。

## 接线验收

业务不调用 draw；批注确认一次事务，取消零事务；UI 回填不触发重复命令；
revision 不连续重建；GPU 重建不改数据库；设备失败和退出保护未保存批注；
局部失败保留对象级报告；未知源单位显示图纸单位。Android/Web 真机与浏览器分开记录。
