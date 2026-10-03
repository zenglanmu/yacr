
UI 采用 Slint 构建，参考 AutoCAD 的 Ribbon 功能区、图层与属性面板、浮动工具栏、命令输入区、模型/布局标签和状态栏，支持桌面、紧凑、移动及纯画布布局。设计版本化的 `ViewerConfig` 配置协议，支持 Rust 类型与 JSON 序列化，至少包含：`schemaVersion`；`ui`，用于配置预设、布局模式、组件显隐、位置、顺序及工具分组；`features`，用于限制查看、测量、批注创建/修改/删除、导入导出等操作能力；`view`，用于配置坐标轴、网格、选择高亮、捕捉提示及批注覆盖层；`interaction`，用于配置鼠标、触控和快捷键行为。`ui` 提供 `full`、`minimal`、`canvasOnly` 预设，布局模式提供 `auto`、`desktop`、`compact`、`mobile`；自动布局依据容器逻辑尺寸、可用高度、输入方式及安全区域计算，断点可配置。Ribbon、侧栏、浮动工具栏、命令栏、布局标签和状态栏分别支持显隐配置，工具项通过稳定的 `CommandId` 引用共享命令注册表，支持顺序、分组、图标/文字显示方式和溢出菜单；配置中不得包含可执行脚本或任意回调代码。配置解析顺序为“内置默认值 → 预设 → 宿主覆盖 → 宿主允许的用户偏好”，缺省字段继承、显式 `false` 必须生效，数组默认整体替换；操作能力由宿主策略设定上限，用户偏好不得重新启用被禁止功能。组件最终可见性同时受纯画布总开关、组件配置、布局规则及功能能力约束，命令可执行性另由当前会话状态决定，隐藏入口不等于禁用命令。`canvasOnly` 必须隐藏全部应用 UI、取消布局占位并清理其输入命中区域，画布覆盖内容由 `view` 独立控制。提供 `setConfig` 和结构化 `updateConfig` 接口，完整校验后原子应用，失败时返回字段路径及原因并保留旧配置；运行时调整布局不得重新创建 CAD 会话或丢失文档、相机、选择和批注，若禁用当前正在执行的工具，应取消未提交预览并返回导航状态。Slint 通过解析后的 `UiPresentationModel` 渲染界面，所有操作仍通过统一命令与事务接口执行，并提供有效配置查询与配置变更事件，便于宿主集成和自动化测试。

配置实例可采用下面的结构，名称均为项目自定义协议：

```json
{
  "schemaVersion": 1,
  "ui": {
    "preset": "full",
    "layout": {
      "mode": "auto",
      "breakpoints": {
        "compactBelow": 1200,
        "mobileBelow": 720
      }
    },
    "components": {
      "ribbon": {
        "visible": true,
        "tabs": [
          {
            "id": "review",
            "label": "审阅",
            "groups": [
              {
                "id": "measure",
                "label": "测量",
                "commands": [
                  "measure.distance",
                  "measure.angle",
                  "measure.area"
                ]
              },
              {
                "id": "annotate",
                "label": "批注",
                "commands": [
                  "annotation.text",
                  "annotation.leader",
                  "annotation.cloud"
                ]
              }
            ]
          }
        ]
      },
      "layerPanel": {
        "visible": true,
        "placement": "auto",
        "initiallyOpen": false
      },
      "propertiesPanel": {
        "visible": true,
        "placement": "auto",
        "initiallyOpen": false
      },
      "navigationToolbar": {
        "visible": true,
        "placement": "right",
        "commands": [
          "view.fit",
          "view.pan",
          "view.orbit"
        ]
      },
      "commandBar": { "visible": false },
      "layoutTabs": { "visible": true },
      "statusBar": { "visible": true }
    },
    "commandOverrides": {
      "annotation.cloud": {
        "visible": false
      }
    },
    "userCustomization": {
      "allowedPaths": [
        "ui.components.layerPanel.initiallyOpen",
        "ui.components.propertiesPanel.initiallyOpen"
      ]
    }
  },
  "features": {
    "measure": true,
    "annotations": {
      "create": true,
      "update": true,
      "delete": false,
      "import": true,
      "export": true
    }
  },
  "view": {
    "overlays": {
      "axes": true,
      "grid": false,
      "selectionHighlight": true,
      "snapHints": true,
      "annotations": true
    }
  },
  "interaction": {
    "pointer": true,
    "touch": true,
    "keyboardShortcuts": true
  }
}
```

断点单位为**逻辑像素**，示例数值可调整。`visible: true` 表示允许出现，`initiallyOpen` 表示面板初始是否展开，两者分别控制。

切换纯画布只需要更新：

```json
{
  "ui": {
    "preset": "canvasOnly"
  }
}
```

该预设应具有“隐藏应用 UI”的强制规则，即使此前配置了 `ribbon.visible: true`，也不能残留工具栏。恢复其他预设后，可以重新使用原有组件配置。
