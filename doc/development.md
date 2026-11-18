# 开发说明

Coquou 是一个 [Tauri 2](https://tauri.app) 应用：Rust 后端 + 纯静态网页前端（没有打包器、没有 npm 依赖）。代码约定见仓库根目录的 [AGENTS.md](../AGENTS.md)。

## 环境

- Rust stable
- Node.js：只用来通过 `npx @tauri-apps/cli@2` 打包和生成图标
- Python 3：开发脚本和离线自测
- Google Chrome / Edge：生成图标时用无头模式渲染
- Windows 另需 Visual Studio Build Tools（C++ 桌面开发）和 WebView2，见 [testing-windows.md](testing-windows.md)

## 常用命令

所有命令都在仓库根目录执行。

```bash
cargo run                      # 开发运行；日志打印在终端
cargo test                     # 单元测试
cargo clippy --all-targets     # 静态检查
npx @tauri-apps/cli@2 build --debug --bundles app   # 打一个调试版应用包（macOS）
```

日志同时写到终端和日志文件（按大小轮转，保留最近几份），网页里的 `console.info/warn/error` 和未捕获的错误也会写进去：

| 系统 | 日志文件 |
|---|---|
| macOS | `~/Library/Logs/io.github.coquou/Coquou.log` |
| Windows | `%LOCALAPPDATA%\io.github.coquou\logs\Coquou.log` |

Windows 上，从打包成 MSIX 的应用里启动的 Coquou（比如在 Claude 桌面版的会话里 `cargo run` 或运行安装版）也算在那个应用的包里：写 AppData 和 HKCU 都会被重定向到 `%LOCALAPPDATA%\Packages\<包名>\LocalCache\...`。所以：

- 在那里看到的日志、`settings.json` 和宠物只是包里的副本，不是从开始菜单或资源管理器启动的 Coquou 用的那份。
- 包里的副本早就记了首次启动已完成，首次运行的流程（自动打开主窗口等）在那里测不出来。要验证首次运行或用户真实的表现，请从资源管理器安装和启动。
- 判断一个进程是不是卡死，可以对它的窗口调 Win32 `IsHungAppWindow`。卡死时页面不会再写日志，日志文件里看不出来。

开发用的启动参数：

| 参数 | 作用 |
|---|---|
| `--window` | 启动时打开主窗口 |
| `--pet <key>` | 用指定宠物启动并记住，例如 `--pet preset:tuantuan` |
| `<文件>` | 安装 `.pet` / `.zip` / 宠物目录 |
| `--hatch-selftest <地址> <images\|chat> [transparent] [persist]` | 孵化离线自测，见下 |

## 目录结构

```
Cargo.toml / Cargo.lock / tauri.conf.json   清单与 Tauri 配置
README.md / AGENTS.md / LICENSE
doc/                     全部文档（图片在 doc/images/）
src/
  core/                  Rust 后端
    main.rs              启动与组装：插件、命令注册、窗口事件、启动参数
    overlay.rs           桌宠窗口：点击穿透、原生光标推送、电源与全屏监控
    tray.rs              托盘菜单（也处理 macOS 应用菜单的动作）
    windows.rs           主窗口、macOS 应用菜单、打开文件夹和链接
    prefs.rs             宠物偏好（宠物、大小、活动范围、活跃程度、睡觉、说话、首次启动）
    settings.rs          应用设置（外观、语言等）与 settings.json 读写
    i18n.rs              界面语言：菜单和返回给界面的提示（tr! 宏）
    pets.rs              宠物发现（list_pets）与限定在宠物目录内的文件读取（read_pet_file）
    install.rs           .pet / .zip / 目录的校验、安装、导出
    library.rs           宠物库命令：安装、导入、拖放、导出、删除
    ai.rs                AI 接口配置、钥匙串、两种 OpenAI 协议
    hatch_run.rs         未完成孵化的存盘与恢复
    screens.rs           显示器选择与热插拔
    system.rs            全屏与电池检测（macOS / Windows 各一套）
    update.rs            自动更新
    migrate.rs           从旧名 Perchy / Pipkin 迁移数据
    fsutil.rs            原子写入与容错的 JSON 读取
    sync.rs              共享状态加锁
    build.rs             构建脚本；capabilities/ 是窗口权限
  ui/                    前端（纯静态文件）
    common.js            两个窗口共用：日志、界面语言（t / data-en）、平台、Codex 图集常量与读取宠物
    index.html / main.js 桌宠窗口：状态机、物理、命中检测、台词气泡、通用反应（拎起摇摆、落地压扁）
    sprite.js            Codex 图集 + coquou.json 渲染器（所有宠物都走它）
    app.html / app.css / app.js   主窗口
    hatch.js / hatch-pipeline.js  主窗口的孵化页与图像流水线
  assets/
    icons/               应用与托盘图标（source/ 是原图）
    presets/             预设宠物，随应用打包
  tools/                 开发脚本
    build-icons.py       从原图生成全部图标（icon-from-artwork.html 做图像处理）
    release.py           发版脚本，见 releasing.md
  tests/                 测试工具
    mock-image-api.py    模拟图像接口（孵化离线自测）
    analyze-atlas.py     图集质量分析
.github/workflows/       Deploy：推送版本标签后自动发版，见 releasing.md
out/                     发版产物（不进仓库）
target/ / gen/           编译输出（不进仓库）
```

## 原理

### 桌宠窗口与点击穿透

桌宠窗口透明、无边框、置顶、不可聚焦，铺满一块显示器的工作区。窗口默认忽略鼠标（点击穿透到下面的窗口）。穿透状态下网页收不到鼠标事件，所以由 Rust 线程以约 60 Hz（用电池时 30 Hz）读取系统光标，只在光标移动时推送 `cursor` 事件；网页在宠物的精灵缓冲里读该点的 alpha，不透明就关掉穿透接管鼠标，透明就打开。拖拽期间一直接管。

### 状态与偏好

- 宠物偏好（当前宠物、大小（所有宠物共用）、各宠物的自定义名字 / 台词 / 隐藏、活动范围、活跃程度、自动睡觉、说话、首次启动标记）由 `prefs.rs` 管理，和应用设置一起存在 `<应用配置目录>/settings.json`。主窗口用 `prefs_set` 修改，变化以 `prefs` 事件广播，桌宠窗口照着应用。
- 自定义（`custom`）只存在偏好里、不改宠物文件，所以只读的预设和 Codex 宠物也能改。`list_pets` 返回的 `displayName` 已经套用了自定义名字（`ownName` 是宠物自己的，有当前界面语言的 `translations` 时用译名），台词由桌宠窗口按「自定义 → coquou.json（当前语言的 `translations`，再到 `lines`）→ 通用台词」取用；隐藏的宠物只在没有别的可选时才会被自动选中。
- 「桌面上正在显示哪只宠物」是运行时状态（`report_current_pet`）：记住的宠物不存在时，桌宠会临时换一只，但不改掉用户的选择。
- 换宠物要先加载素材，所以走桌宠窗口：主窗口发 `pet-command`（`pet:<key>`），托盘菜单发 `reset`、`sleep`。
- 右键宠物弹出托盘菜单，但不含「退出」（`show_pet_menu`，只接受桌宠窗口调用），点击结果同样走 `tray::on_menu`。

### 外观与语言

- 外观（`theme`：`system` / `light` / `dark`）用 Tauri 的 `AppHandle::set_theme` 设置整个应用的外观，网页的 `prefers-color-scheme` 随之变化，所以 `app.css` 仍只按媒体查询写一套深色变量，标题栏和原生菜单也一起切换。
- 语言（`language`：`system` / `zh` / `en`）在 Rust 里解析（`i18n.rs`，跟随系统时 zh 开头的系统语言用中文，其余用英文），`get_settings` 返回解析后的 `lang`。中文是源文本，英文写在旁边：Rust 用 `tr!("中文", "English")`，脚本用 `t("中文", "English")`，`app.html` 的静态文字用 `data-en`（属性用 `data-en-placeholder` / `-title` / `-aria-label`）。
- 切换语言时 Rust 重建托盘和 macOS 应用菜单，并向两个窗口发 `settings-changed`；主窗口重新读设置、`setLang` 后重画脚本写的文字（`relabel`、`hatchRelabel`），桌宠窗口只用它来决定气泡提示和通用台词的语言。主窗口新建时会注入 `__COQUOU_LANG__`，首屏就是正确的语言。
- 新增界面文字时两种语言都要写；已经显示出来的状态提示保持原来的语言，不追改。

### 系统适配

| 能力 | macOS | Windows |
|---|---|---|
| 全屏应用时自动隐藏 | 前台普通窗口盖满所在显示器（含菜单栏）即判定全屏；不需要录屏权限 | `SHQueryUserNotificationState`（全屏、D3D 独占、演示模式） |
| 电池供电降到 30 fps | IOKit `IOPSGetProvidingPowerSourceType` | `GetSystemPowerStatus` |
| 多显示器 | 主窗口「通用 → 显示器」选择；把宠物拖到另一块屏松手即搬过去；拔掉显示器回到主屏，插回来再回去 | 同左 |
| 不抢焦点 | 窗口不可聚焦，并标记为「不激活应用」（AppKit 私有的 `_setPreventsActivation:`，公开的不激活面板样式只对 NSPanel 有效。这是维护者同意的权宜之计，等 Tauri 能把窗口建成不激活的面板就换掉，见 `overlay.rs` 的 `click_without_activating`）：点宠物不会打断正在输入的应用，也不会把被挡住的主窗口带到前面 | 窗口不可聚焦，点宠物不会打断正在输入的应用 |

## 预设宠物

团团（`tuantuan`）是用 AI 孵化（OpenRouter + gpt-image-2，参考图是早期程序绘制的团团）做出来、再加上台词的普通宠物包，放在 `src/assets/presets/tuantuan/`，随应用打包；只读，不能删除或覆盖，但和其他宠物一样能改名、改台词、隐藏。要改它就直接改目录里的文件。所有宠物（预设、安装的、Codex 的）走同一个渲染器；预设随应用打包，所以一定有宠物可显示。

## 图标

原图在 `src/assets/icons/source/`（`coquou-color.webp` 彩色、`coquou-mono.png` 单色）。换了原图后：

```bash
python3 src/tools/build-icons.py
```

会生成应用图标（彩色原图裁进 macOS 风格圆角方形）、各平台尺寸、托盘图标（macOS 用单色模板图），以及主窗口用的 `src/ui/app-icon.png`。

## AI 孵化

**接口**：用户自己填地址、key、模型，支持两种 OpenAI 协议：

| 协议 | 请求 | 取图 |
|---|---|---|
| Images | 无参考图 `POST {地址}/images/generations`（JSON）；有参考图 `POST {地址}/images/edits`（multipart，`image[]`），该路径 404/405 时（如 OpenRouter）改用 `POST {地址}/images` + `input_references` 并记住 | `data[0].b64_json` 或 `data[0].url` |
| Chat 输出图片 | `POST {地址}/chat/completions`，`modalities: ["image","text"]`，参考图作 `image_url` data URL | `choices[0].message.images[].image_url.url`；兼容服务放在 `content` 里的图片段、data URL、Markdown 链接也能取 |

接口拒绝的可选参数（例如不支持透明背景）会自动去掉重试并关闭该选项；不发送 `n`。key 存系统钥匙串（服务名 `io.github.coquou`，和应用标识相同；旧版存在 `io.github.coquou.ai` 下的 key 第一次用到时自动搬过来），读到后在内存里缓存；请求在 Rust 侧发出，key 不进网页。

**流程**（`src/ui/hatch-pipeline.js`）：

1. 形象：参考图或描述 → 一张标准形象（纯色抠像底：默认绿，角色偏绿时换品红；会排除参考图自己的背景色）。
2. 每个动作生成一张网格图（8 帧 4×2、6/5 帧 3×2、4 帧 2×2），以标准形象为参考；「向左走」由「向右走」镜像。可选 16 个看向方向：先生成上右下左 4 个锚点，再以锚点为参考生成两行。共 9–12 次调用。
3. 抠图：按与背景色的距离给出 alpha；混入背景色的半透明像素按抠像通道的超出量反混合还原颜色；轮廓附近再去一次溢色。
4. 切帧与缩放：连通域按质心分到格子 → 待机行的身高定下宠物大小，其余各行按角色的不透明面积缩放到和待机行一样大（模型在窄格子和跳跃图里会把角色画小）→ 统一地面线（生成图里每一行网格各自对齐；只有跳跃保留离地高度，其他动作每帧都站在地面线上，否则模型随手画高画低的几像素会让走路上下颠），水平方向让同一行每帧的上半身质心落在同一条竖线上，放得下时就是格子中线（按外框居中的话，走路时腿一张一合，身体会左右滑）→ 拼成 1536×2288 的 Codex v2 图集，再从原图拼一份 2× 高清图集（`coquou.json` 的 `hd`）。
5. 自动检查：缺帧、多出姿势、碰到图片边缘、越出格子、背景不纯；每行可单独重新生成。模型有时自己画一层底色、再在每个姿势背后垫一块抠像色，这时两种颜色都会抠掉；抠完帧里还留着成片抠像色（超过 5%）的，和缺帧一样算这一行失败：自动重试一次，仍失败就不能安装。
6. 可靠性：失败的行自动重试一次（4xx 不重试，429 除外）；进度存盘在 `<应用数据目录>/hatch-run/`，关掉窗口后可「继续上次」，安装成功后清除。
7. 安装：走和 `.pet` 相同的校验。标准形象、参考图和各行的原始网格图一起存进宠物的 `hatch/`，在 `coquou.json` 的 `hatch` 里登记（见 [pet-format.md](pet-format.md)「孵化素材」）。
8. 从已有宠物继续：宠物库的「在孵化页修改」载入一只 `ai-hatched` 宠物，写成和「继续上次」相同的存盘（`hatch-run/` 里多存一份图集 `atlas.png`，`meta.json` 的 `source` 记着原宠物），再走同一个恢复流程：有原图的行重新抠图切帧；没有的行用 `HATCH.atlasFrames` 从图集格子里切帧（`atlas: true`），和网格图切出的帧一起交给 `layoutAll` 统一缩放对齐。依赖图集行的动作（看向两行依赖锚点）用 `framesSheet` 把帧画回抠像底网格当参考图。没有素材的旧宠物用待机第 1 帧铺上抠像底色当标准形象，并在界面上说明。安装时可以替换原宠物（同一个 id，保留 `coquou.json` 里的台词、翻译等，用户在 Coquou 里改的名字和台词本来就存在偏好里）或另存为新宠物。

**离线自测**（不花钱，不需要 key）：

```bash
python3 src/tests/mock-image-api.py 8765
```

```bash
cargo run -- --hatch-selftest http://127.0.0.1:8765/v1 images
```

把 `images` 换成 `chat` 测另一种协议；末尾加 `persist` 额外验证存盘恢复（会清掉未完成的孵化）；模拟接口加 `--openrouter` 模拟没有 `/images/edits` 的服务。日志里出现 `selftest PASS` 即通过。自测会装上一只 `selftest-*` 宠物，测完可在主窗口删除。装好后还会测「从已有宠物继续」：载入刚装的宠物，图集要和装的完全一致（`selftest edit OK`），重新生成一行后替换原宠物；再把同一份图集按旧格式（没有孵化素材）装成 `selftest-*-old`，载入后重新生成待机和一行看向方向，另存为新宠物（`selftest edit old OK`），测完删掉这两只。

质量指标（色边比例、各行身体宽度是否一致、各行身体左右漂移多少）：

```bash
python3 src/tests/analyze-atlas.py <宠物目录>/spritesheet.png
```

## 发版与自动更新

见 [releasing.md](releasing.md)。

## 已知局限

- AI 孵化在 macOS 上用真实模型（OpenRouter + gpt-image-2）跑通过；之后改进的抠图、统一缩放和看向方向只在模拟接口上验证过。
- Windows 真机已验证窗口、穿透和系统适配；宠物包安装导出、AI 孵化和主窗口还要按 [testing-windows.md](testing-windows.md) 补测。
- 未签名的版本首次打开会被 Gatekeeper / SmartScreen 拦一下，见 README「下载安装」。
