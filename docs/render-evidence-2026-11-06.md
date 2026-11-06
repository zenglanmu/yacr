# 渲染实机证据（2026-11-06）

> **适用范围有限，不得外推。** 本文件只对下面这一组硬件/会话/拓扑成立。它不是 GPU 性能上限、
> 不是像素保真结论、不是其他平台证据。

## 环境

| 项 | 值 |
|---|---|
| 会话 | GNOME/Mutter **Wayland**（`loginctl` session 1，seat0，`/run/user/1000/wayland-0`，屏幕未锁定） |
| 输出 | `eDP-1` 1920x1080@60.001，**scale = 1.0** |
| 扫描输出 GPU | **Intel i915**（`card2`，`pci-0000:00:02.0`，接 `eDP-1`） |
| 渲染 GPU | **NVIDIA Quadro P620**（`card1`，`pci-0000:01:00.0`，无连接器），驱动 580.178.04 |
| 拓扑 | **混合显卡**：CAD 表面在 P620 上渲染，合成器在 i915 上扫描输出，中间经跨 GPU dmabuf 导入 |
| 二进制 | `target/release/yacr`（2026-11-06 09:18），前端 `apps/desktop-ui/dist`（09:16，custom-protocol 内嵌） |
| 启动 | 直接运行 release 二进制，`WAYLAND_DISPLAY=wayland-0`、`XDG_RUNTIME_DIR=/run/user/1000`、`XDG_SESSION_TYPE=wayland`，**未设置** `GDK_BACKEND` |
| 图纸 | `fixtures/complex-test.dwg` |

**跨 GPU 合成是本文件的前提**：下面 `present` 阶段包含 dmabuf 导入/合成代价，**不是**
“P620 直接扫描输出”的呈现延迟。

## 后端/设备决议（进程日志，逐字）

```
[probe] display backend policy=Auto resolved=Wayland source=exported by host
[probe] top-level native Wayland surface=0x592cabe926e0 display=0x592cabaa1840
[probe] backend-policy wgpu_backend=auto (vulkan preferred)
[probe] device-limits adapter_max_buffer_size=4292870144 requested_max_buffer_size=4292870144
        granted_max_buffer_size=4292870144
        requested_max_storage_buffer_binding_size=134217728
        granted_max_storage_buffer_binding_size=134217728 degraded=false reason=-
[probe] gpu-time requested=Features { .. } granted=Features { .. } state=disabled
[probe] present mode requested=MailboxElseImmediate supported=[Mailbox, Fifo] chosen=Mailbox
[probe] initial configure took 12519 us surface=hwnd
[probe] bridge wayland presenter ready generation=1 backend=vulkan adapter=Quadro P620
        device_type=discrete_gpu present_mode=Mailbox configure_us=12519
[probe] resize configure 1819x750 took 7956 us
```

结论：`backend=vulkan`、`adapter=Quadro P620`、`device_type=discrete_gpu`、`present_mode=Mailbox`
（从 `[Mailbox, Fifo]` 选出）、`degraded=false`、storage 绑定按**策略刻意**留在 128 MiB（不是适配器
上限 2,147,483,644 B）。

## 帧阶段耗时（稳态，逐字）

```
[probe] status backend=Some("vulkan") adapter=Some("Quadro P620") size=1853x1048 frames=55
        ticks=109 present_mode=Some("Mailbox") stopped=false marker=0.080
        stages_us[write=0 acquire=24 configure=0 submit=120 scope=1 present=810 total=969]
```

首帧（含上传/首配置）为 `write=646 acquire=653 configure=0 submit=56 scope=0 present=862 total=1578`。

## 一个必须记住的读数特征

`frames` 在 **55** 之后**不再增长**，而 `ticks` 继续走：

```
status ... frames=55 ticks=159 ... stages_us[.. total=969]
status ... frames=55 ticks=209 ... stages_us[.. total=969]
status ... frames=55 ticks=259 ... stages_us[.. total=969]
```

即**该应用是 on-demand 重绘**：静止时没有新帧。因此：

- **“fps” 在这里不适用**，不能用 `ticks` 或墙钟时间除以帧数算帧率；
- `createFpsSampler`（`apps/desktop-ui/src/fps.ts`，消费 `cad_status.frameCount`）只有在**连续拖动**
  时才有意义；
- `gpu_ms` 恒为 `None`（`gpu-time state=disabled`），**任何 GPU 时间结论都不能从本文件得出**。

## 本轮可用的结构性断言（真实 GPU，`tests/render_perf`）

`YACR_PERF_FRAMES=6 YACR_PERF_WARMUP=1 YACR_PERF_SCALES=debug-default-fonts:512`：

```
adapter: api=vulkan device_type=discrete_gpu name=Quadro P620 driver=NVIDIA
... [instanced] [full]: tier=accelerated accelerated=true draw_calls=7
    instanced_draws=1 instanced_batches=506 streams=1 segments=1012 uniform_writes=1
```

并且 `run_frames` 的**累计上传围栏**通过：计时帧之间 `FrameStats::uploaded_bytes` 不增长
（该计数器是累计值，`init.rs` 置 0，只在 upload 时增加，从不按帧重置）。

## 明确 NOT RUN / 不适用

- 脚本化连续拖动下的帧间隔分布（本轮只做了 3 px/帧的短程 pan，6 帧）；
- 透视投影路径的像素/几何误差量化；
- 真实浏览器 WebGL2 / WebGPU（本机无浏览器运行证据；CI `web-smoke` 是 SwiftShader，**不可**替代）；
- Windows / macOS / Android / 其他 GPU / 其他驱动 / 其他合成器；
- 128 MiB storage 绑定之外的 pulled 容量行为（见 `docs/compatibility.md` 的 limits 行）。
