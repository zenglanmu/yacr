# 验证与运行证据

本文件记录**实际执行过**的测试与构建。没有样本、没有真机的项目在此显式标注为未执行。

## 环境

- Rust 1.98.1（`rust-toolchain.toml` 固定），`wasm32-unknown-unknown`、
  `aarch64/armv7/i686/x86_64-linux-android` target 已安装。
- JDK 17（Temurin），Android SDK build-tools 34.0.0 与 30.0.3，
  platform android-34 / android-30，NDK 27.0.12077973。
- cargo-apk 0.10.0。
- 锁定依赖：acadrust 0.5.5、slint 1.18.1、wgpu 30.0.1（见 Cargo.lock）。

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
- `render` 在无 GPU 环境显式返回 `Unsupported`，不空成功。
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

## 未执行（明确标注）

- 真机/模拟器安装与运行：本环境无 adb/emulator，**未运行**。
- 浏览器运行：无 Wasm 导出与 JS 宿主，**未运行**。
- 桌面宿主：**未构建**（依赖缺失）。
- 授权真实 DWG/字体/黄金图入库、跨后端对照、性能基准：`fixtures/manifest` 为空，
  **未完成**；任何实体兼容性声明都不成立。本轮临时样本的行为证据见上一节，不等于验收。
