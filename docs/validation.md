# 验证记录

环境：Linux x86_64，rustc/cargo 1.98.1；检查日期 2026-09-30。
下面记录本次框架快照检查，不代表产品功能验收。
检查通过后另有并发工作开始接入 Slint（cad-ui-slint manifest/build.rs/source 变化）；
以下 workspace/Wasm 结果适用于接入前的契约框架，不能覆盖后续变化。
最新全量 clippy 检查因构建锁等待超时，未取得最终通过证据。

| 检查 | 结果 |
|---|---|
| cargo check --workspace | 通过，23 包 |
| cargo test --workspace | 通过：34 个合成/核心测试（数据库 7、几何 13、代理 9、框架契约 5） |
| cargo check --workspace --lib --target wasm32-unknown-unknown --locked | 通过；不代表浏览器运行 |
| python3 scripts/check-architecture.py | 通过；DAG/核心边界 |
| cargo clippy 全量 | 未通过：先发现 CommandPayload 大枚举已改 Box，后续检查遇到并发构建锁超时 |
| Android APK/真机、iOS/桌面 | 未运行，尚未交付 |
| Slint 编译/布局、wgpu/shader/GPU、WebGL2/WebGPU | 本次未完成验证；另有进行中的接入变更 |
| 真实 DWG、天正/探索者、字体、黄金图、性能预算 | 未运行，无授权样本 |

Wasm 曾因 acadrust 传递依赖 getrandom 0.3 的后端缺配置失败；通过
导入适配的 target-specific wasm_js feature 和 `.cargo/config.toml` cfg 修正，
未修改 acadrust 或其传递依赖源码。仍需浏览器/Worker 运行验证随机数可用性。

恢复中保留已有 DB/几何/代理新增实现，不将其重置为空；它们的合成测试仅是基础证据。
