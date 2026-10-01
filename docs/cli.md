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
| `--locale <tag>` | **仅**人类 stderr 文字的语言，`zh-CN`（默认）或 `en`；机器输出不变 |
| `--allow-fingerprint-mismatch` | 图纸指纹不匹配时仍导入批注 |
| `--font <name=path>` | 注册 TTF/OTF/WOFF 字体用于文字成型（可重复；省略 `name=` 时取文件名） |
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

固定视口 GPU 帧需要 CLI 不拥有的设备，因此**显式未运行**：

- 始终以 `unsupported` 失败并退出 `1`；
- stderr 错误文档的 `message` 说明需要 GPU 环境；
- 绝不返回空成功（审计条目 F11/§11）。

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
- `render_is_explicitly_unsupported_and_exits_non_zero`：`unsupported` + 退出 `1`。
- `unknown_operation_exits_two_with_usage_code`：退出 `2`，code `usage`。
- `missing_required_points_is_a_non_zero_failure_not_empty_success`：
  输入不足是非零失败，不是空成功。
- `out_writes_file_and_keeps_stdout_empty`：`--out` 写文件、stdout 为空。
- `failed_run_leaves_no_partial_out_file`：`--out` 写失败退出 `1`、
  返回 `output_write_failed`，目录无 `.tmp` 残留。
- `failed_operation_never_creates_the_out_file`：操作失败不创建 `--out`。
- `locale_does_not_change_machine_keys`：`zh-CN` 与 `en` 的成功 JSON 完全相等，
  错误文档机器键也完全相等。

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

**仍开放（不在本轮范围，本文不声称完成）：**

- `render` 仍无条件 `unsupported`：需要 GPU runner / 平台宿主接线，
  位于 `cad-cli-tools` 之外的执行环境 → OPEN。
- 能力表 importer 侧（`read` 恒 `Verified`、render/pick 复制 semantic、
  `model_render` 未按表示/scene/GPU/拾取分别判定）位于
  `crates/cad-import-acadrust` → OPEN，CLI 只如实转发。
- 结构化批注 CRUD 入口、真实授权 DWG/字体/黄金图与跨后端验收：
  `fixtures/manifest` 为空，**未运行、无证据**。
