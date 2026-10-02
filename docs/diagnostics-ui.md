# 诊断抽屉与双语 chrome（N01/U08/N02）

本轮把 **UI chrome 的双语化** 落到真实目录，并把诊断抽屉接到结构化诊断模型；
同时新增 `i18n-contracts` CI job。渲染未在本机运行（见“未完成”）。

## 已实现

### N01 — 双语 catalog 覆盖真实 chrome
- `crates/cad-ui-slint/i18n/zh-CN.json` 与 `en.json`：各 **136** 个 key，键与占位符
  **完全一致**；默认 `zh-CN`，`en` 可接受 `en-US` 等并归一化。
- 按钮/标签/空态/后端名/工具提示等**不再硬编码**，统一经 `MessageSource` 取文案；
  测量/批注类型标签、后端标签、诊断文案均由 catalog 生成，避免第二份易失配的真相。
- 缺失 key 渲染为显式标记（非空串），并有可诊断的回退策略。

### U08 — 状态栏与诊断抽屉
- `status.rs`：
  - `ReasonText` / `severity_text` / `completeness_text` / `parameter_text`：
    把 `cad-diagnostics` 的稳定 code + 参数在表现层本地化（core 保持 locale 无关）；
  - `DiagnosticsPanelState::from_model`：**保留每个对象与每条原因**，按严重度分桶，
    `summary_text` 只给简要结论；对象级详情与 backend/恢复动作分开展示；
  - 状态栏摘要（加载/工具/单位/未保存）不嵌入诊断正文，避免压住提示。
- 抽屉为**真实数据**驱动；空模型是显式空态，不造行。
- **HTML `#host-state` 不再常驻**（U08）：它是加载/失败专用技术行，轮询判定就绪后
  `<body>` 加 `renderer-ready`，CSS 隐藏该行，使 Slint 底部状态可见；失败时重试
  按钮出现并经 `:has` 守卫重新显示该行。面向屏幕阅读器的播报改由视觉隐藏的
  `#a11y-status`（U12）承担，`#host-state` 为 `aria-live="off"`，两者不重复播报。
  新增目录键 `a11y.canvas_label`、`a11y.status_failed`。

### N02 — `i18n-contracts` job
- `.github/workflows/core.yml` 新增 `i18n-contracts`：运行 `scripts/check-i18n.py`
  （键/占位符一致、缺 key、硬编码 chrome 字面量检测，含受控白名单）。
- `scripts/check-workflows.py` 的必需 job 列表同步更新为
  `core-quality / wasm-check / i18n-contracts / android-check`。

## 未完成（显式）

- **Slint 渲染未运行**：本机缺 fontconfig，`cad-ui-slint` 不能原生构建；其 Rust 单测
  不在本机执行，仅由 wasm32 `cargo check` 保证可编译。无视觉/真机/浏览器验收。
- **HTML/JS 启动文案同步**：`apps/app-web` 的 `html lang`/title/noscript 与 JS 提示
  尚未与当前 locale 同步。
- **语言偏好持久化**：设置项与重启恢复未接线（宿主层）。
- **结构化诊断的完整 UI**：抽屉已接模型，但对象级逐条展开的交互（搜索/跳转对象）
  仍待补。
- **其余 N02 job**：web-build / web-smoke / android-apk / shader-validation 仍未实现
  （见 `docs/ci.md`）。

## 验证

```bash
python3 scripts/check-i18n.py       # 136 keys, 2 catalogs, consistent
python3 scripts/check-workflows.py  # required jobs present, no continue-on-error
node --test scripts/test-web-a11y.mjs  # U12/U08 DOM + 播报漏斗契约（无需 wasm）
cargo check --workspace --lib --target wasm32-unknown-unknown --locked
```
