# unasfan_fpk

这是从 U-NAS Kepler 安装镜像中提取并重写控制核心的风扇控制包。适用于已确认以下硬件条件的设备：

**机型限制：仅支持万由 HS201P、HS401P。其他机型不保证兼容，请勿安装。**

```text
Super I/O: 端口 `0xa8f0`
fan registers: `0x6b`, `0x73`, `0xa3`
CPU temperature: /sys/class/thermal/thermal_zone1/temp
```

## 构建

```bash
make
```

构建前会使用 Rust musl target 静态编译 Super I/O 控制工具和守护进程并检查 JSON 配置，需要 Rust、musl 工具链和当前工作区中的 `fnpack`。当前版本生成的文件位于 `dist/unasfan_fpk-1.5.0-x86.fpk`。

GitHub Actions 位于 `.github/workflows/release.yml`。手动触发或推送 `v*` 标签时，流水线使用 UTC 当前时间生成 `YYYYMMDDHHMMSS` 版本号，构建 FPK 并发布对应 GitHub Release。

## 依赖

应用固定使用 Rust 工具通过 `iopl(3)` 直接写入 Super I/O，同时控制 3 个风扇通道（寄存器 `0x6b/0x73/0xa3`）。只需要应用以 root 运行，不需要安装 `i2c-tools`，也不需要加载 `i2c-dev`。

## 默认曲线

守护进程按反编译的 `WY2Hardware` 和 `wygpio::MyGpio::aiFanSpeed()` 运行。CPU 直接读取 `coretemp` hwmon（等价于官方 `sensors -j` 中 `coretemp-isa-0000` 的最大 `_input` 值）；硬盘直接读取 `drivetemp`/`nvme`/ATA/SCSI hwmon 和磁盘 thermal zone，最后取成功读取值的最大值。整个应用不依赖 `fdisk`、`smartctl`、`sensors` 或 `i2c-tools`。需要注意：如果内核没有加载硬盘温度驱动、sysfs 中没有暴露温度，任何不依赖外部工具的实现都无法读取该硬盘温度。调速状态边界为 30/45/55/60°C，保留最近 6 次样本，并使用官方的上一温度、平均温度、当前 PWM、状态计数器和状态相关温差系数（状态 2 为 `5/3`，状态 3 为 `5/5`，状态 4 为 `10/5`，分别对应突升/平均升温路径）。状态输出范围也按官方跳转表执行：状态 2 为 `50..125`（不超过 40°C 固定 50），状态 3 为 `80..180`（不超过 50°C 固定 80），状态 4 为 `150..255`；最终硬件写入仍按官方 `50..250` 限制。低温状态固定 PWM 50，高温状态请求 PWM 255。也可在安装配置中填写 `disk_temp_path` 覆盖硬盘温度。温度读取失败或停止服务时会切换到 PWM 250。

风扇控制器的有效值按原 U-NAS 守护进程限制为 `50..250`；100% 会换算为 254 后再被限制为 250，不能把 255 当作更高转速。温度读取失败或停止服务时使用 250 作为故障保护值。

## `wygpio` 迁移结论

镜像中的 `wygpio` 是依赖 Qt5、`libi2c`、`libapt-pkg` 和 U-NAS RPC 的完整守护程序，不能直接复制到飞牛系统：当前飞牛环境缺少这些 ABI，且 `/unas` 路径和 RPC 服务不存在。反编译确认其温度语义来自 `sensors -j`、`fdisk -l`、`smartctl --attribute`；本包使用内核 hwmon/sysfs 直接实现相同的温度选择语义，硬件路径包含 I2C `0x54/0xF0`，自动调速核心每 10 秒读取 CPU 与磁盘最高温度、维护历史窗口并按状态调速。

本包移植了原程序中未被命令行调用的 Super I/O 路径，并以 Rust 重写为 `superio-fanctl`；同时将反编译得到的 `aiFanSpeed()` 状态机重写为 `fan-daemon`。`wygpio` 的 Qt/RPC 外壳暂不移植，Super I/O 只替代官方 I2C 写入后端，自动调速逻辑使用官方实现。
