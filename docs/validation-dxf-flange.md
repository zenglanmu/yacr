# QCAD `flange` DXF 离屏渲染验证（2026-10-03）

本轮按用户要求把 QCAD 的 `flange.dxf` / `flange.png` / `flange.pdf` 作为渲染回归
样本提交进仓库，记录来源，补测试代码并跑 Linux 原生离屏渲染；随后实现无匿名块的
`DIMENSION` 显示几何合成。通用步骤见 [testing-dwg.md](testing-dwg.md)；本文只记录
本轮实际执行的证据。

## 1. 样本与来源

提交位置 `fixtures/dxf/qcad-flange/`，来源与 SHA-256 详见同目录
[SOURCE.md](../fixtures/dxf/qcad-flange/SOURCE.md)，并登记在 `fixtures/manifest`。

| 文件 | 字节 | SHA-256 | 说明 |
|---|---|---|---|
| `flange.dxf` | 287223 | `df469e7e91e38c901fced51c9621c8d2b6b6568678803e563b29c9ecf8dec84b` | AC1027 ASCII DXF，被测图纸 |
| `flange.png` | 95087 | `26d730aa0a758adbe9cab8e2b05a17266a715649442c49452f53f5098e3cea36` | QCAD 上游 1024×768 参考图，仅人工参考 |
| `flange.pdf` | 199962 | `b33726c83edf31710268b9b143574d91520770d2fe704ffbb00ced468c6f830b` | QCAD 上游单页矢量参考 |

来源：<https://github.com/qcad/qcad> `examples/`，`master` `dcf5754b0a19d8e57eddd467d18bf802ac12c2e2`，
2026-10-03 经代理 `http://192.168.8.1:10809` 下载，字节未改写。许可见 SOURCE.md
（QCAD LICENSE.txt：源码 GPLv3 附加例外，图标/文档 CC BY 3.0；`examples/` 无逐文件声明）。

## 2. 实现：无匿名块的 DIMENSION 合成

`flange.dxf` 的 6 个 `DIMENSION` 没有匿名块（group 2 为空），原先只落为 `Opaque`、
报告 `Partial`。新增 `crates/cad-import-acadrust/src/dimension.rs`：

- 有匿名块仍沿用 `INSERT` 展开（`canteen` 等路径不变）；无块名且块不存在时才合成。
- 线性/对齐/半径/直径：由定义点 + `DIMSTYLE`（DIMASZ/DIMEXO/DIMEXE/DIMTXT/DIMGAP/
  DIMTAD/DIMLFAC/DIMDEC/DIMZIN）生成尺寸界线、尺寸线、实心三角箭头（`Mesh`）与测量
  文字；`<>` 占位符与 `R`/`⌀` 前缀、`DIMZIN` 去尾零、`DIMSCALE` 缩放均处理。
- 文字样式经 handle→TEXTSTYLE 解析，字体由宿主 `--font name=path` 提供。
- 角度/坐标/圆弧长/大半径、以及非世界 XY 平面的标注仍显式 `Partial`，不猜测绘制。
- 单元测试 `dimension.rs` 6 项：对齐尺寸的线/箭头数、文字落点 (0,-28.125)、竖直
  文字旋转、半径/直径测量、去尾零与 `<>` 替换。

完整性文案由“no display representation for:”改为“display representation not
verified for:”，因为无块标注现在**有**显示几何，剩余未验证的是依赖宿主字体的文字。

## 3. 测试代码

- `scripts/check-dxf-reference.py`（仅标准库）：跑 `scan` → `build-representation`
  → `render`，断言导入/表示/非空帧/适配器，解码渲染 PNG 与参考 PNG 输出
  `summary.json` 及 `review-side-by-side.png` / `review-overlay.png`；支持重复
  `--font name=path`。它**不**断言 SSIM/IoU 等保真分数。
- `crates/cad-cli-tools/tests/dxf_fixture.rs`（无需 GPU，进默认 `cargo test`）：固定
  导入、毫米、`Partial`；表示非空、`meshes >= 8`（合成箭头）、`texts >= 6`（标注文字）。

## 4. 实际执行

环境：`target/release/cad-cli-tools`；`VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json`；
真实 wgpu/Vulkan，适配器 **CPU / llvmpipe**，Mesa `26.0.8-1ubuntu0.3`（LLVM `21.1.8`）。
无 X11/Wayland、无 Slint、无浏览器。

```bash
cargo build -p cad-cli-tools --release --locked
# 无字体：几何 + 未整形文字占位（文字不绘制）
python3 scripts/check-dxf-reference.py --out /tmp/opencode/yacr-dxf-reference
# 带宿主回退字体：测量文字与 MTEXT 整形为线段
python3 scripts/check-dxf-reference.py \
  --font "Arial.ttf=/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf" \
  --out /tmp/opencode/yacr-dxf-reference-font
cargo test -p cad-cli-tools --test dxf_fixture --locked
```

| 阶段 | 无字体 | 带 `Arial.ttf` 回退 |
|---|---|---|
| `scan` | 419 entities，223 model，13 layers，Millimeter，`status=partial`（`display representation not verified for: AcDbDimension, AcDbMText`） | 同左 |
| `build-representation` | 329 primitives：311 lines + 11 meshes + 7 texts，0 failures | 344 primitives：333 lines（文字整形）+ 11 meshes + 0 texts，0 failures |
| `render` 1024×768 | 14413 非背景像素（1.83%），322 draws，11 triangles | 14730 非背景像素（1.87%），344 draws，2333 vertices |
| 取消尾零后的标注文字 | `42`、`60`、`2`、`6`、`R5`、`⌀28` | 同左（整形为线段） |

`cargo test -p cad-cli-tools --test dxf_fixture` → **2 passed**。退出码 0、非空 PNG、
`error=None` 只说明出图 smoke 通过，不单独证明视觉正确。

## 5. 三层结论

1. **打开通过**：真实导入该 DXF，实体数/空间/单位合理。`Partial` 现在只指文字依赖宿主
   字体（`display representation not verified for: AcDbDimension, AcDbMText`），不再是
   “标注完全没画”。
2. **出图 smoke 通过**：真实 lavapipe 适配器提交并回读出非空帧。
3. **视觉验收：模型空间几何+标注通过；纸空间图框未覆盖**。渲染图与 QCAD 参考图对照：
   四个视图（主视/剖视/俯视/轴测）、中心线、剖面线一致；`42`/`60`/`2`/`6` 尺寸线、
   实心箭头、`R5` 半径引线与 `⌀28` 直径文字现在绘出，与参考位置吻合。缺项：QCAD 参考
   是整张纸空间图纸（图框、标题栏、`Flange`/`QCAD.org` 文字），而 CLI `render` 只画模型
   空间，故这些不在本图；极小的 `2` 标注 QCAD 会把文字移到尺寸线外侧，本实现仍按默认
   落点。字体是 `LiberationSans` 注册为 `Arial.ttf` 的**显式回退**，非原字体排版验收。
   粗粒度 64×48 IoU 0.176，只作定位，**不是保真度评分**。

人工对照图：`/tmp/opencode/yacr-dxf-reference-font/review-side-by-side.png`、
`review-overlay.png`（参考=红、渲染=蓝、重合=黑）。

## 6. 未完成 / 不宣称

- 未验证角度/坐标/圆弧长/大半径标注、倾斜平面的标注；这些仍 `Partial`。
- 文字仍需宿主字体；未注册字体时不绘文字，且缺失原 `Arial` 时用回退字体。
- 纸空间 `plot` 已修复：纸张单位从标准纸名（`..._MM)`/`..._Inches)`）解析（锁定
  acadrust 未把 LAYOUT 的 `group 72` 应用，曾把 210mm 当 210in → 纹理超限/空白），
  且默认选择带 viewport 的纸空间布局（flange 首个 `*Paper_Space` 为空，真正的图纸是
  `*Paper_Space1`）。`plot` 现可出纸张边框/标题栏（1191×842，非空像素 20075，9 色）；
  视口比例/位置仍为近似，未做视觉保真验收。回归测试
  `crates/cad-cli-tools/tests/dxf_fixture.rs::flange_plot_defaults_to_the_populated_paper_layout`
  固定“单位=毫米 + 选中 `*Paper_Space1` + 非空帧”。超大 `--dpi` 现返回结构化
  `gpu_failure` 而不是 `create_texture` panic。
- 未验证跨 GPU/后端像素一致、真实 GPU、WebGPU、Android；100% 软件渲染。
- 单样本 smoke 不等于 DXF 兼容性；参考对照是人工辅助，不是授权黄金图矩阵。
- 仓库其它历史验证文档按当时事实保留（那时样本未入库），只更新了当前政策类文字。
