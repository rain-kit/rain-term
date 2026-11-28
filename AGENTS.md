# AGENTS.md

Rain Term：基于 Tauri 2 的 SSH / 远程桌面客户端（多标签终端、RDP 远程桌面、跳板机、SFTP）。显示名是 Rain Term，仓库是 `rain-kit/rain-term`；包名、identifier 和数据目录仍为 `rain-dial`，改动后会让已有的连接和密码读不到。后端 Rust 在 `src-tauri`，前端 React + TypeScript 在 `src`。

## 分层

- `commands/`：前端 `invoke` 调用的接口，只做参数转换，返回数据和国际化 key（`Msg`、`ApiError`），不返回文案。
- `service/`：业务逻辑和状态。
- `core/`：协议实现，不依赖 Tauri，方便单独测试。
- 普通请求用 `invoke`；终端输出、传输进度这类连续数据用 `tauri::ipc::Channel`；列表和状态变化用 `emit` 事件通知前端刷新。

## 国际化（必须）

界面支持简体中文、繁體中文、English 三种语言。

- 界面文案改动要同时更新 `src/locales` 下的 `zh-CN.ts`、`zh-TW.ts`、`en.ts`。
- 后端新增的错误和提示用 `error.rs` 的错误码或 `msg!` 宏定义国际化 key，并在三个语言文件里补上翻译。
- 托盘菜单、文件对话框这类由后端直接显示的原生界面，用 `i18n::pick` 或 `tr!` 给出三种语言。
- 日期、文件大小、耗时等格式化放在 `src/lib/format.ts`，文案里的单位也走翻译。

## 更新日志（必须）

每次改动用户能感知到的功能、修复或行为，都要在 [ChangeLog.md](./ChangeLog.md) 记一条，和代码改动一起提交。

- 写进最上面那一节 `## vX.Y.Z`。如果这个版本已经发布（`git tag` 里有 `vX.Y.Z`），就在最上面新建下一个版本的一节，并同步修改版本号（见下文“发布版本”）。
- 每个版本号只能有一节。再次修改尚未发布的版本时，改原来那一节，把新变化合并进去，不要再写一节相同的 `## vX.Y.Z`。
- 标题只能是 `## vX.Y.Z`，不要用 `Unreleased` 之类的写法：发布脚本按版本号取内容，找不到当前版本的一节时发布会失败。
- 这一节会原样显示在 Release 说明和应用内的更新页面里，写给用户看：
  - 中文，每条以 `- ` 开头，一句话说清楚变化；
  - 写用户能看到的效果，不写内部实现、文件名和重构细节；
  - 同一个功能在一个版本内多次修改时，更新原来那条，不要重复追加。
- 纯重构、测试、CI、文档等用户无感知的改动不用写。

## 发布版本

1. 三处版本号保持一致：`package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`（`Cargo.lock` 随编译更新）。
2. `ChangeLog.md` 中有对应的 `## vX.Y.Z` 一节。
3. 推送 `vX.Y.Z` 标签，标签与 `tauri.conf.json` 的版本不一致时发布会失败。

## 依赖管理（必须）

每次增加、删除或升级 npm 依赖，都要同步更新 `package-lock.json`，并和 `package.json` 一起提交。CI 用 `npm ci` 安装依赖，两者不一致时所有平台都会在安装依赖这一步失败。

- 用和 CI 相同版本的 npm 更新锁文件：`npx -y npm@<版本> install --package-lock-only`（目前是 `11.20.0`）。
- 提交前验证锁文件：`npx -y npm@<版本> ci --dry-run --ignore-scripts`，没有报错才算通过。
- 不要手动编辑 `package-lock.json`。

## 改动后的检查

- 前端：`npx tsc --noEmit`
- 后端：在 `src-tauri` 下运行 `cargo test`，编译不能有警告
