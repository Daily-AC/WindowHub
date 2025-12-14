# WindowHub 开发文档

> **版本**: 0.1.0  
> **技术栈**: Tauri 2.x (Rust + WebView2)  
> **平台**: Windows 10/11

---

## 🎯 项目愿景

将分散在桌面上的多个应用窗口**聚合**到一个统一的容器中，实现类似浏览器标签页的操作体验，提升工作流效率。

---

## ✅ 已实现功能 (v0.1)

### 核心功能

| 功能               | 描述                                              |
| ------------------ | ------------------------------------------------- |
| **窗口嵌入**       | 拖拽任意窗口到 WindowHub，自动成为标签页          |
| **标签页管理**     | 点击切换、关闭、弹出窗口                          |
| **快捷搜索**       | `Ctrl+K` 搜索已安装应用或本地文件，回车启动并嵌入 |
| **文件管理器支持** | 支持嵌入 Windows 资源管理器窗口                   |

### 快捷键一览

| 快捷键           | 功能                     |
| ---------------- | ------------------------ |
| `Ctrl+K`         | 打开全局搜索             |
| `Ctrl+W`         | 关闭当前标签页           |
| `Ctrl+D`         | 弹出当前窗口（放回桌面） |
| `Ctrl+Tab`       | 切换到下一个标签         |
| `Ctrl+Shift+Tab` | 切换到上一个标签         |
| `Alt+1~9`        | 快速切换到指定标签       |
| `Alt+Space`      | 显示/隐藏 WindowHub      |
| `Alt+Q`          | 退出应用                 |

---

## 🏗️ 技术架构

```
┌─────────────────────────────────────┐
│           Web Frontend              │
│  (HTML/CSS/JS in src/index.html)    │
│  - 标签栏渲染                        │
│  - 搜索弹窗                          │
│  - 工作区 UI (计划中)                │
└─────────────┬───────────────────────┘
              │ Tauri IPC (invoke)
┌─────────────▼───────────────────────┐
│           Rust Backend              │
│      (src-tauri/src/lib.rs)         │
│  - 窗口枚举 (EnumWindows)            │
│  - 窗口嵌入 (SetParent)              │
│  - 焦点管理 (AttachThreadInput)      │
│  - 进程管理 (launch_app)             │
│  - 全局快捷键 (global_shortcut)      │
└─────────────────────────────────────┘
```

### 关键 API

- `embed_window(hwnd, parent)`: 核心嵌入逻辑，修改窗口样式并 SetParent
- `release_window(hwnd)`: 恢复原始样式，解除父子关系
- `activate_window(hwnd)`: 发送 WM_ACTIVATE/WM_NCACTIVATE 并设置焦点
- `force_repaint(hwnd)`: 强制重绘解决黑屏问题

---

## ⚠️ 已知限制与技术难点

### 1. 跨进程嵌入的本质问题

`SetParent` 属于 Windows "黑魔法"，微软**不推荐**用于跨进程窗口。

**表现**:

- 部分应用标题栏交互异常（如 Chrome 地址栏点击延迟）
- 嵌入后需要强制激活才能输入

**缓解措施**:

- 发送 `WM_NCACTIVATE` + `WM_ACTIVATE` 模拟激活
- 剥离 `WS_POPUP`/`WS_CAPTION` 样式强制子窗口化

### 2. 输入法 (IME) 兼容性

| 输入法类型 | 兼容状态 | 备注           |
| ---------- | -------- | -------------- |
| 讯飞/搜狗  | ✅ 良好  | 使用传统 IMM32 |
| 微软拼音   | ⚠️ 部分  | TSF 框架受限   |
| 微信键盘   | ⚠️ 部分  | 同上           |

**原因**: 微软 TSF 框架对跨进程输入上下文支持有限。
**缓解措施**: `AttachThreadInput` 延时 200ms 释放

### 3. 渲染问题

部分硬件加速应用（如网易有道词典）嵌入后可能黑屏。
**缓解措施**: 嵌入后自动调用 `RedrawWindow` + `InvalidateRect`

### 4. 最小化幽灵检测

已修复：最小化时不再响应拖拽检测（`IsIconic` 检查）

---

## 🚀 Roadmap

### v0.2: 工作区功能 (Workspaces)

- [ ] 保存当前会话（记录嵌入窗口的 EXE 路径）
- [ ] 一键恢复工作区（批量启动并自动嵌入）
- [ ] 工作区管理 UI

**技术要点**:

- 通过 `GetWindowThreadProcessId` + `QueryFullProcessImageNameW` 反查 EXE
- JSON 配置文件持久化

**局限**: 只能恢复"应用"，无法恢复"内容状态"（如 VS Code 打开的文件）

### v0.3: 分屏布局 (Split View)

- [ ] 左右/上下分屏
- [ ] 拖拽调整比例
- [ ] 四宫格布局

### v0.4: 侧边栏小部件

- [ ] 剪贴板历史
- [ ] 快捷备忘录
- [ ] 系统资源监控

---

## 📁 项目结构

```
WindowHubRust/
├── src/                    # 前端代码
│   ├── index.html          # 主界面 + JS 逻辑
│   └── styles.css          # 样式
├── src-tauri/
│   ├── src/lib.rs          # Rust 后端核心
│   ├── Cargo.toml          # Rust 依赖
│   └── tauri.conf.json     # Tauri 配置
└── DEVLOG.md               # 本文档
```

---

## 🛠️ 开发命令

```bash
# 开发模式
npm run tauri dev

# 构建发布版
npm run tauri build
```

---

_最后更新: 2024-12-14_
