# yudao-scaffold-ui

Tauri 2 + Vue 3 + Element Plus 桌面应用，内置脚手架引擎，提供页面式交互体验。用户下载安装包后可直接启动，不需要额外配置引擎目录、Node 或 Git。

## 一句话架构

```
┌─────────────────────────────────────┐
│  Vue 3 + Element Plus 前端          │  ← src/
│  - 步骤式表单、模块多选卡片         │
│  - 原生目录选择器                    │
│  - 实时执行日志 + 进度条             │
└──────────────┬──────────────────────┘
               │ invoke / events
┌──────────────┴──────────────────────┐
│  Tauri Rust 内置引擎                │  ← src-tauri/src/
│  - load_meta:   返回内置模块/模板元数据 │
│  - run_scaffold:下载/缓存/展开模板   │
│    通过 emit 转发进度到前端          │
└──────────────┬──────────────────────┘
               │ HTTPS zip 下载 + 本地缓存
┌──────────────┴──────────────────────┐
│  官方公开模板仓库                    │
│  - ruoyi-vue-pro / yudao-cloud       │
│  - yudao-ui-admin-* / mall / go-view │
│  - 缓存到 ~/.yudao-scaffold-ui       │
└─────────────────────────────────────┘
```

## 开发

开发需要：Node 18+、pnpm、Rust 1.77+、对应平台的系统 webview（macOS/Windows 自带，Linux 需要 webkit2gtk）。这些只用于开发和打包；安装包用户不需要安装 Node、pnpm、Rust 或 Git。

```bash
cd yudao-scaffold-ui
pnpm install                          # 装 Vue/Element Plus/Tauri JS deps
pnpm tauri:dev                        # 启动 Vite + 编译 Rust + 弹出窗口
```

第一次跑要 5–10 分钟（Rust 全量编译 + 下载 crates）。后续增量编译只要几秒。

## 打包发布

```bash
pnpm tauri:build
```

产物位于 `src-tauri/target/release/bundle/`：
- macOS: `dmg/yudao-scaffold_0.1.0_aarch64.dmg`（~10 MB）
- Windows: `msi/yudao-scaffold_0.1.0_x64_en-US.msi`
- Linux: `deb/yudao-scaffold_0.1.0_amd64.deb` + `appimage/...`

跨平台打包必须在对应平台系统上执行（macOS dmg 必须 macOS 出包，Windows msi 必须 Windows 出包）。

## 关键文件

| 文件 | 作用 |
|---|---|
| `src/App.vue` | 全部 UI：4 步 stepper、模块卡片、前端复选、镜像配置、执行进度 |
| `src/api.ts` | Tauri invoke 封装 (`loadMeta` / `pickDirectory` / `startScaffold`) |
| `src/types.ts` | TS 类型定义，与 Rust 内置引擎保持一致 |
| `src-tauri/src/lib.rs` | Tauri 启动、命令注册和平台集成 |
| `src-tauri/src/scaffold.rs` | 内置脚手架引擎、模板下载缓存、解压复制和事件转发 |
| `src-tauri/tauri.conf.json` | 窗口尺寸、bundle id、icon 配置 |
| `src-tauri/capabilities/default.json` | dialog/shell/event 权限白名单 |

## 运行时数据流

1. **窗口启动 → onMounted**：调用 `load_meta` Tauri 命令
2. Rust 内置引擎直接返回模块、前端、模板源和缓存状态
3. Vue 用元数据填充模块卡片、前端列表、各模板的"本地/缓存/可下载"状态
4. **用户在 4 个 step 里填表 → 点"开始生成"**
5. Vue 调 `run_scaffold(payload)`，并预先 `listen('scaffold-event', ...)` 注册回调
6. Rust 内置引擎按需下载模板 zip、解压到本地缓存，并复制到输出目录
7. Rust 端通过 `app.emit('scaffold-event', ...)` 实时转发阶段、日志和完成事件
8. Vue 收到事件 → 进度条、日志面板实时更新

## 浏览器模式（无 Tauri）

如果暂时不想装 Rust，也可以纯前端跑：

```bash
pnpm dev   # 仅启动 Vite，浏览器打开 http://localhost:1420
```

但浏览器里 `@tauri-apps/api/core` 的 `invoke` 会失败（没有 Tauri runtime），所以这只能看 UI 静态布局，不能真正跑流程。生产用法是 Tauri 窗口。
