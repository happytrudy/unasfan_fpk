# unasfan_fpk

这是从 U-NAS Kepler 安装镜像中提取并重写控制核心的风扇控制包。适用于已确认以下硬件条件的设备：

**机型限制：仅支持万由 HS201P、HS401P。其他机型不保证兼容，请勿安装。**

```text
I2C adapter: 自动匹配名称以 `SMBus I801 adapter at ` 开头的适配器
I2C address: `0x54`
fan register: `0xf0`
fan output: I2C `0x54/0xf0` (rewritten every poll)
CPU temperature: /sys/class/thermal/thermal_zone1/temp（coretemp 优先）
```

## 构建

```bash
make
```

构建前会使用 Rust musl target 静态编译 I2C 控制工具和守护进程并检查 JSON 配置，需要 Rust、musl 工具链和当前工作区中的 `fnpack`。本地构建生成的文件位于 `dist/`。

GitHub Actions 位于 `.github/workflows/release.yml`。手动触发或推送 `v*` 标签时，流水线使用 UTC 当前时间生成 `YYYYMMDDHHMMSS` 版本号，构建 FPK 并发布对应 GitHub Release。

## 依赖

应用使用 Rust 工具直接调用 Linux `i2c-dev` ioctl，不依赖 `i2c-tools`、`sensors`、`fdisk` 或 `smartctl`。应用必须以 root 运行，内核需要提供 `/dev/i2c-N` 和 `i2c-dev`，并且必须存在名称匹配 `SMBus I801 adapter at ` 的 I801 适配器。

## 默认曲线

守护进程按反编译的 `WY2Hardware` 和 `wygpio::MyGpio::aiFanSpeed()` 运行。CPU 直接读取 `coretemp` hwmon（等价于官方 `sensors -j` 中 `coretemp-isa-0000` 的最大 `_input` 值）；硬盘直接读取 `drivetemp`/`nvme`/ATA/SCSI hwmon 和磁盘 thermal zone，最后取成功读取值的最大值。整个应用不依赖 `fdisk`、`smartctl`、`sensors` 或 `i2c-tools`。需要注意：如果内核没有加载硬盘温度驱动、sysfs 中没有暴露温度，任何不依赖外部工具的实现都无法读取该硬盘温度。调速状态边界为 30/45/55/60°C，保留最近 6 次样本，并使用官方的上一温度、平均温度、当前 PWM、状态计数器和状态相关温差系数（状态 2 为 `5/3`，状态 3 为 `5/5`，状态 4 为 `10/5`，分别对应突升/平均升温路径）。状态输出范围也按官方跳转表执行：状态 2 为 `50..125`（不超过 40°C 固定 50），状态 3 为 `80..180`（不超过 50°C 固定 80），状态 4 为 `150..255`；最终硬件写入仍按官方 `50..250` 限制。低温状态固定 PWM 50，高温状态请求 PWM 255。也可在安装配置中填写 `disk_temp_path` 覆盖硬盘温度。温度读取失败或停止服务时会切换到 PWM 250。

风扇控制器的有效值按原 U-NAS 守护进程限制为 `50..250`；最高档调试命令可请求 I2C `255`。守护进程默认每 2 秒重写一次 I2C，轮询间隔仍可在安装配置中调整。温度读取失败或停止服务时使用 250 作为故障保护值。

## `wygpio` 迁移结论

镜像中的 `wygpio` 是依赖 Qt5、`libi2c`、`libapt-pkg` 和 U-NAS RPC 的完整守护程序，不能直接复制到飞牛系统：当前飞牛环境缺少这些 ABI，且 `/unas` 路径和 RPC 服务不存在。反编译确认其温度语义来自 `sensors -j`、`fdisk -l`、`smartctl --attribute`；本包使用内核 hwmon/sysfs 直接实现相同的温度选择语义，硬件路径包含 I2C `0x54/0xF0`，自动调速核心每 10 秒读取 CPU 与磁盘最高温度、维护历史窗口并按状态调速。

本包以 Rust 重写官方 `wygpio` 的 I801 I2C 写入后端，保留命令名 `superio-fanctl` 以兼容已有安装脚本；该命令会自动扫描并选择 I801 总线，然后对 `0x54/0xf0` 执行 SMBus byte-data 读写。根据 HS401P 实测结果，Super I/O 写入路径已从自动控制中移除；`--read-superio` 和 `--probe-superio` 仅保留只读诊断。`wygpio` 的 Qt/RPC 外壳暂不移植，自动调速逻辑使用反编译得到的官方实现。

手动验证控制器读写：

```bash
/vol1/@appdata/unasfan_fpk/bin/superio-fanctl --read
/vol1/@appdata/unasfan_fpk/bin/superio-fanctl --read-superio
/vol1/@appdata/unasfan_fpk/bin/superio-fanctl --probe-superio
/vol1/@appdata/unasfan_fpk/bin/superio-fanctl --value 100 --i2c-only
/vol1/@appdata/unasfan_fpk/bin/superio-fanctl --value 255 --i2c-only --legacy-max
```

`--read` 会输出实际选择的 `/dev/i2c-N`、地址和 `0xf0` 当前值；`--read-superio` 和 `--probe-superio` 只读旧版 Super I/O 寄存器，不会写入。自动调速只写 I2C。若找不到 I801 适配器或 ioctl 失败，命令会返回错误。
