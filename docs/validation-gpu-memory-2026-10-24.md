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


## 7. 显存百分比预算（2026-10-24 追加，默认 90 %）

命令：`cad-cli-tools render fixtures/complex-test.dwg --gpu high`（Vulkan / Quadro P620 / 驱动 580.178.04）。

```
exit=0
gpu-memory: total=2107964952 buffer=2100592152 texture=7372800 peak=2107964952
            budget=3865470566 cap=3865470566 vram=4294967296 percent=90
            budget_source=vram-percent source=accounted
            allocator_total=2109386656 reserved=2617245696
```

- **VRAM 代理 = 最大 device-local heap = 4,294,967,296 B（4.00 GiB）**；90 % →
  **cap = 3,865,470,566 B（3.60 GiB）**，低于物理容量；实测常驻 2.11 GB，仍有余量。
- 交叉核对：驱动 allocator `2,109,386,656 B`，与记账差 **0.07 %**。

### 7.1 被实测否决的口径：device-local heap 求和

同一卡上"所有 device-local heap 求和"得到 **4,552,916,992 B**（比物理容量高 **6.0 %**，
多出的是 host-visible device-local 堆）；其 90 % = 4,097,625,292 B（3.82 GiB），逼近物理上限、
几乎不留余量。**已改为最大单堆**；求和值作为 `vram_all_heaps_bytes` 保留（透明，不驱动 cap）。
（先前记录里"90 % 超过物理显存"的表述**有误**：3.82 GiB < 4 GiB；正确结论是"余量过小"。）

### 7.2 拒绝与优先级

- `--gpu-budget-percent 10` → cap `429,496,729` →
  `exit=7`、`gpu-memory-refused: origin=application-budget category=mask used=1573159200 requested=144 cap=429496729 cap=…`、
  **无 PNG**。
- `--gpu-budget-percent 10 --gpu-budget-mb 4096` → 成功且 `percent=none`、
  `budget_source=explicit-bytes`（显式字节 > 百分比）。
- 设置持久化路径：`view.gpuBudget.percent`（默认 90，10..=100，选项对话框可改；越界**拒绝**不裁剪）。

### 7.3 边界

- `vram_bytes` 是**堆容量**，不是空闲显存；也不含其它进程占用。
- 只对 Vulkan 生效；D3D12/GL/Metal 本轮为 `None` → 3 GiB `Estimate` 兜底（D3D12 的 DXGI 查询未做）。
- 未测：其它 GPU/驱动、多 GPU、共享显存 iGPU、真 X11/Windows。


## 8. 探测不到 = 不设预算（2026-10-24 追加）

- 决策：**去掉 3 GiB 估算兜底**。无 `vram_bytes` 且无显式 `--gpu-budget-mb` 时
  `cap_bytes = None`、`budget_source = unprobed`；拒绝路径不会因缺少探测而触发。
- 单测覆盖（本机 P620 能探测，**无法**现场构造未探测后端）：
  - 无探测 → 巨型常驻集合也**不拒绝**，报告 `budget=none cap=none budget_source=unprobed`；
  - 配置了百分比但无探测 → 仍 unprobed（百分比不生效）；
  - 无探测 + 显式字节 → **仍会拒绝**（显式上限不依赖探测）。
- 现场可复验的部分（Vulkan/P620）：默认 90 % 行为不变
  （`vram=4294967296`、`cap=3865470566`、`budget_source=vram-percent`，exit 0）。
- 边界：`budget_source=unprobed` 的**运行时**证据为 **NOT RUN**（本机后端可探测），只有单测；
  D3D12/GL/Metal 的未探测路径同样只有单测。
