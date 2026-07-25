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

构建前会使用 Rust musl target 静态编译 Super I/O 控制工具和守护进程并检查 JSON 配置，需要 Rust、musl 工具链和当前工作区中的 `fnpack`。生成的文件位于 `dist/unasfan_fpk-1.1.1-x86.fpk`。

GitHub Actions 位于 `.github/workflows/release.yml`。手动触发或推送 `v*` 标签时，流水线使用 UTC 当前时间生成 `YYYYMMDDHHMMSS` 版本号，构建 FPK 并发布对应 GitHub Release。

## 依赖

应用固定使用 Rust 工具通过 `iopl(3)` 直接写入 Super I/O，同时控制 3 个风扇通道（寄存器 `0x6b/0x73/0xa3`）。只需要应用以 root 运行，不需要安装 `i2c-tools`，也不需要加载 `i2c-dev`。

## 默认曲线

原 U-NAS 程序的 CPU 曲线为 55/65/75/80°C 对应 31/39/55/100%。守护进程每次取 CPU 温度和硬盘最高温度中的较高值参与曲线计算；硬盘默认自动扫描 `drivetemp`/`nvme` hwmon 传感器，也可在安装配置中填写 `disk_temp_path`。本包在 55°C 以下使用可配置的最低转速，默认 31%，避免风扇完全停转。温度读取失败或停止服务时会切换到 PWM 250。

风扇控制器的有效值按原 U-NAS 守护进程限制为 `50..250`；100% 会换算为 254 后再被限制为 250，不能把 255 当作更高转速。温度读取失败或停止服务时使用 250 作为故障保护值。

## `wygpio` 迁移结论

镜像中的 `wygpio` 是依赖 Qt5、`libi2c`、`libapt-pkg` 和 U-NAS RPC 的完整守护程序，不能直接复制到飞牛系统：当前飞牛环境缺少这些 ABI，且 `/unas` 路径和 RPC 服务不存在。反编译确认其原始硬件路径包含 I2C `0x54/0xF0`，而其自动调速核心是每 10 秒读取 CPU 与磁盘最高温度、维护历史窗口并按状态平滑调速。

本包移植了原程序中未被命令行调用的 Super I/O 路径，并以 Rust 重写为 `superio-fanctl`；同时以 Rust 重写了温控轮询核心为 `fan-daemon`。`wygpio` 的 Qt/RPC 外壳暂不移植，包内只保留 Super I/O 风扇控制部分。
