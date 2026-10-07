# inphic — Inphic KP 8K 鼠标自制驱动

逆向官方 HUB 得到的配置协议，用 Python 直接构造设备命令：**按前台程序自动切换鼠标方案**（灯光、DPI、回报率、按键、宏……），
不依赖官方软件，不装任何键盘/鼠标钩子。仅 Windows（需要 HID 访问）。

## 快速开始

```powershell
# Rust 版在 rust/ 目录(Python 原版在 python/ 目录, 两者共用根目录的 config.yaml / macros / fixtures)
cd rust && cargo build --release          # 编译(Windows; 产物 rust/target/release/inphic.exe)
cargo run --release -- -c ../config.yaml run               # 常驻运行：进入 crossfire.exe 用 cf 方案，其他程序用 default
cargo run --release -- -c ../config.yaml show              # 用中文打印配置里每个方案的设置（只读，不碰鼠标）
cargo run --release -- -c ../config.yaml status            # 读一次电量（只读；蓝牙连着时自动走蓝牙）
cargo run --release -- -c ../config.yaml macros list       # 看宏库里有哪些宏（只读）
```

配置写在 [`config.yaml`](config.yaml)，写法和每种选项见 **[docs/配置说明.md](docs/配置说明.md)**（每种用法一个可复制的例子）。
写错的配置会明确报错，不会被悄悄忽略。

| 命令 | 作用 |
|---|---|
| `inphic run [-c 配置]` | 启动先用中文打印当前设置，然后监听前台程序、切换方案；写失败每 3 秒重试，鼠标拔插后自动重新应用；电量变化实时写进日志（≤20%、≤10% 各警告一次）；按鼠标上的 DPI 键会记一行「DPI 档位切换到第 N 档 = X DPI」 |
| `inphic show [-c 配置] [--payloads]` | 只打印上面那份中文设置摘要，不连鼠标（★ = 与出厂默认不同的按钮）；`--payloads` 另外打印将要发送的字节（干跑） |
| `inphic status [--watch] [--via bluetooth]` | 读鼠标主动上报的状态（电量等）；`--watch` 持续打印，可观察 DPI 键；蓝牙时不能和 HUB / `run` 同时用 |
| `inphic macros list \| show <分组.名字>` | 查看宏库，展开后的步骤、占几个槽位、播放多久 |

## 代码结构

依赖只能从下往上：下层不知道上层的存在。

```
inphic/
  protocol/   线上协议（纯函数，不碰设备、不读配置）
    frame        帧格式与校验：[04, len+5, 分片序号, 数据, sum16]
    keycodes     HID 键码表
    functions    按钮功能码表、动作名、修饰键位
    macro        宏编码：事件 → 111 字节消息 → 分片帧
    status       设备主动上报的状态报告（电量等）
    bluetooth    蓝牙常量与纯函数（UUID、地址解析、通知 → 状态报告）
  profile/    方案
    model        方案的内存结构（HUB 存档里一个方案）、字段读写、按钮绑定
    commands     方案 → 命令 payload → 64 字节帧（0x0c/04/05/06/07/08 + 宏）
    build        从 yaml 配置构建方案并校验
    defaults     工厂默认方案
  config/     配置
    loader       读取 config.yaml
    macro_library 宏库：分组、use/times 复用、锚点、多文件
  runtime/    与设备和系统打交道
    channel      2.4G 接收器：HID 通道，找集合、写帧、检测在线
    ble          蓝牙：WinRT 访问 GATT 服务 FEE0（FEE3 写、FEE4 通知）
    monitor      后台读状态、电量日志
    foreground   前台程序检测
    switcher     切换循环：重试、断线重连
  cli/        命令行入口（run / show / status / macros）
config.yaml   你的配置        macros/   宏库示例        docs/   使用说明
tests/        单元测试，按包分目录；fixtures/ 是真实 USB 抓包提取的帧
```

## 测试

```powershell
cargo test                                # 在 rust/ 下; Python 版在 python/ 下 uv run python -m unittest discover -s tests -t .
```

测试不需要连接鼠标（硬件相关的部分用假的 `hid` 模块）。帧级测试用真实抓包做**逐字节**对照。

## 验证状态

灯光、`1/6/5` 键位、default 方案、切方案不再卡键盘：已在真机验证。宏的键盘按键、鼠标左右键、四种启动方式：有抓包验证。
**蓝牙**：读电量（标准电池服务和 FEE4 通知）已在真机验证；**往 FEE3 写配置还没在真机上试过**（载荷格式来自 HUB 反编译，见 wiki），请先 `show --payloads` 看一眼再让 `run` 去写。
回报率非 1000、自定义 DPI、性能页各项、组合键、鼠标移动宏：只有代码依据和单元测试，建议每次只改一项观察。
逆向笔记（协议字段、验证情况、待办）在项目的 Gitea wiki 里。
