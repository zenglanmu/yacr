# 验证与运行证据

本文件记录**实际执行过**的测试与构建。没有样本、没有真机的项目在此显式标注为未执行。

## 环境

- Rust 1.98.1（`rust-toolchain.toml` 固定），`wasm32-unknown-unknown`、
  `aarch64/armv7/i686/x86_64-linux-android` target 已安装。
- JDK 17（Temurin），Android SDK build-tools 34.0.0 与 30.0.3，
  platform android-34 / android-30，NDK 27.0.12077973。
- cargo-apk 0.10.0。
- 锁定依赖：acadrust 0.5.5、slint 1.18.1、wgpu 30.0.1（见 Cargo.lock）。
- 软件 Vulkan：Mesa **lavapipe**（ICD `/usr/share/vulkan/icd.d/lvp_icd.json`，
  adapter `llvmpipe`，`deviceType=CPU`，Mesa 26.0.8-1ubuntu0.3 / LLVM 21.1.8，
  Vulkan instance 1.4.341）。无头渲染证据在 `VK_ICD_FILENAMES=.../lvp_icd.json`
  下取得，见下文“软件 Vulkan 无头渲染”。
- Android：无头模拟器 `emulator-5554`（AVD `dev_api35`，API 35 `google_apis` x86_64，
  KVM + `-gpu swiftshader`），`cargo-apk 0.10.0`。**是模拟器，不是真机。**
- Web：`wasm-bindgen-cli 0.2.129`、Node 22；无头 Chromium
  `Google Chrome for Testing 153.0.8010.12`（Playwright core 1.63，SwiftShader）。

## 已执行的测试

纯核心 crate 单元/契约测试（`cargo test -p ...`）全部通过，含：

- cad-db：事务原子性、revision 递增、空提交不推进 revision、失败不留部分状态、
  Builder 引用校验（悬空图层拒绝）、包围盒。
- cad-geometry：弧长离散容差、bulge 半圆顶点、样条端点、面积/自交/非共面拒绝、
  变换、射线与工作平面求交、局部交点。
- cad-proxy：metafile 边界、未知 opcode 保守降级、缺失缓存不伪造、raw DWG 不与
  graphic_data 混用、超限拒绝。
- cad-import-acadrust：垃圾输入不 panic、超限拒绝、取消生效、DWG 签名检查。
- cad-representation / cad-spatial / cad-scene：提供器选择与歧义拒绝、网格查询、
  相对原点精度、stamp 过期拒绝、变更集局部失效、预算淘汰。
- cad-measure：3D 距离、三点角、非有限拒绝、自交面积拒绝、纸空间拒绝。
- cad-annotations：命令经事务、JSON 往返、未知字段保留、指纹不匹配拒绝、高版本拒绝。
- cad-query：分页、Mixed/Unset、文档切换过期、变更集修订检测。
- cad-app：模式权限在命令层、创建/撤销/重做、过期文档拒绝、图层覆盖不改底图、
  未保存批注离开需决策。
- cad-resources / cad-dependencies / cad-history：路径策略、依赖传播、元数据不触发
  重绘、撤销合并与预算、恢复日志。

`cad-ui-slint` 不在主机运行测试（Linux 宿主缺 fontconfig/freetype 开发头与
pkg-config，本环境无 sudo）；其 Slint 编译由 Android target 检查覆盖。

## 已执行的构建

- `cargo check --target aarch64-linux-android -p cad-ui-slint`：通过。
- `cargo check --target aarch64-linux-android -p app-android`：通过。
- 探针 APK（Slint Android 后端 + `unstable-wgpu-30`）：`cargo apk build` 产出
  `slintprobe.apk`（arm64-v8a，debug 258 MB），证明 Slint+Android 打包路径。
- 本仓库发布 APK：`cargo apk build -p app-android --target aarch64-linux-android
  --lib --release` 产出 `target/release/apk/yacr.apk`（11.6 MB，arm64-v8a，
  使用本地 gitignored 开发密钥签名；manifest package=dev.yacr.app，
  实测 minSdk 23 / targetSdk 30）。
- 本仓库调试 APK：`... --lib` 产出 `target/debug/apk/yacr.apk`（445 MB，调试签名）。

## 真实 DWG 端到端（本轮执行，样本未入库）

2026-10-01 用 `target/debug/cad-cli-tools` 对 11 份从公开 GitHub 仓库临时下载的真实
DWG 执行 `scan` / `proxy-report` / `build-representation` / `measure`。样本只存于
`/tmp/opencode/dwg-samples`，**未写入 `fixtures/manifest`**（授权未核实），因此下列
结果只是“当前代码在真实输入上的行为证据”，**不构成任何兼容性验收**。

GitHub `raw.githubusercontent.com` 被网络策略重置，改由 `api.github.com/.../contents`
的 base64 内容下载。

| 样本 | 版本 | 字节 | sha256 | 来源仓库/路径 |
|---|---|---|---|---|
| AutoCAD_97_98.dwg | AC1014 | 133818 | cad3556a2fe78dc65a084935dd461dc6185947b8482055ceec1588e7380cc5d7 | tinytales03/SampleDWG TestData |
| AutoCAD_2000.dwg | AC1015 | 127929 | 512ed9275a2abd1d6c6465245155762759ab25cc97cf6d1fedf655556977902e | tinytales03/SampleDWG TestData |
| AutoCAD_2004.dwg | AC1018 | 40346 | 0791bc751005adf772f164099b18b3452d691a379eb6dde65a8a563ecc6c8e73 | tinytales03/SampleDWG TestData |
| AutoCAD_2007.dwg | AC1021 | 70304 | 23be31ae795a6cb2185d4776e93a7f7b411599ab77f37cfd2de3e605343e747c | tinytales03/SampleDWG TestData |
| AutoCAD_2010.dwg | AC1024 | 60328 | b1c94caee6a7b13cc59e17acf555f9a8a2a28810b5bf0a43c7fa46f54e602d55 | tinytales03/SampleDWG TestData |
| AutoCAD_2013.dwg | AC1027 | 41593 | 7a55a45a0be09663fb93351d2973bd2a99bc96472f5581de7a7285727aaee9cc | tinytales03/SampleDWG TestData |
| Kitchens-master-1.dwg | AC1032 | 55869 | f26a175bd45f673cdbd52d3c7b3f40a68fa52a7caede6b990df93181114241ba | MadhukarMoogala/fda-dwgcompare testDrawings |
| Kitchens-revised-1.dwg | AC1032 | 55933 | 38b2fda97ecf705f1934e122bcde08623c49190f14716d4df6801876293c5a72 | MadhukarMoogala/fda-dwgcompare testDrawings |
| korean-DBCS-hangul.dwg | AC1032 | 49112 | ce9528a94e93b2fd35c014b464114b7cbe0cbdd5601b895a9c2c6159f8eb8a08 | MadhukarMoogala/aps-automation-customfonts |
| anonymous-names.dwg | AC1032 | 13947 | ea5b55f7e99d2ad412779ef7f3e71ff3bd7f6c4147f6217d936eed02171291b9 | hakanaktt/acadrust tests |
| point_object_id.dwg | AC1032 | 13637 | 9eef9375c77d72881dc202b9fb94b9c414a048dfdfab8374419b44830337c49d | hakanaktt/acadrust tests/datatable |

### 导入结果（B15、B20 修复后）

`scan` 分别报告总实体、模型空间实体与块定义数；`build-representation` 会展开 INSERT；
completeness 反映“实际能画什么”，不再只看解析是否成功。

| 样本 | entities | model | 块定义 | layers | primitives | 绘制类型 | completeness | 模型空间 bounds |
|---|---|---|---|---|---|---|---|---|
| AutoCAD_97_98.dwg | 34 | 33 | 0 | 1 | 30 | 19 line + 11 text | partial（Text/ATTDEF 不可绘） | [0,0]–[73.8,23.0] |
| AutoCAD_2000.dwg | 34 | 33 | 0 | 1 | 30 | 19 line + 11 text | partial（同上） | [0,0]–[73.8,23.0] |
| AutoCAD_2004.dwg | 34 | 33 | 0 | 1 | 30 | 19 line + 11 text | partial | [0,0]–[73.8,23.0] |
| AutoCAD_2007.dwg | 34 | 33 | 0 | 1 | 30 | 19 line + 11 text | partial | [0,0]–[73.8,23.0] |
| AutoCAD_2010.dwg | 34 | 33 | 0 | 1 | 30 | 19 line + 11 text | partial | [0,0]–[73.8,23.0] |
| AutoCAD_2013.dwg | 3 | 3 | 0 | 1 | 3 | 3 text | **missing**（无可绘内容） | [721.4,921.8]–[18318.7,3205.4] |
| Kitchens-master-1.dwg | 782 | 21 | 21 | 1 | 757 | 757 line | **complete** | [-121.9,0]–[241.9,179.5] |
| Kitchens-revised-1.dwg | 782 | 21 | 21 | 1 | 757 | 757 line | **complete** | [-61.3,0]–[241.9,179.5] |
| korean-DBCS-hangul.dwg | 79 | 6 | 2 | 2 | 5 | 2 line + 3 text | partial（Text/Table 不可绘） | [347.5,-11.4]–[825.1,247.0] |
| anonymous-names.dwg | 10 | 5 | 5 | 1 | 0 | —（块内仅 ATTDEF） | **missing** | none |
| point_object_id.dwg | 0 | 0 | 0 | 1 | 0 | — | complete（空图） | none |

Kitchens 的 model=21 就是 21 个 INSERT，757 条线段来自这些 INSERT 展开的块几何；
两份 Kitchens 的 bounds 不再相同（master 最小 x=-121.9，revised 最小 x=-61.3），
说明展开确实按各自实例变换计算，而不是把块库堆在原点。

`point_object_id.dwg` 的对应 DXF `ENTITIES` 段与 DWG 均为 0 实体，实体数为 0 不是缺陷。
图层数经 acadrust 直读核对属实（这些图纸确实只用图层 `0`）。

### 已修复（本轮，审计 B15）

- **块定义不再当作模型空间**：块内实体以新的 `SpaceId::Block(BlockId)` 存入
  `cad-import-acadrust`，`cad-db::model_space()` 不再返回它们（此前 97/98 因大写
  `*MODEL_SPACE` 被重导入成 67 个实体，现为 34）。
- **INSERT 展开**：`cad-representation::ProviderRegistry::build_expanded` 按块定义递归展开
  INSERT，应用实例变换，嵌套片段保留 `InstancePath`；超过 32 层或成环时报告 `Partial`
  并给出 `representation.instance_cycle`，缺块定义报告 `Missing`
  （`representation.missing_block`），不再是静默空成功。
- **bounds 展开**：`DrawingDatabase::bounds` 递归展开 INSERT，Kitchens 的拟合范围现在覆盖
  装配后的真实布局。
- **R14 大小写归一**：`*MODEL_SPACE`/`*PAPER_SPACE` 与混合大小写一并识别
  （`is_space_block_name`）。
- **CLI 统计所有图元**：`build-representation` 报告 `lines/meshes/texts/instances/images`
  分解，不再只看 `Lines`。

### 已修复（本轮，审计 B20）

- **render/pick 不再由解析完整性推导**：`note_capability` 改为接收独立的
  `render`/`pick` 状态，由 `display_support(geometry)` 判定——文本与 Opaque/ACIS 为
  `Unsupported`，INSERT 继承其块定义的解析状态。CLI `proxy-report` 现在如实显示
  `AcDbText/AcDbMText render=unsupported`，Kitchens 的 `AcDbBlockReference render=verified`
  （因其块几何确实可绘）。
- **导入 completeness 反映可绘性**：`aggregate_completeness` 结合 import 诊断与模型空间
  渲染状态；全部可绘才 `Complete`，混合为 `Partial`，完全不可绘为 `Missing`。纯文字图
  （AutoCAD_2013）由 `complete` 改为 `missing`，块库图（Kitchens）保持 `complete`。

### 仍开放的缺陷（未修复）

- **TEXT 不进场景**：`cad-scene/src/lib.rs` 仍跳过 `Text`/`Instance`/`Image`，故
  AutoCAD_2013 这类纯文字图纸 GPU 渲染为空；文字 shaping/图集未实现。`build_expanded`
  之后已无 `Instance` 片段，但 `Text` 仍需专门子系统。
- **ATTDEF 为 Opaque**：`anonymous-names` 的块内只有 `AttributeDefinition`，展开后无可绘制
  图元（现在 completeness 如实报 `Missing`，属性文本本身仍未渲染）。
- **截断 DWG** 退出码为 0、`completeness=partial`、实体 0；损坏与合法空图不易区分，
  F01“损坏 DWG”验收未闭环。

### 通过的行为

- AC1014/AC1015/AC1018/AC1021/AC1024/AC1027/AC1032 均能解析并建库（版本覆盖）。
- 非 DWG 输入被拒绝、退出码非零：DXF、随机字节、空文件均报
  `CorruptData(missing AC10xx signature)`。
- 指纹不匹配的批注导入默认拒绝，`--allow-fingerprint-mismatch` 才放行（F09/B10 生效）。
- `measure` 距离/角度/折线长度返回结构化 JSON 与单位；未知单位显示 `DrawingUnits`。
- `render`：原生**有适配器**时驱动无头 wgpu 渲染器真实出帧（lavapipe，见下节）；
  无适配器时以 `gpu_failure` 失败（结构性路径，本机未实际触发）；wasm 仍
  `unsupported`。任何情况都不空成功。
- 本轮同时复核：`cargo test`（核心，117 passed/0 failed）、完整 workspace Wasm `--lib`、
  `cargo fmt --check`、`clippy`（0 警告）、`scripts/check-architecture.py` 均通过。

### 第二组：mlightcad/cad-data 语料

同一日期另取 5 份来自 `mlightcad/cad-data` 的真实 DWG（`cad-viewer` 网页版使用的同一
数据仓库，经 jsDelivr CDN 获取）。样本同样只存 `/tmp`，未入库。

| 样本 | 版本 | 字节 | sha256 |
|---|---|---|---|
| baseline-sample.dwg | AC1032 | 81659 | 4a8e5195e600ac8f6b4e37869cac812fe54c5359b4b97ca19e27ad57422ddc64 |
| canteen.dwg | AC1021 | 2618816 | 818f54cd3b413ce3ab00a6aa849bc29cd8cc8581a39fc31a723691f40141fdbc |
| lockers.dwg | AC1021 | 2245248 | fb82491c63fb4d4b5cb80a534bbfe56ee53c46f201a685d9ce1dabbd4bfb3ee4 |
| map-of-uae.dwg | AC1021 | 195040 | b154073d6edd9b074d5bca40e7ad34bd8b75e89ca67ffb4cc99e10f4f3eb14bc |
| patient-chairs.dwg | AC1021 | 568736 | bba7327d855e0efb665fbd1b5aa280d1a07be7450ea8d9a0ac387d1c4f22f51e |

`cad-cli-tools` 结果（`build-representation` 全部 `failures=0`）：

| 样本 | entities | model | 块定义 | layers | primitives | 绘制类型 | build_ms | completeness |
|---|---|---|---|---|---|---|---|---|
| baseline-sample | 192 | 166 | 9 | 21 | 1086 | 1021 line + 5 mesh + 60 text | 0.5 | partial（Text/Region/Insert…） |
| canteen | 29212 | 25123 | 108 | 14 | 43191 | 42749 line + 119 mesh + 323 text | 978.8 | partial（Text/3dSolid…） |
| lockers | 1834 | 1801 | 3 | 4 | 1829 | 1796 line + 33 text | 8.1 | partial（**仅 AcDbText** 不可绘） |
| map-of-uae | 137 | 129 | 3 | 6 | 231 | 173 line + 26 mesh + 32 text | 2.4 | partial（Text/Insert…） |
| patient-chairs | 11884 | 27 | 27 | 3 | 11855 | 11855 line | 42.1 | **complete** |

要点：

- `patient-chairs`：模型空间只有 27 个 INSERT，展开后 11855 条线段，全部可绘 → `complete`。
- `lockers`：唯一不可绘项是 `AcDbText`；一旦文字渲染落地即可 `complete`。
- `canteen`：2.6 MB / 25k 模型实体，展开 108 个块约 979 ms，无失败；缺口是
  Text/3dSolid，属已知未支持项。
- 所有样本都产出 `text` 基元；`cad-scene` 会跳过未整形的 `Text`。注入 outline/SHX 字体后
  文本被转成线段并绘制。字体来源、`FontEngine` 与 `--font` 用法见 `docs/fonts.md`。
- 文字整形实测（`build-representation --font ...`，lines/texts）。注册
  `simplex/txt/romans.shx` + `arial.woff` 后所有文本均整形（SHX 与 outline 均覆盖），
  图纸引用了未注册字体（如 canteen 的 GOST）时靠回退链替代：

  | 样本 | 无字体 | 有字体+回退 | 说明 |
  |---|---|---|---|
  | AutoCAD_2000 | 19 / 11 | 55 / 0 | SHX |
  | AutoCAD_2013 | 0 / 3 | 30 / 0 | outline |
  | baseline-sample | 1021 / 60 | 3298 / 0 | outline |
  | lockers | 1796 / 33 | 2747 / 0 | outline |
  | map-of-uae | 173 / 32 | 545 / 0 | outline |
  | korean-DBCS-hangul | 2 / 3 | 96 / 0 | 回退 |
  | canteen | 42749 / 323 | 44964 / 0 | GOST SHX 未注册，靠回退 |

- **DIMENSION** 现展开其匿名块（`*D...`）的线/箭头/文字，`canteen` 的 28 处标注带来
  +224 线段 / +28 文字；无块名或块缺失时记 `Partial`，不伪造几何。
- **HATCH** 现渲染：边界环恒绘制；单环实心填充经简化后耳切三角化；图案填充按偶奇扫描线
  生成图案线（含 dash 与双线）；多环实心/渐变记 `Partial`（只画边界），超过
  `MAX_FILL_POINTS` 的复杂单环也记 `Partial` 而不做三次方裁剪。`baseline` 86→1021 线
  +5 网格、`map-of-uae` 87→173 线 +26 网格、`canteen` +1155 线 +119 网格。

## 软件 Vulkan（lavapipe）无头渲染（本轮执行）

运行前 `export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json` 强制 Mesa
lavapipe（CPU）。适配器报告：`backend=vulkan`、`device_type=cpu`、`driver=llvmpipe`、
`driver_info=Mesa 26.0.8-1ubuntu0.3 (LLVM 21.1.8)`。

### 渲染器测试（lavapipe）

`cargo test -p cad-render-wgpu`：

| 套件 | 结果 |
|---|---|
| 单元（geometry / draw-order / renderer） | 31 passed |
| `tests/headless_render.rs` | 6 passed |
| `tests/render_effects.rs` | 6 passed |
| `tests/wgsl_validation.rs` | 3 passed |

覆盖：适配器报告；矩形线框出帧且内部保持背景；三角形网格 `triangles>0`；`render_3d`
非空；PNG 签名/ IHDR；未渲染回读 `NotInitialized`；透明 `alpha=0.5` 合成且与 `alpha=1.0`
像素不同、`alpha=0` 计 `invisible_batches`；超预算 `over_budget.skipped_batches>=1`；
`note_device_lost` 后 `DeviceLost`；同场景两次渲染逐字节一致；大 `local_origin` 不丢精度；
PNG 解码逐字节往返。无适配器时测试显式跳过并打印，不假装通过。

### 真实 DWG 出图（样本未入库）

`cad-cli-tools render <dwg> --png <file> --width 800 --height 600`：

| 样本 | draw_calls | scene vertices | non_background | coverage | PNG bytes | 墙钟 |
|---|---|---|---|---|---|---|
| patient-chairs | 11855 | 26032 | 22381 | 0.0466 | 10109 | 2.9 s |
| lockers | 1796 | 92052 | 12511 | 0.0261 | 6681 | 1.4 s |
| baseline-sample | 1026 | 2377 | 9195 | 0.0192 | 4927 | 1.1 s |
| map-of-uae | 199 | 112684 | 2031 | 0.0042 | 6532 | 1.2 s |
| canteen | 42868 | 2760779 | 19981 | 0.0416 | 9353 | 8.9 s |

- PNG 经人眼核对：`patient-chairs` 家具平面、`lockers` 柜体、`map-of-uae` 国家轮廓、
  `canteen` 整层平面+立面，均正确居中铺满。
- `baseline-sample` 可见线与表格但画面稀疏，因其 `completeness=partial`（大量
  `AcDbText` 不绘制）——是已知文字缺口，不是渲染失败。
- `anonymous-names` / `point_object_id` 无可绘制批次 → `invalid_input`
  （`no drawable geometry to render`），退出 1，不伪造空帧；缺失文件 → `invalid_input`。
- 相机按**实际绘制批次**拟合；`canteen` 4.3 万 draw call 在默认 1 秒提交界定下会被误报
  `DeviceLost`，无头路径改用 `Renderer::set_poll_timeout(600s)` 后成功
  （见 `docs/headless-render.md` §4.5）。

**不是兼容性/性能验收**：样本仅存 `/tmp`、未入库、未授权；lavapipe 是 CPU 软件渲染，
其耗时不能用于任何性能结论。

## Linux release 打包（本轮执行）

`scripts/package-linux-release.sh` 构建 release `cad-cli-tools`
（`--release --locked --offline`），用 `--help` 验证二进制可运行，可选地用
`scripts/render-smoke.sh` 真实出图，并把 `bin/` + `docs/` + `scripts/` +
`README.md`/`LICENSE`/`THIRD_PARTY_NOTICES.md`/`PACKAGE.txt` 打包：

- 产物：`target/release/dist/yacr-0.1.0-linux-x86_64.tar.gz`（在 gitignored
  `target/` 下，**不入库**；脚本入库，可重复构建）。
- 本轮大小 4241231 字节；sha256 `9ed0e1499ceb3a7b289b9a10a9b9ba721540a1ac52871d5b80435e048e7594f5`
  （打包内含构建时间，重跑哈希会变，仅记录本轮）。
- 打包内 smoke：`VK_ICD_FILENAMES=.../lvp_icd.json` 下
  `render patient-chairs.dwg` 成功（draws=11855，non_background=22381，PNG 10109B）。
- 解包后独立运行 `bin/cad-cli-tools render map-of-uae.dwg` 成功
  （draws=199，non_background=2031，PNG 6532B）。

该包不含真实 DWG/字体/黄金图（`fixtures/manifest` 为空），也不含 lavapipe；
运行时的软件适配器由宿主发行版的 Mesa 提供。

## 集成轮：Android/Web 运行与显示链（2026-10-02 执行）

四个并行 workstream 已合入 `main` 并按下述命令验证。详细证据见
`docs/validation-android.md`、`docs/validation-web.md`、`docs/view-3d.md`、
`docs/render-order.md`、`docs/proxy-support.md`。

### 合入验证（在 main 的集成树上执行）

- 核心测试 `cargo test --workspace --exclude cad-ui-slint --exclude app-android
  --exclude app-web --locked`：**573 passed / 0 failed**（33 个测试目标）。
- `VK_ICD_FILENAMES=.../lvp_icd.json cargo test -p cad-render-wgpu --locked`：
  **47 passed / 0 failed**（含新增透明合成测试，真实提交到 lavapipe）。
- `cargo fmt --all --check`、`cargo clippy ... --all-targets --locked`（0 warning）、
  `python3 scripts/check-architecture.py`（23 packages）、
  `cargo check --workspace --lib --target wasm32-unknown-unknown --locked`、
  `cargo check --target aarch64-linux-android -p cad-ui-slint -p app-android --locked`、
  `python3 scripts/check-i18n.py`（116 keys）、`check-workflows.py`、
  `check-fixture-manifest.py`：全部通过。

### Android：模拟器真实运行（**非真机**）

- x86_64 release APK（约 12.8 MB，本地开发签名）在 `emulator-5554` 安装、启动；
  进程存活，无 `FATAL`/`ANR`；内置演示几何真实渲染；后端日志
  `preference=WebGpu actual=WebGpu ... max_texture_dimension=8192`，底层为模拟器
  SwiftShader 上的 Vulkan（wgpu）+ Skia 合成。合入四个 workstream 与宿主
  `sync_session` 接线后重建的 APK 再次安装运行（pid 4625），画布拖动像素 diff
  **12951/2592000（0.50%）**，无崩溃——合并未破坏渲染与导航。
- 修复：Android 宿主此前未接 `ViewInput`，画布拖动 **0 像素变化**；修复后拖动像素 diff
  **21415/2592000（0.83%）**，点“适应”恢复到基准。截图与 logcat 见
  `docs/evidence/android-runtime/`。
- 局限：`safe_insets`/surface 尺寸未接线（顶部工具栏被状态栏遮挡，打开 DWG **NOT RUN**）；
  量测/批注拾取未安装；图层/布局/批注/诊断面板状态未推送；SAF 未实现；真机未运行。

### Web：无头 Chromium 真实运行（WebGL2）

- `scripts/build-web.sh` 产出 `web-dist/`；工作流构建 `pkg/yacr_bg.wasm`
  14,792,865 字节（sha256 `e742d153…`），合入 3D 接线与本轮宿主改动后的集成重建为
  14,915,522 字节（sha256 `a49c3c6b…`）；`serve-web.py` MIME 为 `application/wasm`。
- `check-web-ui.mjs` 在 Chrome for Testing 153 下通过（集成重建的 `web-dist` 再次运行
  通过）：`chosen=WebGl2 adapter=Some(WebGl2)`，CAD 区域非空，导航后画面变化，语言
  `zh-CN→en` 切换保留文档并持久化，`consoleErrors=0`、`pageErrors=0`。
- 修复：桥固定 WebGPU、Slint 缺 `renderer-femtovg-wgpu`、wasm 轮询误判设备丢失、
  WebGL2 多重采样 present 失败；B29 冒烟脚本改为裁剪 CAD 区域、导航后必须变化、
  隐藏页停止轮询、意外错误失败。
- 局限：WebGPU（本机无 `navigator.gpu`）、真实 GPU、移动/桌面浏览器矩阵 **NOT RUN**；
  wasm 未做 `wasm-opt`。

### 3D / 纸空间宿主接线与透明

- `cad-ui-slint` 桥现在按 `SpaceSelection` 与 2D/3D 模式分派 `render`/`render_3d`，
  UI 暴露 2D/3D、投影、标准视图、拖动轨道与布局选择；退化相机/不支持布局显式诊断。
  宿主编排者补齐 `CadView::sync_session`（空间+相机+模式）。GPU 上的真实 3D 出图
  未在真机观察。
- 底图透明度：acadrust `Transparency` → `DisplayFragment.alpha` → `RenderBatch.alpha`；
  代理保留全部片段与真实来源/精度。软件 Vulkan 透明合成测试通过。plot-style 表 alpha、
  透明线条深度写入策略仍为显式未实现。

## 集成轮 2：曲线 / ACIS / 布局 / 捕捉（2026-10-02 执行，Linux 核心）

四个核心 workstream 合入 `main`；仅构建/测试 Linux 核心目标（按委托要求），
未构建 wasm/Android（合入前后各做一次 wasm `--lib` 检查）。

- 核心测试 `cargo test --workspace --exclude cad-ui-slint --exclude app-android
  --exclude app-web --locked`：**667 passed / 0 failed**；fmt、clippy（0 warning）、
  架构（23 包）、i18n、`check-fixture-manifest.py` 全通过。
- **曲线（B23）**：`cad-geometry` 新增真实 NURBS（源 degree/knot/weight，
  求值与一阶导、弦高自适应离散）；椭圆保留 OCS 法向（`minor = cross(normal, major)`）；
  非均匀仿射把圆/弧/椭圆重新解算为真椭圆（含镜像翻转）；bulge 在非均匀缩放下解析为
  椭圆弧；线段/圆/弧/椭圆解析交点。详见 `docs/curve-geometry.md`、ADR 0004。
- **ACIS（F15）**：`cad-import-acadrust` 用 acadrust 0.5.5 的 `entities::acis`
  （`SatParser`/`SabReader`/`SatDocument`）把 SAT/SAB 提升为 `cad-kernel-adapter`
  的中性 `BrepData`（acadrust 不越过导入边界）；内核离散平面多边形（含内环/孔）、
  完整无环球面/环面、`sin_half_angle==0`（圆柱）侧面，闭合则 `Success`，否则
  `Partial` + `kernel.missing_face/open_edge/dropped_shell`。**合成** SAT 夹具
  `fixtures/acis/*.sat` 已入 `fixtures/manifest`（6 项，synthetic，本仓库自制）。
  锥面/非圆椭圆/带环球环面/样条面仍 `Unsupported`（明确，不伪造）。
- **布局与测量（F04/F06，B22）**：纸空间视口以真实 4 角矩形重建，比例方向修正
  （1:100 → paper_per_model=0.01）；不支持的视口不发几何并降为 `Partial` + 稳定
  原因码；测量区分纸面/视口模型（经**已验证**逆变换），无有效逆变换则显式禁用模型测量。
- **捕捉与填充**：对象捕捉（端点/中点/圆心/象限/垂足/局部交点）按逻辑像素容差换算，
  只取局部候选、拒绝射线背后点，候选带来源与精度；HATCH 多环实心填充按偶奇支持孔洞，
  超预算/自交/退化仍 `Partial` 并保留边界。见 `docs/measure.md`。

## 集成轮 3：样式 / MTEXT / 导入视口 / 拾取（2026-10-02，Linux 核心）

四个 workstream 合入 `main`（含一个语义冲突的手工整合：`cad-scene::highlight` 补
`RenderBatch` 新增的 color/lineweight 字段）。

- 核心测试 **735 passed / 0 failed**；软件 Vulkan 渲染测试 **50 passed**；fmt、clippy(0)、
  架构、i18n、fixtures 全通过；wasm `--lib` 检查通过。
- **实体样式（§3.2/§7.1）**：importer 按 ByObject→ByLayer→ByBlock 解析颜色（ACI/RGB）
  与线宽（mm）→ `DisplayFragment` → `RenderBatch` → line/mesh shader，lavapipe 帧差证明
  不同颜色渲染不同；线宽被携带但**不绘制**（无便携宽线，显式 `render.lineweight_not_drawn`），
  LINETYPE 虚线留待后续（`docs/entity-style.md`）。
- **MTEXT（§3.2/§7.3）**：`parse_mtext` 解析分组/`\P`/`\H`/`\W`/`\Q`/`\f`/`\C`/`\c`/`\S` 等，
  按 run 整形与换行；堆叠分数/颜色/装饰/`\A` 等为**显式 Partial**（带诊断码，不冒充保真）；
  未知/畸形转义按字面降级不 panic（`docs/mtext.md`）。
- **导入视口/块/OCS（B22/B31）**：4 角纸空间裁剪 + 完整 model→paper 变换（1:100 →
  paper_per_model=0.01），INSERT 基点/OCS/旋转/阵列，SOLID/3DFACE 边界顺序与 2D OCS 抬升；
  扭转/透视/复杂裁剪显式 Partial（`docs/layouts.md`）。**round-2 的导入端缺口已闭合。**
- **拾取与高亮（F05/F14）**：网格面经 `face_sources` 映射为 `SubElementId`，两个 INSERT
  实例可独立区分；新增选择高亮叠加层（独立 `highlight.rs`，带可配置 tint/alpha，不修改
  权威场景；隐藏选择为空、不可解析来源显式报告）（`docs/picking-3d.md`）。

## 集成轮 4：线型 / 出图 / 预算 / 渐变填充 / 异步导入（2026-10-02，Linux 核心）

五个 workstream 合入 `main`。除各自的语义冲突外，整合期修复了两处跨 workstream 的
构造点（`DisplayFragment`/`RenderBatch` 新增 linetype 字段；网格批次新增顶点色，
故上传字节预算计入颜色缓冲）。

- **LINETYPE 虚线（§3.2/§7.1）**：importer 读 `line_types` 表与实体
  `linetype`/`linetype_handle`/`linetype_scale`（含 `$LTSCALE`），按
  ByObject→ByLayer→ByBlock 解析；`cad-geometry::dash` 按弧长把折线细分 dash/gap
  子折线再提交（渲染器仍是普通 `LineList`）；复杂线型的形状/文字段丢弃但保留 dash，
  未知/退化模式显式 `Partial`（`import.linetype_*`/`representation.linetype_fallback`）。
- **出图（打印）**：新增 `PlotSettings` 导入（独立 `PLOTSETTINGS` 优先，否则内嵌
  `Layout` 字段）+ 纯几何排版规划器（纸张 mm/边距/比例/旋转）+ CLI `plot` op，
  用既有 headless `read_target_rgba`+`encode_png` 输出 PNG；缺数据用显式 A4 默认页
  （`DefaultPage{reason}`），不伪造厂商值。无矢量 PDF/HPGL，无 CTB/STB
  （`docs/plot.md`）。
- **性能与预算（§8）**：`SceneBudget` 的 `cpu_bytes`/`queued_tasks`/
  `upload_bytes_per_frame`（含顶点色）全部真实计费且超限返回类别原因（不再静默丢弃）；
  `MeasuredTimings/MeasuredMemory` 记录真实 parse/build/完整时间与内存类别，CLI
  `benchmark` 输出可复现 JSON（`claim: "measurement, not compatibility"`）；无 GPU 设备
  的阶段显式 `null`（`docs/performance.md` 重写）。
- **渐变 HATCH**：读 `Hatch.gradient_color`，按既有偶奇填充三角化把渐变**烘焙为逐顶点
  颜色**（新增 `Mesh::colors` → `RenderBatch::colors` → 第三顶点缓冲 `@location(2)`，
  非渐变网格默认白色、字节不变）。`LINEAR`/`SPHERICAL`/`CYLINDER` 为 `Complete`，
  其余渐变种类显式 `Partial`。附带修复 `fill_rings` 在锥形带（三角形边界）误判退化
  的缺陷（也影响实心填充）（`docs/hatch-gradient.md`）。
- **异步可取消导入（F01）**：`ImportPhase`/`ImportProgress`/`ImportProgressSink` 与
  `Importer::import_with_progress`；`cad-app` 无 Tokio 的 `std::thread` worker
  （`ImportManager`/`ImportJob`）以 `TaskStamp` 守卫发布；取消 → `Cancelled`，
  被取代 → `StaleResult`，**过期结果绝不发布**；合成四线 DWG 契约夹具
  （`docs/import-async.md`）。

集成测试（lavapipe）**824 passed / 0 failed**；fmt、clippy(0)、架构、i18n、fixtures、
wasm `--lib` 通过。默认 Vulkan loader（未强制 `VK_ICD_FILENAMES`）下
`cad-render-wgpu::render_effects` 会因环境中存在损坏 ICD 而在并发创建设备时偶发
SIGSEGV；强制 lavapipe 后稳定通过，与本轮 CPU 侧改动无关。

## 未执行（明确标注）

- **Android 真机**：未运行（仅模拟器 SwiftShader）。SAF、surface 尺寸/安全区、
  量测/批注拾取、面板状态推送未接线。异步导入的宿主进度面板/后台打开按钮未接线。
- **WebGPU / 真实 GPU**：未运行；Web 仅无头 Chromium 的 WebGL2 软件路径。
- **ACIS（F15）真实样本**：已用 acadrust 解析 + 中性 B-rep 离散平面/球/柱/环面子集，
  但夹具均为**本仓库自制合成 SAT**；无授权 3DSOLID/BODY/REGION/SURFACE 真实样本，
  锥面/带环球面/非圆椭圆/样条面仍 `Unsupported`。真实图纸上的 ACIS 端到端 **未运行**。
- **出图**：仅光栅 PNG；无矢量 PDF/HPGL/SVG、无 CTB/STB 打印样式、无打印设备配置、
  无黄金图。
- 桌面/iOS/macOS/Windows 宿主：**未构建**；仅 `cad-platform` 抽象。
- 授权真实 DWG/字体/黄金图入库、跨后端对照、**手机内存预算与 FPS 实测**：仍未有
  授权样本与真机测量；`docs/performance.md` 只记录可复现的宿主测量方法，
  无兼容性/性能声明。
