# 参与开发

欢迎反馈兼容性问题、修复 bug 或完善使用说明。提交较大的功能前，请在迟迟维护的仓库开 Issue，说明使用场景与预期行为。[项目 Issues](https://github.com/dh666i/daozai/issues)

## 本地开发

安装 Rust stable、Visual Studio C++ 构建工具和 Windows SDK，克隆仓库后运行：

```powershell
cargo build --locked --bin daozai
.\scripts\prepare-recorder.ps1 -OutputDirectory .\target\debug\recorder
cargo test --locked --all-targets --features record -- --test-threads=1
```

构建正式安装包和便携包需安装 NSIS 3，然后在项目根目录运行 `.\scripts\package.ps1`。脚本从 Cargo 清单读取版本号，在 `dist` 生成安装包、ZIP 和 `SHA256SUMS.txt`。NSIS 安装在自定义位置时，可通过 `-NsisCompiler` 传入 `makensis.exe` 的完整路径。

为开发实例指定独立目录，避免覆盖日常使用数据：

```powershell
$env:DAOZAI_DATA_DIR = Join-Path $env:TEMP 'daozai-development'
cargo run --locked --bin daozai
```

常规测试使用合成数据。带 `#[ignore]` 的真实账号测试或截图导出测试须按需要显式运行；不要在公共 CI 中连接私人账号。

录屏助手的构建脚本调用 Windows 自带的 .NET Framework x64 C# 编译器，使用 Visual Studio Build Tools 的 x64 C++ 可再发行运行库，从 NuGet 下载并校验固定版本的 ScreenRecorderLib。运行 `scripts/package.ps1` 打包时会自动执行这一步。便携包应完整解压，保留与 `daozai.exe` 同级的 `recorder` 目录。

## 代码导航

| 目录／文件 | 内容 |
| --- | --- |
| `src/main.rs` | Windows 外壳、系统状态和后台服务衔接 |
| `src/app.rs`、`src/pages.rs` | 状态与主界面 |
| `src/calendar.rs`、`src/calendar_ui.rs` | CalDAV、重复日程、月历与列表 |
| `src/mail.rs`、`src/mail_reader.rs` | IMAP、MIME、验证码与邮件阅读 |
| `src/quota*.rs` 及 provider 模块 | AI 额度与连接 |
| `src/native.rs`、`src/system.rs` | 原生窗口、输入、拖放和系统事件 |
| `src/ui_tests.rs` | 无窗口交互与布局回归测试 |
| `src/bin/daozai-record` | 合成数据演示与软件渲染器 |
| `src/screen_tools.rs`、`helpers/recorder/` | 屏幕录制界面与 Windows MP4 录屏助手 |
| `src/screen_snip.rs`、`scripts/screen-snip.ps1` | 框选、文字识别与二维码识别 |
| `src/clipboard.rs`、`src/clipboard/` | 图文剪贴板历史与图片格式处理 |
| `vendor/` | 少量必要的上游补丁，见依赖说明 |
| `installer/`、`scripts/` | 安装、卸载与打包 |

## 提交要求

修 bug 时提供能复现的测试；界面变更附不含私人信息的截图。保持改动聚焦，不把个人配置、授权码、Cookie、编译产物或真实账号数据提交到仓库。

源码贡献按仓库 MIT 许可提交；第三方素材须同时提交准确的来源与许可声明。不要修改或移除 vendored 依赖的原始版权声明。
