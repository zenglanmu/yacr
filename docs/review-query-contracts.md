# cad-query 小范围契约修复（2026-10-04）

依据：`CAD_IMPLEMENTATION_SPEC.md` v2.0 §4.4 的选择文档身份，以及 §4.9 的
分页、Mixed/Unset 和文档切换后丢弃旧结果要求。

## 修复与回归契约

1. **空选择属性自相矛盾**：此前先生成 `entity=Mixed`，再在 `count` 后追加
   `entity=Unset`；相邻去重无法移除两行，第一页甚至只显示错误的 Mixed。
   现在只生成一个 `entity=Unset`，同时保留 `count=0`。
   `empty_selection_reports_unset` 检查完整结果的唯一属性键、准确总数与第一页。
2. **属性查询缺少文档输入验证**：此前 `properties` 未调用其他查询共用的
   新鲜度检查，还会把其他文档的选择引用投影为当前文档属性并更新查询状态。
   现在先检查请求文档，再验证每个选择引用的文档；前者返回 `StaleResult`，
   后者返回 `InvalidInput`，失败均不发布状态。
   `properties_reject_foreign_document_selections_without_publishing` 覆盖全部外部
   文档与混合文档选择；`properties_reject_document_switch_without_changing_query_state`
   覆盖空选择也不能绕过切换检查，以及失败后原文档仍可查询。
3. **分页越过调用者上限**：此前共享分页函数把 `limit=0` 强制转换为 1，
   返回行数超过请求上限。现在零上限只返回总数，不返回行。
   `zero_limit_returns_no_rows_but_preserves_total` 使用非空图层与属性集合，
   检查行数、总数与回传请求上限；公共 `QueryRequest.limit` 注释明确该语义。

## 执行证据与边界

- **NOT RUN（子代理）**：未运行编译、测试、检查器或格式化器，未提交或推送。
  主控统一格式化、提交、合并后完成 debug 全目标编译、严格 clippy 与 wasm 检查；
  Rust 合成契约仅编译，未执行。最终快速门禁与中止记录见 `docs/handoff.md`。
- 仅修改 `crates/cad-query` 和本文档，不修改共享交接文档、其他 crate 或依赖。
- `properties` 没有数据库参数，本轮只校验文档身份，不宣称已验证实体存在、
  实例路径或子元素有效性；同实体不同实例仍按原实现显示实体 ID 属性。
- 未改动 `update` 的修订连续性/文档绑定机制，也未扩展分页实现为数据库级
  惰性查询。正数上限行为保持不变；依赖零上限隐式返回一行的调用方需调整。
