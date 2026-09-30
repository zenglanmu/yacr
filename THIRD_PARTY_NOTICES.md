# 第三方来源与许可

- acadrust：正式 0.5.5，唯一依赖入口 cad-import-acadrust；未修改上游，未使用 Cargo patch。
  当前导入仍占位；采用 API 与对外分发前审计锁定包许可及其传递依赖。
- Slint/wgpu：技术方向保留，尚未实际接线/锁定兼容组合，不能宣称集成完成。
- OpenCADStudio：未复制代码，尚未锁定 commit/许可；迁移必须保留逐文件来源/版权/许可。
- glam/serde/serde_json/thiserror 等实际依赖见 Cargo.lock；依赖许可证需建立分发审计。
- 未捆绑字体、用户图纸、图标或厂商插件。资源授权独立核对，不由主仓库许可推断。
