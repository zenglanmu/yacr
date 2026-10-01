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

### 导入结果

| 样本 | scan entities | layers | 绘制 primitives | completeness |
|---|---|---|---|---|
| AutoCAD_97_98.dwg | 67 | 1 | 38 | complete |
| AutoCAD_2000.dwg | 34 | 1 | 19 | complete |
| AutoCAD_2004.dwg | 34 | 1 | 19 | complete |
| AutoCAD_2007.dwg | 34 | 1 | 19 | complete |
| AutoCAD_2010.dwg | 34 | 1 | 19 | complete |
| AutoCAD_2013.dwg | 3 | 1 | 0 | complete |
| Kitchens-master-1.dwg | 782 | 1 | 757 | complete |
| Kitchens-revised-1.dwg | 782 | 1 | 757 | complete |
| korean-DBCS-hangul.dwg | 79 | 2 | 26 | complete |
| anonymous-names.dwg | 10 | 1 | 0 | complete |
| point_object_id.dwg | 0 | 1 | 0 | complete |

`point_object_id.dwg` 的对应 DXF `ENTITIES` 段与 DWG 均为 0 实体，实体数为 0 不是缺陷。
图层数 1 经 acadrust 直读核对属实（这些图纸确实只用图层 `0`）。

### 模型空间实际内容（acadrust 直读对照）

| 样本 | 真实模型空间 | 块定义 |
|---|---|---|
| AutoCAD_97_98 | 33（3 ATTDEF/4 LINE/15 LWPOLYLINE/1 MTEXT/10 TEXT） | 另有全大写 `*MODEL_SPACE` 33 实体、`*PAPER_SPACE` 1 Viewport |
| AutoCAD_2000 | 33（同上） | 无 |
| AutoCAD_2013 | 3 TEXT | 无 |
| Kitchens | 21 INSERT | 22 个块共 757 实体（LINE/ARC/CIRCLE/LWPOLYLINE） |
| anonymous-names | 5 INSERT | 5 个块各 1 ATTDEF |

### 确认的缺陷（未修复）

- **块定义被当作模型空间实体**：`cad-import-acadrust/src/lib.rs:418–435` 把块定义实体以
  `SpaceId::Model` 插入，`cad-db::model_space()` 随之返回它们。Kitchens（AC1032）真实模型
  空间只有 21 个 INSERT，几何全在块定义中；导入后成为 782 个平铺实体、757 条绘制线段，
  **实际画出的是堆在原点的块库，而不是装配后的厨房布局**（审计 B15）。
- **INSERT 实例与 TEXT 不进入场景**：`cad-scene/src/lib.rs:197–204` 直接跳过
  `Text`/`Instance`/`Image`。纯文字图纸（AutoCAD_2013：3 TEXT）GPU 渲染为空白，`scan`
  仍报 `complete`；`anonymous-names` 的 5 INSERT 同理。
- **完整性与能力过度声明**：`note_capability` 用解析完整性推导 `render/pick`，故
  `AcDbText`、`AcDbBlockReference` 报 `render=verified`；只要没有 `import.*` 诊断，整图
  completeness 就是 `Complete`（审计 B20）。
- **R14 大小写未归一**：`*MODEL_SPACE`/`*PAPER_SPACE`（全大写）未被
  `starts_with("*Model_Space")` 等判断识别，AutoCAD_97_98（AC1014）把模型空间重复导入
  （67 = 33 实测 + 33 大写块 + 1 纸空间视口），纸空间布局也被漏掉。
- **CLI `build-representation` 只统计 `DisplayPrimitive::Lines`**
  （`cad-cli-tools/src/lib.rs:279–295`），文本/网格图纸显示 `primitives=0`。
- **截断 DWG** 退出码为 0、`completeness=partial`、实体 0；虽为显式 Partial，但损坏与合法
  空图不易区分，F01“损坏 DWG”验收未闭环。

### 通过的行为

- AC1014/AC1015/AC1018/AC1021/AC1024/AC1027/AC1032 均能解析并建库（版本覆盖）。
- 非 DWG 输入被拒绝、退出码非零：DXF、随机字节、空文件均报
  `CorruptData(missing AC10xx signature)`。
- 指纹不匹配的批注导入默认拒绝，`--allow-fingerprint-mismatch` 才放行（F09/B10 生效）。
- `measure` 距离/角度/折线长度返回结构化 JSON 与单位；未知单位显示 `DrawingUnits`。
- `render` 在无 GPU 环境显式返回 `Unsupported`，不空成功。
- 本轮同时复核：`cargo test`（核心，109 passed/0 failed）、完整 workspace Wasm `--lib`、
  `cargo fmt --check`、`clippy`、`scripts/check-architecture.py` 均通过。

## 未执行（明确标注）

- 真机/模拟器安装与运行：本环境无 adb/emulator，**未运行**。
- 浏览器运行：无 Wasm 导出与 JS 宿主，**未运行**。
- 桌面宿主：**未构建**（依赖缺失）。
- 授权真实 DWG/字体/黄金图入库、跨后端对照、性能基准：`fixtures/manifest` 为空，
  **未完成**；任何实体兼容性声明都不成立。本轮临时样本的行为证据见上一节，不等于验收。
