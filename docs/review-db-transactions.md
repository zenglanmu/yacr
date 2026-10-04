# cad-db 事务正确性审查（2026-10-04）

范围仅为 `crates/cad-db`；依据 `CAD_IMPLEMENTATION_SPEC.md` v2.0
§4.3、§4.4、§4.6 与 §18.1。未修改其他 crate、共享交接文档或 SHAPE bounds。

## 修复

1. **实体迁移后遗留旧块引用**：写入路径原先只向目标块添加成员，没有从旧块成员、
   动态可见性受控集合及各状态集合移除。迁移至其他块或模型空间后，旧块查询仍返回
   同一实体，后续状态切换也会报告已迁出的实体。现在在全批验证通过后、实际替换前
   清理旧块引用，再按新空间挂接。复用删除路径的引用清理，但保留仍存活实体的显示属性。
   同一空间内更新不会清理可见性配置。
2. **可见性切换的虚假几何变更**：切换前集合包含非受控成员，切换后集合却仅计算受控成员，
   导致始终可见的成员被错误报告为隐藏。现在前后均以完整块成员集合计算，非受控成员
   始终保留；仅真实可见性差异进入有序 ChangeSet。状态确实改变但可见集合相同时仍递增
   revision，不伪造几何变化。

## 新增合成契约测试

文件：`crates/cad-db/tests/transaction_membership.rs`。

- `moving_entity_between_blocks_prunes_old_visibility_references`：事务迁移、旧状态引用清理、
  新块归属与后续可见性 ChangeSet。
- `moving_entity_to_model_space_removes_old_block_membership`：直接共享写入路径迁至模型空间。
- `failed_batch_leaves_space_membership_and_visibility_unchanged`：后续非法变更拒绝整批，
  对象、块、动态状态、revision 与分配器均保持原值。
- `visibility_switch_does_not_report_ungoverned_entities_as_changed`：已解析及未解析的起始
  状态均不把非受控成员列入变化。
- `equivalent_visibility_sets_change_state_without_geometry_delta`：只切换状态、不改变可见
  几何集合时仍记录新 revision。

## 执行证据与限制

**NOT RUN（子代理）**：未编译、未运行测试、未执行格式化或检查，只写代码、测试与本文档。
主控已格式化、提交并合并分支，最终 debug 全目标编译、严格 clippy、wasm 与静态门禁通过。
新增 Rust 合成契约仅编译、未执行，不声称测试通过。统一执行证据见 `docs/handoff.md`。

没有真实 DWG、GPU、窗口、浏览器、模拟器或真机验收。此次没有全面审查导入 Builder
的宽松验证、分配器耗尽、revision 溢出、历史重放或下游缓存行为；不据此宣称数据库
全量正确。空间迁移不自动继承目标动态块的受控状态：新成员沿用现有写入语义，作为
非受控成员可见。导入阶段已存在的成员/空间不一致不在本轮修复范围内。
