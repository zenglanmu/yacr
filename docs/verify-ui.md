# verify-ui：无头环境中的 UI 调试循环

2026-10-03 新增。对应测试计划的**第二层**：在无窗口环境里实际运行应用、自动操作控件、
截图、收集 panic 与超时，让界面与功能迭代不依赖真机。入口
`scripts/verify-ui.sh`，汇总脚本 `scripts/verify-ui-summary.py`。

第一层是纯核心/契约测试（`cargo test`）；第二层是本文件；第三层（真机、真实 GPU、
窗口系统、设备输入）**不在本入口范围内**，任何本入口的通过都不能推广到第三层。

## 关键区分

三层能力必须分开记录，不能互相冒充：

| 能力 | 本环境提供者 | 本入口结论 |
|---|---|---|
| 窗口运行环境 | Slint 官方 offscreen `Platform`/`WindowAdapter` | 已运行 |
| GPU/绘制能力 | Mesa lavapipe（软件 Vulkan，`VK_ICD_FILENAMES=lvp_icd.json`） | 已运行，`device_type=cpu` |
| 控件自动化与截图 | Slint 回调 + `WindowEvent` + `window().take_snapshot()` | 已运行 |
| 真实 GPU / 窗口系统 / 真机 | 无 | **not-run** |

**为什么这里不用 Xvfb**：本仓库既定约束是不安装桌面/X11/Wayland、不改系统配置。
Slint 的 offscreen 平台已经同时提供“窗口运行环境 + 真实 Slint 组件 + 共享
Device/Queue 的 CAD 纹理合成”，并且是 `--headless` 主验收同一条渲染路径；Xvfb 只
增加窗口系统，并不会让 lavapipe 变成真实 GPU。环境探测会如实记录 `xvfbAvailable`，
但自动化路径不依赖它。若日后提供带真实 GPU 的 X11/Wayland，可在本入口之外再加一层
真机/窗口验收，而不是把软件 Vulkan 结论改写成窗口结论。

渲染路径是**正式的** Slint → 共享 Device/Queue → CAD 纹理 → Slint `Image`，由
`crates/cad-ui-slint/src/offscreen.rs` 的 `FemtoVGWGPURenderer` 在 wgpu 上合成；
没有用静态图片替代 CAD 渲染，因此 bridge/纹理生命周期类问题仍能被测到。

## 运行

```bash
bash scripts/verify-ui.sh
# 指定外部图纸时，额外运行真实图纸打开 smoke（这一层失败会判失败）：
YACR_TEST_DWG=/abs/sample.dxf bash scripts/verify-ui.sh
# 可选环境变量：
#   YACR_VERIFY_OUTPUT   证据目录（必须不存在；默认 /tmp/opencode/yacr-verify-ui-<时间>-<pid>）
#   YACR_VERIFY_TIMEOUT  单层墙钟预算秒数（默认 900）
#   YACR_VERIFY_PROFILE  debug|release（默认 debug，迭代用）
#   VK_ICD_FILENAMES     覆盖软件 Vulkan ICD
```

脚本层（`run_layer`）对每一层做 `timeout`：退出码 124/137 记为 `timeout`（挂起），
非零记为 `failed`，并要求关键层通过。后台线程 panic、回调 panic 通过日志中的
`panicked at` 扫描与测试失败双重判定，不只看进程是否存活。

## 层与覆盖

1. `ui-unit`：`cargo test -p cad-ui-slint --lib`。目录/状态模型纯契约，无 GPU。
2. `ui-offscreen`：`concept_offscreen`。真实 Slint/lavapipe 组件，桌面/紧凑/移动/窄屏
   布局矩阵、canvas-only 预设、移动工具/图层面板、点击命中、`shell_geometry` 断言与截图。
3. `scenario`：`apps/app-linux/tests/verify_ui.rs`。真实 `LinuxApp` 固定场景：
   测量（开放型可确认→取消不写库；距离两点自动完成→存为批注）、撤销/重做、
   LINE 绘制提交、文字批注需先有文字、图层临时覆盖与恢复、布局与所有标准视图、
   **2D→3D→2D 无损往返（合成帧逐像素相等）**、canvas-only 预设、中英切换、
   侧车导出/回导、桌面/紧凑/移动/窄屏尺寸矩阵。
4. `host-contracts`：`host_contracts`。命令/事务/文件选择器注入契约。
5. `app-build` + `app-smoke`：构建并运行真实 `yacr-linux --headless`，产出
   `report.json` 与 PNG（导航像素变化断言）。
6. `app-dwg`（设置 `YACR_TEST_DWG` 时）：真实图纸打开 smoke。

## 证据包

```
OUTPUT/
  verify-ui.json        每层状态、缺失/失败层、panic、截图计数、真实设备=not-run
  environment.json      Slint offscreen、lavapipe、显示服务器探测、工具链版本、app report 摘要
  panic.txt             仅当检测到 panic 时生成
  .layers.tsv           机器可读的层状态（供汇总脚本）
  logs/<layer>.log      每层完整输出（含 `timeout` 判定依据）
  screenshots/          合并后的截图（slint-* 与 scenario-* 前缀）
  slint/*.png           concept_offscreen 原始截图
  scenario/*.png        verify_ui 场景截图
  app/{report.json,initial.png,navigation.png}
  app-dwg/...           真实图纸证据（如启用）
```

`verify-ui.json.status` 只有在所有要求层通过、无 panic、截图均为合法 PNG 时才为
`passed`；`environment.json.realDevice` 恒为 `not-run`。

## 与主门禁的关系

`verify-ui.sh` 是**本地无头循环**，不是 CI 必需层，也不替代：
`bash scripts/check-linux-app.sh`（release Linux App 主验收）、
`docs/testing-dwg.md` 的原生/浏览器真实图纸回归、以及 `docs/ci.md` 的分层 workflow。
它覆盖的是“改代码→启动→操作→截图→检查→再改”的快速迭代，约 1 分钟（缓存后）。

## 已知限制 / 未完成

- 软件 Vulkan（lavapipe）不等同真实 GPU 驱动、性能与设备丢失恢复。
- 合成 demo 图纸不等于真实 DWG；字体/代理/纸空间保真仍需 `docs/testing-dwg.md`。
- 真实鼠标/键盘/触屏事件由 Slint `WindowEvent` 合成，不覆盖 X11/Wayland 输入栈与真机触摸。
- 目前没有独立的真实窗口（Xvfb/Wayland）层；若提供带 GPU 的显示服务器，应新增一层而不是
  修改本入口的结论。
- 菜单遍历以已接线的回调/模型为准；未接线的禁用项不会发送命令。
