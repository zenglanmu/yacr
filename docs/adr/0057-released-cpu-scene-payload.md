# ADR-0057: release the committed CPU scene payload after the GPU upload

- 状态：Accepted（2026-11-25）
- 关联：ADR-0007（实体即数据渲染）、ADR-0026（GPU 内存记账与预算）、ADR-0052（导出）

## 背景

`fixtures/complex-test.dwg` 的显示表示是 164,762 个 batch / 28.6M 顶点。GPU 上传完成后，
`cad-app::render_scene::ResidentScene` 仍保留一份完整的 CPU 几何（顶点、法线、边、索引、UV、
conic、pick sources），用于增量 append、单实体覆盖重传和设备丢失后的整场景重传。它在
`native-below` 的 P620 实测中占约 0.85 GB；在 4 GB 卡上，这份与 GPU 副本并存的 CPU 表示是
峰值和常驻 RSS 的主要组成之一。

`RenderBatch` 的每个 batch 以 `Arc<RenderBatch>` 共享给控制器、准备 worker、engine 的
`applied_base_scene`、`pending_upload` 和 append predecessor。任何持有者单独 `clear`/替换都
无法释放到最后一个引用，且直接清空会让重传路径“成功”提交 0 个 batch。

## 决策

1. **共享可释放的 `BatchSet`**：prefix 和每个 appended segment 各持有一个 `Arc<BatchSet>`。
   `BatchSet` 的 `len` 与内容 generation 在锁外/原子量中，几何 payload 在 `RwLock` 中；release
   写入锁后把所有 batch 换成 metadata-only（保留 origin/topology/style/texture/PBR/draw order/
   counts，丢弃 vertices/normals/edges/indices/uvs/colors/conics/sources）。所有 clone 立即看到
   `GpuOnly`。
2. **release 不改变身份**：`same_as` 比较 `BatchSet` 指针 + 该场景创建/替换时记录的 generation
   快照；release 不 bump generation，`replace_batch` 才 bump。append 准入、flags-only 发布和
   `resident_scene_presentable` 的语义不变。
3. **几何门是显式的**：`materialized(range)`/`resident_batches()`/`to_delta()` 返回
   `Result<_, MissingCpuPayload>`；范围切片计数改读 `vertex_count`，metadata 读取走
   `metadata()`。任何全量重传在触摸 renderer 前先检查 prefix payload；失败转换为稳定的内部
   token `render.base_payload_released`，由 `RenderSessionHandle` 捕获。
4. **只在一个提交后释放**：`CadRenderRuntime::release_committed_base_payload` 要求 Ready、
   `applied_revision`/`applied_base` 存在、`pending_upload` 为空；释放 `applied_base_scene`。
5. **显式重建漏斗**：捕获 token 后丢弃 partial upload，`CadSceneController::force_full_base_next_prepare`
   清空 base/append/update/retire 快捷路径，从 State 的 drawing/fonts/images/layers/vendor/display
   重新 `full_base`；`publish_controller` 强制新的 base revision；重试一次后仍失败才上抛。
   设备丢失、拒绝 overwrite、拒绝 append 的全量重传都走这一条。
6. **宿主在事件点归还空闲堆**：Linux/glibc 在 upload 开始和 payload release 后调用
   `malloc_trim(0)`（每事件最多一次，非每帧）；非 glibc 宿主为 no-op，不声称跨载体行为。
7. appended segment 的 `SceneDelta` 只保留 stamp/facts，不再保留 `added` 的第二份几何副本。

## 后果

- 正常打开/编辑/append 不触发重建；append 只 stage 新 tail，单实体覆盖仍走 in-place 路径。
- 设备丢失或拒绝 overwrite 的恢复成本是**一次完整 representation 重建**（本文件约十余秒），
  以显式诊断和一次重试表示；不会静默画空。重建会重新编号 pick key，selection 由 flags 重推。
- CPU 几何 release 后仍可通过 `metadata()` 提供 texture/PBR/counts，供纹理键收集和状态读取。
- Linux/glibc 的 `malloc_trim` 是宿主策略；Windows/macOS/Android 的等价归还策略未实现/未验证。

## 证据

- `fixtures/complex-test.dwg`，P620 / Vulkan / Linux `native-below`，2026-11-25：
  - 默认 universal、budget 80%：GPU 记账 1.07 GB；
  - 上传 staging 每 tick flush 后峰值 3.22 GB；
  - 本 ADR 的 payload release + 宿主 trim 后：进程树峰值 **2.47 GB**，稳态 **1.55 GB**，
    release 记录 `cpu_payload_releases=1`、`cpu_payload_bytes_released=548,732,592`；
  - 无 present 错误，帧继续产生。
- 契约测试：`cad-app` resident release/forced rebuild/append-after-release（全部通过）、
  `cad-engine` runtime/overwrite/append（134 passed，16 ignored hardware-only）、
  `cargo test -p cad-app --lib` 3375 passed。
- 未验证：真实设备丢失后的重建、Windows/macOS/Android/Web 的 RSS 归还与 trim；这些必须
  在对应载体单独运行，不能由 Linux 结果推广。
