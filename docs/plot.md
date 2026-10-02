# 打印/出图（plot：纸空间布局 → 光栅 PNG）

规范 §3.3（纸空间布局、视口矩形裁剪、视图变换与比例）、§5.2/§6/§8（离屏渲染）、
§19（CLI 结构化输出）。相关文档：`docs/layouts.md`（视口几何与表示层）、
`docs/headless-render.md`（无头 GPU 与回读纪律）、`docs/cli.md`（CLI 契约）。

本文描述 `plot` 能力的契约与已实现范围：把一个具名纸空间布局按纸张尺寸、页边距、旋转
与打印比例映射到给定像素画布，用现有无头 GPU 回读 + PNG 编码写出光栅文件；并说明导入
的 PLOTSETTINGS 数据、**缺省页**的显式来源，以及明确未实现的部分（矢量 PDF/HPGL、
打印样式表）。

## 1. 能力与数据流

```
DWG ──cad-import-acadrust::plot──▶ cad-db plot_settings 表
                                          │
布局名 ──cad-representation::enumerate_layouts──▶ LayoutId
                                          │
cad-representation::build_paper_space ────▶ 纸面坐标(mm)表示（视口变换已应用）
                                          │
cad-representation::plot::plan_plot ──────▶ PlotPage（像素画布 + paper→pixel 仿射）
                                          │
cad-cli-tools::run_plot ──────────────────▶ 仿射到像素 → SceneCache → wgpu 离屏 →
                                             read_target_rgba → encode_png → PNG
```

- **纸空间布局**：`plot` 操作选择一个具名布局（`--layout <name>`，缺省取第一个布局）。
  布局来自数据库真实布局表，不发明布局。
- **纸张尺寸/页边距/旋转/比例**：来自该布局的 `PlotSettingsRecord`（见 §3）。
- **视口变换**：`build_paper_space` 已把每个受支持视口的 `model→paper` 变换应用到模型
  几何并按视口矩形裁剪；因此 1:100 这类比例由视口携带，`plot` 不重复施加。
- **像素映射**：`plan_plot` 把纸张毫米映射到像素画布，处理页边距、旋转（0/90/180/270
  及任意角，任意角经 `to_radians` 计算）与打印比例（对纸张内容的缩放）。
- **出图**：CLI 用 `Renderer::render` 绘制、`read_target_rgba` 回读、`encode_png` 编码，
  原子写入 PNG。

## 2. 纯规划器（无 GPU）

`cad-representation::plot` 是纯函数模块，无 GPU、无窗口、无数据库副作用，可独立单测：

| 项 | 角色 |
|---|---|
| `PlotScale { numerator, denominator }` | 纸张内容的统一打印比例；`factor()` 取分数 |
| `PlotTarget::Dpi(f64)` / `PlotTarget::Pixels{width,height}` | 画布尺寸的两种给定方式 |
| `PlotPage` | 结果：`width`、`height`、`dpi`、`pixels_per_mm`、`rotation_degrees`、`printable_mm`、`paper_to_pixel` |
| `PlotPage::map_paper_point` | 纸张毫米 → 像素（y 向下，图像约定） |
| `PlotPage::map_model_point` | 模型点经受支持视口 → 像素；不支持视口返回 `None` |
| `plan_plot` / `plan_plot_rotated` | 由记录（+ 显式角度）生成 `PlotPage` |
| `plan_plot_for_record` | 直接使用记录内的 `scale_numerator/denominator` |
| `normalize_degrees` | 角度归一化到 `[0, 360)` |
| `to_millimetres` | 英寸→毫米；`Pixels` 无物理尺寸，显式 `Unsupported` |

语义要点：

- **画布 = 可打印区域**：页边距之外的纸张边框不进入输出，画布尺寸由 `纸张尺寸 − 页边距`
  在旋转后的外接框决定。
- **旋转**：纸面为 y 向上（制图约定），画布为 y 向下（图像约定）；`plan_plot` 的
  `paper_to_pixel` 已包含 y 翻转，90°/270° 交换画布宽高。CLI 再翻转一次得到渲染器需要的
  y 向上像素空间，两者互逆，不会二次翻转。
- **比例**：`PlotScale` 缩放纸面内容本身。**纸空间布局出图通常为 1:1**（`layout_plot_scale`）；
  模型空间 1:100 由视口变换承担。二者分离，避免比例被重复施加。
- **校验**：纸张尺寸/页边距/比例非正、非有限，或页边距吃光可打印区，均返回显式
  `InvalidInput`；`Pixels` 纸张单位无 DPI 时返回 `Unsupported`。

单测覆盖：A4@300dpi、A4 90° 旋转后的画布与角点、页边距扣除、1:100 比例、1:100 视口
模型点→像素、`Pixels` 适配与居中、任意角旋转、单位换算、缺省页。

## 3. 导入：PLOTSETTINGS 与布局内嵌字段

`cad-import-acadrust::plot`（新模块，**不修改** `src/entity.rs`）在布局读取后运行，按优先级
读取两处来源：

1. **独立 `ObjectType::PlotSettings` 对象**（DXF PLOTSETTINGS）：通过 `owner` 指向布局
   对象句柄关联；同一布局下优先级最高。`page_name` 仅作为回退匹配（匹配布局显示名）。
2. **布局对象 `ObjectType::Layout` 的内嵌 plot 字段**（native DWG 的存储方式）。

关联**必须**经句柄：数据库 `LayoutId` 由块记录名（`*Paper_Space`）键控，而 `Layout`
对象携带显示名（`Layout1`）与块记录句柄；按名字直接比较会把两者错配。

映射到 `cad-db::PlotSettingsRecord` 的字段：

| acadrust 字段 | 数据库字段 |
|---|---|
| `PlotSettings.paper_size` / `Layout.paper_size` | `paper_size_name` |
| `paper_width` / `paper_height` | `paper_width` / `paper_height`（原样，单位见 `paper_units`） |
| `margins.{left,bottom,right,top}` / `plot_margin_*` | `PlotMargins` |
| `rotation`（枚举）/ `Layout.plot_rotation`（code） | `PlotRotation`（经 `to_degrees` 归一） |
| `scale_numerator` / `scale_denominator` | 同名字段 |
| `plot_type` | `PlotType` |
| `paper_units` | `PlotPaperUnits` |

`has_paper()` 要求宽高均为正；零尺寸记录视为“无数据”不落库——否则会伪装成已导入但不可用
的配置。落库记录 `provenance = Imported`。

## 4. 精确 vs 缺省/Partial

- **Imported（精确）**：记录来自上述两处来源之一，字段原样保存，`provenance =
  Imported`。CLI JSON 的 `paper.provenance = "imported"`。
- **DefaultPage（显式缺省，非精确）**：布局在本文件中**没有**任何 plot 数据时，
  `DrawingDatabase::plot_settings_for` 返回一张显式的缺省页：**ISO A4（210 × 297 mm）**、
  零页边距、无旋转、1:1，`provenance = DefaultPage { reason }`，CLI JSON 的
  `paper.provenance = "default_page"` 且 `paper.default_reason` 给出原因。A4 是 ISO 标准值，
  不是厂商/打印机配置；缺省页只为“可出图”提供一个中立画布，不声称来自文件。
- **Synthetic（合成场景）**：输入 DWG 没有任何纸空间布局时，`plot` 仍用内存中的合成 A4
  纸面（含矩形框与对角线）走完规划器与编码器，并在 JSON 标记
  `layout.synthetic = true`。合成图仅验证链路，不构成对导入图形的证据。
- **Partial（显式不完整）**：视口若非受支持状态（透视、扭转、倾斜、非矩形裁剪、缺失比例
  等），`build_paper_space` 按既有契约产出 `Partial` 诊断、不猜变换；该视口内容缺席，
  但图纸本身与其受支持内容仍会出图，`completeness` 如实反映。此逻辑与
  `docs/layouts.md` 一致，`plot` 不改写它。

## 5. 回读纪律

`plot` 复用 `cad-render-wgpu` 无头路径，严格遵守 `docs/headless-render.md`：

- GPU 提交封装在渲染器内部；CLI 只经 `cad-render-wgpu` 公开接口拿到图像字节，**不**直接
  `use wgpu`。
- **整幅 GPU→CPU 回读仅发生在“取一帧作证据”的离屏诊断路径**；`plot` 正是这样一次证据
  输出，绝不在生产合成中逐帧回读。
- `renderer.set_poll_timeout(600s)`：软件适配器（Mesa lavapipe）的大帧可合法超时，避免把
  慢速 CPU 帧误报为设备丢失。
- 无适配器时返回显式 `gpu_failure`，绝不空成功；PNG 仅在成功出帧后原子写入。

## 6. CLI 契约

```
cad-cli-tools plot <input.dwg> [--layout <name>] [--dpi <f64>|--width <u32> --height <u32>]
                              [--png <file>] [--out <file>] [--locale <tag>]
```

- `--layout <name>`：布局名（显示名或块记录名，大小写不敏感）；缺省取第一个布局。找不到
  时报 `invalid_input` 并列出可用布局。
- `--dpi <f64>`：由纸张尺寸推导画布；与 `--width/--height` 二选一（给了 DPI 用 DPI）。
  非正/非有限/非数字为用法错误（退出码 2）。
- `--width/--height`：像素画布（缺省 1280 × 720），按纸张外接框等比适配并居中（letterbox）。
- `--png <file>`：PNG 路径；缺省为输入同目录 `<stem>.plot.png`。仅在成功出帧后原子写入。

结果 JSON（stdout，`--out` 时写文件）：

```json
{
  "schema_version": 1,
  "operation": "plot",
  "layout": { "id": 1, "name": "*Paper_Space", "synthetic": false },
  "width": 320,
  "height": 452,
  "dpi": 38.7,
  "paper": {
    "size_name": "ISO_A3_(297.00_x_420.00_MM)",
    "width": 297.0,
    "height": 420.0,
    "units": "millimeters",
    "rotation_degrees": 90.0,
    "printable_mm": [273.0, 396.0],
    "scale": { "numerator": 1.0, "denominator": 1.0, "factor": 1.0 },
    "provenance": "imported",
    "default_reason": null
  },
  "output": "/tmp/a4-layout.plot.png",
  "bytes": 4321,
  "pixels": { "non_background": 578, "coverage": 0.0026, "distinct_colors": 2 },
  "frame": { "draw_calls": 1, "vertices": 2, "triangles": 0 },
  "timings": { "plan_ms": 0.01, "gpu_ms": 169.5, "total_ms": 215.0 },
  "adapter": { "backend": "vulkan", "name": "llvmpipe (LLVM 21.1.8, 128 bits)", "device_type": "cpu" },
  "completeness": { "status": "complete" },
  "status": "ok",
  "note": "raster paper-space plot; no vector PDF/HPGL export and no plot styles (see docs/plot.md)"
}
```

`status` 恒为 `"ok"`（失败走向结构化错误文档，退出非零）。`paper.provenance` 与
`default_reason` 是判断精确性的唯一依据。

## 7. 测试与证据

- `cad-db`：plot 表可缺省 + 缺省页、记录往返（旋转/页边距）、未知布局写入被拒。
- `cad-import-acadrust`：`fixtures/plot/a4-layout.dwg`（**合成**，acadrust `DwgWriter`
  产出）导入后取到独立 PLOTSETTINGS 的 A3/90°/12mm 值；无记录布局解析为显式缺省 A4。
- `cad-representation`：规划器纯单测（§2）。
- `cad-cli-tools`：`plot` 名称解析；缺文件在任何 GPU 工作前 `invalid_input`；未知布局
  `invalid_input`；在 lavapipe 上对合成 fixture 出图，断言 PNG 非空且 IHDR 尺寸与请求一致、
  `non_background > 0`、`paper.provenance == "imported"`。

复现（原生、软件 Vulkan）：

```bash
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json \
  cargo test -p cad-cli-tools -p cad-representation -p cad-import-acadrust -p cad-db --locked
```

实际执行证据由控制器汇总到 `docs/validation.md`，**本文不代填**。

## 8. 明确未实现（gap）

- **矢量导出（PDF/HPGL/SVG）未实现**：`plot` 只输出光栅 PNG，边界为 `encode_png`。
  矢量导出需要新的后端与线宽/字体处理，属独立工作。
- **打印样式表（CTB/STB）未实现**：`current_style_sheet` 未导入、未应用；颜色/线宽按既有
  实体样式渲染，不按打印样式映射。
- **打印机/绘图仪配置未实现**：`printer_name` 未使用；输出尺寸由用户 DPI/像素决定，不查询
  系统打印机能力。
- **`Pixels` 纸张单位**：无物理尺寸，规划器显式 `Unsupported`，需用户给 DPI。
- **渲染色 / 隐藏线 / 着色模式**：`shade_plot_mode`、`shade_plot_dpi`、
  `shade_plot_resolution`、`plot_hidden` 等未应用；`plot` 走统一的 2D 线/网格渲染。
- **打印偏差/居中标志**：`plot_centered`、`origin_x/y`、`paper_image_origin_*` 未应用；
  当前画布总是居中纸张可打印区。
- **黄金图未入库**：合成 fixture 只固定链路，不构成兼容性或视觉黄金证据。

以上未实现项均**不伪造成功**：相关字段不读取即不声称支持；缺省页与合成场景都有显式
provenance 标记。
