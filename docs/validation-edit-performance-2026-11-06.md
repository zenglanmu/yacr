# 大图编辑提交性能排查（2026-11-06）

范围：保留 React DOM + wgpu、共享 React 树与各载体唯一 `cad-db` 所有权。
用户报告 `fixtures/complex-test.dwg` 放大细节画线后 Enter 提交卡死。
以下证据不能替代用户操作、视觉验收或跨平台运行验收。

## CPU 基线

```sh
cargo test -p cad-app --test commit_real_fixture --release --locked \
  -- --ignored --nocapture
```

仓库 complex fixture；无字体引擎、无 vendor 插件、无 GPU；单次本机 release
诊断，不设耗时阈值，不是通用性能结论：

| 阶段 | 时间 |
| --- | ---: |
| DWG 导入 | 1004.559 ms |
| 首次 CPU 场景生成（145776 batches） | 4711.223 ms |
| 数据库候选深复制 | 195.955 ms |
| 插入一条直线的数据库事务 | 0.987 ms |
| 提交后完整 CPU 场景生成（145777 batches） | 4854.521 ms |

此基线证明编辑后的全图表示重建有可观成本，但不证明它是用户卡死的唯一原因。
测量日志：本机 `/tmp/opencode/commit-fixture-baseline.log`（非仓库产物）。

## CPU 增量对比

```sh
cargo test -p cad-app --test commit_real_fixture incremental_line_commit_matches_full_fixture \
  --release --locked -- --ignored --nocapture
```

同一 complex fixture，无字体/vendor/GPU；真实 `HostController::execute_traced_with_edit`
产生不透明凭据，结构断言和完整 reference 比较通过：

| 阶段 | 时间 |
| --- | ---: |
| 首次 controller 完整准备 | 4901.806 ms |
| CreateLine 命令（含候选复制） | 202.079 ms |
| 增量 controller 准备 | 396.396 ms |
| 同轮独立完整 reference 重建 | 4612.818 ms |

保留 145776 个 batch，只新增 1 个；`added`、images、masks 与独立全量构建完全相等，前序
Arc 身份断言通过。约 11.6 倍仅指**本次 CPU 准备阶段**，不包含 GPU/界面/真实输入，也不是
渐近 O(delta) 结论（旧 CPU batch payload 仍复制）。日志 `/tmp/opencode/commit-fixture-append.log`。

## Linux 实机驱动

`YACR_PROBE_TOOL_EVIDENCE=1` 的原四线驱动保持不变。额外设置
`YACR_PROBE_TOOL_PERF=1` 才切换为大图复现：启动后 90 s 在视口中央发送合成
wheel 放大，95 s 记录基线，96 s 启动 LINE，98/108 s 在视口内放置两点，118 s
核对 `tool.ready` 后发送合成 Enter 键，155/157 s 读取并记录真实状态。固定等待不是就绪证明；必须
同时查看 `YACR_PROBE_CAD_LOG=1` 日志，确认初始底图已完成提交，并且新 revision
已提交和呈现。`entityCount + 1` 本身也不是 GPU 完成证明。

先构建前端再构建宿主，真实 Wayland 会话中运行（显示环境按本机配置）：

```sh
(cd apps/desktop-ui && npm run build)
cargo build --release --locked -p app-linux --bin yacr --features tauri/custom-protocol
YACR_PROBE_TOOL_EVIDENCE=1 YACR_PROBE_TOOL_PERF=1 YACR_PROBE_CAD_LOG=1 \
  timeout --signal=TERM --kill-after=5s 180s \
  target/release/yacr --open fixtures/complex-test.dwg > evidence.log 2>&1
```

外部 timeout 结束运行通常返回 124，不得写成程序正常退出。
本驱动使用合成 DOM 输入，**不是真实鼠标/键盘**；缺失元素或工具捕获失败均不得当作成功。
本机 GPU：Quadro P620 / Vulkan；Wayland / Mutter。未设置软件渲染覆盖。

初轮旧驱动在加载期间提前提交，实体数 44591→44592，但不覆盖“加载完成后放大再提交”；
与 postload-pan 同时运行的该轮退出码为 3，不能作为通过证据。

首次放大驱动的 98/101/104 s 紧密时序也未提交成功：输入处理分别耗时 2.778 s、
1.840 s、1.847 s、1.849 s，Enter 越过尚未完成的第二个 point-up，宿主显式拒绝
`draw kind 'line' needs at least 2 point(s), has 1`；最终 delta=0、tool=draw.line。
因此修正了点间等待并增加 `tool.ready` 检查。该轮不是提交通过证据，但建立了秒级输入
阻塞的证据。该轮初始化全量 GPU commit 2813 ms（pulled 1799 ms、pick 953 ms），
该 commit 所在帧总耗时 4402 ms。日志为本机
`/tmp/opencode/complex-zoom-enter-baseline.log`。外部 timeout 返回 124。

## 状态与边界

增量修复和最终实机结果：待本轮实现、契约验证与复测后补充。
Windows/macOS/Android/Web 运行、真实鼠标、截图/像素、完整 workspace：**NOT RUN / NOT VERIFIED**。
