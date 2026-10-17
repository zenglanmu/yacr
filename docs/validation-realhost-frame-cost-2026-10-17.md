# 真实宿主帧成本探针（2026-10-17）

**目的**：解释一次用户手测现象——用 `./target/release/yacr` 打开
`fixtures/debug-default-fonts.dwg` 时 pan/zoom 较流畅，打开 `fixtures/complex-test.dwg` 时明显卡顿；
且**深度放大到大部分图元出屏后仍卡**。本文件记录**本轮实测**的量化结果与边界。

**这不是**性能验收、不是 GPU 时间测量、不是 vendor/发布版结论；它只解释"为什么这一对图纸手感不同"，
并为 `docs/render-optimization-backlog.md` 的 P0/P1/P7 提供真实场景证据。

## 环境

| 项 | 值 |
|---|---|
| 机器/会话 | 本机 GNOME/Mutter 桌面会话（`gnome-shell --mode=ubuntu` + `Xwayland :0`），同一 UID，无提权、非 root、未屏录 |
| 后端 | **Wayland**（宿主决议并打印：`policy=Auto resolved=Wayland source=exported by host`） |
| 适配器 | **直接证据**：两次运行的 `cad_status` 均报 `"backend":"vulkan","adapter":"Quadro P620"`（complex 腿部该组合出现 **153** 次）；全程 **零** `OOM`/`device lost`，`error=None` |
| 渲染档 | **`tier=accelerated`**（两个 fixture 一致） |
| 二进制 | `target/release/yacr`，`sha256 61d5d4ae354f8ff81a4195f3619821789c52db24ebc9ffa7103d3f983fec408b`，构建于 2026-10-17 08:53（**prebuilt，未在本轮重新构建**） |
| 计时口径 | 宿主逐帧 `[probe] actual-render …`（状态变化或约 1 s 心跳）。`gpu_ms=None`——**没有 timestamp query，GPU 时间仍未测量**；`wait_ms` 只是 GPU/呈现等待的**代理** |

## 方法

在用户自身会话内直接运行**已有的** release 宿主，用**有界的** post-load pan 驱动自动施加一次真实平移手势
（8 s，`admitted_moves/ups` 已由宿主确认），全程用 `YACR_PROBE_CAD_LOG=1` 记录逐帧状态：

```bash
env -u DISPLAY XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-0 \
  DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus \
  YACR_PROBE_CAD_LOG=1 YACR_PROBE_POSTLOAD_PAN=1 YACR_PROBE_NO_MAXIMIZE=1 \
  timeout 150 ./target/release/yacr --open <fixture>
```

两次运行均 `exit=0`（`natural_exit=0`）。原始日志与逐帧 `results.tsv` 落在
`${YACR_EVIDENCE_ROOT:-/tmp/opencode}/yacr-evidence-20261017T024549Z-realhost-probe/`（仅本地，未上传）。
`YACR_PROBE_NO_MAXIMIZE` 仅为不占满用户桌面，**不改渲染语义**。

## 结果

| fixture | entityCount | 帧数 | `over_budget=true` | `complete=false` | 中位 `draw_calls` | 中位 `verts` | 中位 `tris` | 中位 `wait_ms` | p90 `wait_ms` | 中位 `cpu_ms` | `wait_ms>16.6ms` 占比 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `complex-test.dwg` | 44,591 | 359 | 354 | 354 | **3,027** | **5,454,559** | 1,999,514 | **19.382** | 22.652 | 8.274 | **61.3%** |
| `debug-default-fonts.dwg` | 10,656 | 479 | 473 | 473 | 1,195 | 2,170,905 | 1,999,961 | **2.895** | 5.369 | 2.072 | 0.4% |

## draw 类别分解（我方 app 逐帧日志，取中位；已排除 demo 文档帧）

`draw_calls` 的构成见 `crates/cad-render-wgpu/src/renderer_impl.rs:4870`：
`legacy_opaque + instanced_draws + mesh_draws + arc_draws + transparent_batches + images`。
宿主逐帧日志目前只打印 `instanced`/`mesh` 两个分量，其余需插桩才能逐项分离。

| fixture | `draw_calls`/页 | `instanced` | `mesh` | 残余（legacy/arc/transparent/image） |
|---|---:|---:|---:|---:|
| `complex-test.dwg` | 3,027 | 1,615 | 324 | **1,088（35.9%）** |
| `debug-default-fonts.dwg` | 1,195 | 823 | 268 | 104（8.7%） |

**待定（决定 P1 还是 P7）**：那 1,615 次 `instanced` draw 在加速档下究竟是"**按 chunk 的 run**"
（则可被 chunk 级 AABB 剔除跳过）还是"**binding 尺寸的 segment**"（则不可，须走 VS 侧 early-out）。
这**必须插桩确认**；**不得**沿用合成场景（3/19 次 draw）的结论下判断。

## 与同轮 harness 报告（`docs/validation-render-perf-2026-10-17.md`）的关系

同轮 harness 报告记录 `complex-test.dwg` 在 **P620（4 GB）** 上**三次 OOM**，失败点在
**legacy 逐批顶点缓冲**（`Renderer::vertex_buffer` ← `realscene_render_perf` 的 `legacy.upload`），
原始留痕见 `/tmp/opencode/yacr-perf/p0-realscene-debug-complex/`（空目录＝失败轮次的留痕）。

**两者不冲突——走的是不同路径**：

- harness 的 `legacy` 腿为**每个批次**建 GPU 顶点缓冲（该图 44,591 个批），超出 4 GB 卡；
- **app 真实宿主走 `tier=accelerated`（顶点拉取 storage buffer），不需要逐批顶点缓冲**，
  因此同一张卡、同一张图**完整跑通 359 帧**。

⇒ **"P620 上 complex-test 无法渲染"只对 legacy 逐批路径成立，对 app 的实际路径不成立**；
不得用 harness 的 OOM 否定 app 的可运行性，也不得用 app 的成功掩盖 legacy 路径的容量上限
（后者是**独立的真实缺陷**，值得单独立项）。

另外该报告的**合成腿**已含 full/zoom 两个视图态，其结论是"**`draw_calls` 在两种视图态下完全相同**、
wall 差异 <20%"——这是本轮"**放大不改善**"的**脚本化证据**；但它属于 harness 路径证据，
**不等于** app 路径证据。

## 结论（本轮实测支持的部分）

1. **两档都是 `tier=accelerated`、真实 GPU（Quadro P620 / Vulkan）**，不存在"软件后端冒充"。
2. **两图每一帧几乎都超预算并进入渐进分页**（`over_budget=true`、`complete=false` 各占 99%），
   单页三角形**饱和在约 2,000,000**（1,999,514 / 1,999,961）——说明存在**按页预算截断**，
   整帧被切成多页呈现。complex 的整帧工作量单位总量为 **166,346**（debug 为 **10,548**，约 15.8×）。
   **口径更正（2026-10-17，同批日志）**：`over_budget=true` 说的是"本帧**计划**越过了**按页预算**"，
   **不是**"这一帧慢"——该标志本身**不**是耗时指标。实测：`over_budget=true` 在 debug 为 **473/479** 帧，
   而 `wait_ms > 16.6 ms` 仅 **0.4%**、每帧 ≈ 2.072 + 2.895 ≈ **5 ms**（60 fps 预算之内）。
   ⇒ **debug 不是时间受限**（其帧被切成多页是撞上 2M 三角形**页上限**，不是帧超时）；只有 complex 是。
3. **手感差异由 GPU/呈现等待主导，而非 CPU 相位**：complex 中位 `wait_ms` **19.4 ms**（≈1.17 个 60 fps
   帧预算），CPU 相位合计中位 **8.3 ms**，单帧合计约 **28 ms（≈36 fps）**，且有 2 帧 `wait_ms>33 ms`；
   debug 为 **2.9 + 2.1 ≈ 5 ms**（60 fps 上限内）。**complex 有 61.3% 的帧仅"等待"就已超过 60 fps 预算**
   （debug 0.4%）。
4. **与"放大后仍卡"一致**：整帧工作量由**整套图纸的计划**决定，**不随可见矩形收缩**——
   这正是**缺少视口剔除**的直接后果（backlog P1/P7 的动机）。
5. **本轮实测同时修正了一条由合成场景得出的推断**：合成 scale-matched 场景下加速档只有 3/19 次 draw，
   据此曾推断"P1 在加速档≈省 0 次 draw"。**真实 complex 场景下加速档每页发出约 3,027 次 draw**，
   与合成场景差三个数量级。"P1 在加速档是否无收益"**必须按 draw 类别分解验证**——初步分解见上表
   （1,615 instanced + 324 mesh + 1,088 残余），**但哪一类可被 chunk 剔除仍未确定**，须插桩。

## 相位分解与分页重启（本文件日志的**再分析**，未新跑）

同一批逐帧日志（`results.tsv` / 原始 `stderr.log`）的二次汇总，排除 demo 文档帧。**仍是测量**，
不是新运行。

### 每帧 CPU 相位（中位，ms）

| fixture | `cpu_ms` | = plan | + uniform | + encode | + **submit** | 未覆盖 | `wait_ms` | 帧 ≈ cpu+wait |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `complex-test.dwg` | 8.364 | 2.590 | 1.123 | 1.131 | **3.412** | 0.109 | **19.389** | **27.75** |
| `debug-default-fonts.dwg` | 2.072 | 0.484 | 0.126 | 0.482 | 0.936 | 0.044 | 2.895 | 4.97 |

`submit_ms` 是 CPU 侧最大相位（占 complex CPU 的 **41%**；3,027 draws ⇒ ≈1.13 µs/draw），
与"通用 per-run 路径的逐 draw 记账"一致。**帧时间由 `wait_ms` 主导**（19.389 / 27.75 ≈ 70%）。

### 分页重启：相机一变，本帧完成度塌缩

**更正（2026-10-17，同批日志）**：下表「**变更后第 1 帧完成度（中位）**」列（debug 的 **86.3%** 就在此列）是
**相机变更后第 1 帧的完成度中位**，
**不是**"超预算帧占比"，也**不是**耗时指标。debug 的 **86.3%** 应读作"**debug 变更后一帧基本就画完了**"
（相机变更 471 次里 **0** 次把完成度压到 <5%），**不得**读成"debug 也有 86.3% 的帧超预算/慢"。
与之对照，`over_budget=true` 在 debug 为 473/479 帧，但那说的是**计划越过了按页预算**（2M 三角形页上限
饱和），其 `wait_ms > 16.6 ms` 仅 **0.4%**、每帧 ≈ 5 ms —— **在 60 fps 预算内**。

| fixture | 帧数 | 相机变更次数 | **变更后第 1 帧完成度（中位；非"超预算帧占比"）** | 完成度 <5% 的变更 | `complete=true` 帧数 |
|---|---:|---:|---:|---:|---:|
| `complex-test.dwg` | 356 | 182 | **4.6%** | **182/182（全部）** | **2** |
| `debug-default-fonts.dwg` | 479 | 471 | **86.3%** | 0 | 6 |

**推论（源码 + 实测一致）**：相机变化使渐进帧重启（`FrameKey` 含相机；`renderer_impl.rs:3856-3890`），
于是交互期间每帧只呈现**"draw-order 前缀的第 1 页"**；complex 一页只接纳 ≈ 4.6% 的批
（≈7,716/166,346），**356 帧里只有 2 帧真正画完**。

### `tier` 与 `accelerated` 是两个字段（源码核对）

`FrameStats` 同时有 `tier: RenderTier` 与 `accelerated: bool`（`crates/cad-render-wgpu/src/lib.rs:353-364`，
其文档注释明确：加速设备在**回退到通用 per-run 的分页帧**上仍报告 `Accelerated`）以及
`accelerated_draws`。本文件早先记录的 `tier=accelerated` 是**设备档**，**不等于**该帧走了 per-class
组织。`use_pulled` 要求 `converted_accepted == converted_total`（`renderer_impl.rs:4085-4090`）
⇒ **分页/超预算帧一律走通用 per-run**；这也解释了 complex 每页 `instanced`=1,615（按 stream 合并的
run）而非 pulled 路径的 ~15 段量级。

## 产品决定：**接受**运动过程中的帧内容不全（2026-10-17，用户）

- 用户明确接受"相机运动中每帧只呈现 draw-order 前缀页（complex ≈4.6%）"这一可见行为。
- **后果（范围收缩）**：**不**为"运动帧内容更全"引入可见性排序页内容（原 N1 中唯一产品可见、
  风险最高的子项）；本文件原拟的"运动中帧"截图判读**不再需要**（该行为已被接受，无需视觉判定）。
- **未被接受的部分照旧**：分页重启带来的**代价**不变——每帧仍重付 plan/submit（上表），
  `wait_ms` 仍主导帧时间；`over_budget`/`complete=false` 仍必须**显式**报告（铁律 2）。

## `complex-test.dwg` 在 4 GB 卡上的 OOM（实测，含 backtrace）

**这不是性能结果，是容量缺陷**；本轮 3/3 次复现（其中一次带 `RUST_BACKTRACE=1`）。

```
thread 'main' panicked at wgpu-30.0.1/src/backend/wgpu_core.rs:1619:18:
wgpu error: Out of Memory
  4: <CoreDevice as DeviceInterface>::create_buffer
  6: <cad_render_wgpu::Renderer>::rebuild_pulled
  7: <cad_render_wgpu::Renderer>::commit_upload
  8: <cad_engine::render::RenderSessionHandle>::apply_ready_updates
  9: <app_linux::gtk_bridge::GtkOverlayOwner>::frame_cad
 11: app_linux::host::tick
→ exit=101
```

崩溃前的上传上下文（同一轮）：

```
upload=seq=256 ... base_changed=true committed=false batches=165272 prepare_us=8021
upload=seq=257 ... base_changed=true committed=false batches=165784 prepare_us=15510
upload=seq=258 ... base_changed=true committed=false batches=166296 prepare_us=20080
```

**归因（backtrace 直证 + 算术）**：该图产生 **166,296 个 batch**；加速档的 `rebuild_pulled` 需为
**整幅图纸**分配顶点拉取实例缓冲（段数在 2×10⁷ 量级 × 每实例字节 ⇒ **~1.7–1.8 GB 的单次 `create_buffer`**），
而上传阶段的逐批缓冲此时已驻留，4 GB 的 P620 无法满足该单次分配；wgpu 30 的 `create_buffer`
（`wgpu_core.rs:1619` 默认错误处理器）**没有可失败返回路径 ⇒ panic**。

**同时暴露的两点**（独立于容量）：
1. 崩溃前的 `prepare_us` 为 8.0 → 15.5 → 20.1 —— 该字段单位是 **µs**，即 **8/15/20 毫秒**
   （早先文字误读为"秒"，**已更正**）；三轮均 `commit[-]` 未提交。整次打开实测约 **49 s**，
   含 DWG 导入、526 个分批 staging 工作单元共 ≈5.7 s、两次全场景 commit 各 ≈2.3–2.4 s，
   **不是**"单次分配冻结 20 秒"；
2. 失败形态是 **panic 而不是显式容量诊断** ⇒ 违反铁律 2 的精神（失败必须显式建模）。

**边界**：`debug-default-fonts.dwg`（10,656 实体）不受影响（正常打开、`pulled_rebuilds=1`）。
本项**不**说明其它 GPU/显卡容量的行为；也不构成性能结论。修复方向（进行中）= **容量预检 + 显式回退**
（超限则不建 pulled、走通用 per-run 并显式上报），而非放宽任何判据。

## 未运行 / 边界（不得省略）

- **NOT RUN（app 路径）**：zoom 腿。宿主只有 `YACR_PROBE_POSTLOAD_PAN[_GESTURES]`，**没有 zoom 驱动**，
  故 app 路径下"放大后仍卡"是**用户观察**＋结构性推理。**视图态不变性**已由同轮 harness 报告在
  **合成腿**上脚本化证实（见上），但那是 harness 路径，**不等于** app 路径证据。
- **NOT RUN**：GPU 时间本身。`gpu_ms=None`，无 `TIMESTAMP_QUERY`（本仓默认不向宿主设备请求 feature）；
  `wait_ms` 是代理，**不是** GPU 时间。
- **NOT RUN**：全量门禁/测试矩阵、wasm/Android/Windows/macOS 目标、任何视觉/精度/vendor/golden 判定。
- 单机、单适配器、一对图纸、**prebuilt** release 二进制、无 release/debug 对照、无重复轮次统计。
  不得外推到其它图纸、其它 GPU/vendor、发布版或浏览器。
- **本文件是 pacing 之前的口径**（`over_budget`/页预算均为**静态** 8M 顶点 / 2M 三角形）。pacing 落地后
  另有独立运行（790 / 458 帧）记录于 `docs/validation-frame-pacing-2026-10-17.md`；两组帧数与
  `budget_*` 字段**不得混算**。
- 口径提醒：本文件 complex 的 `entityCount=44,591` 与
  `docs/validation-render-perf-2026-10-15.md` 的 122,006 **不是同一口径**，**不得混用或相除**。
- 用户手测的启动命令需核对：`LaunchOptions::from_args` 只识别 `--open/--fonts-dir/--images-dir/--probe`，
  **裸位置参数被忽略**（`_ => {}`）；若当时写的是 `yacr <path>`，则那次对比的加载前提不成立。
