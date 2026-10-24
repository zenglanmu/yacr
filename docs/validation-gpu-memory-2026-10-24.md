# 验证记录：原生 GPU 显存记账与预算（2026-10-24）

> 范围：**单机、单后端、单样本**。这是 ADR 0026 的落地证据，不是性能、不是 vendor 兼容、
> 不是"显存总量"结论。

## 1. 环境

| 项 | 值 |
|---|---|
| GPU | Quadro P620（`nvidia-smi`：4096 MiB，驱动 **580.178.04**） |
| 后端 | Vulkan（`--gpu high`） |
| 工具 | `cad-cli-tools render`（debug 构建，`--locked`） |
| 样本 | `fixtures/complex-test.dwg`（4,228,788 B，authorized fixture） |

## 2. 成功路径（默认预算 3 GiB）

```
exit=0
gpu-memory: total=2107964952 buffer=2100592152 texture=7372800 peak=2107964952
            budget=3221225472 cap=3221225472 source=accounted
            allocator_total=2109386656 reserved=2617245696
png bytes 27833
```

分类明细（`--gpu-memory-detail`）：

```
gpu-memory-detail: vertex=303645612 index=80201976 uniform=4325456 instance=16159896
                   stream=1695124632 mask=446436 color-target=3686400 depth-target=3686400
                   image-texture=0 mask-texture=0 other=688144
```

可证明：
- 渲染器**自身记账**（`source=accounted`）与驱动的 allocator 报告
  （`allocator_total=2109386656`）相差 **0.07 %** —— 记账不是自说自话。
- 该样本在 GPU 侧常驻 **2.11 GB**，其中 **`stream` 1.70 GB** 是最大头（> 同文件 wasm 堆 1.43 GB）。
- `peak == total`（单点原子提交，没有旧/新场景同时驻留的瞬态被隐藏）。

不能证明：显存**总量**（wgpu 不提供）、其它 GPU/驱动、其它样本、性能/帧率。

## 3. 预算拒绝（显式，且不产出假产物）

```
$ cad-cli-tools render fixtures/complex-test.dwg --gpu high --png /tmp/opencode/gpu-no.png --gpu-budget-mb 8
exit=7
gpu-memory-refused: origin=application-budget category=vertex used=8040572 requested=2034216 cap=8388608 limit=8388608
（/tmp/opencode/gpu-no.png 不存在）
```

- 退出码 **7**，与 0/1/2 区分；`gpu-memory-refused:` 为稳定行，含 origin/类目/已用/请求/上限。
- 拒绝发生在 staging 之前，**没有**写出一张"看起来成功"的 PNG。

## 4. 默认值的证据驱动修正（记录一次真实缺陷）

- 初版默认 **512 MiB**，在同一命令上**拒绝了合法图纸**：
  `origin=application-budget category=mask used=1573159200 requested=144 cap=536870912`。
- 依据本文 §2 的实测（2.11 GB）改为 **3 GiB**，并把该回归钉成单测
  （`the_default_budget_admits_the_measured_fixture`，同时断言旧 512 MiB 会拒绝 1,573,159,200 B 状态）。
- 3 GiB 仍是**应用估算**，不是该卡 VRAM；每台机器可 `--gpu-budget-mb` / `YACR_GPU_BUDGET_MB` 覆盖。

## 5. 优先级

`--gpu-budget-mb` > `YACR_GPU_BUDGET_MB` > 默认：
`YACR_GPU_BUDGET_MB=16 … --gpu-budget-mb 4096` → 成功（flag 胜出）；
`YACR_GPU_BUDGET_MB=4096` 单独使用 → 成功且 `budget=4294967296`。

## 6. 明确的 NOT RUN / NOT VERIFIED

- **驱动 OOM（`origin=device-out-of-memory`）未现场触发**：仅由 error-scope 分类 + 单测覆盖；
  现场触发需要把 GPU 压到 OOM，风险高且会污染会话，未做。
- **D3D12 阈值（80 %/95 %）未在任何 Windows/D3D12 环境验证**；Vulkan 上启用该阈值**已被实测否决**
  （同进程第二个 device 首次 72 B 写入即 OOM，本 crate 3 个 headless 测试稳定失败），故限定 D3D12。
- 未测：其它 GPU/驱动/样本、显存总量、跨进程显存占用、性能与帧时间。
- `stream` 1.70 GB 的**优化**未做（ADR 0026 §4 明确另立项）。
