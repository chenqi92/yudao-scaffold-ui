# yudao-scaffold-ui

Tauri 2 + Vue 3 + Element Plus 桌面壳，包裹 [yudao-scaffold](../yudao-scaffold) 引擎，提供页面式交互体验。

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
│  Tauri Rust 命令 (~150 行)          │  ← src-tauri/src/lib.rs
│  - load_meta:   spawn bin/meta.ts   │
│  - run_scaffold:spawn bin/run.ts    │
│    解析 \x01-prefixed NDJSON 事件   │
│    通过 emit 转发到前端             │
└──────────────┬──────────────────────┘
               │ JSON_EVENTS=1, stdin/stdout
┌──────────────┴──────────────────────┐
│  yudao-scaffold (Node + tsx 引擎)   │
│  - bin/meta.ts:    元数据 dump       │
│  - bin/run.ts:     headless 执行     │
│  - 与 CLI 共用 1700 行生成器代码    │
└─────────────────────────────────────┘
```

## 开发

需要：Node 18+、pnpm、Rust 1.77+、对应平台的系统 webview（macOS/Windows 自带，Linux 需要 webkit2gtk）。

```bash
cd yudao-scaffold-ui
pnpm install                          # 装 Vue/Element Plus/Tauri JS deps
pnpm tauri:dev                        # 启动 Vite + 编译 Rust + 弹出窗口
```

第一次跑要 5–10 分钟（Rust 全量编译 + 下载 ~250 个 crate）。后续增量编译只要几秒。

引擎位置在 dev 模式下从 `CARGO_MANIFEST_DIR/../../yudao-scaffold` 推断，可通过 `YUDAO_SCAFFOLD_ENGINE=/abs/path` 覆盖。

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
| `src/types.ts` | TS 类型定义，与 yudao-scaffold 引擎保持一致 |
| `src-tauri/src/lib.rs` | Rust 命令、Node 子进程编排、NDJSON → Tauri event 转发 |
| `src-tauri/tauri.conf.json` | 窗口尺寸、bundle id、icon 配置 |
| `src-tauri/capabilities/default.json` | dialog/shell/event 权限白名单 |

## 运行时数据流

1. **窗口启动 → onMounted**：调用 `load_meta` Tauri 命令
2. Rust 端 `spawn node --import tsx bin/meta.ts`，捕获 stdout JSON
3. Vue 用元数据填充模块卡片、前端列表、各模板的"本地/缓存/需 clone"状态
4. **用户在 4 个 step 里填表 → 点"开始生成"**
5. Vue 调 `run_scaffold(payload)`，并预先 `listen('scaffold-event', ...)` 注册回调
6. Rust 端 `spawn node ... bin/run.ts`，把 payload 写到 stdin，**`JSON_EVENTS=1`** 让引擎在每行 stdout 前加 `\x01` 控制字符再 emit
7. Rust 异步逐行读 stdout：以 `\x01` 开头的剥前缀解析为 JSON event → `app.emit('scaffold-event', ...)` 转发到前端
8. Vue 收到事件 → 进度条、日志面板实时更新

## 浏览器模式（无 Tauri）

如果暂时不想装 Rust，也可以纯前端跑：

```bash
pnpm dev   # 仅启动 Vite，浏览器打开 http://localhost:1420
```

但浏览器里 `@tauri-apps/api/core` 的 `invoke` 会失败（没有 Tauri runtime），所以这只能看 UI 静态布局，不能真正跑流程。生产用法是 Tauri 窗口。
