# cad-query 更新契约修复（2026-10-04）

依据：`CAD_IMPLEMENTATION_SPEC.md` v2.0 §4.4、§4.6、§4.9；前轮查询修复见
`docs/review-query-contracts.md`。

## 修复内容

- `update` 不再写入伪造的 `DocumentId(0)`；成功保持已发布查询的文档身份，
  失败保持文档、修订及数据库绑定不变。
- 复用 `ChangeSet::follows`：数据库身份一致、`before` 等于已观察修订、
  `after` 恰为下一修订才成功。其检查使用受检减法，没有未检查的修订加法；
  最大修订、跳号、倒退、不推进、重放均返回 `StaleResult`，不把重复通知
  或失败伪装成空成功。
- 图层和批注查询记录实际数据库的 `DatabaseId`，不推导 DocumentId 与
  DatabaseId 的映射，也不假定二者数值相等。
- 没有查询快照，或最新查询仅为选择属性时，数据库身份未知，更新明确返回
  `StaleResult`。选择属性查询没有数据库参数，因此清除先前数据库绑定，
  防止把调用者提供的属性修订冒充数据库修订。重新执行数据库查询可恢复绑定。

## 兼容性与边界

- 无公共签名变更，无其他 crate、依赖或共享交接文档修改。
- 行为收紧：此前首次通知、重放、外部数据库及部分不连续通知可能返回成功，
  现在必须处理 `StaleResult` 并从数据库重建查询快照（§4.6），不能继续使用
  旧投影作为新修订结果。失败保留最后成功状态，并不表示旧缓存仍然新鲜。
- 当前服务仍只跟踪最新查询的单个数据库。底图、批注查询交替时由最新实际
  查询替换绑定；属性查询即使修订数值相同也不建立数据库身份。多数据库独立
  游标、文档所属数据库注册表、显式重建状态机需要后续应用层契约设计；本轮
  不猜测映射，也不宣称完成多数据库订阅。
- `update` 仅跟踪已接受通知的修订，不自行更新行模型、重建快照或核实真实
  数据库内容；ChangeSet 的数据库标识不能证明数据库实例或内容 generation。
  同 ID、同修订重开与异步发布保障仍需宿主约束，未在此修复。

## 新增合成回归测试（9 项）

1. `update_preserves_document_and_accepts_consecutive_changes`：不同数值文档/
   数据库身份、连续更新、拒绝零文档、原文档仍可查询。
2. `update_without_snapshot_is_stale_without_binding_document`：初始化更新拒绝且
   不污染状态，数据库查询后恢复。
3. `update_rejects_unknown_database_after_selection_query`：仅属性查询不能确认
   通知来源数据库。
4. `selection_query_does_not_carry_forward_database_binding`：属性查询不能把任意
   修订与先前数据库身份拼接为可信绑定。
5. `update_rejects_foreign_database_without_mutating_binding`：外部数据库通知拒绝，
   原绑定仍可接受有效通知。
6. `update_rejects_discontinuous_and_non_advancing_revisions`：错误 before、跳号、
   不推进与倒退拒绝且不污染状态。
7. `update_rejects_replay_without_silent_success`：重复通知返回过期错误。
8. `update_at_max_revision_is_stale_without_overflow`：合成最大修订状态下不溢出。
9. `database_backed_query_replaces_observed_database_binding`：图层/批注查询切换
   后仅最新观察的数据库可以更新。

## 执行证据

**NOT RUN（子代理）**：新增 9 项测试未执行；未运行编译、测试、检查器或
格式化器，未提交或推送。主控负责统一合并后的 debug 编译及验证。
主控已统一格式化、提交并合并，最终 debug 全目标编译、严格 clippy、wasm 与静态检查通过；
Rust 契约仅编译、未执行，最终执行证据见 `docs/handoff.md`。
以上仅为新增合成契约，不能作为真实 DWG、渲染、GPU 或真机验证结论。
