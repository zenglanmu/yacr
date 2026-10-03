# CLI（`cad-cli-tools`）

无界面命令行入口，规范 v2.0 §19。所有数据操作都走与 UI 相同的
Application/数据库命令路径，不复制业务实现；输出为结构化 JSON，
错误为结构化 stderr + 非零退出码，绝不返回不透明的成功字符串。

审计条目 **B30（CLI 原子导出/诊断不完整）** 与 **N01 §5.3（机器输出不因语言变化）**
在本轮于 `crates/cad-cli-tools` 内闭环，见本文末尾“已验证 / 仍未闭环”。

## 1. 调用方式

```
cad-cli-tools <operation> <input.dwg> [options]
```

- 成功时 **stdout 仅包含一份 JSON 结果文档**；人类可读文字（若有）全部走 stderr。
- 失败时 stdout 为空，stderr 先打印一行人类提示，再打印结构化错误文档，
  进程以非零状态退出。
- 指定 `--out` 时结果写入文件，stdout 保持为空（见 §4）。

### 选项

| 选项 | 说明 |
|---|---|
| `--notes <file>` | 批注 sidecar 路径（`import-notes` / `export-notes`） |
| `--points "x,y;x,y;..."` | `measure` 的点，图纸单位；2 点为距离、3 点为角度、≥4 为长度 |
| `--out <file>` | 将 JSON 结果**原子**写入 `<file>`；stdout 保持为空 |
| `--png <file>` | `render`：将离屏帧**原子**写为 PNG（仅渲染成功后写入）；`plot`：输出文件路径（任意格式） |
| `--plot-format <fmt>` | `plot`：输出格式 `png`（默认，无头 GPU 光栅）\| `svg` \| `pdf`；`svg`/`pdf` 为纯 CPU 矢量路径，不需要 GPU 适配器 |
| `--width <u32>` | `render`：帧宽（像素，默认 `1280`）；非数字或 `0` 为用法错误 |
| `--height <u32>` | `render`：帧高（像素，默认 `720`）；非数字或 `0` 为用法错误 |
| `--locale <tag>` | **仅**人类 stderr 文字的语言，`zh-CN`（默认）或 `en`；机器输出不变 |
| `--allow-fingerprint-mismatch` | 图纸指纹不匹配时仍导入批注 |
| `--font <name=path>` | 注册 TTF/OTF/WOFF/SHX 字体用于文字成型（可重复；省略 `name=` 时取文件名）；`render` / `plot` 也使用这些字体 |
| `--help`, `-h` | 打印用法并退出 0 |

### 语言（`--locale`）

- 默认 `zh-CN`；`en` 及任意 `en-*` 规范化为 `en`；其它未知 tag 回退 `zh-CN`。
- `--locale` **只影响 stderr 上的人类提示行**（如 `[measure] 错误 ...` vs
  `[measure] error ...`）。JSON 结果的键、`operation`、`schema_version`、
  `error.code`、`error.context`、数值格式一律与语言无关（验收见 §6）。
- 完整 GUI 双语仍由前端负责；CLI 的 `--locale` **不是** GUI 双语的替代品。

## 2. 退出码契约

| 退出码 | 含义 |
|---|---|
| `0` | 成功；stdout（或 `--out` 文件）为完整 JSON 结果 |
| `1` | 操作失败：输入/解析/不支持/权限/写失败等；stderr 为结构化错误文档 |
| `2` | 用法错误（`usage`）：未知操作、缺参、未知选项、参数格式非法 |

未知操作同时打印用法文本（人类可读），随后打印结构化错误文档并退出 `2`。

## 3. 机器结果文档

所有成功结果都含 `schema_version`（当前为 `1`，常量 `CLI_SCHEMA_VERSION`）
与 `operation`（稳定机器键）。各操作的稳定字段：

### `scan`

```json
{
  "schema_version": 1,
  "operation": "scan",
  "entities": 0,
  "model_entities": 0,
  "block_definitions": 0,
  "layers": 1,
  "bounds": { "min": [x, y, z], "max": [x, y, z] } | null,
  "units": "DrawingUnits",
  "completeness": { "status": "complete" | "partial" | "missing" | "unverified", "items": [...] },
  "diagnostics": [ { "code": "...", "message": "<redacted>" } ]
}
```

- `bounds` 在无可绘内容时为 `null`。
- `completeness.items` 只在 `partial`/`missing` 时出现，且**非空**（真实缺口不静默）。
- `diagnostics[].message` 经过 `cad_diagnostics::redact_text` 脱敏；`code` 不脱敏。

### `proxy-report`

```json
{
  "schema_version": 1,
  "operation": "proxy-report",
  "completeness": { "status": "...", "items": [...] },
  "entity_types": [
    { "type": "...", "read": "...", "semantic": "...", "render": "...", "pick": "...", "measure": "..." }
  ],
  "proxy_diagnostics": [ { "code": "...", "message": "<redacted>" } ],
  "note": "..."
}
```

- `entity_types` 按 `type` 排序；能力值为稳定键
  `not_implemented|unsupported|unverified|partial|verified`，与语言无关。
- `proxy_diagnostics` 覆盖**全部** `import.proxy*` 与 `import.unknown*` 诊断；
  无缓存的代理一定会出现，不会被静默吞掉（审计 B30）。

### `measure`

```json
{
  "schema_version": 1,
  "operation": "measure",
  "input_points": [[x, y, z], ...],
  "measurement": {
    "algorithm": "Distance3d" | "Angle3Points" | "PolylineLength" | ...,
    "inputs": [[x, y, z], ...],
    "plane": { "origin": [...], "u": [...], "v": [...] } | null,
    "value": 5.0,
    "units": "DrawingUnits",
    "source": "UserPoints",
    "precision": "Analytic"
  } | null,
  "results": [ { "code": "measure.result", "message": "..." } ],
  "units": "DrawingUnits"
}
```

- `measurement.value` 是 locale 无关的数值；`algorithm`/`source`/`precision`
  为稳定枚举名。`results[].message` 是诊断文本，仅供人阅读。

### `import-notes`

```json
{
  "schema_version": 1,
  "operation": "import-notes",
  "imported": 3,
  "undo_recorded": true
}
```

- 指纹不匹配且未传 `--allow-fingerprint-mismatch` 时返回 `invalid_input` 失败，
  不静默附加。

### `export-notes`

```json
{
  "schema_version": 1,
  "operation": "export-notes",
  "annotations": 3,
  "bytes": 1234,
  "revision": 7,
  "saved": true
}
```

- 先原子写入 sidecar，成功后才按导出的**精确 revision** 标记已保存
  （`confirm_annotation_export`）；写失败则不标记（审计 B07/B30）。

### `build-representation`

```json
{
  "schema_version": 1,
  "operation": "build-representation",
  "primitives": 6,
  "vertices": 40,
  "kind_counts": { "lines": 4, "meshes": 0, "texts": 1, "instances": 1, "images": 0 },
  "failures": [ { "entity": "6", "error": "..." } ]
}
```

- 单个实体构建失败会进入 `failures`（非空即真实故障），不会被静默忽略。

### `benchmark`

```json
{
  "schema_version": 1,
  "operation": "benchmark",
  "file_bytes": 2048,
  "representation_build_ms": 1.23,
  "representation": { /* build-representation 文档 */ },
  "environment": { "release_build": true, "gpu": "not required for geometry benchmark" }
}
```

### `render`

**原生**路径驱动真实的无头 wgpu 渲染器：导入 DWG → 按 provider registry 构建
显示表示 → `SceneCache` 分批 → 无头软件适配器上传并渲染一帧 → 回读 RGBA →
可选写 PNG。wasm 没有可拥有的设备，仍显式 `unsupported`。

以下为一次真实运行（Mesa lavapipe，某授权样张）的字段形状，具体数值随文件与
适配器变化，不是兼容性或性能结论：

```json
{
  "schema_version": 1,
  "operation": "render",
  "adapter": {
    "backend": "vulkan",
    "name": "llvmpipe (LLVM 21.1.8, 128 bits)",
    "device_type": "cpu",
    "driver": "llvmpipe",
    "driver_info": "Mesa 26.0.8-1ubuntu0.3 (LLVM 21.1.8)"
  },
  "width": 800,
  "height": 600,
  "png": { "path": "/tmp/frame.png", "bytes": 8903 },
  "pixels": { "non_background": 22381, "coverage": 0.0466, "distinct_colors": 2 },
  "frame": {
    "draw_calls": 11855, "vertices": 26032, "triangles": 0,
    "opaque_batches": 11855, "transparent_batches": 0, "invisible_batches": 0
  },
  "scene": { "batches": 11855, "vertices": 26032 },
  "completeness": { "status": "complete" },
  "note": "software/headless frame; not a compatibility or performance claim"
}
```

- `png` 仅在传入 `--png <file>` 时非 `null`；PNG 只在渲染成功后**原子**写入
  （临时文件 + rename），渲染失败绝不留下半截图片。
- `pixels.non_background` 统计与左上角背景色差异超过容差 8 的像素数；
  `coverage = non_background / (width*height)`。纯背景帧是真实结果，不伪造像素。
- `completeness` 与 `scan` 一致，来自本次导入报告；没有导入报告时为
  `{"status":"unverified"}`。
- 相机按**实际绘制的批次**（`local_origin + vertex`）拟合，而不是
  `drawing.bounds()`；后者包含未绘制内容（文字、块定义几何），会使出图偏小偏心。
- `SceneCache::build` 只产出线/网格批次；Text/Instance/Image 由各自子系统
  负责，本帧不计入（文档化行为，不是静默丢弃）。
- **无可用适配器**时以 `gpu_failure` 失败并退出 `1`，`message` 显式说明；绝不
  返回空成功（审计条目 F11/§11）。
- 没有任何可绘制批次时以 `invalid_input`（`no drawable geometry to render`）失败，
  不伪造空帧。
- 大图纸（数万 draw call）在软件适配器上可能超过交互式 1 秒提交界定；无头路径
  使用更长的有界等待（`Renderer::set_poll_timeout`），避免把慢的 CPU 帧误报为
  设备丢失（F12）。
- wasm (`target_arch = "wasm32"`) 保持 `unsupported`：浏览器从宿主 canvas 取得
  设备，CLI 在 wasm 下没有无头设备可拥有。

### `plot`

**原生**路径把一个具名纸空间布局按纸张尺寸/边距/旋转/比例出图。`--plot-format png`
（默认）走无头 GPU 回读 + PNG；`--plot-format svg|pdf` 走纯 CPU 矢量路径，不创建 GPU
设备（可在无适配器机器与单元测试中运行）。两路共享同一布局选择、`plan_plot_for_record`
与 `build_paper_space`（视口变换已应用、INSERT 已展开），确保报告与几何不漂移。
wasm 下 `plot`（含矢量）仍为 `unsupported`：该 CLI 宿主没有自己的文档读取与输出写入路径，
只有原生宿主具备；矢量路径虽不需要 GPU，但不代表 wasm CLI 宿主已接线。

结果文档含 `format` 字段；矢量路径给出 `width_mm/height_mm/paths/diagnostics`，输出文件
由 `--png` 指定，缺省在输入名后追加所选格式扩展名（`.plot.svg` / `.plot.pdf`），不会把
矢量写进 `.png` 名字。无法表达为路径的图元逐项 `vector.*` 诊断，不静默丢图。详见
`docs/plot.md` §9。

## 4. `--out` 与原子性

- 指定 `--out <file>` 时，结果 JSON 写入该文件，**stdout 保持为空**。
- 写入是**原子**的：先在目标同目录创建唯一临时文件
  （`.<name>.<pid>.<seq>.tmp`），`write_all` → `flush` → `sync_all`，最后
  `rename` 覆盖目标。
- 任一步失败（创建、写、同步、rename）都会删除临时文件并保持原有目标不变，
  因此**失败的运行绝不会留下半截文件**。
- `--out` 指向无法替换的目标（例如已存在的目录）时返回结构化错误
  `output_write_failed` 并退出 `1`；目录中不留 `.tmp` 残留。
- 操作本身失败时不会创建 `--out` 文件。

## 5. 结构化错误文档

失败时 stderr 在人类提示行之后打印：

```json
{
  "schema_version": 1,
  "operation": "scan",
  "error": {
    "code": "invalid_input",
    "message": "InvalidInput(\"read failed: No such file or directory (os error 2)\")",
    "context": null
  }
}
```

- `error.code` 是稳定机器键（与 locale 无关）：`invalid_input`、`unsupported`、
  `not_implemented`、`resource_missing`、`corrupt_data`、`gpu_failure`、
  `invariant`、`permission_denied`、`cancelled`、`stale_result`、`usage`、
  `output_write_failed`。
- `error.context` 默认 `null`；仅当存在结构化上下文时填入（例如
  `output_write_failed` 含 `{ "path": "..." }`）。文件内容、原始文本与身份值
  不会进入默认错误文档。
- `error.message` 是技术诊断文本，可能随语言变化（人类可读），但**不承载机器契约**。

## 6. 验证（本轮新增契约测试）

`crates/cad-cli-tools/tests/cli_contracts.rs` 通过 `CARGO_BIN_EXE_cad-cli-tools`
调用真实二进制（未引入 `assert_cmd`），覆盖：

- `scan_success_is_pure_json_on_stdout`：成功时 stdout 是纯 JSON，stderr 为空。
- `measure_success_reports_structured_numbers`：测量 `value`/`units` 为结构化数值。
- `missing_input_exits_non_zero_with_structured_error`：退出 `1`，stdout 为空，
  stderr 含 `{schema_version, operation, error:{code,message,context}}`。
- `render_on_empty_drawing_is_an_input_error_not_empty_success`：无可绘制边界时
  退出 `1`、code `invalid_input`，不是空成功。
- `render_zero_width_is_a_usage_error`：`--width 0` 退出 `2`、code `usage`。
- `render_without_png_still_returns_structured_json`（需 `YACR_TEST_DWG` 指向真实
  DWG，否则打印跳过并返回）：退出 `0`、stdout 纯 JSON、`adapter.backend` 非空、
  `pixels.non_background > 0`；传 `--png <tmp>` 时 PNG 以 `\x89PNG\r\n\x1a\n`
  开头且 `png.bytes > 0`。
- `unknown_operation_exits_two_with_usage_code`：退出 `2`，code `usage`。
- `missing_required_points_is_a_non_zero_failure_not_empty_success`：
  输入不足是非零失败，不是空成功。
- `out_writes_file_and_keeps_stdout_empty`：`--out` 写文件、stdout 为空。
- `failed_run_leaves_no_partial_out_file`：`--out` 写失败退出 `1`、
  返回 `output_write_failed`，目录无 `.tmp` 残留。
- `failed_operation_never_creates_the_out_file`：操作失败不创建 `--out`。
- `locale_does_not_change_machine_keys`：`zh-CN` 与 `en` 的成功 JSON 完全相等，
  错误文档机器键也完全相等。

`crates/cad-cli-tools/tests/dxf_fixture.rs`（无需 GPU）固定已提交的 QCAD flange 样本：
`committed_qcad_flange_scan_reports_millimetres_and_partial`（导入、毫米、`Partial` 原因）
与 `committed_qcad_flange_builds_a_non_empty_representation`（无失败、line/arrow
`meshes >= 8`/标注 `texts >= 6`），防止无匿名块 DIMENSION 合成回归。

`src/lib.rs` 单元测试另覆盖 locale 规范化、错误文档 schema、原子写失败清理。

运行：

```bash
cargo test -p cad-cli-tools --locked
```

## 7. 已验证 / 仍未闭环

**本轮已在 `cad-cli-tools` 内闭环：**

- 机器输出稳定、locale 无关；`--locale` 仅人类消息。
- 结构化错误 + 退出码契约；成功 stdout 纯 JSON。
- `--out` 原子写（临时文件 + rename），失败无半截文件。
- `export-notes` 原子导出并绑定导出 revision 后标记保存。
- `proxy-report` 汇总全部代理/未知诊断，不静默吞故障。
- `render` 原生无头路径：真实适配器出帧 + 回读统计 + 可选原子 PNG；无适配器
  `gpu_failure`、无几何 `invalid_input`，绝不空成功。
- `plot` 原生路径：光栅 PNG，以及纯 CPU 矢量 SVG/PDF（`--plot-format`）；未整形文字与
  不可表达图元逐项诊断，不空成功。

**仍开放（不在本轮范围，本文不声称完成）：**

- `plot` 的矢量路径仍是路径级：不按打印样式（CTB/STB）改色、无 HPGL、不嵌入字体；
  真实图纸的矢量视觉验收未运行。

- `render` 的原生路径已接线并可在软件适配器（如 lavapipe）上出帧；仍需真人核对
  的真实样张黄金图与跨 GPU/后端（Vulkan/GL、不同驱动）矩阵 → OPEN。
  wasm 仍为 `unsupported`（浏览器由宿主 canvas 提供设备）。
- 能力表 importer 侧（`read` 恒 `Verified`、render/pick 复制 semantic、
  `model_render` 未按表示/scene/GPU/拾取分别判定）位于
  `crates/cad-import-acadrust` → OPEN，CLI 只如实转发。
- 结构化批注 CRUD 入口、跨后端（Vulkan/GL、不同驱动）与不同 GPU 的对照验收：仍
  **未运行、无证据**；`fixtures/manifest` 现有 QCAD flange 样本可作回归输入，但不等于
  兼容性或黄金图验收。
