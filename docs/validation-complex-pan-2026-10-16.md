# 复杂图纸平移/等待诊断（2026-10-16，仅诊断，未修复）

状态：**只加了只读诊断（instrumentation）**，**没有**修改任何渲染策略/轮询/裁剪/UI。**尚未在运行时复现验证**；
不是性能/视觉/vendor 验收，也不是“已修复”。

## 问题（已观测，来自基线）

最终 release 二进制（当时 `7618d0d2…`）在 `fixtures/complex-test.dwg`（44,591 实体）上：加载完成后所有 CAD 帧集中
在一个 ~2.6–2.9 s 的突发里（受 16 ms 上限，约 61 fps），随后 **presented 帧计数冻结**（163/178），而 ticks 继续
升到 ~1681；此后每个 tick 都是 `cad-frame waiting`。在 ~8 s 的合成相机平移/缩放会让相机改变但**不产生任何 presented
帧**（画布不更新）。X11 同样冻结 → 不是 Wayland 特有。原生提交阶段本身很便宜（CPU 侧 `total` p50 ≈ 0.27 ms），
但 **GPU 执行时间未采集**，因此不宣称“GPU 瓶颈”。原始报告：
`.slim/deepwork/native-wayland/pan-performance-baseline.md`。

关键观测：`frame_cad` 在真实视图变化时持续返回 **`Ok(None)`**，同时 `view_current` 前进。因此需要区分
**“CPU 场景准备没出结果”** 与 **“上传被重置/饿死”** 这两类等待——这正是本诊断要回答的问题。

## 诊断（只读，env-gated）

- 引擎新增只读快照 `cad_engine::render::RenderSessionHandle::diagnostics() -> RenderDiagnostics`：
  - 门控原因 `gate`：`Detached` / `Failed` / `Preparing` / `UploadingBase` / `UploadingOverlay` /
    `BaseIdentityMismatch` / `Ready`（不改变、不放宽既有严格门控）。
  - 修订号：`input_revision`、`requested_token`（最近请求 = `content_revision`）、`prepared_token`（最近发布）、
    `prepared_identity`、`view_revision`、`scene_publication_revision`、`base_publication_revision`。
  - 已发布/已提交：`ready_revision`、`ready_base_revision`、`applied_revision`、`applied_base`、
    `resident_base_batches`。
  - 上传：`uploading` + `upload{scene_revision, group, offset, base_changed}`，以及计数
    `uploads_started` / `upload_revision_resets` / `ready_none_clears` / `last_upload_reset`。
  - 其它：`overlays_suppressed`、`overlay_suffix_commits`、`frames_rendered`（引擎是否真的渲染过）、
    `preparation_busy`、`result_status`（真实失败/诊断，绝不把失败改写成成功）。
- 宿主（`apps/app-linux/src/host.rs`）在**既有 `YACR_PROBE_CAD_LOG` 门控**下打印一行
  `[probe] pan-wait …`，**仅在状态变化或约 1 s 心跳**时输出（不逐帧刷屏）。
- 快照是**纯投影**：只读借用状态，**不**推进 upload/revision/view，也**不**从遥测触发引擎工作。

如何读（典型）：
- `gate=Preparing` 且 `requested_token != prepared_token` / `prep_busy=true` → CPU 准备尚未产出当前场景。
- `gate=UploadingBase|UploadingOverlay` 且 `upload=<rev>/<group>/<offset>`：观察 `upload` 的 `offset` 是否随时间
  增长——不增长或 `upload_rev_resets`/`ready_none_clears` 增长即为**重置/饿死**信号。
- `frames_rendered` 不增长 = 引擎确实没有渲染过（与 presented 冻结一致）。
- `gate=BaseIdentityMismatch`（无 pending 上传但基座不驻留）是另一类需要单独判断的原因。

## 读数口径更正（避免把等待时间/批次数误解成 CPU/GPU/绘制数）

- 早期报告的 **“21 s 处于 `Preparing`”** 只是**门控状态**的墙钟时长，**不是**准备函数的精确 CPU 时间
  （无 `cpu_ms`/GPU 时间戳归因）。
- **“10.4 s stage+commit”** 是分片上传的**墙钟**时长，**不是** GPU 传输时间（未采集 GPU 时间戳）。
- **`resident_batches=166345`** 是**基座批次前缀长度**（resident base batch prefix），**不是** draw call 数；
  draw call 数应看 `actual-render` 行的 `draw_calls`（真实一次渲染器的调用统计）。
- **首次复杂图加载**是导入/准备/基座上传导致的**重建**，**不是**“pan 触发底图重建”；当时的 8 s 输入/18 s resize
  发生在其**准备期间**，不能作为因果证据。
- 原始 B 报告（`pan-performance-baseline.md`、`pan-waiting-runtime.md`）**保持原样**（raw），以上更正只在本文给出。

## 实际渲染观察（`actual-render`，仅真实渲染器调用）

引擎现在把每次**真实** `Renderer::render`/`render_3d` 调用的 `FrameStats` 捕获为
`RenderSessionHandle::take_render_stats_observation()`（**每次真实渲染最多交付一次**，retained/waiting
帧**不会重放**旧统计）。字段：`render_seq`、`device_epoch`、`view_rev`、`applied_rev/base`、
`cpu_ms`、`gpu_ms`（**无则 `None`，不伪造 GPU 时间戳**）、`plan_ms/uniform_ms/encode_ms/submit_ms/wait_ms`、
`draw_calls`、`verts`、`tris`、`instanced`、`mesh`、`tier`、`over_budget`、`prog=cursor/total`、`complete`、
上传计数。宿主在既有 `YACR_PROBE_CAD_LOG` 下**仅当出现新的真实渲染**时打印一行 `[probe] actual-render …`。

## 加载后持续平移驱动（`YACR_PROBE_POSTLOAD_PAN`，默认 OFF）

**⚠️ 先前一次该驱动的运行结果作废（mis-gated 假成功）**：驱动在 0.89 s 被**未加载的 `yacr-demo` 占位**满足，
手势跑在 demo 上（`draw_calls=2`），主机只收到 `pointer_down`（0 move / 0 up），随后 **3.33 s 的 WM resize**
造成的 view 变化被误当成“最终帧”，应用在 **3.37 s 自然退出 0**。因此**不存在**有效的复杂图加载后平移数据；
该次任何 FPS/指标都**无效**（不得引用）。

修复后的准入与完成条件（有界，独立 env-gate；**不**触发旧的 8 s 输入/18 s resize/minimize）：

- **准入必须命中“请求的图纸”**（非 demo）：会话状态为**非 loading、无 error、`import_count >= 1`**，且
  `document_label` 的**文件名等于 `--open`/`YACR_TEST_DWG` 请求的文件名**（用真实打开目标身份，不用硬编码
  entityCount；`yacr-demo` 标签天然不匹配）；同时该图纸**已 `current_drawing_presented()`** 且**最新真实渲染**
  与**当前 view/device 匹配**。无请求图纸时**显式失败**。
- **手势完成必须是结构化 ACK**（不是“view 变了”）：JS 以唯一 `run_id` 记录 `downs/moves/ups/elapsed_ms/failed`，
  宿主每 250 ms 用 `eval` 响应轮询解析（tracked marker，无任意用户 JS）；要求 `done`、无 `failed`、
  `ups>=1`、`moves>=60`、`elapsed_ms>=8000`，**且宿主真正接收到**（admitted）`pointer_move>=1`、`pointer_up>=1`。
  JS 侧为 rAF 主循环 + 定时**防停滞兜底** + 硬 `setTimeout` 结束（即使异常也记录 `failed` 并保证 `pointerup`）。
- **最终帧必须匹配本次平移**：`current_drawing_presented()` 且出现**更新的真实渲染**（`render_seq` 大于手势
  结束时的值）且**view 与手势前不同**；**WM resize 在手势/最终等待期间被显式标记为失败**
  （`concurrent_resize`），并发 resize 使报告**显式失败**（`exit 3`），绝不假通过。
- **边界**：总体 90 s + 手势后 10 s；成功才 `natural_exit=0`，超时/无 move/无 up/相机未变/并发 resize 一律
  显式 FAILED（`exit 3`）。默认 OFF；未设置时**不注入任何输入**（不回归手工使用）。

## 二次运行结果作废（报告解码缺陷 + 退出码未传播）

**⚠️ 第二次运行（二进制 `004c6ffa…`）也是无效的**：目标图纸身份门禁已正确（44,591 实体、
`import_count=1`、相机确实被平移改变），但驱动在 90 s 上限处**显式失败**，原因有二：

1. **报告解码缺陷**：`eval_with_callback` 回调传入的是 **JSON 编码字符串**（形如
   `"run=1 … failed="`，带引号/转义），而 `parse_pan_report` 直接按原始文本解析，首个 token 变成
   `"run=1` → 永不匹配 → `report=None` → `gesture_complete` 永远 false。日志中 JS 侧
   `run=1 done=true moves=80 ups=1 elapsed_ms=8049` 与主机 admitted `moves=126 ups=4` 其实都满足。
2. **退出码未传播到 OS**：`AppHandle::exit(3)` 之后外部仍观测到 OS 退出码 **0**，失败只体现在日志里。

因此该次“**完整 54 s 窗口**”（35.71→90.00 s）**不是**有效的 8 s 平移 FPS 窗口：它是**手势失败 + 等待**
的墙钟时长，**不得**当作平移性能数字。指标（`wait_ms` 等）只能作为该失败窗口内的原始观测，**不构成**
性能结论；**不宣称 FPS**。

### 本次修复（仅测试驱动 + 退出码传播；无渲染优化）

- `parse_pan_report` 先用已有 `serde_json::from_str::<String>` **解码 JSON 字符串**（无 `serde_json` 新依赖），
  再解析结构化 marker；**同时接受原始 marker**；**拒绝格式错误/wrong run_id**（`run=` 非数字 → `None`；
  报告 `run_id` 必须等于手势响应里捕获的唯一 `run` id）。
- `entry.rs` 在事件循环返回后**传播**驱动记录的状态：`resolve_process_exit(requested)`（纯函数，测试覆盖）
  → 失败 `exit 3` 到达 OS；**默认/手工运行与成功路径仍自然 `exit 0`**（默认 OFF，不回归手工使用）。
- 修复只涉及测试驱动/解析/退出码；**未**改变渲染策略/轮询/裁剪/UI/engine；**不是**性能行为改动。

## 复现（下一次有界复现，由 B 执行）

```bash
# 原生（非 XWayland）：pan-wait 诊断 + 真实渲染行 + 有界加载后平移驱动
# 二进制 sha256 978b8aef…（本次 report/exit 修复构建）
env -u DISPLAY -u XAUTHORITY GDK_BACKEND=wayland WAYLAND_DISPLAY=wayland-0 \
    XDG_RUNTIME_DIR=/run/user/1000 YACR_PROBE_NO_MAXIMIZE=1 \
    YACR_PROBE_CAD_LOG=1 YACR_PROBE_POSTLOAD_PAN=1 \
    ./target/release/yacr --open fixtures/complex-test.dwg --fonts-dir fonts
```

## 关键阶段计时（仅诊断，待运行时定位）

为定位 B 复现里“主机单帧 ~3.79 s，而引擎 `FrameStats` 只有 ~10 ms + ~20 ms 等待”的缺口，加入**只读**
阶段计时（**不改**任何渲染/提交/上传/轮询/缓存语义）：

- **主机单帧括号**（`[probe] cad-stages`，`YACR_PROBE_CAD_LOG` 门控）：`apply_us`（`apply_ready_updates`）、
  `render_us`（引擎 `render`，含等待/门控，**不是 GPU 时间**）、`present_us`（`present_texture`）、
  `frame_total_us`。仅在**有上传工作**或单帧 ≥ 50 ms 时输出**一行聚合**（绝不逐批）。
- **上传阶段**（同一行 `upload=`）：`scene_rev/base_changed/committed/group/batches/prepare_us/append_us/commit_us/total_us`。
- **`commit_upload` 子阶段**（`commit[...]`）：`rebase/append/gc/pulled/pick/lineweight/total` —— 覆盖
  “suffix commit 全量重建 resident GC/pulled buffers/pick table”的候选嫌疑；**未确认前不假设**。
- 观测均为 **take-once**（序号守卫），retained/waiting 帧**不会重放**；全部为 `Instant` 墙钟微秒，
  **不冒充 GPU 时间戳**。

二进制 sha **`aee434f3…`**（本次计时构建）。关键阶段**运行复现仍待 B**；在得到该行数据前**不作**任何
阶段归因结论。用户排队的进度条/默认最大化仍未实现。

## Pulled 前缀复用快速路径（仅加速档；待 Oracle/硬件复核）

针对 B 复现里 pan/overlay 提交约 4 s（`resident_batches=166346`）的根因：`rebuild_pulled`（~2 s）在每次
suffix 提交时**重编码并重传整个基座几何**，尽管基座从未变化。加入**保留基座几何的前缀复用**：

- **渲染器私有 checkpoint**（`PulledPrefixCheckpoint`：device generation / cache epoch / 保留批次数 /
  资源前缀计数 / 保留 segment 计数 / 基座 range）＋ `PulledLineSegment.geometry_buffer`（mesh 已有
  vertex/index buffer）。
- **准入在变异前由渲染器自证**：保留前缀长度匹配、device generation/cache epoch 匹配、加速档、保留资源与
  segment 计数仍在、**保留批次引用的正是保留资源且为恒等顺序**（与 GC 无操作同条件）；**不**采信调用方的
  `base_changed`。
- **边界感知分段**：segment 永不跨越基座边界，故基座 range/缓冲在 suffix 提交间不变。
- **准入后**：保留的基座 segment 只**重建 bind group**（针对新的整场景 chunk table），仅编码/上传 suffix
  几何；保留前缀的 run/chunk 索引不变。`rebuild_pick_table`/flags/draw plan/lineweight/透明/顺序/ID 渲染
  **一律照旧**。
- **验证与全量回退（绝不跳过）**：复用前必须校验收敛后的基座 range 与 checkpoint 记录一致、range 数与保留
  segment 数一致、每个保留 segment 的实例/顶点数与重算一致；任何不一致 → 清除复用计划并**失效 checkpoint**，
  该类**全量重建**（不可构建时仍走 universal 回退）。
- **失效时机**：attach/重建、detach/设备丢失、`clear_batches`、测试钩子 `set_pulled_line_segment_elems_override`、
  基座替换（keep_prefix=0）、以及任何回退。
- **预期**：首次可复用提交先做一次**全量预热**建立 checkpoint，之后才有效益。

**未运行**：颜色与拾取等价、微小 binding 上限/universal 回退、增删 overlay 不重传基座等 **GPU 合同**
需硬件，属 Oracle 复核后的父级步骤；**无软件冒充真实 GPU**。源门禁 attempt1 为 REJECT，已按下述修复：

- **（1 阻断）全量预热**：`Full{establish}` 现在用存活前缀的**所有权证明**推导保留边界并强制分段边界，
  且**每个类成功构建时都记录自己新建的基座 range**（不再只在复用分支记录）；仅在“所有类所有权成立、
  至少一个 pulled 类有保留前缀、每个有资源的类都产出了基座 segment”时才发布 checkpoint（回退类直接拒绝）。
- **（2）** `checkpoint_scalar_admits` **只对“有资源的类”要求 segment**，且至少一个 pulled 类；line-only/
  mesh-only 均可准入（矩阵合同）；arc-only 拒绝；`reuse_failed` 同步放宽。
- **（3）** `set_pulled_binding_limit_override` 现在**同样失效 epoch**（合同覆盖两个钩子）。
- **（4）** 失败的复用**不再计入** `reuses`（纯 `prefix_outcome`：仅 `is_reuse && !reuse_failed` 计数）。
- **（5）** 准入与发布前均**显式拒绝被替换 suffix 引用保留资源**（`IncomingResources::disjoint_from`
  使用变更前 prepared 的 rebase 后引用；`owned_prefix_count` 复核恒等前缀 + suffix 不相交）。

**残留（记录，不优化）**：通用资源 GC 仍全量扫描计数（不做 O(delta) 改写，按 Oracle 延后）；
`rebuild_pick_table()`（~1.4 s）**保持不变**。**源门禁 attempt2 亦为 REJECT（前缀复用未核验“实际完成复用”）**，
已按下方修复：

- **最终完成性证明 `reuse_completion`**：在 `prefix_outcome` 之前，对**每个 checkpoint 拥有的类**核验“确实完成复用”：
  pulled 类仍存在、记录的基座 range 与 checkpoint 记录**完全一致**、所需保留 segment 全部复用。若某类回退到
  universal（suffix 过大或 chunk table 超 binding 上限——这也会丢弃保留的基座几何），则 `reuse_failed=true`、
  失效 checkpoint、**不计入 `reuses`**；**另一有效类保持已构建，不触发第二次整场景重建**。矩阵纯合同已加。
- **计时口径更正**：`commit_started` 前移到 incoming 引用收集与准入之前，故 `CommitTiming::total_us >= 各子阶段之和`
  （还覆盖前缀扫描/分配与 append）；它**位于 stale 校验守卫之后**，故 stale 校验既不计入任何子阶段也不计入 `total_us`
  （可忽略且不做宣称）；**仅文档说明，不新增公共字段**。
- **注释更正**：incoming 引用指**新的** suffix（非被移除者）。
- **残留 O(N)（不宣称 O(delta)）**：前缀所有权扫描、通用资源 GC、Pick、lineweight 缓存均为全量扫描。

**源门禁 attempt3：ACCEPT-WITH-LIMITATIONS（无源码阻断）**——剩余限制是真实 GPU 合同（颜色与拾取等价、
预热→suffix 过大用例、binding 上限/universal 回退、增删 overlay 不重传基座）由 B 独立硬件测试文件运行；
**这不是性能/硬件验收**。**在强制实际硬件验证完成前不得发布 release。** 用户排队的进度条/默认最大化仍在性能
测试收口之后。

## 明确 NOT RUN / 范围

- **未在运行时复现验证修复**（复杂图纸 pan 窗口仍待 B 运行）；**未实现任何渲染优化**；
  未改变渲染策略/轮询/裁剪/UI/依赖/lock。**不宣称任何 FPS**。
- 未采集 GPU 时间戳（`gpu_ms=None` 诚实为无）；CPU/GPU 不可分离；合成 DOM 输入 ≠ OS 真实指针；X11 结果**不是**原生证据。
- 未做全 workspace 测试全扫、无 npm、无系统安装。**不是**性能/视觉/vendor/精度验收，也**不是**最终验收。
- 已知未验证：分数 DPR、设备丢失、vendor 广度、裁剪/剔除策略（`~7627 draws with no culling` 仍未处理）。
- 用户后续排队项（**未实现**）：文件加载进度条、桌面默认**最大化**（非全屏）与 AGENTS 策略写入。

## 隐藏实体不可拾取（有界语义修复；仅源码 + 编译/纯单测验证，GPU 合同待跑）

**结论**：id pass 现在按 `FLAG_HIDDEN` 抑制整条“整实体”绘制，与颜色 pass 的片元 `discard` 语义一一对应，
故隐藏实体**不可拾取**，拾取会“穿透”到其下可见几何（无下层则返回空）。改动范围仅
`crates/cad-render-wgpu/src/pick.rs`（`render_id_pass` + 一个私有 helper + 纯矩阵单测）与新增文件
`crates/cad-render-wgpu/tests/pick_hidden_flags.rs`；**不改**颜色输出、`PickTable`/`rebuild_pick_table`/
`source_index`/key 分配/flag 更新路径/checkpoint/管线/WGSL，无重构、无 release/性能运行、无依赖/lock 变更。

- 语义对齐颜色 pass（逐 shader 核对，非假设）：8 个颜色 shader 均为
  `if (flags_ctl.enabled != 0u && (flag_bits(key) & FLAG_HIDDEN) != 0u) { discard; }`，故 helper 为
  `self.flags_enabled && (self.pick_key_flags(key) & cad_scene::FLAG_HIDDEN) != 0`；id shader 不采样 flags 纹理，
  因此抑制放在**计划层**：`render_id_pass` 的逐 batch 与逐 image 循环在推入任何 run/slot **之前** `continue`。
- 按实际代码核对：slot 是**逐 run** 分配（非逐 batch）；跳过整批即不为其分配 slot，其余 slot 连续，
  `ensure_slots`/`flush(total_slots)` 无需保留未用槽位。id 目标每趟 `Clear`（颜色+深度），因此隐藏后“露出下层”而非残留旧 key。
- **纯矩阵单测**（`pick::hidden_pick_tests`，无 GPU）：enabled+HIDDEN ⇒ 抑制；enabled+HIDDEN｜SELECTED ⇒ 抑制（按位包含）；
  enabled+SELECTED/HOVERED/0 ⇒ 不抑制；toggle off + HIDDEN ⇒ 不抑制（与 shader 短路一致）；key 0 与未记录 key ⇒ 不抑制。**2/2 通过**。
- **GPU 合同（已在真实 P620 运行，8/8 通过、0 skip）**：`tests/pick_hidden_flags.rs`（P620 断言 + 显式 skip≠pass）覆盖：隐藏⇒`pick_at` 为空且颜色丢弃、
  揭开恢复；隐藏上层⇒解析下层且颜色与该实体“不存在”逐字节一致；toggle off ⇒ 仍可拾取；隐藏 image 不可拾取；flag 更新无需 commit
  即生效且不重传几何（`instance_stream_uploads`/`pick_table_len` 不变）；`SELECTED`/`HOVERED` 仍可拾取（`HIDDEN` 优先）；
  子元素 overlay 在实体隐藏时**仍可拾取**（实体级 flag 只匹配整实体 source）；活跃 pulled 前缀复用下隐藏 suffix/base 实体
  不改变 `reuses`/`rebuilds`，且后续 suffix 提交仍复用。
- 交互 **CPU** 点击路径（`cad-host/src/input.rs:229` 的 `cad_app::picking::pick_at_screen`）：**未改**。唯一 hidden 来源是渲染场景
  瞬态输入 `OverlayInputs.hidden`（`cad-app/src/render_scene/overlay.rs:163`），而宿主在 `cad-host/src/state.rs:55` 将其硬编码为空
  （注释明确“宿主尚未产生逐实体 hide”）；session 仅有图层可见性/覆盖（另一概念，`cad-app/src/layers.rs`）。因此**当前无任何用户可达路径
  能隐藏实体**，点击行为不变；若未来宿主喂入 hidden 集合，CPU 路径需过滤（集成点：`select_at_mode` 中 `pick_at_screen` 之后、
  mode 匹配之前，按整实体身份匹配并保留子元素 overlay）。该改动需要新增宿主状态/交互设计切片，**超出有界修复范围，已如实上报待决**。
- 交叉车道：B 的 `tests/pulled_prefix_equivalence.rs`（本次未改动）已断言同一语义（隐藏 overlay 不再可拾取、交叉像素回退到仍可见 image）；
  该 fence 在本次修复前为红，**现已在真实 P620 转绿**（该文件 9/9 通过，含 hidden-pick 合同）。
- 实际硬件结果（串行、真实 P620 Vulkan、0 skip、无 lavapipe/软件痕迹；先 `--no-run` 编译 0 失败）：
  - `--test pick_hidden_flags` → **8 passed / 0 failed / 0 ignored / 0 filtered**（exit 0）。
  - `--test pulled_prefix_equivalence`（另一车道）→ **9 passed / 0 failed / 0 ignored / 0 filtered**（exit 0）。
  - 日志：`/tmp/opencode/yacr-pan-perf-20261016/hidden-pick-gpu/{00-compile-norun,01-pick-hidden-flags,02-pulled-prefix-equivalence}.log`。
- 其他门禁：`fmt` 0 / `check -p cad-render-wgpu --all-targets` 0 / `clippy -D warnings` 0 / 纯矩阵 **2/2 通过** / i18n 门禁 OK；
  无 release/性能运行、无 Pick 重构、无其他车道源码改动。

## 相机静止去抖：相机专用网格重准备合并为一次提交（有界实现，纯合同已通过）

- **实现**（车道：`cad-engine/src/render/mod.rs` + `contracts.rs` + 本文档）：`State` 新增 `grid_refresh_pending`、
  `last_camera_change`、`settle_timeout`（默认 **250 ms**，常量 `CAMERA_SETTLE_TIMEOUT`；**`Duration::ZERO` = 关闭去抖 =
  今日即时行为**，同时是测试 kill-switch；wasm 上永不启用，因为没有可结算的异步泵）。
- **准入条件（仅相机）**：`apply_view_snapshot` 中，仅当新旧快照**只**在相机字段上不同（`camera` 变，而 `document`/`space`/`mode`/
  `canvas_size` 全等）且视口世界边界发生变化时才延后；否则维持今日即时路径。**缩放已覆盖**：2D 缩放是相机字段
  `Projection::Orthographic { scale }`，缩放同样改变边界并延后。
- **结算**：`request_preparation`/`poll_preparation`（native）每次仅做 **一次 `Instant::now()` + 一次比较**，
  `now - last_camera_change >= settle_timeout` 时执行**恰好一次** `advance_content()`；无忙轮询。`advance_content()` 会清除
  pending（推进内容修订本身就等于“覆盖当前边界的东西已推进”），因此 pending 不可能越过提交存活，也不会谎报最终态。
- **不延后（即时并清除 pending）**：RESIZE / 画布尺寸、投影模式、document/space 切换、图层、字体/图像资源、图纸同步（几何编辑/撤销）、
  overlay 的 selection/hover/visibility/preview/snap 变更。**如实上报“不宜延后”的类别**：隐藏(visibility)（会多画最多一个超时时间）、
  实时预览（橡皮筋/吸附提示，合并会让交互反馈变钝）、选中/悬停高亮延迟、画布尺寸/模式（边界语义随之改变）、以及所有基底身份输入
  （正确性而非延迟）。`set_overlays` 仅 `viewport_bounds` 变化的路径按 Oracle 计划**保持即时**（只延后快照站点）。
- **诊断**：新增 `grid_refresh_pending()` 与字符串 `grid_refresh_diagnostic()`（`grid-refresh-pending` / `grid-refresh-idle`），
  与 `overlays_suppressed()` 并列，诚实：pending 期间不报告内容推进/发布。
- **宿主助推未接线**：引擎此路径无输入入口（宿主终结输入走 cad-ui-core/cad-app 的 `CadSession::handle_input`），
  接线并非“既有平凡管线”，需要本车道之外的宿主改动；**超时是唯一权威**。
- **纯合同（无 GPU，`render/contracts.rs` 新增 9 项，全绿）**：相机（平移+缩放）手势 ⇒ 0 次手势内推进 + 1 次结算；单次相机变化在超时内提交；
  零超时 kill-switch；RESIZE 即时且清 pending；selection/visibility 即时且清 pending；document 切换/几何编辑即时且清 pending；
  结算后发布**最终**视口边界且复用基底；结算落在**在途**准备期间只发布一次且为最终边界；诊断字符串生命周期诚实（不谎报提交）。
- **实测证据（本车道）**：`cargo fmt -p cad-engine` 0；`cargo test -p cad-engine --lib --locked` **99 passed / 0 failed / 8 ignored**
  （新增 9 项）；`cargo check -p app-linux --all-targets --locked` 0；`cargo test -p app-linux --lib --locked` **68 passed**；
  `clippy -D warnings` cad-engine + app-linux 均 0。
- **待父级硬件窗口（未运行、无结论）**：原生 GPU 下一次平移/缩放结算应使 `overlay_suffix_commits` **总计 +1**（而非每步 +1）、
  `pulled_prefix_reuses` 增长而 `pulled_prefix_rebuilds` 不增，且最终帧与**非延后参考**（`set_grid_settle_timeout(ZERO)`）逐像素一致。
  **在测到之前不作流畅度/性能结论**；无 release、无 GPU、无依赖/清单/锁改动。

## 相机静止去抖：Oracle 复审两项修正（预发布必修）

- **Finding 1（材料性、罕见，已修）**：`mod.rs` 中 `!camera_only && !bounds_changed` 分支原为无条件 `clear_camera_settle()`，
  会丢弃**当前边界**的待办刷新却从不重准备（如 fit-on-resize：画布与相机同时变化、世界范围不变，故 `camera_only=false` 且
  `bounds_changed=false`）。现在该分支在有 pending 时执行一次 `advance_content()`（同时清 pending）；真正的即时路径（身份/边界变化）不变。
  新纯合同 `bounds_preserving_view_change_covers_a_pending_camera_refresh`：缩放 ×2 且画布 ×0.5 ⇒ 恰好一次内容推进、pending 清除、
  身份不变 —— **通过**。
- **Finding 2（关键证据缺口，已验证不成立）**：新增 `cad-host/src/tests.rs::host_tick_preserves_the_deferred_camera_grid_refresh_across_a_pan_gesture`
  （纯测试、无 GPU；使用既有 `core()` 夹具与真实 tick 路径 `sync_render()`），按生产顺序驱动 `apply_view_snapshot` 与基于
  `canvas_bounds()` 的 `set_overlays`：3 次平移、1 次缩放、以及**宿主层 fit-on-resize** 每一步 `grid_refresh_pending()` 均存活；
  空闲 tick 既不arm也不churn内容修订（固定 state.rs 45–48 的无churn 不变式）。**结论：所测路径下宿主与引擎的边界计算按位一致，未发生分歧**，
  因此**未改动 cad-host/state.rs**（不存在“改哪一侧”的记录），空闲 tick 行为原样保留。这是对所测形状的证据，不是任意相机的证明；
  真正确认仍由父级的原生宿主驱动平移验收完成。
- **引擎结算假设（文档化）**：宿主**必须**泵 `request_preparation`/`poll_preparation` 才能结算延后刷新；app-linux 的 `RedrawPump`
  是无条件泵，安全；仅事件驱动的嵌入方可能推迟结算（期间不谎报状态：`grid_refresh_pending()` 保持 true）。**同步 `prepare()`
  不会结算**延后（它按当前内容修订准备），结算发生在异步泵或 kill-switch（`set_grid_settle_timeout(ZERO)`）。延后仅原生启用；零超时关闭；缩放属相机字段，同样覆盖。
- **本轮实测**：`fmt` cad-engine+cad-host 0/0；`cad-engine --lib` **100 passed**（原 99）；`cad-host --lib` **87 passed**；
  `app-linux --check --all-targets` 0；`app-linux --lib` **68 passed**；`clippy -D warnings` cad-engine/cad-host/app-linux 均 0。
  无断言弱化、无范围蔓延、无 pick/cad-render-wgpu 改动、无 GPU/原生/性能运行、无依赖/锁改动。
  **旧 release `7d4ec114…` 已过期**（早于本次修正），待父级原生验收后重新构建。

## 相机静止去抖：原生 P620 实测（含重建）

- **诊断暴露（最小、CAD_LOG 门控）**：`overlay_suffix_commits` 早已作为 `suffix_commits` 出现在变更或 ~1 Hz 的 `[probe] pan-wait` 行；
  本次仅新增 `RenderDiagnostics.grid_refresh_pending`/`grid_refresh`（`cad-engine/src/render/mod.rs`，标签由共享 `grid_refresh_label` 生成）
  与该行的 `grid_refresh_pending={} grid_refresh={}`（`apps/app-linux/src/host.rs`）。无逐帧刷屏、无行为变化。
- **重建**：`CARGO_BUILD_JOBS=2 cargo build --release --locked -p app-linux --bin yacr --features tauri/custom-protocol` → **exit 0**（2m15s）。
  二进制 sha256 **`f3021fe19315b8b9dd65ed72574fd3972fc2a9a0f9e3af9f2d451e36deddc135`**，size 27,081,832B，mtime 2026-10-17 01:44:20 +0800；
  **旧 `7d4ec114…` 已过期**。
- **原生实测（complex-test.dwg，44,591 实体；仅 `YACR_PROBE_POSTLOAD_PAN=1`+`YACR_PROBE_CAD_LOG=1`，Wayland，无 DISPLAY/XAUTHORITY）**：
  driver **Finish、自然退出 0**；真实手势 8.036s（`moves=171`、`ups=1`、`admitted_moves=170`）。
  - **手势内**：`grid_refresh_pending=true` 全程保持（`diag=grid-refresh-pending`），内容修订 `requested_token` **冻结在 8（0 次推进）**，
    `overlay_suffix_commits` **冻结在 1（0 次提交）**，`applied_rev` 冻结 `Some(3)`，`view_revision` 6→172；实际渲染 **282 次 ≈ 35.1/s**
    （间隔 p50 30.1ms、p95 42.2ms、max 59.2ms），呈现 285 帧，`frame_us` p50 28.7ms、p95 38.8ms、**max 42.7ms**。
  - **结算（t≈48.67s）**：`pending=false requested=9 suffix=2 applied_rev=Some(4)` ⇒ **恰好一次结算提交**（内容 +1、suffix +1）。
    末帧 `view_current=173 complete=current_scene`、末次 actual-render `view_rev=173 applied_rev=Some(4) draw_calls=236` ⇒ 结算后网格反映最终相机。
  - 全程 `frame_us` 峰值：**6,240,775µs @35.51s（手势前的目标导入/初始准备，非平移成本）**与 **3,868,087µs @48.67s（唯一一次结算提交）**；
    actual-render `cpu_ms` p50 8.9 / p95 15.9 / max 18.2，`wait_ms` p50 19.5 / p95 23.6 / max 737.7；`cad-frame error=0`、无 FAILED。
- **对比旧基线 `aee434f3`（同 8s 手势）**：旧为 **2 次实际渲染 / 5 帧 / 手势内最大帧 4.09s / 每步提交**；现为 **282/285 / 42.7ms / 手势内 0 次提交 /
  结算 1 次（约 3.87s，位于手势之后）**。**变量不同（如实注明）**：旧树既无 pulled 前缀快路径也无结算去抖，新树两者兼有，故此差异是合并效果，不可单独归因于去抖。
- **未测**：`pulled_prefix_reuses/rebuilds` **未被诊断暴露**（grep 证实），本运行无法证实“reuses 增长 / rebuilds 持平”，该结论属合同车道
  （`pulled_prefix_equivalence`）；**fit-on-resize 原生序列 NOT RUN**（需 resize 驱动语义，超出本车道授权），仅由引擎与宿主序列纯合同覆盖
  （`bounds_preserving_view_change_covers_a_pending_camera_refresh`、`host_tick_preserves_the_deferred_camera_grid_refresh_across_a_pan_gesture`）。
  不作流畅度/性能结论以外的外推；结算提交仍是单次阻塞提交（按设计一次）；无 install/依赖/锁改动、无断言弱化、无 cad-render-wgpu/pick 改动、运行后无残留进程。

## 结算提交 ~3.9s 的阶段剖析（原生 P620，有界）

- **诊断补充（最小、CAD_LOG 门控）**：`pulled_prefix_reuses()/pulled_prefix_rebuilds()` 此前**未暴露**（grep 证实）；现于
  `cad-engine/src/render/runtime.rs` 的 `RuntimeDiagnostics` 增加两个计数字段（无渲染器时为 0），并在既有 `[probe] pan-wait`
  行增加 `pulled_reuses={} pulled_rebuilds={}`。仅此；无行为变化。二进制重建一次：sha256
  **`69e6c674dada2043f2a4b9906b28c8dec7bba5981757762af147e9abfbe4f1af`**（旧 `f3021fe1…` 已过期）。
- **快速路径是否命中？——未命中（以证据为准）**：手势前基线提交（t≈35.17s，`seq=255 base_changed=true group=2 committed=true
  batches=166346`）后 `pulled_reuses=0 rebuilds=0`；手势中 `pending=true`、内容修订冻结 7、suffix 冻结 0；结算（t≈48.44s，
  `seq=256 base_changed=false`）后 `reuses=0 rebuilds=1`。⇒ 结算前**根本不存在 checkpoint**（基线提交为 keep_prefix=0，
  无可复用前缀），结算提交是首个保留基前缀的提交，因此**建立** checkpoint 并执行一次完整重建；单次手势运行不存在第二个 suffix
  提交，故本次无法观察到 reuse（GPU 合同车道已证第二次 suffix 复用；再次手势会使 `reuses` 增长）。
- **结算提交阶段拆分（单次，t≈48.44s）**：`frame_total=3,920,833µs = apply 3,503,304 (89.35%) + render 417,199 (10.64%) +
  present 330 (0.008%)`；apply 内几乎全部为 `commit_upload`（`commit_us=3,503,259`）：**pulled 2,013,109（占 commit 57.46%、
  占整帧 51.34%）+ pick 1,474,168（42.08% / 37.60%）+ gc 14,554（0.42%）+ lineweight 1,314 + append 107 + rebase 0**；
  未计余量 ≈ **7µs**（commit 与 apply 均如此）。保留前缀 `resident_batches=166,345`，`group=2`（已提交），`base_changed=false`。
  对照初始基线提交（t≈35.17s）：frame 6,026,975µs，pulled 2,483,391（占帧 41.20%）+ pick 1,292,893（21.45%），且 apply 内
  **未计余量 1,500,414µs（28.14%）**（与 `aee434f3` 剖析一致）。
- **同一运行的手势内节奏**：`grid_refresh_pending=true`、内容修订冻结、suffix 冻结（本运行初始基线提交为 seq 255，结算才是首个
  suffix 提交）；实际渲染 **294 次 / 8.041s ≈ 36.6/s**（间隔 p50 30.0ms、max 55.7ms），呈现 297 帧，手势内最大 `frame_us`
  **39,547µs**；driver Finish、自然退出 0；`cad-frame error=0`、无 FAILED。
- **松手后仍在阻塞什么**：一次约 **3.92s** 的主线程提交，其中约 3.50s 为 `commit_upload`（**2.01s pulled 重建 + 1.47s pick
  重建**）加约 0.42s 引擎 render；该提交本身即 checkpoint 建立型完整重建（无既有 checkpoint），所以快速路径无从使用；
  第二次结算（同会话）已有 checkpoint，应走 reuse。
- **未宣称**：无性能验收、无超出上述数字的流畅度结论；无 Pick 算法/实现行为改动（本轮仅门控日志）；`gpu_ms` 仍为 `None`，
  pulled/pick 为 `Instant` 墙钟而非 GPU 时间；初始基线提交的 apply 未计余量 28% 仍未归因；保留既有证据，无 install/依赖/锁改动，
  无 `cad-render-wgpu`/pick 或他人测试文件改动，运行后无残留进程。

## 重复拖拽（单进程 3 次手势）的结算成本实测

- **驱动增强（最小、默认关闭）**：新增 `YACR_PROBE_POSTLOAD_PAN_GESTURES`（**默认 1 = 语义不变**，钳制 1..=10）；`PostloadPanState`
  增 `gestures_target/gestures_done`，总预算 `90s + 15s×(target-1)`（故 N=1 仍是历史 90s），每次手势开始时重置该手势的输入计数（与历史单次运行等价）。
  完成时日志为 `postload-pan settle N/M … settled; re-arming for the next gesture` 并重新武装；N=1 时走的仍是原来的 `Finish`（`end … natural_exit=0`）。
  二进制重建一次：sha256 **`f7283221cf0d087d0d22b8d51b676c5236ddbad4f0bed626df9a118c992c32ce`**（旧 `69e6c674…`/`f3021fe1…` 已过期）。
- **纯手势窗口（起点→gesture-complete，不含结算提交）**：g1 8.30s / 300 次渲染 ≈ **36.1fps** / 间隔 p50 30.0ms max 52.1ms / 最大帧 42.7ms；
  g2 8.23s / 300 ≈ **36.5fps** / p50 29.9 max 60.4 / 41.9ms；g3 8.29s / 303 ≈ **36.5fps** / p50 29.8 max 47.8 / 41.3ms。每次手势内
  `grid_refresh_pending=true`、**0 次内容推进、0 次手势内提交**。
- **每次结算提交**（均 `base_changed=false group=2 batches=166346`，保留 166,345 批基底）：
  | 结算 | frame_total | apply | render | present | commit[pulled / pick / gc / append / lineweight] | 计数 |
  | --- | --- | --- | --- | --- | --- | --- |
  | g1 @48.59s | **3,880,087µs** | 3,451,433 | 428,402 | 252 | **1,966,421 / 1,468,264** / 15,284 / 123 / 1,282 | reuses 0→0，rebuilds 1→2 |
  | g2 @59.53s | **1,561,545µs** | 1,528,551 | 32,652 | 342 | **625 / 1,503,908** / 13,646 / 108 / 1,318 | reuses 0→**1**，rebuilds 持平 2 |
  | g3 @70.52s | **1,587,539µs** | 1,554,286 | 32,893 | 360 | **655 / 1,532,478** / 12,466 / 87 / 1,334 | reuses 1→**2**，rebuilds 持平 2 |
- **复用结论（以证据为准）**：第 1 次结算是**建立 checkpoint 的完整重建**（该 166k 批复杂基底此前无可复用 checkpoint：t≈3.16s 的旧 checkpoint 属小规模初始准备，
  `rebuilds` 1→2）；**从第 2 次手势起复用生效**（`reuses` 0→1→2、`rebuilds` 持平），`pulled` 由 1.97s 塌缩到 **0.6ms**。
- **仍阻塞什么（实测，非假定）**：**pick 表重建 1.47→1.50→1.53s**（连复用后的结算也由它主导）＋约 33ms 引擎 render ＋ 13–15ms gc；
  复用后 `pulled` 已不重要，后两次结算约 **1.56s** 主线程阻塞，其中约 96% 是 pick。
- 每次结算后的最终帧均为 `complete=current_scene`（g1 后 frame_id=515 view_current=198；g2 后 frame_id=853 view_current=399；g3 后末帧 view_current=601）。
- **一句未来设想（未实现）**：基线提交（keep_prefix=0）也可顺带为其刚建好的基底登记 checkpoint，从而让会话中**第一次**结算也能复用、免付完整 pulled 重建。
- **未宣称**：无性能验收、无超出上述数字的流畅度结论；无 Pick 算法/实现行为改动（本轮源码仅驱动默认关闭的 N 手势开关）；`gpu_ms` 仍为 `None`；
  无 cad-render-wgpu/pick 或他人测试文件改动；无 install/依赖/锁改动；保留既有证据；运行后无残留进程。
