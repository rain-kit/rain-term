![Rain Term](../public/logo.svg)

# Rain Term

基于 Tauri 的轻量级 SSH / 远程桌面客户端：多标签终端、Windows 远程桌面（RDP）、跳板机和 SFTP 文件管理。

Languages: [English](../README.md) · 简体中文 · [繁體中文](./README_zh-TW.md)

[![Release](https://img.shields.io/github/v/release/rain-kit/rain-term?style=flat-square)](https://github.com/rain-kit/rain-term/releases)
[![Downloads](https://img.shields.io/github/downloads/rain-kit/rain-term/total?style=flat-square)](https://github.com/rain-kit/rain-term/releases)
![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-blue?style=flat-square)

## 安装

请到[发布页面](https://github.com/rain-kit/rain-term/releases)下载对应平台的安装包。

| 平台 | 架构 | 文件 |
| :--- | :--- | :--- |
| Windows 10 / 11 | x64 | `Rain.Term_windows_x.y.z_x64-setup.exe` |
| Windows 11 | ARM64 | `Rain.Term_windows_x.y.z_arm64-setup.exe` |
| macOS | Apple Silicon（M 系列芯片） | `Rain.Term_macos_x.y.z_aarch64.dmg` |
| macOS | Intel | `Rain.Term_macos_x.y.z_x64.dmg` |
| Debian / Ubuntu | x64 | `Rain.Term_linux_x.y.z_amd64.deb` |
| Debian / Ubuntu | ARM64 | `Rain.Term_linux_x.y.z_arm64.deb` |
| RHEL / Fedora / CentOS | x64 | `Rain.Term_linux_x.y.z_x86_64.rpm` |
| RHEL / Fedora / CentOS | ARM64 | `Rain.Term_linux_x.y.z_aarch64.rpm` |

Linux 上用 `sudo apt install ./<文件名>.deb` 或 `sudo dnf install ./<文件名>.rpm` 安装。

在 Windows 上运行新版安装包时，如果已经装过旧版本，安装程序会推荐先卸载当前版本。选择“请勿卸载”即可直接覆盖安装；在应用内直接更新则不会出现这一步。无论选哪个，连接、设置和已保存的密码都会保留。

> [!NOTE]
> 安装包目前还没有代码签名。
>
> - **Windows**：如果 SmartScreen 拦截了安装程序，点击“更多信息”，再点“仍要运行”。
> - **macOS**：如果提示“已损坏，无法打开”，在终端执行一次：
>
>   ```shell
>   xattr -cr "/Applications/Rain Term.app"
>   ```

## 功能

- **连接管理**：分组、颜色标记、备注、搜索、拖动排序和复制。支持导入、导出连接，也可以从 FinalShell、MobaXterm 导入。
- **终端**
  - 可以同时开多个标签，同一台服务器的多个标签共用一条 SSH 连接。
  - 选中即复制、右键粘贴、`Ctrl+Shift+C` / `V` 复制粘贴、`Ctrl+Shift+F` 搜索，链接可点击。
  - 中文、emoji 显示宽度正确，`vim`、`top`、`htop` 显示正常。使用 WebGL 渲染，不支持时自动改用普通渲染。
  - 断开后显示原因和退出码，可一键重新连接；按保活间隔检测断线。
  - `Ctrl+Shift+K`（macOS 为 `⌘K`）快速连接。重新打开程序时恢复上次的标签，不会自动连接。
- **远程桌面（RDP）**：在标签页中打开 Windows 远程桌面，支持网络级身份验证；分辨率跟随标签大小，文字可以双向复制粘贴，工具栏可发送 `Ctrl+Alt+Del` 和 Win 键；首次连接时确认服务器证书。
- **跳板机**：终端和远程桌面都支持多级跳板，每一级都确认主机指纹，连接失败时提示是哪一台跳板机。
- **SFTP**
  - 和终端共用一条连接，上万个文件的目录也能流畅浏览。
  - 新建文件夹、重命名、删除、复制路径。
  - 上传、下载文件和整个文件夹，可从系统拖入文件上传。传输任务排队执行，显示进度和速度，可随时取消，取消或失败时删除没传完的文件。目标位置已有同名文件或文件夹时先确认是否覆盖。
- **多种认证方式**：密码、私钥（支持带密码的私钥）和系统 ssh-agent。
- **默认安全**
  - 首次连接时确认服务器指纹并保存，指纹变化时拒绝连接；`~/.ssh/known_hosts` 里的指纹同样会被信任。
  - 密码加密保存在本机，不和连接配置放在一起，导出的文件也不含密码。
  - 连接可以标记为“生产环境”：显示为红色，往它的终端粘贴任何内容前都要确认。往任何终端粘贴多行文本前也会确认。
- **个性化**：终端字体、字号、配色方案（内置 One Dark、Dracula、Nord、Solarized 等）、光标样式和回滚行数；浅色、深色、跟随系统三种主题；简体中文、繁體中文和 English。
- **其他**：托盘菜单列出最近的连接，支持应用内更新。

## 隐私

Rain Term 没有遥测，不收集任何个人数据，所有数据都保存在你自己的设备上。它只会发起以下网络连接：

- 连接你配置的 SSH 服务器（以及跳板机）。
- 访问 GitHub 检查更新：启动后以及之后每 6 小时，应用会向 `api.github.com` 请求最新版本信息，请求中只包含应用版本号。选择更新时，会从 GitHub Releases 下载安装包。和任何网页请求一样，GitHub 能看到你的 IP 地址，详见 [GitHub 隐私声明](https://docs.github.com/site-policy/privacy-policies/github-general-privacy-statement)。

## 开发

环境要求：

- [Node.js](https://nodejs.org/) 22.12+
- [Rust](https://www.rust-lang.org/tools/install) 1.88+（stable）
- 对应平台的 [Tauri 依赖](https://tauri.app/start/prerequisites/)

```shell
# 安装依赖
npm install

# 以开发模式启动
npm run tauri dev

# 构建当前平台的安装包
npm run tauri build

# 前端类型检查
npx tsc --noEmit

# 运行后端测试
cd src-tauri
cargo test
```

`cargo test` 还会在 Docker 里的真实 OpenSSH 服务器（`src-tauri/tests/docker`）上跑集成测试，需要 Docker Compose v2。本机没有 Docker 或设置了 `RAIN_DIAL_SKIP_DOCKER` 时跳过。

想在没有真实服务器的情况下试用界面，可以在 `src-tauri` 下运行 `cargo run --example dev_sshd`，它会在 2222 端口启动一个本地测试 SSH 服务器，用户 `tester`，密码 `pa55`。

### 发布

推送以 `v` 开头的标签会触发 [GitHub Actions](../.github/workflows/release.yml)，构建 Windows、macOS 和 Linux（各含 x64 和 ARM64）的安装包并发布到 Releases。推送到 `main` 只编译，用来检查构建并为下次发布预热缓存。

安装包会同时生成签名的应用内更新包和 `latest.json`，所以构建时需要更新签名私钥。在仓库的 Secrets 里设置 `TAURI_SIGNING_PRIVATE_KEY`（私钥有密码时再设置 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`）；本地运行 `npm run tauri build` 时设置同名环境变量。对应的公钥在 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`。

1. 修改 `package.json`、`src-tauri/tauri.conf.json` 和 `src-tauri/Cargo.toml` 中的版本号。
2. 在 [ChangeLog.md](../ChangeLog.md) 中添加 `## vX.Y.Z` 一节。它会成为 Release 说明和应用内更新页面的“更新内容”；缺少这一节时发布会失败。
3. 提交后创建并推送对应的标签：

   ```shell
   git tag v1.0.0
   git push origin main --tags
   ```

标签与 `tauri.conf.json` 中的版本不一致时，工作流会失败。

## 参与贡献

欢迎提交 Issue 和 Pull Request！

## 致谢

Rain Term 基于以下项目构建：

- [tauri-apps/tauri](https://github.com/tauri-apps/tauri)：用 Web 前端构建更小、更快、更安全的桌面应用。
- [Eugeny/russh](https://github.com/Eugeny/russh)：Rust SSH 客户端和服务端库。
- [xtermjs/xterm.js](https://github.com/xtermjs/xterm.js)：Web 终端组件。
- [tokio-rs/tokio](https://github.com/tokio-rs/tokio)：Rust 异步运行时。
- [facebook/react](https://github.com/facebook/react)：用于构建 Web 和原生界面的库。
- [vitejs/vite](https://github.com/vitejs/vite)：下一代前端构建工具。
- [tailwindlabs/tailwindcss](https://github.com/tailwindlabs/tailwindcss)：实用优先的 CSS 框架。
- [lucide-icons/lucide](https://github.com/lucide-icons/lucide)：美观、统一的图标库。

## 许可证

[MIT](../LICENSE)
