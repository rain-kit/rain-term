![Rain Term](../public/logo.svg)

# Rain Term

基於 Tauri 的輕量級 SSH 用戶端：多分頁終端機、跳板機和 SFTP 檔案管理。

Languages: [English](../README.md) · [简体中文](./README_zh-CN.md) · 繁體中文

[![Release](https://img.shields.io/github/v/release/rain-kit/rain-term?style=flat-square)](https://github.com/rain-kit/rain-term/releases)
[![Downloads](https://img.shields.io/github/downloads/rain-kit/rain-term/total?style=flat-square)](https://github.com/rain-kit/rain-term/releases)
![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-blue?style=flat-square)

## 安裝

請到[發布頁面](https://github.com/rain-kit/rain-term/releases)下載對應平台的安裝檔。

| 平台 | 架構 | 檔案 |
| :--- | :--- | :--- |
| Windows 10 / 11 | x64 | `Rain.Term_windows_x.y.z_x64-setup.exe` |
| Windows 11 | ARM64 | `Rain.Term_windows_x.y.z_arm64-setup.exe` |
| macOS | Apple Silicon（M 系列晶片） | `Rain.Term_macos_x.y.z_aarch64.dmg` |
| macOS | Intel | `Rain.Term_macos_x.y.z_x64.dmg` |
| Debian / Ubuntu | x64 | `Rain.Term_linux_x.y.z_amd64.deb` |
| Debian / Ubuntu | ARM64 | `Rain.Term_linux_x.y.z_arm64.deb` |
| RHEL / Fedora / CentOS | x64 | `Rain.Term_linux_x.y.z_x86_64.rpm` |
| RHEL / Fedora / CentOS | ARM64 | `Rain.Term_linux_x.y.z_aarch64.rpm` |

Linux 上用 `sudo apt install ./<檔名>.deb` 或 `sudo dnf install ./<檔名>.rpm` 安裝。

在 Windows 上執行新版安裝檔時，如果已經裝過舊版本，安裝程式會建議先解除安裝目前的版本。選擇「請勿解除安裝」即可直接覆蓋安裝；在應用程式內直接更新則不會出現這一步。無論選哪個，連線、設定和已儲存的密碼都會保留。

> [!NOTE]
> 安裝檔目前還沒有程式碼簽章。
>
> - **Windows**：如果 SmartScreen 攔截了安裝程式，點選「其他資訊」，再點「仍要執行」。
> - **macOS**：如果提示「已損毀，無法打開」，在終端機執行一次：
>
>   ```shell
>   xattr -cr "/Applications/Rain Term.app"
>   ```

## 功能

- **連線管理**：分組、顏色標記、備註、搜尋、拖曳排序和複製。支援匯入、匯出連線，也可以從 FinalShell、MobaXterm 匯入。
- **終端機**
  - 可以同時開啟多個分頁，同一台伺服器的多個分頁共用一條 SSH 連線。
  - 選取即複製、按右鍵貼上、`Ctrl+Shift+C` / `V` 複製貼上、`Ctrl+Shift+F` 搜尋，連結可點選。
  - 中文、emoji 顯示寬度正確，`vim`、`top`、`htop` 顯示正常。使用 WebGL 繪製，不支援時自動改用一般繪製。
  - 中斷後顯示原因和結束代碼，可一鍵重新連線；依保活間隔偵測斷線。
  - `Ctrl+Shift+K`（macOS 為 `⌘K`）快速連線。重新開啟程式時恢復上次的分頁，不會自動連線。
- **跳板機**：支援多層跳板，每一層都確認主機指紋，連線失敗時提示是哪一台跳板機。
- **SFTP**
  - 和終端機共用一條連線，上萬個檔案的目錄也能流暢瀏覽。
  - 新增資料夾、重新命名、刪除、複製路徑。
  - 上傳、下載檔案和整個資料夾，可從系統拖入檔案上傳。傳輸工作排隊執行，顯示進度和速度，可隨時取消，取消或失敗時刪除沒傳完的檔案。目標位置已有同名檔案或資料夾時先確認是否覆蓋。
- **多種驗證方式**：密碼、私鑰（支援有密碼的私鑰）和系統 ssh-agent。
- **預設安全**
  - 首次連線時確認伺服器指紋並儲存到 `known_hosts`，指紋變更時拒絕連線。
  - 密碼儲存在系統憑證庫（Windows 認證管理員 / macOS 鑰匙圈 / Linux Secret Service），不寫入設定檔，匯出的檔案也不含密碼。
  - 連線可以標記為「正式環境」：顯示為紅色，在它的終端機裡貼上任何內容前都要確認。在任何終端機貼上多行文字前也會確認。
- **個人化**：終端機字型、字級、配色（內建 One Dark、Dracula、Nord、Solarized 等）、游標樣式和回捲行數；淺色、深色、跟隨系統三種主題；简体中文、繁體中文和 English。
- **其他**：系統匣選單列出最近的連線，內建日誌檢視器，支援應用程式內更新。

## 資料

所有資料都儲存在你自己的裝置上，資料目錄為：

- Windows：`%AppData%\rain-dial`
- macOS：`~/Library/Application Support/rain-dial`
- Linux：`~/.config/rain-dial`

| 檔案 | 內容 |
| :--- | :--- |
| `connections.json` | 連線設定（不含密碼） |
| `config.yaml` | 應用程式設定 |
| `known_hosts` | 已信任的伺服器指紋 |
| `workspace.json` | 上次開啟的分頁 |
| `logs/` | 日誌檔 |

`~/.ssh/known_hosts` 裡的指紋同樣會被信任。

設定環境變數 `RAIN_DIAL_DATA_DIR` 可以改用其他資料目錄，例如可攜使用或測試。

## 隱私

Rain Term 沒有遙測，不收集任何個人資料。它只會發起以下網路連線：

- 連線到你設定的 SSH 伺服器（以及跳板機）。
- 存取 GitHub 檢查更新：啟動後以及之後每 6 小時，應用程式會向 `api.github.com` 要求最新版本資訊，要求中只包含應用程式版本號。選擇更新時，會從 GitHub Releases 下載安裝檔。和任何網頁要求一樣，GitHub 能看到你的 IP 位址，詳見 [GitHub 隱私權聲明](https://docs.github.com/site-policy/privacy-policies/github-general-privacy-statement)。

## 開發

環境需求：

- [Node.js](https://nodejs.org/) 22.12+
- [Rust](https://www.rust-lang.org/tools/install) 1.88+（stable）
- 對應平台的 [Tauri 相依套件](https://tauri.app/start/prerequisites/)

```shell
# 安裝相依套件
npm install

# 以開發模式啟動
npm run tauri dev

# 建置目前平台的安裝檔
npm run tauri build

# 前端型別檢查
npx tsc --noEmit

# 執行後端測試
cd src-tauri
cargo test
```

`cargo test` 還會在 Docker 裡的真實 OpenSSH 伺服器（`src-tauri/tests/docker`）上執行整合測試，需要 Docker Compose v2。本機沒有 Docker 或設定了 `RAIN_DIAL_SKIP_DOCKER` 時略過。

想在沒有真實伺服器的情況下試用介面，可以在 `src-tauri` 下執行 `cargo run --example dev_sshd`，它會在 2222 連接埠啟動一個本機測試 SSH 伺服器，使用者 `tester`，密碼 `pa55`。

### 發布

推送以 `v` 開頭的標籤會觸發 [GitHub Actions](../.github/workflows/release.yml)，建置 Windows、macOS 和 Linux（各含 x64 和 ARM64）的安裝檔並發布到 Releases。推送到 `main` 只編譯，用來檢查建置並為下次發布預熱快取。

安裝檔會同時產生簽章的應用程式內更新套件和 `latest.json`，所以建置時需要更新簽章私鑰。在儲存庫的 Secrets 裡設定 `TAURI_SIGNING_PRIVATE_KEY`（私鑰有密碼時再設定 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`）；本機執行 `npm run tauri build` 時設定同名環境變數。對應的公鑰在 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`。

1. 修改 `package.json`、`src-tauri/tauri.conf.json` 和 `src-tauri/Cargo.toml` 中的版本號。
2. 在 [ChangeLog.md](../ChangeLog.md) 中新增 `## vX.Y.Z` 一節。它會成為 Release 說明和應用程式內更新頁面的「更新內容」；缺少這一節時發布會失敗。
3. 提交後建立並推送對應的標籤：

   ```shell
   git tag v1.0.0
   git push origin main --tags
   ```

標籤與 `tauri.conf.json` 中的版本不一致時，工作流程會失敗。

## 參與貢獻

歡迎提交 Issue 和 Pull Request！

## 致謝

Rain Term 基於以下專案建置：

- [tauri-apps/tauri](https://github.com/tauri-apps/tauri)：用 Web 前端建置更小、更快、更安全的桌面應用程式。
- [Eugeny/russh](https://github.com/Eugeny/russh)：Rust SSH 用戶端和伺服器端函式庫。
- [xtermjs/xterm.js](https://github.com/xtermjs/xterm.js)：Web 終端機元件。
- [tokio-rs/tokio](https://github.com/tokio-rs/tokio)：Rust 非同步執行環境。
- [facebook/react](https://github.com/facebook/react)：用於建置 Web 和原生介面的函式庫。
- [vitejs/vite](https://github.com/vitejs/vite)：下一代前端建置工具。
- [tailwindlabs/tailwindcss](https://github.com/tailwindlabs/tailwindcss)：實用優先的 CSS 框架。
- [lucide-icons/lucide](https://github.com/lucide-icons/lucide)：美觀、一致的圖示庫。

## 授權

[MIT](../LICENSE)
