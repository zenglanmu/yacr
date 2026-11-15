# 逐轮交接（handoff）

> **用途**：只记录**最近一轮**的工作事实——改了什么、证据跑到哪、哪些还没闭环，给下一轮接手的人。
> 新条目加在「最近一轮交接」之上（新 → 旧）；每条都要写清证据边界（实跑 / NOT RUN / NOT VERIFIED）。
>
> **历史归档**：`2026-11-12` 及更早的逐轮条目已迁到
> [`docs/handoff-archive/2026-11.md`](handoff-archive/2026-11.md) 与
> [`docs/handoff-archive/2026-10.md`](handoff-archive/2026-10.md)，原文保留、各带条目索引。
> 其它文档里「见 `docs/handoff.md` 的 2026-10-17 条目」这类**按日期引用**现在落在对应月份归档里，
> `grep <日期> docs/handoff-archive/*.md` 即可定位。
>
> **不在这里重复**：平台验收状态与证据边界以 `AGENTS.md` 平台表和 `docs/compatibility.md` 为准；
> 架构不变量见 `docs/architecture.md`、`docs/core-invariants.md`。

## 当前状态（截至 2026-11-15）

**版本与产物**：工作区 `0.1.13`，tag `v0.1.13` → `7600b316`，Release 与落地页 `v0.1.13` 已发布
（11/11 job 绿）。tag 树里 `rvt` 仍是 0.17.0，`main` 已把补丁依次 rebase 到 **0.22.0**
（`ed14d147`）再到 **0.24.0**（本轮）——**tag 产物与 main 源码不完全同源**。

**载体**：一棵 React 树 + 多种载体（`apps/desktop-ui` 是唯一 React UI 包）；`native-below`
（Linux 原生 Wayland、Windows、macOS、Android）由 native 会话持有 `cad-db`，`dom-canvas`
（X11/XWayland、Web）由 wasm 会话持有。细节与逐平台证据边界见 `AGENTS.md` 平台表。

**最近完成（2026-11-13 → 2026-11-15）**

- 3D orbit 灵敏度基础值 0.008 → **0.0035 rad/逻辑像素**，并新增设置
  `interaction.orbitSensitivityPercent`（25–200，默认 100，Options → Drafting「三维旋转灵敏度」，
  鼠标与触控共用同一倍率；ViewCube 经 `cad_status.navigation` 取同一值）。真实手感 **NOT VERIFIED**。
- `vendor/rvt` 透明度补丁由 0.22.0 重挂到 **0.24.0**（本轮）：上游 `src/ifc/gltf.rs` 只多了
  material 索引越界过滤，已合并进我们的选择链；`SHA256SUMS` 209 个文件重生成；
  台账 `docs/rvt-patches.md` / ADR-0035 / `THIRD_PARTY_NOTICES.md` / `AGENTS.md` 同步。
  证据：`scripts/test-rvt-patch.sh` **7 passed**、`cargo test -p cad-import-rvt --locked`
  **33 passed**、本地 `rmeadvancedsampleproject.rvt` 导入 `explicit_opacity = 746`
  （与 0.22.0 补丁同值），逐像素/GPU 与 Windows/macOS/Android/Web 运行时本轮 **NOT RUN**。
- 第三方依赖补丁级升级（本轮，2026-11-15，与上面的 `vendor/rvt` 0.24.0 同一轮）：
  - STEP 解析器 `cadmpeg-codec-step`/`cadmpeg-ir`/`cadmpeg-tessellate` **0.6.11 → 0.6.12**
    （workspace 直接依赖，三处版本串 + 注释同步）。0.6.12 在 IR 里新增 feature-tree operation
    `insert_components`/`component_transfer`（`IR_VERSION` 不变）：本仓库**不消费**
    `cadmpeg_ir::features`（`cadmpeg-codec-step` 0.6.12 也不解码 AP242 feature tree），所以新变体
    到不了 `cad-import-step`；本 crate 唯一的穷尽 IR `match`（`placement.rs` 的 `OccurrenceParent`）
    没有 `_` 兜底，新增变体会是**编译错误**而不是静默丢弃（已写进 crate 模块文档）。0.6.12 的三条
    STEP 侧改动（pole-bounded spindle torus 导出、保留整块闭合曲面、在表示不确定度内保留开放边）
    在现有夹具上**不改变**结果：`cargo test -p cad-import-step --locked` **13 passed**，断言未放宽。
  - 精确钉住：`wgpu` 30.0.1 → **30.0.2**（`naga` 30.0.1 → **30.0.2**，必须成对；同一补丁轨的
    `wgpu-core`/`wgpu-hal`/`wgpu-types`/`naga-types` 也一并 30.0.1 → **30.0.2**——`wgpu` 30.0.2 对它们
    只要求 caret `30.0.0`，但 30.0.2 的修复实际落在这些子 crate 里，半升会留下 30.0.1 的运行时；
    升后 `cargo check --workspace --all-targets` 与 `wgsl_validation` 12 passed 复跑通过）、`tauri`
    2.12.3 → **2.12.5**、`tauri-build` 2.7.2 → **2.7.4**（runtime 家族解析值不变：tauri-runtime
    2.12.2、tauri-runtime-wry 2.12.3、tauri-macros/codegen 2.7.4、tauri-utils 2.10.2、tao 0.37.1、
    wry 0.57.0、webview2-com 0.39.1）、`objc2` 0.6.4 → **0.6.5**、`wasm-bindgen` 0.2.129 →
    **0.2.130**（连带 `wasm-bindgen-futures` 0.4.79 → **0.4.80**、`js-sys`/`web-sys` 0.3.106 →
    **0.3.107**）。
  - caret 内只动 lock：`rstar` 0.13.0 → **0.13.1**、`glam` 0.33.11 → **0.34.3**（manifest `"0.33"`
    → `"0.34"`；只被 `cad-proxy`/`cad-render-wgpu` 声明，源码零改动。lock 里仍留着 `glam 0.33.11`
    这一条，它是 `nalgebra` 的可选 feature 边，与同条目里的 0.30/0.31/0.32 同性质，`cargo tree -i`
    已解析不到）、`encoding_rs` 0.8.42 → 0.8.44、`cc` 1.5.1 → 1.11.2（连带 `find-msvc-tools`
    0.1.14 → 0.1.15）、`log` 0.4.34 → 0.4.35、`flate2` 1.1.10 → 1.1.11、`serde_json` 1.0.151 →
    1.0.154。
  - 机械升级：`sha2` 0.10.9 → **0.11.1**（`Digest`/`Sha256::new/update/finalize` 用法不变；
    `tauri-codegen` 仍依赖 0.10.9，锁里因此有两个 `sha2`）、`pollster` 0.4.0 → **1.0.1**
    （`block_on` 不变）。
  - wasm 工具链同步：`scripts/lib/wasm-toolchain.sh` 的 pin、`build.yml`（2 处）与 `core.yml`
    （1 处）的 SHA-256 分支改为 0.2.130；sha256 =
    `9687f3bb551b9f66f37ed32a40d7913a00f7962db653054e56857bc306c3227f`，**本轮从官方 release 资产实测**
    （`curl -fsSL .../wasm-bindgen-0.2.130-x86_64-unknown-linux-musl.tar.gz | sha256sum`，11,860,397 B），
    不是沿用旧值或编造。`docs/ci.md`、`docs/build.md`、`README.md`、`README.zh-CN.md` 的 0.2.129 说法同步。
  - **明确不升**：`uuid` 1.29.2（MSRV 1.89）、`rstar` 0.14.0（MSRV 1.90）、`ashpd` 0.14.0
    （MSRV 1.89 + 0.12→0.14 portal API 破坏性）、`gtk`/`gdk`/`gdkx11`/`gdkwayland-sys` 0.19
    （MSRV 1.92，且必须与 tauri 解析到的 0.18 家族一致）、`jni` 0.22.4 / `android_logger` 0.15.1
    （Android-only，本机无法编译验证；jni 0.22 是破坏性重写）、`getrandom`（`cad-import-acadrust`
    精确钉 `=0.3.4`，受 acadrust 约束）。workspace 的 `rust-version = "1.88"` 未变：`tauri` 2.12.5
    与 2.12.3 一样声明 1.90，不是本轮引入的偏差。
  - **NOT RUN**：整仓 `cargo test --workspace`；Android（`aarch64-linux-android`）/Windows/macOS
    目标只到 `--workspace --all-targets` 的 cfg 层面（未交叉编译）；wasm-bindgen CLI 0.2.130
    **本机未安装**，因此没有跑 bindgen 后处理、`wasm-opt` 与 bundle 构建/冒烟；真实 GPU/像素/性能
    证据未重跑（`wgsl_validation` 是离线 WGSL/naga 校验，不是渲染证据）。
- DGN V7/V8 矢量导入 lane（`cad-import-dgn`，`cadkit-dgn =0.5.2`，合并提交 `7e9ff9f4`）。
- 插件 API 可用：写通道 + 实体读 + 装载/命令派发 + 异步提示通道 + `documentManager`/`revision`
  （`ts-api` 工作树合并；ADR 0034）。
- 多格式导入接入 `.skp` 预览 lane；OS 文件关联注册完整 importer 集（单源 `formats.json`）。
- canvas ViewCube + `cad_orbit` + 跨载体 pinch；右侧导航坞改为浮动覆盖。
- `[probe] status` 1 Hz 证据行改为默认关闭（`YACR_PROBE_CAD_LOG` 存在性门禁）。

**未闭环 / 下一轮候选**（都能在下面条目里找到出处）

1. **DGN 命令面仍是具名拒绝**：`DGNIMPORT`/`DGNATTACH`/`DGNCLIP`/`DGNLAYERS`/`DGNADJUST`/
   `DGNEXPORT`；level display/freeze/lock、references/xrefs、Item Types/XAttributes、
   dimensions/multiline/mesh/solid/surface 未建模；无 DGN 导出。真实样本只有 GDAL 的 V7/V8
   （ODA Teigha 写出），V8i/CONNECT/ORD 与真实 MicroStation 未验证。
2. **插件 API 还差交互式能力**：能力表 8 supported / 10 pending（`editor.get*/selectAll/
   writeMessage`、`document.lock`）；插件安装 UI/持久化商店、Worker/权限/签名隔离未做；
   M2–M5 的边界见 ADR 0034。
3. **跨载体证据缺口**：Windows 真实 spike 仍 PENDING、macOS 只有编译证据、Android 设备运行
   NOT VERIFIED、Web 冒烟是 SwiftShader；真实 GPU 的像素与性能一律未验收（见 `AGENTS.md` 平台表）。
4. **门禁只覆盖改动范围**：DGN 合并后只跑了相关 crate 测试 + `app-web` wasm check，
   整仓 `cargo test --workspace` 与全量门禁 **NOT RUN**。

## 关键设计索引（不在此重述，按需点开）

| 主题 | 唯一权威 |
|---|---|
| 载体与合成（`native-below` / `dom-canvas`） | ADR 0010、0023、0024、0027；`AGENTS.md` 平台表 |
| 增量更新与渲染（ChangeSet → 失效 → 显示表示） | `docs/architecture.md`、`docs/core-invariants.md`、ADR 0007、ADR 0012、`docs/render-optimization-backlog.md` |
| 光标/合成层（transient overlay、base/overlay 分离） | ADR 0028、ADR 0023 |
| 设备协商与 GPU 预算 | ADR 0013、ADR 0026 |
| 多格式导入（DWG/DXF/RVT/IFC/STEP/OBJ/STL/PLY/SKP/DGN） | ADR 0033、`docs/compatibility.md`、`docs/dgn.md` |
| 依赖补丁策略 | ADR 0031 + `docs/acadrust-patches.md`；ADR 0035 + `docs/rvt-patches.md` |
| 插件（厂商 ABI、拾取/属性、TS API） | ADR 0029、0030、0034、`docs/plugin-api.md` |
| 命令面与官方清单 | `docs/commands.md`、`docs/autocad-2026-command-coverage.md` |
| 宿主 I/O 与文件 | `docs/host-files.md`、`docs/react-linux-app.md`、`docs/windows-app.md`、`docs/macos-app.md`、`docs/android-host.md` |
| 证据边界 | `AGENTS.md` 平台表、`docs/compatibility.md`、`docs/validation*.md` |

## 最近一轮交接（2026-11-13 → 2026-11-14）

> 新 → 旧。标题带日期与主题；每条末尾写证据边界（实跑 / NOT RUN / NOT VERIFIED）。
> 再往前（`2026-11-12` 及更早）的条目见 `docs/handoff-archive/`。

## 2026-11-15 晋升计划 P4/P5：`LAYDEL`/`LAYTHW`/`RENAME`/`LAYMRG`/`LAYOUTWIZARD`/`LENGTHEN`/`FILLET`/`CLIP`/`CLIPIT` 升为完整实现（真机 GUI 证据）

- **范围**：覆盖清单「部分实现 → 完整实现 晋升计划」的 **13 个命令名**晋升为**完整实现**
  （总计 **107 → 120 完整 / 872 → 859 部分**，981 条总数不变）：
  - `LAYDEL`/`-LAYDEL`：删除范围从「模型空间实体」扩到**模型空间 + 每个图纸布局 + 每个块定义内**的全部实体
    （单一 `entities` 表按 `layer` 过滤），并剪除各图纸视口该层的 `layer_overrides` 键；选择输入也接受
    **图纸空间顶层实体**（块内成员/子元素按 `laydel.block_space` 拒绝）；实体补丁 + 布局补丁 + 层行
    一条事务/一步撤销，重做再次删除。`laydel.layer_in_use` 拒码删除；锁定层仍由 `layer.locked` 拒绝且不发布。
  - `LAYTHW`：同时解开绘图级 `frozen` 与**每个图纸布局、每个视口**的冻结覆盖，保留显式 `false` 覆盖、
    视口裁剪/矩阵与 `frozen_in_new_viewport`；层补丁 + 布局补丁一条事务/一步撤销。旧文档「本构建无视口模型」
    是**陈旧借口**——先写测试再改实现时两个新测试确实失败。
  - `LAYMRG`/`-LAYMRG`：合并时把源层从**每个图纸视口的 `layer_overrides`** 里剪除（此前留下指向已删层的死键、
    `layout_patches` 硬编码为空），与实体重写、层行删除同一条记录/一步撤销；交互 Settings 属全局范围，
    目标固定为当前层是声明的偏离。
  - `LAYOUTWIZARD`：新增 `ARRANGE <Single|TwoVertical|TwoHorizontal|ThreeVertical|ThreeHorizontal|Four>`
    等分格视口排列，铺在本次 Authored 纸张上（复用 `VPORTS` 的网格助手），布局行/视口/页面设置一条
    `LayoutPatch`/一步撤销；未知排列/未知键/重复键/缺值具名拒绝且不写；打印设备/打印样式/标题栏逐条
    `layoutwizard.*_not_implemented`（明说未应用）。
  - `LENGTHEN`：开放样条**延长**改为声明的**端点切线直尾**策略（沿单位端切线追加一个钳制节点段，
    `degree+1` 个共线等距控制点，尾长恰为增量；原曲线在自身节点域上逐点不变；接缝只 C0/G1、曲率不延续），
    缩短仍是精确子曲线；闭合/未钳制/切线退化/非正增量具名拒绝。
  - `FILLET`：`R>0` 覆盖 SPLINE 与 LINE/ARC/CIRCLE/ELLIPSE/SPLINE 的任意组合（SPLINE 用精确 `sub_curve`），
    `R=0` 覆盖全部平面对（CIRCLE↔CIRCLE 复用 TRIM 的两交点 CIRCLE→ARC 规则把两个圆都切成拾取侧弧；
    LINE↔ARC 现在也修剪弧，旧实现只动直线）；`Multiple`/`Undo`/多解循环属全局交互范围。
  - `CLIP`/`CLIPIT`：从具名拒绝改为真实命令，分派到既有 `IMAGECLIP`/`XCLIP` 写路径
    （`RECT`/`POLY`/`ON`/`OFF`/`DELETE`；`CLIPIT` 用选择里的 `LINE`/`ARC`/`LWPOLYLINE` 建闭合边界），
    并在写入前按名字拒绝表示层画不出的情形（块多边形裁剪、非凸/自交图像环、旋转/倾斜参照、底图），
    所以没有"存了但不画"的静默成功；UI 退出拒绝名单并接线键入语法。
  - `STRETCH`：新增 `stretchWindow` 工具参数（0 矩形 / 1 多边形 / 2 lasso）；多边形用连续点击
    （`CONFIRM` 或点回首顶点闭合）、lasso 用一次自由拖拽采样（复用既有 CROSSING transient 预览作轨迹），
    两者都进同一条 `StretchPolygon` → `polygon_window` 规划；退化/非有限/零面积/未闭合环具名拒绝，
    取消不写，一条事务/一步撤销。
  - `RENAME`/`-RENAME`：`LINETYPE` kind 不再是具名拒绝；改名与层属性/实体属性的线型引用级联在
    `cad-db` 的**单次校验写入**内完成（一次 revision、一次 `ChangeSet`），撤销/重做对称。`Continuous`
    不可改名，创建/删除线型仍不支持。
- **真机 GUI 证据**（Linux Wayland `native-below`，Vulkan `Quadro P620`/驱动 580.178.04，Mailbox；
  `scripts/collect-evidence.sh --backend wayland --lifecycle none`，Mutter ScreenCast 合成帧）：
  - 新增通用证据钩子 `YACR_PROBE_CAD_SCRIPT`/`_FILE`/`_AT`（默认关，`[probe] cad script:` 行），
    用真实 webview 的 `cad_command`/`cad_status`/`cad_layers` IPC 驱动命令，结果从宿主读回。
  - `fixtures/dxf/qcad-flange/flange.dxf`（419 实体）：`PSPACE` + `QSELECT LAYER = border`（图纸空间 1 个）
    → `LAYDEL` 成功，**419 → 337**（该层 82 个成员跨图纸/块空间删除）、层行消失 → `UNDO` **337 → 419**、
    层与实体全部恢复、无错误（`/tmp/opencode/evidence-laydel-paper4/`）。
  - 同图：`-LAYER FREEZE` 后 `LAYTHW` → 所有 `frozen` 转 false；`UNDO` 恢复（`/tmp/opencode/evidence-laythw/`）。
  - 同图：`RENAME LINETYPE DASHDOT DASHDOT_RENAMED` → 使用该线型的 `center` 层读回 `DASHDOT_RENAMED`；
    `UNDO` 恢复 `DASHDOT`（`/tmp/opencode/evidence-rename/`）。
  - 同图：`QSELECT LAYER = hatch` + `LAYMRG` → `hatch` 并入当前层 `0`、层行消失、实体数不变（419）；
    `UNDO` 恢复（`/tmp/opencode/evidence-laymrg/`）。
  - 同图：`LAYOUTWIZARD SheetGui A5 148 210 ARRANGE Four` → 新增布局 `SheetGui`、`viewportCount=4`；
    `UNDO` 移除；`ARRANGE ThreeRight` 按 `layoutwizard.unknown_arrangement` 拒绝且不建布局
    （`/tmp/opencode/evidence-layoutwizard/`）。
  - 空图纸（`/tmp/opencode/empty.dxf`）：`LINE` 画一条线 → `SELECTALL`（选中 1）→ 设
    `lengthenMode=0`/`lengthenValue=1.5` → `LENGTHEN` 受理且 `error=null`；`UNDO` 移除该线
    （`/tmp/opencode/evidence-lengthen/`）。**边界**：`cad_status` 没有几何通道，真机只证"命令在真机宿主上
    受理且不报错"，长度/控制点的正确性由契约测试钉住。
  - FILLET 真机：既有 `YACR_PROBE_DRAW_EVIDENCE` 驱动在真机上画出两条线（`entityCount` 4→5→6）、设半径 0.8/0.12、
    FILLET 两次拾取都推进到 `points=2, ready` 并派发 CONFIRM（`/tmp/opencode/evidence-fillet-driver3/`）；
    宿主的证据生命周期在 CONFIRM 后立刻退出，**没有**抓到 `+1` 的 entityCount 快照，故 FILLET 的几何结果只由
    契约测试（78 fillet + 134 trim）证明。
  - CLIP 真机（`flange.dxf`）：`QSELECT TYPE = INSERT` 没有命中（该图模型空间无 INSERT），`CLIP RECT` 返回
    `clip.no_selection` 具名拒绝（`/tmp/opencode/evidence-clip/`）——证明命令在真机宿主上接线并被拒绝路径覆盖；
    成功写入路径由 64 个 clip 契约测试（含真实图像/块写入、旋转拒绝、非凸拒绝）证明。
- **真机验证抓到两个契约测试没覆盖的真问题（已修）**：
  1. `LAYDEL` 在图纸空间选择时被 `laydel.selection` 拒绝——选择预检写死模型空间。改为命令内自带的
     「模型空间或任一图纸布局的顶层实体」规则（`layer_extended.rs` 本地 helper，不动共享
     `layer_commands.rs` 的模型空间预检）。
  2. `LAYDEL` 的撤销在真实图纸上失败：`Invariant("invalid entity geometry: line is zero length")`，
     什么都没恢复。根因是**历史重放**会重新跑 `validate_geometry`，而导入器刻意宽松、能存下写入校验
     拒绝的几何（该夹具的块/图纸里有零长度线）。`cad-db` 新增显式的**历史重放写模式**
     （`EntityGeometryCheck::RestoreStored`，crate-private，只有 replay 入口能选）：重放按字节恢复
     数据库自己曾持有的行，**仍**校验引用完整性（层/块/布局存在性、`SpaceId` 形状、INSERT 目标、
     显示状态规则、块成员不变式、表批次规则、`layer.locked`），新写入路径的几何校验不变。ERASE 的同
     类撤销也一并修复（`erase_undo_restores_an_imported_zero_length_line`）。
- **测试**：`cargo test -p cad-app --locked laydel` **10 passed**、`laythw` **5 passed**、`laymrg` **6 passed**、
  `layer_commands` **69 passed**、`rename` **15 passed**、`layout` **47 passed**、`layoutwizard` **7 passed**、
  `lengthen` **37 passed**、`fillet` **78 passed**、`chamfer` **33 passed**、`trim` **134 passed**、`clip` **64 passed**、
  `stretch` **56 passed**、`cad-host` **338 passed**、`desktop-ui` **1763 passed**、
  整个 `cad-app` **2661 passed / 3 ignored**、`history` 16 passed、`undo` 170 passed、`cad-ui-core
  layout_arguments` 12 passed、`cad-db` 全量（245+13+5）passed、`cad-history` 28 passed；`cargo clippy -p cad-db -p cad-app --all-targets` 干净；
  `cargo fmt` 干净；`check-file-size.py` / `check-i18n.py`（2913 键）/ `check-architecture.py` 通过。
- **NOT RUN**：整仓 `cargo test --workspace`；全量门禁（`cargo fmt --all`、workspace clippy、
  `check-tauri-capabilities.py`、`check-fixture-manifest.py`、`check-command-docs.py` 之外的 Python 门禁）；
  Windows/macOS/Android/Web 运行时；真实鼠标（脚本是合成 DOM/IPC 输入）。GUI 截图来自
  **同一 UID Mutter ScreenCast**、真实 GPU、真实 `--open` 路径，但只证明「命令在真机宿主上按预期提交并读回」，
  不是像素/性能验收。

## 2026-11-15 3D orbit 灵敏度降到 0.0035 rad/px + `orbitSensitivityPercent` 设置

- **问题**：3D 下鼠标拖拽与 Android 单指触控调整视角都过灵敏。原速率是宿主
  `ORBIT_RADIANS_PER_PIXEL = 0.008`（≈0.46°/逻辑像素）：桌面 1200px 全宽拖拽 ≈550°（1.5 圈），
  手机 380px 划一下 ≈183°——两处抱怨都能由这个数字解释。触控更糟是因为同一个"每逻辑像素"的速率
  在只有 380 个逻辑像素宽的屏上，每厘米划动转过的角度是桌面的数倍；Android 触控与鼠标走同一条
  `cad-host` 指针路径（3D + Primary），没有第二套原生手势。
- **默认值**：`0.0035 rad/逻辑像素`（≈0.20°/px）→ 桌面 1200px ≈241°、手机 380px ≈76°，
  150–250px 的拇指划动是 30–50°。`orbit_delta` 仍保持 `yaw = -dx·K`、`pitch = +dy·K`。
- **设置项**：新 `interaction.orbitSensitivityPercent`（f64 闭区间 **25.0..=200.0**，默认 **100.0**，
  越界/非有限**拒绝**而不是 clamp）→ `CadSettingKey::OrbitSensitivityPercent`（`ALL` 23→**24**）→
  `cad_settings.orbitSensitivityPercent`；有效速率 `0.0035 × percent/100`，倍率再防御性 clamp 到
  `[0.25, 2.0]`，NaN/∞/非正回落 100%。写盘失败回滚，**实时生效**，无需重开文档。
  Options → Drafting 在「左键拖动平移」之后新增「三维旋转灵敏度」（`kind:'real'` 而不是 `number`：
  宿主接受 f64，整数控件会把手工写进 `config.json` 的 `60.5` seed 成"未报告"且无法覆盖）。
- **ViewCube**：`cad_status.navigation.orbitSensitivityPercent`（`CadNavigation`，缺省 100.0）是前端唯一
  来源，`cad/orbit.ts` 增 `orbitRadiansPerPixel()`，`orbitDelta(from,to,rate)` 接速率参数；旧宿主缺该字段
  按 100 回落（不整条 `CAD_STATUS_INVALID`）。`CadNavigationSettings`/`parseCadNavigation` 抽到新的
  `cad/navigation.ts`（`cad-status.ts` 曾因改动涨到 1050 行，拆分后 982，过 1000 行门禁）。
- **结构**：新增 `crates/cad-host/src/orbit_sensitivity.rs`（读取器 + 写入/回滚）；
  `HOST_ALLOWED_PATHS_JSON` 迁到 `settings_root.rs`（crate 根仍 `pub use`，调用点不变）——
  `settings.rs` 978 行、`lib.rs` 971 行。
- **证据（实跑）**：`cargo test -p cad-host` **330 passed**（含 `settings_orbit` 8 项与
  `orbit_rate_tests` 5 项）、`cad-ui-core` **678**、`cad-app` **2642**、`cad-desktop-shell` **49**；
  速率实测 dx=100px 时 100%→yaw **-0.35 rad**、50%→**-0.175**、200%→**-0.700**，竖直 100px 的
  pitch **+0.35**；边界 25/200 接受、24.9/200.1/NaN/∞ 拒绝，持久化 `900` 在启动时被拒并回落 100%；
  前端 `npm test` **1755 passed**、`tsc --noEmit` 通过（新增 `options-orbit-sensitivity.test.ts` 覆盖
  `60.5` 的 seed/写入）；`check-file-size` / `check-i18n` / `check-architecture` /
  `check-fixture-manifest` 通过；`cargo check --workspace --all-targets`、`--lib --target
  wasm32-unknown-unknown`、`clippy -D warnings`、`cargo fmt --all -- --check` 通过。
- **NOT RUN / 边界**：真实鼠标与触控手感 **NOT VERIFIED**（无设备手势采样、无 Android 设备运行）；
  `0.0035` 来自几何推算（全宽 ≈240°、半屏 ≈120°），不是用户测试结论；若真机仍觉偏快，下一轮动基础值
  （0.0030–0.0032）而不是动区间；全量 `cargo test --workspace` 与 GPU/像素证据未跑。

## 2026-11-14 Bentley DGN V7/V8 矢量文档导入（`cadkit-dgn`）

- **新增 lane**：`crates/cad-import-dgn`。DGN 与 DWG/DXF 同属**矢量 CAD 文档**，映射 level、
  linetype、text style、shared cell/Insert、Line/Polyline/Circle/Arc/Ellipse/Spline/Text/Point
  到 `cad-db`，不是网格预览（ADR-0033 D11，D2 的明确例外）。`DGNIMPORT`/`DGNATTACH`/
  `DGNCLIP`/`DGNLAYERS`/`DGNADJUST`/`DGNEXPORT` 仍是具名拒绝：打开 `.dgn` 文档 ≠ 合并进当前 DWG。
- **依赖**：`cadkit-dgn`/`cadkit-core` **`=0.5.2`**（MIT OR Apache-2.0，纯 Rust，`unsafe` 禁止，
  wasm 可编译）；`default-features = false` 关闭 V8 writer 与可选 `cfb`。未修改上游、无 patch、
  无 vendoring。crate 很新（2026-10-01 首发、单一维护者），因此精确固定；`docs/dgn.md` 记录
  上游 5 个 open V8 正确性 issue 与本仓库的显式降级。
- **检测**：V7 固定魔数 ⇒ `DrawingFormat::Dgn`；V8 是 OLE/CFB，`detect_format` 返回
  `DrawingFormat::Ole`，`cad_app::import::FormatImporter` 在 `max_file_bytes` 检查之后先调
  `DgnImporter::sniff`（`Dgn~H` 根存储），命中走 DGN，否则仍走 RVT。不按扩展名猜，不在
  `cad-import-core` 复制 CFB 解析器。
- **映射/损失**：layer 只写名称/可见性（cadkit 不解码 level 颜色/线宽/线型，不伪造
  `LayerAttributes`）；linetype dash pattern 是 **UOR**，按 `dgn.uor_per_master` 缩放到主单位；
  单位按 `meters_per_unit` 精确匹配 m/mm/in/ft，其余回退 drawing units；text style 解析或新建
  `Standard`，不留悬空 `StyleId`；shared cell 两遍建 `BlockDefinition`/`BlockMetadata`，
  缺块 Insert 跳过 + Partial；循环/超深块引用也跳过（不会发出递归 `Insert`）；group 成员扁平化
  （per-child 样式保留，group 编辑语义不保留）。**明确缺口**：level display/freeze/lock 未解码
  （全部可见，恒 Partial）；只导入 `Document::model_space()`；references/xrefs、Item Types/
  XAttributes、dimensions/multiline/mesh/solid/surface 未建模；栅格 `size_px` 已知时写
  `SemanticGeometry::Image`（u/v 每像素，外部 TIF 不加载），未知时 `Opaque`；tags 不进入
  `cad-db`；扩展色 >255 回退 `ByLayer`；无 DGN 导出。未知元素/无几何是 `Unsupported`，不是
  空成功。
- **测试与门禁（本轮实跑）**：`cargo test -p cad-import-dgn` **22 passed**；`cargo test -p
  cad-import-core detect` **21 passed（2 filtered）**；`cargo test -p cad-app import::tests`
  **12 passed（2633 filtered）**；`cargo test -p cad-import-rvt` **33 passed**；`cargo check -p
  app-web --target wasm32-unknown-unknown --locked` **exit 0**（仅既有 warning）；`cargo fmt
  --all -- --check` exit 0；`cargo clippy -p cad-import-dgn -p cad-import-core -p cad-app
  --all-targets --locked -- -D warnings` exit 0；`python3 scripts/check-architecture.py`、
  `check-file-size.py`（手动核对新 crate 最大 660 行；脚本只统计 tracked 文件）、
  `test-file-associations.py`、`check-fixture-manifest.py`、`check-i18n.py`、
  `check-tauri-capabilities.py`、`check-command-docs.py` 全部 exit 0；
  `cargo test -p app-linux open_chooser_globs_cover_the_importer_set` 1 passed；前端
  `node --test test/host-adapter.test.ts` 36/36。
- **真实 GPU 视觉运行（2026-11-14，实跑）**：GNOME/Mutter Wayland 会话内启动嵌套 Weston 14
  （Wayland backend + GL renderer），`target/debug/yacr --open fixtures/dgn/{smalltest,test_dgnv8}.dgn`
  以 `GDK_BACKEND=wayland`、`WGPU_BACKEND=vulkan`、NVIDIA ICD + `DRI_PRIME=1` 运行。宿主 probe：
  `backend=vulkan adapter=Quadro P620 device_type=discrete_gpu present_mode=Mailbox`，窗口
  1841×983，`frames` 持续增长；`bridge wayland` 走的是 **native-below**，不是 X11 `dom-canvas`。
  V7 截图显示 `Demo Text`/圆/闭合红色折线/线，V8 截图显示 `myTéxt`/弧/椭圆/线/共享单元成员。
  证据与命令见 `docs/evidence/dgn-linux-gpu/`（`README.md`、`probe.txt`、两张 PNG）。这是**视觉/
  present 证据**，不是 golden/像素/性能或 Bentley 兼容性验收；debug 构建 + 嵌套合成器各一条边界。
- **NOT RUN / NOT VERIFIED**：整仓测试、Android/macOS/Windows/app-linux 全量构建；非 Linux 真机；
  GNOME 原生合成器下不经嵌套 Weston 的运行；golden/像素/性能；`app-macos` 的 allowed-types 测试在
  Linux 上是 0 个匹配测试（macOS 目标未运行）。
- **证据边界**：只有 GDAL MIT/X11 的 V7 `smalltest.dgn` 与 V8 `test_dgnv8.dgn`（**ODA Teigha
  4.02 写出，不是 MicroStation**）；契约测试证明容器/单位/level/文字/块/插入/group 扁平化/
  UOR 线型缩放/栅格 u-v/Unknown 诊断与写路径，**不证明** V8i/CONNECT/ORD、真实 MicroStation、
  level 显示标志、references、Item Types、尺寸/多线/网格/实体、栅格像素或字体度量。

## 2026-11-14 发布 v0.1.13（插件 API 可用 + canvas ViewCube/cad_orbit + 跨载体 pinch + SKP 导入 + OS 文件关联）

- **版本**：workspace `Cargo.toml`、`Cargo.lock`（36 处）、四个 `tauri.conf.json`、Flatpak metainfo
  全部 `0.1.12` → `0.1.13`（commit `9e00f728`；`7600b316` 只把 notes 移出 tag 树）。
- **tag / run**：`v0.1.13`（annotated，指向 `7600b316`）→ `CI build layers` run
  [`43543058296`](https://github.com/zenglanmu/yacr/actions/runs/43543058296) **11/11 job 绿**
  （四个 release + 三个 check + shader + web-build/smoke/deploy）；`CI` run
  [`43543058229`](https://github.com/zenglanmu/yacr/actions/runs/43543058229) 全绿。
- **首轮 tag 失败与修正**：第一次 tag（`9e00f728`）把 `docs/release-notes/v0.1.13.md` 提前放进 tag
  树，`core-quality` 的 `test-landing-site.py` 报 `apps/landing/changelog.json is stale`（changelog
  由 notes 生成，而 `downloads.json` 只能带出包后的真实尺寸）；同一轮 `macos-release` 在排队阶段被
  取消（`runner_name` 空、`steps: []`、无构建日志）。按 v0.1.12 的顺序改成“先 tag/出包、后补 notes
  与落地页”（`7600b316`），重打 tag 后 `macos-release` 正常拿到 runner 并成功。
- **发布**：`publish-release` run
  [`43547594928`](https://github.com/zenglanmu/yacr/actions/runs/43547594928) 绿 →
  <https://github.com/zenglanmu/yacr/releases/tag/v0.1.13>（非 draft / 非 prerelease；自动生成 notes
  后用 `docs/release-notes/v0.1.13.md` 覆盖）。
- **artifact（Release API 核对）**：`yacr-0.1.13-linux-x86_64.flatpak` **113,559,224 B**
  + `.sha256` + `.flatpak.gpg`（GPG 签名）；`yacr-0.1.13-windows-x86_64.zip` **108,573,995 B**
  + `.sha256`（unsigned，无 `.cer`）；`yacr-0.1.13-macos-universal.tar.gz` **155,080,263 B**
  + `.sha256`（ad-hoc、未公证）；`app-universal-release.apk` **115,516,961 B**
  + `.sha256`（仓库自签名，v2/v3 verified）。合计 492,730,443 B（≈470 MiB）。
- **落地页（提交侧）**：commit `b7a646bd`；`downloads.json` → v0.1.13（R2 自定义域 + GitHub
  fallback，真实 size/sha256）、`changelog.json` 由 notes 生成、`site.yml` 默认
  `release_tag=v0.1.13`、`index.html` 版本/尺寸/SHA fallback、`docs/landing-site.md` 与
  `scripts/test-landing-site.py` 同步；main 上 `CI` run
  [`43547755289`](https://github.com/zenglanmu/yacr/actions/runs/43547755289) 复验落地页契约通过。
- **落地页部署**：`site.yml` run
  [`43547756186`](https://github.com/zenglanmu/yacr/actions/runs/43547756186) 绿。线上复核：
  `/downloads.json` = v0.1.13，`/changelog.json` 首版 v0.1.13（2026-11-14），页面含 v0.1.13；
  R2 HEAD `releases/v0.1.13/yacr-0.1.13-linux-x86_64.flatpak` **200 / 113,559,224 B**、
  `app-universal-release.apk` **200 / 115,516,961 B**。
- **并发说明**：发布过程中另一条 lane 的 `ed14d147`（`rvt` 0.22.0 补丁 rebase）先进入 main 并随本次
  推送发布；tag `v0.1.13` 仍指向 `7600b316`（rvt 0.17.0），因此 **tag 产物与 main 当前源码的 `rvt`
  版本不同**。该 lane 仍有未提交的 `probe_log` 改动留在工作树，本条未触碰、未暂存。
- **发版前测试（本轮实跑）**：`cargo test --workspace --locked -- --test-threads=1`
  **6623 passed / 0 failed / 39 ignored**；`apps/desktop-ui` `npm test` **1752 passed** +
  `npm run build`；`fmt --check`、`clippy -D warnings`（修 `cloned_ref_to_slice_refs`）、
  `cargo check --workspace --all-targets`、workspace wasm lib、`app-web` 与 wasm target；11 个 python
  门禁。**未运行**：真实 GUI/GPU/真机；`YACR_TEST_DWG` 未设，DWG 用例被跳过（跳过不是已运行证据）。
- **NOT VERIFIED**：Windows/macOS/Android 真机、真实 GPU、像素与性能仍未验收；发布、打包、签名、
  上传与落地页部署成功都不是运行时验收。

## 2026-11-14 `[probe] status` 1 Hz 证据行改为默认关闭（共享证据门禁）

- **问题**：正常命令行启动（`run-react-linux.sh --open`）后 stderr 每秒一条
  `[probe] status backend=… frames=… stages_us[…]`，长时间运行刷成噪声。核对后发现：Linux tick 里
  `cad-frame`/`actual-render`/`pan-wait`/`gpu-memory`/`cad-stages` **都已**在
  `YACR_PROBE_CAD_LOG` 门禁下，只有这条 1 Hz 汇总行是无条件输出；Windows/macOS/Android 的同名
  汇总行同样无条件。
- **改法（跨载体一份契约）**：`cad-desktop-shell` 新增 `probe_log` 模块
  （`EVIDENCE_LOG_ENV = "YACR_PROBE_CAD_LOG"`、`evidence_log_enabled()`、纯 `env_flag_present` 与
  存在性单测 `evidence_log_gate_is_presence_based`，语义与 `YACR_PROBE_ALLOW_CPU_ADAPTER` 一致：
  只看存在、不看取值）。四个宿主的 1 Hz 汇总行都走该门禁：`app-linux`
  `host/tick_evidence::log_status_line` 与 `app-android` 同名函数提前返回，`app-windows`/`app-macos`
  tick 的 `telemetry.last_log` 节流条件前加同一判断。`commands::input_trace_enabled` 改为复用该
  函数（语义不变，去掉第二处字符串）。
- **证据运行不变**：`scripts/collect-evidence.sh` 与 `scripts/host-smoke-linux.sh` 本来就
  `YACR_PROBE_CAD_LOG=1`，`[probe] status`/`[probe] geometry`/`[probe] cad-frame` 照旧输出；
  `host-smoke-check.py` 只解析 geometry/cad-frame 行，不受影响。要看这条汇总行就显式设
  `YACR_PROBE_CAD_LOG=1`。UI 侧读的是 `probe_status` 命令，不依赖 stderr。
- **测试与门禁（本轮实跑）**：`cargo test -p cad-desktop-shell`（49 单测含新 1 + 3 + 6 + 12 + 7 + 4
  golden，全绿）、`cargo check -p app-linux --all-targets`、
  `clippy -p cad-desktop-shell -p app-linux --all-targets -D warnings`、`cargo fmt --all --check`、
  `check-architecture.py`、`check-file-size.py`。**未运行**：全量 workspace 测试、任何 GPU/视觉证据
  重跑。Windows/macOS/Android 的同构改动**仅代码同形**，未编译、未运行（NOT VERIFIED）。

## 2026-11-13 ViewCube 改 canvas 3D + 导航栏重排 + `cad_orbit` + 原生 pinch

- **范围**：右缘导航 dock 重设计（收起/展开移到立方体正下方；移动端收起只剩“立方体 + 一个展开图标”
  的浮动卡片，不再是一条整屏竖线）；视图立方由 CSS 3D + 六个 DOM 面改为 **2D canvas 软件投影**
  （背面剔除、画家排序、平面着色、面标签不镜像/不倒置）；立方体拖拽经新 IPC **`cad_orbit`** 真实
  旋转相机；Linux 触控板 pinch 改为 CAD 缩放而不是页面缩放。
- **立方体（`apps/desktop-ui`）**：`cad/viewcube.ts` 的 `viewCubeMatrix` 换成
  `viewCubeBasis()`（返回三条 cube-local 轴在屏幕空间的 `[x, y(down), depth]` 像；FRONT 为恒等，
  TOP/RIGHT 与旧 `matrix3d` 列一致，退化/非有限 → `null`）。新 `cad/viewcube-canvas.ts`
  （`projectCube`/`pickFace`/`drawCube`/`readCubePalette`，DOM-free，有单测）、`cad/orbit.ts`
  （`ORBIT_RADIANS_PER_PIXEL = 0.008`、`yaw = -dx·K`、`pitch = +dy·K`，与 `cad-host` 的
  `orbit_delta` 完全一致；`OrbitDrag` 每帧合并一次、单请求在飞、被拒即停）。`ViewCube.tsx` 的 canvas
  是 `aria-hidden` 装饰，六个面仍是 `sr-only` 真实按钮（焦点高亮对应面）；点击面 = 标准视图，
  拖动 = 轨道；`null` 相机/取不到 2D context 都渲染明确不可用态。尺寸由 `--viewcube-size`（48px）
  决定，DPR 与 `data-theme` 变化会重读 token 重绘，无 rAF 常驻。
- **Dock（`apps/desktop-ui`）**：新增 `layout/nav-rail-state.ts`（桌面 = 宿主 `navbarVisible`，
  toggle 是真 `NAVBAR`；紧凑 = 本地状态、跨断点复位）；`NavigationDock` 结构为
  `.navdock[data-rail] > .navdock__head(立方体 + toggle) + NavRail(open)`，`NavRail` 变为纯投影
  （关闭时返回 `null`，不再渲染空 toolbar）。桌面仍是 56px 绝对全高列（分区矩形不变）；紧凑布局
  同一元素变成 `top/right: 8px` 的内容尺寸卡片（`max-height: min(60dvh, calc(100% - 16px))`，
  内部滚动），卡片宽度 = `--navrail-w`，开合不改变宽度。
- **`cad_orbit`（跨载体）**：共享 wire 行 `cad_orbit { yaw, pitch }`（弧度、有限；与
  `cad_standard_view` 同门禁顺序：模态 → CAD 模式 → 有界队列；`{accepted:true}` 只是**已入队**）。
  `CadSession::orbit` 派发真实 `CommandId::Orbit` + `CommandPayload::Orbit`，2D 是宿主真实拒绝
  （`Orbit is only defined in the three-dimensional view`）。`hostProfile.orbit` 三态：
  `null` = 未声明（`native-below`）、`false` = 已声明不可用（`dom-canvas` 2D-only）。四个桌面 tick
  与 `app-web` DirectSink 均已接线。
- **pinch（跨载体）**：Linux 原生手势 + Windows/macOS 的 DOM 信号，两条入口都汇到同一个 `InputKind::Pinch`。根因已核实——WebKitGTK 的触控板 pinch 是 UI 进程
  `GtkGestureZoom` → `pageScaleFactor`，**不产生 DOM 事件**，`preventDefault()` 无效，也没有
  `WebKitSettings` 可关；公开的 `WebKitWebView:magnification` 要 2.54（本机 2.52.6）。
  做法：`gtk_bridge/pinch.rs` 在 webview 创建时按 `pinch_plan(has_magnification, has_private_gesture)`
  选择：≥2.54 钉 `magnification=1.0`；2.52.6 把私有 `"wk-view-zoom-gesture"` 的 **propagation phase
  置为 `NONE`**（对象/数据不销毁不释放），再在 widget 的 **`event` 信号**上拦截
  `GdkEventTouchpadPinch`、取 `scale`/位置并返回 `Stop`（事件到不了 WebKit 的 vfunc）。**实测过两条
  弯路**：只销毁 signal handlers 仍会被该 controller 消费（`begin=0`）；同 widget 再挂一个 capture-phase
  `GtkGestureZoom` 也收不到（gesture group 消费），而 `event` 信号先于 vfunc 且 `Stop` 能真正阻止页缩放。
  增量 scale + 位置进 `Shared::cad_pinches`（cap 64），`host/pinch_intake.rs` 在 tick 里按
  `rect.dpr/parent_scale` 换成 partition-local CSS px 并**丢弃 partition 外的样本**，追加到同一批
  `inputs`（DOM 输入优先），`CadSession::handle_inputs` 一次发布。转换/拒绝规则在 `cad-ui-core`
  （`native_pinch_input`，`sequence = 0`，该路径不经 `InputTracker`）。都不可用时只打
  `[probe] pinch: gesture_unavailable`，**不**假装已接管。
  **Windows/macOS（2026-11-14 补）**：两端的 pinch 本来就以 **DOM 信号**到达，不需要原生钩子——
  Windows 把精确触控板 pinch 转成 `WM_MOUSEWHEEL` + `MK_CONTROL`，Chromium/WebView2 转发为可取消的
  **`wheel` + `ctrlKey`**（`deltaMode 0`，delta 是 **120 的量化倍数**，不按 log(scale)）；
  macOS/WKWebView 的 `allowsMagnification` 默认 `false`（页面本就不缩放），但
  `gesturestart/change/end` 仍派发且可取消，`scale` 是**累计值**。共享前端
  `cad/dom-pinch.ts` + `cad/input.ts`：`ctrl`+wheel → `scale = exp(-Δy·0.0015)`（与滚轮缩放同标度）；
  `gesturechange` → 增量比 `scale/previous`；Safari 15+ 同一手势会**同时**发两种信号，所以 gesture
  期间及 `gestureend` 后 **500ms** 内的 `ctrl`+wheel 直接丢弃（页缩放照旧 `preventDefault`），
  避免一次 pinch 缩放两次。Android `onWebViewCreate` 关掉 WebView 页缩放（`setSupportZoom(false)` 等），
  让既有两触点 DOM pinch 走 CAD；macOS 不需要显式关 `allowsMagnification`（平台默认 `false`），
  Windows 由 wry 默认关 `SetIsPinchZoomEnabled`——两端的 **pinch→CAD 由上面的共享 DOM 路线提供**，
  真机交付与“一次手势只缩放一次”的顺序仍 **NOT VERIFIED**（`docs/compatibility.md` 已按载体写明）。
  前端 `defaults.ts` 另加 `gesturestart/change/end` 取消（Chromium/WebView2/Safari 的 DOM 兜底，
  不是 Linux 的修复）。
- **测试与门禁（本轮实跑）**：`cargo test -p cad-ui-core` **677 + 1 + 6**（含新
  `tests_native_pinch` 4、orbit admission/wire/`CadSession::orbit`）；`cad-desktop-shell` 48 + 3 + 6 +
  12 + 7 + 4；`app-linux --lib` **194**（含 `gtk_bridge::pinch` 4 + intake 越界 1）；`app-web` 套件；`cargo check
  -p app-linux --all-targets`、`-p app-web --locked` 与 `--target wasm32`；`cargo fmt --check`；
  `clippy -p cad-ui-core -p cad-desktop-shell -p app-linux -D warnings`。前端
  `tsc --noEmit` 干净、`npm test` **1612 pass / 0 fail**、`npm run build` 通过。脚本门禁
  `check-architecture`/`check-file-size`/`check-i18n`/`check-command-docs`(+自测)/
  `check-fixture-manifest`/`check-tauri-capabilities`/`test-android-host` 全部 exit 0。
- **pinch 实测（合成手势，真实 Wayland 会话 + 真实 `target/debug/yacr`）**：新增
  `YACR_PROBE_PINCH` 证据驱动，用合成 `GdkEventTouchpadPinch`（BEGIN/UPDATE×2/END，fingers=2）
  走真实 GTK 分发 → `event` 信号 → `NativePinch` → tick → `CadSession::handle_inputs`。结果：
  `visualViewport.scale` **1.0 → 1.0（未变）**，`cad_status.camera.scale` **0.003281 → 0.001641**
  （≈ 累计 1.25×1.6 = 2.0）。`[probe] pinch: webkit-gesture-neutralised` +
  `webkit gesture type=GtkGestureZoom phase_after=None` + `own event handler attached` 各一行。
  这**绕过了内核/libinput/合成器的真实手势源**，所以**真实触控板仍需人工按一次**。
- **NOT VERIFIED（必须真机/真手势才能升级）**：真实触控板的 pinch 手感与锚点（Linux 原生、Windows
  WebView2、macOS WKWebView/Safari 15+ 双信号顺序）；立方体拖拽手感、
  `cad_orbit` 实际入队/应用（只有契约与编译）；移动端卡片的真机截图（只有 CSS 结构与契约测试）；
  Windows/macOS/Android 运行时；GPU/像素/性能。`tests_input.rs` 972 行、
  `apps/app-linux/src/host/tick.rs` 1000 行（都在上限内，但已贴边）。
- **并发说明**：工作树里另有本轮之外的未提交改动（`packaging/`、`.github/workflows/core.yml`、
  Android manifest/`tauri.conf.json`、`docs/flatpak.md` 等），本轮未触碰、未暂存、未提交。

## 2026-11-13 OS 文件关联：注册完整支持集（单源 `formats.json` + 跨载体契约）

- **问题（已核实）**：应用内打开对话框早已含全套（三宿主 `OPEN_EXTENSIONS` 末尾 `rft`/`skp`、
  i18n 过滤文案、`WEB_OPEN_EXTENSIONS`），但 **OS 级“用 Yacr 打开”** 只注册了 DWG/DXF：Flatpak
  `.desktop` 的 `MimeType=` 与 AppStream `<mediatype>` 仅有 dwg/dxf；Android
  `bundle.fileAssociations` 只有 6 条 dwg（manifest 六个 intent-filter）；macOS `Info.plist` 的
  `CFBundleDocumentTypes` 只有 dwg/dxf；Windows 产物是 zip、无安装器。
- **单源**：新增 `packaging/mime/formats.json`（15 个扩展 / 21 个 MIME 类型 / 10 个需自带的
  shared-mime-info 类型 / Android 专用 `application/octet-stream` 通配）。全部载体从它派生，新增
  `scripts/test-file-associations.py` 逐项比对：三宿主 `OPEN_EXTENSIONS`（含顺序）、
  `WEB_OPEN_EXTENSIONS`、`.desktop` `MimeType=`、mime XML、metainfo、Android conf + 已提交
  manifest（pathPattern/mimeType 防陈旧）、macOS plist、Windows 文档边界；已接入 `core-quality`。
- **Linux/Flatpak**：新增 `packaging/flatpak/io.github.zenglanmu.yacr.mime.xml`，manifest 安装到
  `/app/share/mime/packages/io.github.zenglanmu.yacr.xml`（flatpak 安装触发器会跑
  `update-mime-database`，沙箱内无需 build 命令）；`MimeType=` 与 `<mediatype>` 列全 21 个类型。
  语义要点：`.coff` 用 `application/x-geomview-coff`，**不得**复用 `application/x-coff`
  （shared-mime-info 2.5 起那是 Windows 目标文件，且其 `*.obj` glob 会抢网格文件）；IFC **不带
  magic**（IFC 与 STEP 同以 `ISO-10303-21;` 开头，magic 会把 `.step` 误判为 IFC）；Revit 四个类型
  只用 glob（同为 OLE/CFB，magic 无法区分）；`.skp` 用 de-facto `application/vnd.sketchup.skp`。
  `Exec=yacr %F` **保持不变**：`--filesystem=home` + 明文路径是既定的同级外部参照取舍，代价是
  `$HOME` 之外的“打开方式”路径沙箱不可读（`%f` 会走 portal 转发但隐藏同级文件）。
- **Android**：`bundle.fileAssociations` 6 → **22 条**（21 个类型 + 覆盖全扩展的
  `application/octet-stream`），由 tauri-build 重新生成已提交 manifest（22 个 intent-filter、
  38 个 pathPattern），**未手改**；`mimeType` 必填（Tauri 无 mime 的分支不带 scheme，实际永不匹配）。
- **macOS**：打包脚本写出的 `Info.plist` 分两组（CAD drawing / 3D model preview）覆盖 15 个扩展，
  `Viewer` + `Alternate`。
- **Windows**：zip 无安装器 ⇒ 注册表关联 **NOT IMPLEMENTED**（Tauri 的 `fileAssociations` 只在
  NSIS/MSI 里写注册表），已在 `docs/windows-app.md` §5.1 显式写明，不做无效接线。
- **证据（本轮实跑）**：`test-file-associations.py` → `OK: 0 mismatches`；`test-package-linux-release.py`
  **23 tests**、`test-package-macos-release.py` **10 tests**、`test-android-host.py`、
  `check-workflows.py` 全部通过；`flatpak-builder --user --force-clean` 构建成功，产物内
  `/app/share/mime/packages/io.github.zenglanmu.yacr.xml` 存在、`.desktop` 的 `MimeType=` 为
  21 个类型；把该 `share/mime` 编译成 mime DB 后，`xdg-mime query filetype` 对 15 个扩展**全部**
  返回预期类型（`model/step`、`model/ply`、`model/off`、`application/x-geomview-coff`、
  `application/x-ifc`、4× `application/x-revit*`、`application/vnd.sketchup.skp`、以及系统自带的
  `image/vnd.dwg`/`image/vnd.dxf`/`model/obj`/`model/stl`）。
  **NOT VERIFIED**：真桌面点“打开方式”（默认应用选择、portal 通配匹配）、Android 真机 DocumentsUI、
  macOS LaunchServices 实际注册；Windows 无安装器不可测。AppStream `appstreamcli validate` NOT RUN。
- **注意**：工作树同时存在并行 lane 的 `cad_orbit`/ViewCube 改动（`apps/app-linux/src/host/tick.rs`
  等），本轮未触碰、未提交；`check-file-size.py` 因其把该文件增至 1014 行而失败，与本轮无关。

## 2026-11-13 合并两个工作树（`revit-texture`：`vendor/rvt` 受控补丁；`ts-api`：插件 API 可用）

- **范围**：`revit-texture` 的 `25da5902`（`vendor/rvt` 0.17.0 固定路径补丁、ADR-0035、
  `docs/rvt-patches.md`、门禁允许名单扩为 `{acadrust, rvt}`）fast-forward 进 `main`；`ts-api` 的
  14 个提交（`1730b9a3`…`7918bcae`：符号表/当前选择/实体读、写通道与 `transaction.commit`、插件装载
  与命令派发、异步提示通道）以 `--no-ff` 合入。唯一冲突 `docs/handoff.md` 按时间倒序排列：RVT
  （15:25）→ 提示通道（14:46）→ 插件 API 可用（14:29）→ SKP（12:17），双方条目完整保留。
- **合并时补齐（分支自身的文档漂移）**：权威表 `crates/cad-ui-core/api/plugin-api.json` 已是
  **17 行 = 16 `supported` + 1 `pending`（`document.lock`）**，而 `docs/commands.md` 仍写着上一轮的
  21 行 / 18 pending，且 `cad_api_call` 只列 3 条只读。已按代码改为 5 条只读（活动文档、文档列表、
  `database.revision`、`database.symbolTables`、`editor.currentSelection`），写明写与交互提示走
  `cad_api_submit` / `cad_api_poll`，拒绝顺序补上 `cad_api.incomplete_snapshot: symbolTables`；
  `api_call.rs` 中“描述符行仍为 pending”的过时模块注释同步改正。
- **证据（本轮实跑，均在合并后的树上）**：`scripts/test-rvt-patch.sh` 4、`cad-import-rvt` 33、
  `cargo check`（cad-ui-core/cad-app/cad-host/cad-desktop-shell/app-web `--all-targets`、app-linux
  `--all-targets`、app-web wasm target）、`cad-ui-core --lib session` 579（1 ignored）、
  `plugin_api_status` 6、`cad-app --lib` 2638（3 ignored）、`cad-host --lib` 317、`app-web` 131、
  `fmt --check`、`check-architecture`（36 包）/ `check-file-size`（2035 文件）/ `check-fixture-manifest`
  （45 fixture）/ `check-i18n`（2907 键）/ `check-command-docs` + 自测 / `check-workflows`（16 job）/
  `test-ui-migration-guards` 23；包 node **77/77**、desktop-ui `npm test` **1717/1717**、
  `npm run build` 产出 `dist`。
  wasm target 的 10 条 dead-code warning 均在合并未触碰的 native-only 代码里，非本轮回归。
  **NOT RUN**：clippy、workspace 全量测试、`check-tauri-capabilities`、workspace 级 wasm 检查、
  android/macos/windows 目标、真实 GUI/GPU/像素/真机、真实 DWG 用例（`YACR_TEST_DWG` 未设，
  被跳过的不是已运行证据）。
- **边界**：合并只让 `main` 拥有这些代码与契约证据。RVT 透明度仍无像素证据（固定离屏视角前后 PNG
  字节相同），插件 API 仍无真实 GUI 装载/提示证据，平台验收状态不变（见平台表）。

## 2026-11-13 RVT 预览透明度：`vendor/rvt` 受控补丁（多材质构件取最透明构件 alpha）

- **合并**：本 worktree 从 `7b809bbd` fast-forward 到 `main` 的 `d0c78c71`（0 冲突），补丁工作
  在 `feat/rvt-transparency` 分支上进行。
- **问题（已定位，非猜测）**：上游 `rvt` 0.17.0 的 `ifc::gltf` 在构件类型由多个材质组成
  （`IfcMaterialConstituentSet` ≥ 2）时把整块预览体退回**类别颜色 + alpha 1**；单材质但颜色未
  读到时也被过滤掉。本地真实样本（`rmeadvancedsampleproject.rvt`，Revit 2024，仓库外）：
  6,804 个 mesh 只有 580 个显式不透明度；17 个玻璃门（`Tür - Glas`）与 149 个玻璃灯具
  （`Glass - Clear, Grey` / `Glass - White, High Luminance`）本应透明/半透明。上游 0.20.0
  只加了 `extras.colourSource`/`alphaSource` 来源标记，**没有**改选择逻辑（升级不修复）；
  GLB 里没有多材质关联，工作区侧无法规避。
- **补丁（`vendor/rvt/src/ifc/gltf.rs`，唯一被改文件）**：① 多材质预览体保留类别颜色，alpha
  取该构件材质里最不透明的 `1 - transparency`（仅在至少一个材质读到 transparency 时生效）；
  ② 未按层绘制的层集构件按层材质同样处理；③ 单材质但颜色未读到（或单 profile 颜色未读到）
  的构件仍取类别颜色，只补上读到的 alpha（`single_material` 过滤条件不变，颜色选取不变）；④ 每个材质写 `extras.colourSource`/`alphaSource`（`"file"`/`"rvt-rs"`），
  类别材质恒 `rvt-rs`、层带材质恒 `file`；几何/节点/颜色/类别色板/IFC/STEP/release 不变。
- **导入侧（`cad-import-rvt`）**：`gltf` 启用 `extras` feature；`GlbScene::alpha_chosen` 统计
  `alphaSource = "rvt-rs"` 的显式不透明度，`import.rvt.alpha_chosen` 稳定诊断报告
  “整块预览体的不透明度取自最透明构件，不是逐部位材质”。GLB 无 extras（旧 GLB/合成夹具）仍
  按文件自身 alpha 读取。
- **策略/门禁**：新增 ADR-0035（`acadrust` 之外的第二个受控补丁例外）与
  `docs/rvt-patches.md` 台账；`scripts/check-architecture.py` 的 patch 允许名单扩为
  `acadrust`/`rvt`，逐 crate 要求台账存在且含 workspace 版本（`0.17.0`）；根 `Cargo.toml`
  加 `[patch.crates-io] rvt = { path = "vendor/rvt" }` 与 `[workspace] exclude =
  ["vendor/rvt"]`（避免 129k 行上游源码进入本仓 fmt/clippy/行数门禁，也不是通用 vendoring）；
  `scripts/test-rvt-patch.sh` 跑补丁自己的单元测试；AGENTS.md 铁律 1、THIRD_PARTY_NOTICES、
  `docs/compatibility.md` 同步。
- **已运行**：`cargo test --manifest-path vendor/rvt/Cargo.toml --lib ifc::gltf` **4 passed**；
  `cargo test -p cad-import-rvt --locked` **33 passed**（新增
  `an_exporter_chosen_opacity_is_stored_and_counted`）；本地真实样本导入
  `explicit_opacity` 580 → **746**（504 来自文件材质：438 玻璃幕墙嵌板 + 66 单材质灯具；
  242 为导出器选择：76 房间类别默认 + 166 个多材质预览体），诊断出现
  `import.rvt.alpha_chosen`；
  `cad-cli-tools render`（Quadro P620，NVIDIA 580，真实 GPU，离屏）`transparent_batches`
  580 → **746**、`invisible_batches = 0`；`check-architecture`（新增固定路径/版本/lock 生效
  检查，四种漂移实测 fail-closed）/ `check-file-size` / `check-fixture-manifest` /
  `check-workflows`、`cargo fmt`/`clippy`（cad-import-rvt）通过；`bash scripts/test-rvt-patch.sh`
  断言“4 passed”（删 `mod tests` 会失败）；`python3 scripts/test-ui-migration-guards.py`
  **23 passed**（新增 5 条 patch 策略契约）。
- **NOT VERIFIED / 边界**：该固定离屏视角下新增的 166 个透明构件被不透明构件遮挡，前后 PNG
  **字节相同**——不构成像素证据，只有批次分类与真实样本计数证据；Revit 透明度保真度无第二份
  样本交叉验证；Windows/macOS/Android/Web 只到编译/契约；未向上游提交 issue/PR（本地无 fork
  凭据）；全量 `cargo test --workspace` NOT RUN。

## 2026-11-13 插件 API 提示通道：会话侧 pending prompt + 命令行数据模式

- **位置**：`native-below` 的点击只到原生表面，WebView 看不到，所以提示状态在
  `HostController`（唯一 pending prompt），`CadStatus.pendingPrompt` 投影给 chrome。回答走已有
  `cad_api_submit`/`cad_api_poll`：提交时无 outcome，用户操作后由 owner tick（原生）或直接 sink
  （wasm）排空。没有新 wire 命令、没有事件关联。
- **输入**：在工具状态机**之前**拦截——点击 → `world_point`；实体/选择 → 与 `SelectAt` 相同的
  拾取/发布；文本（`cad_set_tool_text`）→ string/double/integer/keyword，点接受 `x,y[,z]`；
  Enter 结束选择/接受默认；Esc → `Cancel` 且被吞掉。非法输入保留提示并写 `pendingPrompt.error`
  （re-prompt），结果队列满时拒绝**新**提示而不是丢已完成的答案。
- **chrome**：提示期间命令行是数据输入（绕过 `admitCommandText`，不发 `cad_command`），答案
  草稿与命令草稿分离；Enter→`CONFIRM`、Esc→`CANCEL`，无提示时 Esc 仍关闭面板。问题内联显示，
  keywords 为按钮，宿主拒绝语句与插件 `writeMessage` 回显在命令行上方（移动端为浮层）。
- **能力表**：17 行 = **16 supported / 1 pending**（`document.lock`）；`editor.writeMessage` 改为
  客户端本地（无 row）。
- **证据**：`cargo test -p cad-ui-core --lib session` 579、`-p cad-app --lib` 2636、
  `-p cad-host --lib` 317、`app-web` 131、`plugin_api_status` 6/6；包 node 77/77、
  desktop-ui plugin-api+host-adapter 99/99、chrome `node --test test/*.test.ts` 1577/1577、
  `npm test` 整条 **1717/1717**；fmt/architecture/file-size/i18n 通过。`App.tsx` 999 行。
  **NOT RUN**：真实 GUI 中点击/键入回答提示、真机/WebView、GPU/像素。

## 2026-11-13 插件 API 可用：写通道、实体读、装载与命令派发

- **写通道**：`cad_api_submit`（桌面入队、`dom-canvas` 同步应用）+ `cad_api_poll`（有界
  FIFO，按 `requestId` 关联）。`transaction.commit` 映射到唯一提交路径
  `HostController::plugin_commit → Application::execute_api`；insert 解析 layer 名与 active
  space，translate/delete 由十进制 `EntityId` 构造真实 `SelectionRef`；stale revision →
  `api.revision-conflict`，成功回 `{revision, created}`。`cad-db` 之外没有第二条写路径。
- **实体读**（会话通道）：`database.getObjectId`（handle 线性扫描，未命中是真实 `null`）、
  `transaction.getObject`（按宿主 `typeName` 输出客户端形状；无形状的几何显式
  `unsupported-runtime`）。`decodePolyline` 的字段寻址 bug 已修（此前任何折线都无法解码）。
- **装载与命令**：包内 `Runtime.registerCommand`（本地，无描述符行）+ 宿主 `loadPluginModule`
  （URL 或模块、`definePlugin` 校验、版本拒绝、失败回滚、幂等 deactivate）+ `commands.ts`
  单一命令表；`App` 在宿主准入**之前**查插件命令，插件命令不伪装成 CAD 命令，处理器异常走
  `plugin.command.failed` 通知而不打断漏斗。
- **能力表**：18 行 = **8 supported / 10 pending**。仍 pending 的是交互式 `editor.get*/selectAll/
  writeMessage` 与 `document.lock`；插件安装 UI/持久化商店、Worker/权限/签名隔离未做。
- **证据**：`cargo test -p cad-ui-core --lib session` 564、`app-web` 128（矩阵含提交、
  handle 读、create-then-read 回环）、`plugin_api_status` 6/6、`cargo check -p app-linux
  --all-targets`、fmt/architecture/file-size/i18n；包 node 72/72、desktop-ui
  plugin-api+host-adapter 96/96、`npm test` 整条 **1693/1693**。
  **NOT RUN**：真实 GUI 中装载插件/执行插件命令、真机/WebView、GPU/像素。

## 2026-11-13 多格式导入接入 SketchUp（`.skp`）预览 lane

- **改动**（`cad-import-skp` 本体由另一条 lane 实现，本轮**未改**其公共面）：
  - 检测：`cad-import-core::detect` 新增 `DrawingFormat::Skp`（在 `Rvt` 之后）、`label()` 分支、
    `looks_like_skp` 与 `detect_format` 分支。SKP 是**最先**探测的一项：定长 32 字节签名
    （`FF FE FF 0E` + UTF-16LE `SketchUp Model`），比 OLE/`AC10xx` 嗅探便宜，且没有别的格式以
    `FF FE FF` 开头；头只回答“是 SketchUp 模型”，release 由适配器回答。
  - 派发：`cad-app` 新增 `cad-import-skp` 依赖、`FormatImporter::skp` 字段与
    `DrawingFormat::Skp => self.skp.import_with_progress(...)`（进度转发与 RVT 臂一致）；
    `UNSUPPORTED_INPUT` 现在点名 SketchUp（`.skp`）。
  - 选择器/扩展名：`apps/desktop-ui/src/host/web-adapter.ts`（`WEB_OPEN_EXTENSIONS`、门与文档注释）、
    `app-linux`/`app-windows`/`app-macos` 的 `OPEN_EXTENSIONS` 与三条契约测试、
    `cad-diagnostics::DOCUMENT_EXTENSIONS` 与裸 token 脱敏测试都加入 `.skp`；Android SAF 不对
    扩展名做枚举，无改动。
  - i18n：`linux/windows/macos.picker_filter` 的 zh-CN/en 各加 `SKP`（试运行预览），无新键。
  - 文档：`docs/compatibility.md`（标题、嗅探链、新增 SKP 能力行、crate 测试清单、外部 corpus
    证据边界）、`docs/architecture.md:53`（crate 与嗅探列表）、`THIRD_PARTY_NOTICES.md`
    （`skpwright 0.6.0` GPL-3.0-only 条目 + AGPLv3 §13 组合说明）、`README.md`（导入句/格式 bullet，
    只保留状态标签）、`docs/adr/0033-multi-format-import.md` 新增 D8 SKP（原 D8/D9 顺延为
    D9/D10，交叉引用同步），新增 `fixtures/skp/SOURCE.md`（仅外部夹具，未加入
    `fixtures/manifest`）。
- **测试（已运行，本轮实测）**：
  - `cargo test -p cad-import-core --locked`：**20 passed**（含 3 条 SKP 检测：32 字节头 → `Skp`、
    错误类型串不命中、31 字节截断不命中）。
  - `cargo test -p cad-app import --locked`：lib **21 passed**（含新增 dispatcher 两条：头 + garbage
    走到 SKP lane 得到 `import.skp.parse_failed` 而非 `UNSUPPORTED_INPUT`；三个版本的头不被
    RVT/STEP/IFC/OBJ/STL/PLY/OFF lane 认领）。
  - `cargo test -p cad-import-skp --locked`：**19 passed**（release 上限、材质、三角化、可见性
    裁剪、显式拒绝等；本机重跑确认）。
  - `cargo test -p cad-diagnostics --locked`：**15 passed**；`cargo test -p app-linux platform
    --locked`：**27 passed**。
  - `cargo check -p cad-app --all-targets --locked`、`cargo check -p cad-import-skp --target
    wasm32-unknown-unknown --locked`：通过（wasm 侧无平台依赖，`miniz_oxide` 是纯 Rust）。
  - 门禁：`check-architecture` / `check-file-size` / `check-i18n` / `check-fixture-manifest`、
    `cargo fmt --all -- --check`、`(cd apps/desktop-ui && npx tsc --noEmit)` 全部通过。
  - 真实语料（外部，未提交，见 `fixtures/skp/SOURCE.md`）：全语料 **763** 个 `.skp` ⇒ **728 成功、
    25 `import.skp.edge_only_geometry`、10 `import.skp.no_geometry`、0 失败 / 0 panic**；box
    1 实体/12 三角、house 64/1440、face-with-hole 1/8、instance-scaled 3/36；theater-2017
    1246 实体/96,333 三角与 theater-2026 1246/96,344 的面积 **1020.981869 m²** 一致（相对差
    5e-14）。三角化策略：耳切 + 面积校验，失败回退 even-odd（带洞面直接 even-odd），两者都失败
    才丢弃并记 `import.skp.faces_dropped`。
- **NOT RUN**：全量 workspace 门禁与全量测试（`cargo test --workspace`）、`cargo clippy`、
  Windows/macOS/Android 载体的编译与运行、真实 GUI/GPU/像素/真机；没有真实 Revit/STEP/IFC
  样本对照。`YACR_TEST_SKP`/`YACR_TEST_SKP_2026` 未设置时相关用例跳过，**跳过不是已运行证据**。
- **证据边界**：`.skp` 是**预览**通道（三角形 + 实例变换 + tag + 材质颜色/不透明度），不是测量源
  （`measure` 恒 `Unsupported`，成功导入恒 `Partial`）；2021–2025 只有容器等价（`Unverified`，
  无观测样本）；只有边/曲线的文件显式拒绝 `import.skp.edge_only_geometry`，不是空成功。外部
  corpus 数字是有界扫描，不是 golden 回归、性能或产品验收；平台表与其它平台验收状态不变。

## 2026-11-13 合并 `ts-api` 工作树（插件 API M2 系列进入 `main`）

- **范围**：`ts-api` 工作树的 12 个提交（`76806671`…`1a4e8d9e`）合入 `main`（`merge/ts-api` 分支，
  `--no-ff`）。唯一冲突是 `docs/handoff.md`：按时间倒序把插件 API 三条（M2.2 / M2.1 / 骨架）排在
  v0.1.12 发布条目之前，双方条目都完整保留。另一工作树 `revit-texture` 无待合并内容
  （`7b809bbd` 已是 `main` 祖先）。
- **合并时补齐（两处分支自身的不一致）**：`docs/commands.md` 的 `cad_api_status` 说明仍写“20 行全
  pending”，与实现（21 行 = 18 `pending` + 3 `supported`）矛盾，已改正并补上缺失的 `cad_api_call`
  wire 契约小节（三条只读、拒绝顺序、`cad_waiting` 与“无文档”的区别）；`packages/cad-plugin-api`
  的 `npm test` 原本裸调 `tsc`（该包无 `node_modules`、无 devDependency、无 lockfile，干净检出必失败），
  改为只跑 node 套件，README 注明类型检查由 desktop-ui 门禁
  `tsc -p ../../packages/cad-plugin-api/tsconfig.json --noEmit` 承担。
- **证据（本轮实跑，均在合并后的树上）**：`cargo check`（cad-ui-core / cad-app / cad-desktop-shell /
  app-web `--all-targets`、app-linux `--all-targets`、app-web wasm target）；`plugin_api_status` 6、
  `session::api` 21、`app-web` 123、`cad-desktop-shell` 78；`fmt --check`、`check-architecture` /
  `check-file-size` / `check-i18n` / `check-command-docs` + 其自测；包 node 53/53、desktop-ui
  `npm test` **1648/1648**（三次 typecheck 全部执行）、`npm run build` 产出 `dist`。
  wasm target 的 13 条 warning 均在合并未触碰的文件里（native-only pick 代码），非本轮回归。
- **NOT RUN**：clippy、workspace 全量测试、`check-tauri-capabilities` / `check-fixture-manifest` /
  `check-workflows`、android/macos/windows 目标、`npm ci`、真实 GUI/GPU/像素/真机。
- **边界**：合并只是让 `main` 拥有这份代码与契约证据，不改变任何平台验收状态（见平台表）。

## 2026-11-13 插件 API M2.2：文档列表与 revision

- **新增读取**（同一 `cad_api_call` / `CadApiSnapshot`，无新命令、无队列）：
  `documentManager.documents` → `{documents:[descriptor...]}`（0 或 1；无图时 `[]`），
  `database.revision`（参数 `{database}`）→ `{revision}`。拒绝：`cad_waiting`（无快照）、
  `cad_api.no_document`、`api.invalid_argument: database`；未知操作仍先拒绝。
- **客户端**：`documents()` 解码列表；新增 `Database.revision(): Promise<number>`（每次实时读，
  不缓存、不从开图时的 `version` 推导，因为它是 `expectedRevision` 的比较基准）。能力表
  **21 行 = 18 pending + 3 supported**，JSON 与包内表逐行一致，Rust 断言按名字钉住三项。
- **边界**：符号表、选择、编辑器提示、事务、命令派发仍 pending；镜像滞后一个 owner tick；
  Windows/macOS/Android 未运行。
- **证据**：`cargo test -p cad-ui-core --lib session::api` 21、`-p app-web` 123、
  `plugin_api_status` 6/6、`fmt`、`check-architecture` / `check-file-size` / `check-i18n` 通过；包 node 53/53、
  desktop-ui plugin-api+host-adapter 70/70、`npm test` 整条 **1648/1648**。
  **NOT RUN**：真实 GUI 读取、GUI/GPU/像素/真机、Windows/macOS/Android/Web 运行。

## 2026-11-13 插件 API M2.1：`cad_api_call` + 活动文档只读

- **新增**：共享 wire 命令 `cad_api_call`（`reads` 车道，args `{namespace,operation,parameters?}`，
  回 `{ok,value?,error?}`）。唯一真实操作
  `ApplicationServices.documentManager.mdiActiveDocument` → `{document:{id,name,isReadOnly,
  database:{id,version}}}` 或 `{document:null}`；其余操作 `unsupported-runtime: cad_api.<ns>.<op>`。
- **快照语义**：`CadApiSnapshot{document}` 区分"还没有 tick 镜像"（`cad_waiting`）与"会话存在但
  没有文档"（`ok:true` + `document:null`）；`native-below` 由 4 个原生 tick 镜像
  `Shared.cad_api_snapshot`，`dom-canvas` 直接读实时 `CadSession`。`cad-db` 仍是唯一权威，
  镜像只读、不回写。
- **客户端**：`documentManager.mdiActiveDocument()` / `documents()` 改为 **Promise 方法**（wire 是
  异步的；同步属性只能靠过期缓存）；`DocumentDescriptor` 的数据库身份改为 `{id,version}`，
  `Database.id` 取代借用的 `handle` 名。能力表 **19 pending + 1 supported**，由 JSON 与包内表
  逐行一致并由契约测试按名字钉住；adapter 仅转发该一项，其余仍 `api.adapter-missing` 且不触达
  invoke。
- **边界**：`documents` 列表、符号表、revision、编辑器提示、事务、命令派发仍 `pending`；
  镜像最多滞后一个 owner tick；Windows/macOS/Android 的 tick 赋值未在该平台编译/运行。
- **证据**：`cargo test -p cad-ui-core --lib` 607、`-p app-web` 123、`wire_goldens` 4/4、
  `cargo check -p app-linux --all-targets`、`fmt`、`check-architecture` / `check-file-size`；
  包 node 48/48、desktop-ui plugin-api+host-adapter 66/66、`npm test` 整条 **1639/1639**。
  **NOT RUN**：真实 GUI 读取、GUI/GPU/像素/真机、Windows/macOS/Android/Web 运行。

## 2026-11-13 TypeScript 二次开发 API 骨架（ADR 0034）

- **新增**：`packages/cad-plugin-api`（`@yacr/plugin-api`）——AutoCAD .NET 命名空间镜像
  （ApplicationServices / DatabaseServices / EditorInput / Runtime / Geometry / Colors）+ 参考
  客户端 + manifest 解析；`apps/desktop-ui/src/plugin-api` 宿主 adapter（描述符校验、载体合并、
  `api.adapter-missing` 拒绝）；`cad-ui-core` 的 `cad_api_status` 共享命令与
  `crates/cad-ui-core/api/plugin-api.json` 单一权威。作者文档 `docs/plugin-api.md`。
- **语义**：能力四类拒绝——`pending` → `api.pending: <ns>.<op>`；显式 `unsupported` →
  `unsupported-runtime: cad_api.<ns>.<op>`；宿主表里**没有**该行 → `api.descriptor_missing:
  <ns>.<op>`；`callVoid` 必须拿到 `{accepted:true}`。客户端在触达 wire **之前**门控。
  **本轮 20 行全部 `pending`**（描述符只列宿主操作；Geometry/Colors/manifest 是包内特性，没有
  行）——插件不能读写文档、不能注册可派发命令（显式拒绝，不是占位成功）。事务：嵌套 commit 是
  保存点（`CommitResult{applied:false,revision:null,created:[]}`，不伪造 revision）、`abort` 仅本地、
  `expectedRevision` 必填、commit 信封带 `document`、insert 只接受 Line/Circle/Arc 且带 layer 名 +
  space；`handle` 可空（只作 `source_handle` 出处），图层/块按名字标识，`getObject` 按 `typeName`
  解码。
- **载体**：`cad_api_status` 为 shared+pure，两个载体同一份 JSON；Web 在会话启动前也应答
  （`apps/app-web/src/direct.rs::without_session`），不报假 `no live CAD session`。
- **边界**：插件仍是同权 ES module，不是沙箱；无热重载、无 Worker/签名；M2–M5 见 ADR 0034。
- **证据**：包 `tsc --noEmit` + node 45/45（含 20 行逐行契约、包边界导入扫描）；desktop-ui
  `tsc --noEmit` + `test/plugin-api` 与 `host-adapter` 合计 60/60（含 supported⊆adapter 实现门禁）；
  **`npm test` 整条**（app + 契约包 + `plugins/tch` 三处 typecheck，再加全部 node 套件）
  **1630/1630**——初始化 `plugins/tch` submodule 后跑通，此前 11 个失败全部来自该 submodule 未初始化；
  `cargo test -p cad-ui-core --test plugin_api_status` 6/6、`--lib session::api` 5/5、`wire_goldens`
  4/4、`cargo test -p app-web` 122/122；`cargo check`（cad-ui-core / cad-desktop-shell / app-web
  wasm）、`fmt --check`、`check-file-size` / `check-i18n` / `check-architecture` /
  `check-command-docs` 通过。
  **NOT RUN**：真实 GUI 加载插件、GUI/GPU/像素/真机、Windows/macOS/Android/Web 运行。

## 2026-11-13 发布 v0.1.12（官方命令清单收尾 + 真实 2D 升级 + CI 字体解耦）

- **CI 修复（先于发版）**：`core-quality` 在干净检出上编译失败，根因是 `cad-app` 测试用
  `include_bytes!` 指向被 gitignore 的打包产物 `apps/app-android/font-assets/complex.shx`（不是下载失败）。
  commit `6aada282` 新增 `tests::font_fixture`：`complex.shx` 改为**运行时**按显式目录 → 生成 bundle →
  共享缓存查找，缺面时显式跳过并打印 staging 命令；`core-quality` 只编译、不跑 Rust 测试，因此不再
  下载字体。`fetch-fonts.sh` 同时降并发 12→6、重试 3→5 + 抖动、空响应体按失败处理。
- **版本**：workspace `Cargo.toml`、`Cargo.lock`（35 成员）、四个 `tauri.conf.json`、Flatpak
  metainfo 全部 `0.1.11` → `0.1.12`（commit `7df66a06`）。
- **tag / run**：`v0.1.12`（annotated，指向 `7df66a06`）→ `CI build layers` run
  [`43264539950`](https://github.com/zenglanmu/yacr/actions/runs/43264539950) **11/11 job 绿**
  （linux/windows/macos check + 四个 release + shader + web-build/web-smoke/web-deploy）。
- **发布**：`publish-release` run
  [`43274688545`](https://github.com/zenglanmu/yacr/actions/runs/43274688545) 绿 →
  <https://github.com/zenglanmu/yacr/releases/tag/v0.1.12>（非 draft / 非 prerelease；自动生成 notes
  后用 `docs/release-notes/v0.1.12.md` 覆盖）。
- **artifact（Release API 核对）**：`yacr-0.1.12-linux-x86_64.flatpak` **113,189,968 B**
  + `.sha256` + `.flatpak.gpg`（GPG 签名）；`yacr-0.1.12-windows-x86_64.zip` **107,269,717 B**
  + `.sha256`（unsigned，来自 `yacr-windows-release-unsigned`）；`yacr-0.1.12-macos-universal.tar.gz`
  **151,988,558 B** + `.sha256`（ad-hoc、未公证）；`app-universal-release.apk` **113,993,249 B**
  + `.sha256`（仓库自签名，v2/v3 verified）。合计 486,441,492 B（≈464 MiB）。
- **命令数**：官方 981 条全部有明确状态：**979 已实现（107 完整 + 872 部分）/ 2 同名不同义
  （MEASURE、TRACE）**；README 中英、落地页与 changelog 同步。覆盖率条按 107/872/2 重新配比
  （2px 下限，比例不失真）。
- **落地页（提交侧）**：commit `1c4b3087`；`downloads.json` → v0.1.12（R2 自定义域 + GitHub
  fallback，真实 size/sha256）；`changelog.json` 由 notes 生成；`site.yml` 默认 `release_tag=v0.1.12`；
  `index.html` 版本/下载链接/size/SHA fallback、`docs/landing-site.md`、`scripts/test-landing-site.py`
  同步。
- **落地页部署**：`site.yml` run
  [`43275480945`](https://github.com/zenglanmu/yacr/actions/runs/43275480945) 绿（R2 镜像 + Pages）。
  线上复核：`/downloads.json` = v0.1.12（四个 size 合计 486,441,492）；`/changelog.json` 首版
  v0.1.12（2026-11-13）；页面含 v0.1.12；R2 HEAD `releases/v0.1.12/yacr-0.1.12-linux-x86_64.flatpak`
  **200 / 113,189,968 B**，APK **200 / 113,993,249 B**。
- **NOT VERIFIED**：Windows/macOS/Android 真机、真实 GPU、像素与性能仍未验收；发布、打包、
  上传与落地页部署成功都不是运行时验收。
