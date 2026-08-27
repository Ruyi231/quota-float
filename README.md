# Codex 本地额度

一个面向 Windows 的轻量级 Codex 额度浮窗。它默认从本机 Codex 会话日志读取最近一次额度记录，并在用户手动确认后提供在线额度和重置卡查询。

当前版本：**v1.3.4**
支持平台：**Windows x64**

> 本项目是社区工具，不是 OpenAI 官方产品，也不隶属于 OpenAI。

## 下载与使用

普通用户可前往 [GitHub Releases](https://github.com/Ruyi231/quota-float/releases/latest) 下载最新的 Windows x64 便携版 EXE。

1. 确保已经在这台电脑上登录并使用过 Codex。
2. 运行下载的 EXE；软件不需要安装。
3. 鼠标悬停在圆形浮窗上可展开详情，浮窗可以拖动并记住位置。
4. 点击刷新按钮时，阅读联网说明并确认后即可查询当前在线额度。
5. 点击关闭按钮只会隐藏窗口；可从系统托盘重新打开或彻底退出。

程序目前没有 Windows Authenticode 数字签名，因此首次运行时 SmartScreen 可能显示“未知发布者”。请只从本仓库的 Releases 下载，并核对发布页提供的 SHA-256。

## 功能

- 浮窗主显示 5 小时额度剩余；展开后可查看 5 小时和周额度及各自重置时间
- 显示重置卡剩余次数与到期时间
- 圆形收起、悬停展开、窗口置顶和位置记忆
- 关闭窗口后隐藏到系统托盘，不占用任务栏
- 深海、极光、暖白、樱粉四套内置皮肤
- 支持导入自己的照片作为皮肤；图片只保存在本机
- 自动拒绝比当前显示更旧的本地快照，避免刷新后回退到几小时前或几天前的数据
- 对在线查询设置冷却并遵守 `Retry-After`，减少因频繁刷新触发 429 的情况
- 在线查询失败时优先保留最近一次可信结果，并回退到本地记录

## 数据来源与隐私

| 操作 | 读取的数据 | 是否联网 |
| --- | --- | --- |
| 启动、定时刷新、展开浮窗 | `%USERPROFILE%\.codex\sessions\**\rollout-*.jsonl` 中的额度事件 | 否 |
| 手动刷新在线额度 | 当前 Windows 用户的 `%USERPROFILE%\.codex\auth.json` | 是，仅访问 `chatgpt.com` 的额度接口 |
| 手动查询重置卡 | 同上 | 是，仅访问 `chatgpt.com` 的重置卡接口 |
| 导入照片皮肤 | 用户选择的本地图片 | 否 |

- 软件不会上传对话内容、会话日志或自定义照片。
- 软件不会兑换重置卡、修改账户设置或调用模型；运行浮窗本身不会消耗 Codex 模型额度。
- 访问令牌仅在用户确认的在线查询中用于请求额度接口，不会写入本项目自己的配置文件。
- 如果设置了 `CODEX_HOME`，软件会从该目录读取 `sessions` 和 `auth.json`；否则读取当前 Windows 用户的 `.codex` 目录。
- 皮肤、窗口位置和缓存属于当前电脑上的当前用户，不会打包进 EXE。

请勿把 `.codex`、`auth.json`、会话文件、带个人信息的截图或本地应用数据提交到 Issue 或源代码仓库。

## 是否绑定电脑

不绑定。本项目没有设备 ID、许可证或硬件校验。其他人可以把便携版 EXE 复制到自己的 Windows x64 电脑上直接运行，软件会读取对方电脑上当前用户自己的 Codex 数据，不会携带发布者的登录状态或额度信息。

运行条件：

- Windows 10/11 x64
- 已登录 Codex，并至少产生过一次包含额度信息的本地会话记录
- Microsoft Edge WebView2 Runtime（Windows 10/11 通常已经安装）
- 在线刷新时需要能连接 ChatGPT 服务

## 准确性边界

- 本地模式展示的是 Codex 最近写入会话日志的额度事件，而不是软件自行估算的实时用量。
- 重置卡信息通常不在本地日志中，需要用户手动确认后在线查询。
- 在线查询依赖 ChatGPT 当前的额度接口；如果接口、登录格式或服务状态发生变化，查询可能暂时失败。
- 软件不会用猜测值覆盖已有可信结果。

## 从源码构建

需要准备：

- [Rust stable](https://rustup.rs/)（MSVC toolchain）
- Visual Studio Build Tools 的“使用 C++ 的桌面开发”组件
- Microsoft Edge WebView2 Runtime
- Node.js（仅用于运行前端快照顺序测试）

```powershell
git clone https://github.com/Ruyi231/quota-float.git
cd quota-float\local-quota-widget

node --test qa\snapshot-order.test.mjs

cd src-tauri
cargo test --locked
cargo build --release --locked
```

生成的便携版程序位于：

```text
local-quota-widget\src-tauri\target\release\local-quota-widget.exe
```

## 项目结构

```text
local-quota-widget/
├─ web/                 静态前端界面与快照顺序逻辑
├─ qa/                  前端逻辑测试和窗口检查脚本
└─ src-tauri/
   ├─ src/main.rs       Tauri 窗口、拖动和系统托盘
   ├─ src/quota.rs      本地日志解析与在线额度查询
   └─ src/skin.rs       自定义照片选择、校验和本地存储
```

## 许可证与致谢

本项目采用 [MIT License](LICENSE)。

应用图标来自 [change-42-yhmm/quota-float](https://github.com/change-42-yhmm/quota-float)，原项目同样采用 MIT License。其余说明见 [THIRD_PARTY_NOTICES.md](local-quota-widget/THIRD_PARTY_NOTICES.md)。本项目的应用逻辑和界面实现独立维护。
