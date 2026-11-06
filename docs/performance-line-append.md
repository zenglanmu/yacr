# 有界 LINE 增量显示更新

本轮不改变 React DOM + wgpu、数据库权威或载体所有权。

## CPU 契约

`HostController::{execute_traced_with_edit,confirm_draw_traced_with_edit}` 在真实命令执行前后
捕获数据库快照，返回可选、不透明的 `DrawingEditReceipt`。外部模块不能用独立的快照和
`RawCommandTrace` 拼造凭据。只有成功的单个 `CreateLine`、一个 Insert、相邻修订、计数加一、
无 source handle 的模型空间直线才会产生凭据。一般 insert-only ChangeSet **不是**块/表未改的证明。

`CadSceneController` 仅在**成功发布底图所属的精确 Arc** 等于凭据前序时复用底图；输入采用
状态不算成功发布。借用式 prepare 会取消共享快照复用资格。文档、空间、字体代数和 Arc、图像
缓存 Arc、图层 override、vendor Arc/修订必须保持一致，新增直线绘制顺序必须严格大于旧模型
空间最大值。任一条件不满足就完整重建；不会把失败改成空成功。

新增实体沿用全量构建的 provider、ByLayer/线型/颜色/透明度、visibility、vendor 与 compact
路径。旧 POINT 数据、images、masks 保留；overlay/flags 根据新数据库重新派生。
隐藏直线可以合法地新增零个 batch。

`PreparedScene.base_append` 保存精确前序 CPU 底图 Arc 和旧底图 batch 数，供 GPU 侧证明
resident 前序一致。完整的新 CPU 场景仍保留，用于设备恢复或错过前序时的完整上传。
overlay-only 再发布保留该证明，但不改变底图。

## 明确保留的成本

- 数据库候选仍深复制，保持失败不改变权威状态/分配器/历史的现有契约。
- CPU 旧底图 batch payload 仍深复制；这是减少表示重展开的有界优化，**不是 O(改动量)**。
- 不改变 renderer GC、pulled buffer 规则和渐进底图绘制。
- 其他命令、块/表/资源修改、纸空间、错过前序版本均保留完整路径。
- 对象捕捉的整图候选扫描仍是另一个已观察到的输入阻塞源，未由该增量路径解决。

验证与实测见 `validation-edit-performance-2026-11-06.md`。未运行的载体/真实输入/像素验收
仍为 **NOT VERIFIED**；本实现不能单独证明用户报告的卡死已消除。
