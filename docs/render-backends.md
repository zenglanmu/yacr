# 渲染后端契约（未运行）

Auto/WebGPU/WebGL2 分开设置，UI 与 CAD 后端配置独立。Auto 必须实际尝试初始化并检查
features/limits；强制失败给用户原因/回退选项，不能只检测 navigator.gpu。
WebGL2 基础路径不依赖 compute/storage buffer/indirect draw；增强路径保持相同测量语义。

共享设备候选：Slint 作为唯一呈现协调者，CAD 同 Device/Queue 离屏纹理合成。
Web 受阻可验证双 Canvas 分区；不得每帧 GPU→CPU→GPU 整幅回读。
必须核查纹理 format/usage、MSAA resolve、alpha 预乘、sRGB、队列顺序、resize、device lost。

当前没有 Slint/wgpu 实际版本对齐、纹理桥、shader 编译/运行或平台验证。
Cargo 中未使用的候选版本不是兼容性证据；接入前用 ADR 锁定真实可编译组合。
切换后重建所有 GPU 派生资源，保持文档、相机与批注；失败重试有界，先走未保存保护。
