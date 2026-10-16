# Rust 审查：资源预算与失败传播（2026-10-04）

> **历史文档（Slint 桌面已退役，2026-10-16）**：本文描述的是**迁移前**的 Slint 桌面/web 宿主；
> 其中 `cad-ui-slint`、`yacr-linux`、旧脚本命令均已退役（部分脚本改为非零 NOT IMPLEMENTED）。
> 当前架构与构建见 `docs/architecture.md`、`docs/react-linux-app.md`、`docs/build.md`、
> [ADR 0010](adr/0010-react-linux-host-replaces-slint.md)。下文仅作历史记录。

## 已修复

- `MapResolver::grant` 计算替换后的总量：先减去同一规范化 key 的旧 payload，
  再以 checked addition 加上新 payload；溢出或超预算均显式拒绝。
  校验成功后才替换 map 并更新计数，失败保留旧数据、许可提示、其它资源和总量。
  不重复计算被替换 payload，也不额外克隆新资源的 `Arc`。
- `ResolverChain` 只对 `CadError::ResourceMissing` 继续按优先级查询。
  取消、无效输入、格式或其它错误直接传播，不再被降级为缺资源或被后续成功掩盖。

## 合成回归证据

新增两项契约测试，修复前 **2 failed / 15 passed**；修复后
`cargo test -p cad-resources -p cad-history --locked` 两个 crate 分别
**17 passed / 0 failed**。

- `replacing_a_resource_only_charges_the_resulting_total`：满预算替换、目录/大小写别名、
  缩小再扩大、超限替换拒绝且原资源不变、正确的超限诊断值。
- `chain_falls_back_only_for_missing_resources`：缺失时正常回退，以及三层各自的取消
  错误不得回退；完整缺失仍返回明确错误。

本轮 Linux release lavapipe 离屏 smoke 实际通过：
`/tmp/opencode/yacr-linux-20261004-011217-419337`；2 帧 CAD，导航改变相机和像素，
`renderError=null`。输入为**合成图纸**；不是参考图视觉验收、真实 GPU 或桌面窗口验证。
完整统一门禁结果在 `docs/handoff.md` 按当前代码轮次记录。

## 范围限制

资源预算约束 resolver 当前持有的 payload 总量，外部持有的 `Arc` 不计入该实例预算，
也不是进程峰值内存上限。图片/Xref/BigFont 支持能力没有因这次修复提升；未增加
文件系统或网络访问授权，未修改 acadrust。
