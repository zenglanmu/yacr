# 第三方来源与许可

- acadrust：正式 0.6.8，唯一依赖入口 cad-import-acadrust；未修改上游，未使用 Cargo patch。
  上游 <https://github.com/hakanaktt/acadrust>。DWG/DXF 导入与实验性 DWG 导出都经该适配层；
  采用 API 与对外分发前审计锁定包许可及其传递依赖。
- tobj 4.0.5：Wavefront OBJ 读取器，MIT，上游 <https://github.com/Twinklebear/tobj>。
  仅由 `cad-import-obj` 依赖；`default-features = false`（不启用 `ahash`）以保持
  `wasm32-unknown-unknown` 可编译。
- stl_io 0.11.0：ASCII/二进制 STL 读取器，MIT，上游 <https://github.com/hmeyer/stl_io>。
  仅由 `cad-import-stl` 依赖；纯 Rust、无传递依赖。
- cadmpeg-codec-step 0.6.11：ISO 10303-21（STEP，AP203/AP214/AP242）Part 21 解码器，
  Apache-2.0，上游 <https://github.com/cadmpeg/cadmpeg>。仅由 `cad-import-step` 依赖；未修改。
- cadmpeg-ir 0.6.11：cadmpeg 的中立 CAD IR（拓扑、单位、产品/occurrence、loss 报告与
  `Codec` 契约），Apache-2.0，同一上游。仅由 `cad-import-step` 直接依赖（`Codec`/
  `DecodeOptions` 定义在此 crate，不在 codec crate）；未修改。
- cadmpeg-tessellate 0.6.11：cadmpeg 的 B-rep 逐面三角化器，Apache-2.0，同一上游。
  仅由 `cad-import-step` 依赖；未修改。三个 cadmpeg crate 都未启用额外 feature。
- ifc-lite-core / ifc-lite-geometry 28.2.0：IFC（IFC2X3/IFC4/IFC4X3）Part 21 读取器与
  建筑几何网格化器，**MPL-2.0**，上游 <https://github.com/LTplus-AG/ifc-lite>。仅由
  `cad-import-ifc` 依赖；版本在 workspace 中**精确固定**（`=28.2.0`），未启用额外 feature；
  **两个 crate 的文件均未修改**（未打补丁、未复制其源码，也不修改其源文件）。MPL-2.0 是
  文件级 copyleft：披露义务针对被修改的源文件，本仓库不作任何修改，仅以编译产物形式链接；
  其源码可从上游与 crates.io 取得。该 crate 内含若干传递依赖（如 `i_overlay`、
  `geometry-predicates`、`nalgebra`）；实际版本与许可证见 Cargo.lock，分发前需做完整依赖审计。
- rvt 0.17.0：Autodesk Revit（`.rvt`/`.rte`/`.rfa`/`.rft`）的 clean-room 读取器与
  IFC4/glTF/plan-SVG 导出器，**Apache-2.0**，上游 <https://github.com/DrunkOnJava/rvt-rs>。
  仅由 `cad-import-rvt` 依赖；版本在 workspace 中**精确固定**（`=0.17.0`），未启用额外
  feature。**源码有本地补丁**：上游 0.17.0 的一份完整副本以固定路径 `vendor/rvt` 引入
  （Apache-2.0 允许修改与再分发），只改 `src/ifc/gltf.rs` 的透明度与材质来源标记；改动、
  理由、测试与移除条件见 `docs/rvt-patches.md` 与 ADR-0035，补丁文件本身随仓库分发。
  它是 **preview** 读取器：2008–2023 的构件体是包围盒，2024/2025 为 `partial`，
  2026/2027 为 `experimental`，家族文件没有几何；本仓库不把其输出用于测量。
  其传递依赖（`cfb`、`earcut`、`quick-xml`、`md-5`、`sha2` 等）的实际版本与许可证见
  Cargo.lock，分发前需做完整依赖审计。
- gltf 1.4.1（含 `gltf-json` 1.4.1、`gltf-derive` 1.4.1）：glTF 2.0 / GLB 读取器，
  **MIT OR Apache-2.0**，上游 <https://github.com/gltf-rs/gltf>。仅由 `cad-import-rvt`
  依赖，用于读取 `rvt` 生成的**内部**单缓冲 GLB（不是用户可见格式）；`default-features =
  false` 并只启用 `utils`/`names`/`extras`（`extras` 只读 `rvt` 材质的
  `colourSource`/`alphaSource` 来源字段），因此不引入 `image` 及其编解码依赖；未修改上游
  源码。
- skpwright 0.6.0：SketchUp（`.skp`）的 clean-room 读取器，**GPL-3.0-only**，上游
  <https://github.com/hew3d/skpwright>。仅由 `cad-import-skp` 依赖；版本在 workspace 中
  **精确固定**（`=0.6.0`），未启用额外 feature，上游源码**未修改**（未打补丁、未复制其源码）；
  它只有**一个**依赖 `miniz_oxide`（纯 Rust），本仓库的导入 lane 也是纯 Rust、无平台依赖。
  它是 **preview** 读取器：只输出三角形、实例变换、tag 与材质颜色/不透明度；不导入边/曲线、
  纹理图像与 UV、场景、剖切面、标注，本仓库不把其输出用于测量。主仓许可是 **AGPL-3.0**，
  可与 GPL-3.0 代码组合（AGPLv3 §13）；**skpwright 部分对下游仍按 GPLv3 分发**，不因组合而
  改许可。其唯一传递依赖的实际版本见 Cargo.lock，分发前需做完整依赖审计。
- MiniCAD：轻量开源 2D/3D CAD，仅作为功能与实现参考；未复制其源码，也不是构建或运行时依赖。
- wgpu / Tauri / GTK：桌面 UI 与渲染技术方向。wgpu 仅由 `cad-render-wgpu` 依赖；桌面宿主使用
  Tauri 2（WebView）+ GTK3/WebKitGTK 4.1 + 原生 GTK/wgpu CAD 分区。
  具体版本与传递依赖见 Cargo.lock；
  依赖许可证需建立分发审计。
- OpenCADStudio：历史上曾作为功能规格与 UI 交互参考；该参考练习已于 2026-10-15 结束，本地参考目录 `docs/ui-requirements/` 已移除。其不作为源码采用来源，未复制其代码；此前调查记录保留在 `docs/migration-map.md`，其中记录未复制代码；不构成新的源码/资源复制授权。
- glam/serde/serde_json/thiserror 等实际依赖见 Cargo.lock；依赖许可证需建立分发审计。
- QCAD `flange` 测试样本：`fixtures/dxf/qcad-flange/{flange.dxf,flange.png,flange.pdf}`，
  来自 <https://github.com/qcad/qcad> 的 `examples/`（`master` `dcf5754b`，2026-10-03 经
  操作者代理下载；字节数与 SHA-256 见同目录 `SOURCE.md`）。QCAD `LICENSE.txt` 声明 QCAD 3
  源码为 GPLv3（附加例外）、图标与文档为 CC BY 3.0；`examples/` 无逐文件声明，按上述条款
  使用并保留本署名。仅作渲染回归输入与人工参考，不表示与 QCAD/RibbonSoft 存在关联或背书。
- `fonts/NotoSansCJKsc-Regular.otf`：Noto Sans CJK SC Regular，来自
  <https://github.com/notofonts/noto-cjk> 固定提交
  `f8d157532fbfaeda587e826d4cd5b21a49186f7c` 的
  `Sans/OTF/SimplifiedChinese/NotoSansCJKsc-Regular.otf`，原样分发、未修改。
  字节数 16437364，SHA-256
  `2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b`，
  SIL Open Font License 1.1（文本见 `fonts/NotoSansCJK-OFL.txt`），版权行
  `Copyright 2014-2021 Adobe (http://www.adobe.com/), with Reserved Font Name 'Source'.`。
  未修改即无需按 RFN 改名；仅作为缺字时的默认 CJK 回退轮廓面随发布包分发。
- 未捆绑用户图纸、厂商插件或专有字体；已捆绑的开源 QCAD 样本来源与许可独立核对，
  不由主仓库许可推断。
- `fonts/osifont.ttf`：QCAD（<https://github.com/qcad/qcad>）`fonts/osifont.ttf` 的副本，
  原始字体项目 osifont（<https://github.com/hikikomori82/osifont>），GNU GPL v3 附字体例外。
  下载日期 2026-10-09 经 jsDelivr；来源 URL、字节数与 SHA-256 见 `fonts/SOURCE.md`。
  本仓库为 AGPL-3，可与 GPL 组件组合；仅作为 CAD 文字默认轮廓回退面随发布包分发。
