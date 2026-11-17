# Peekboard

> 面向本科生机器人初学者的无头调试工具，当你心血来潮从海鲜市场淘了块板子却苦于没有显示器，亦或者宿舍桌面空间有限，没关系，一根 USB-C 直连，把笔记本充当开发板屏幕，省去额外开销。
> 同时提供一套可复用的跨平台硬件编码抽象库，服务 Rust + ROS 2（rclrust）生态。

## ✨ 项目动机

很多参加电赛、RoboMaster、做机器人毕设的同学，会用生活费购置 Jetson / RK / Intel N 系列开发板。
但额外采购便携显示器，现场调试环境也经常没有显示器。

现有方案的痛点：

- **VNC / NoMachine**：依赖局域网，野外、底盘离线场景不可用；大多是软编码，CPU 占用高。
- **GStreamer 脚本**：零散、难以维护，不方便嵌入 Rust 机器人程序。
- **HDMI 采集卡**：需要额外硬件，增加成本。

Peekboard 依托板子自带 USB Gadget RNDIS 点对点网卡：**一根 USB 线即可离线调试，不需要交换机、WiFi。**

> ⚠️ **这个方案依然不完美！！！**
>
> Peekboard 仅在 **Linux 系统已经正常启动** 后工作；
> 无法捕获 U-Boot、内核 panic、驱动还未拉起时的画面。
> 这类底层调试场景仍然需要 HDMI/DP 采集卡。

## 📌 平台支持矩阵

| 平台 | 状态 | 备注 |
|---|---|---|
| Jetson Orin（T234, L4T R36） | ✅ 优先开发（因为笔者在用） | RNDIS 原生支持 |
| RK3588 | 📋 规划中 | 二期迭代 |
| Intel N 系列工控机 | 📋 规划中 | 三期迭代 |

> 架构层面已做好多后端扩展设计，采用 Feature 开关编译。
> 当前优先完成 Jetson 链路验证，其余平台为后续迭代目标。

## 🧱 架构设计

Workspace 分层设计，核心是一套统一的 `HwEncoder` trait：

```
peekboard/
├── robot-codec-abstract    # 核心抽象 crate：Frame、Nalu、Encoder trait、配置与错误
├── robot-codec-tegra       # Tegra NVENC 后端：nvbufsurface FFI + V4L2 M2M + NVMM/DMA-BUF
├── robot-codec-rk-mpp      # RK MPP 后端（规划）
├── robot-codec-intel-vpl   # Intel oneVPL/VAAPI 后端（规划）
├── robot-capture           # 屏幕捕获抽象层：X11/XShm（MVP）、Wayland/xdg-desktop-portal（规划）
├── robot-streamer          # 板端推流服务：捕获 → 色彩转换 → 硬件编码 → TCP 推流
└── peek-viewer-egui        # 跨平台查看器：egui GUI、H.264 解码、断线重连、帧率统计
```

### 通信协议

自定义极简帧协议：

```
[ 4 字节大端 NALU 长度 ][ NALU 数据 ]
```

基于 TCP，适合点对点 RNDIS 链路，方便断线同步与帧边界切割。

硬件编码器不可用时，自动降级到 `openh264` 软编码。

## 📋 Roadmap

- [x] 设计统一编码器抽象 trait、Frame / Nalu 数据结构
- [ ] 1：Jetson Orin 完整链路
  - [ ] nvbufsurface FFI、NVMM 内存管理、V4L2 M2M NVENC
  - [ ] X11 + XShm 屏幕捕获、色彩转换
  - [ ] TCP 推流服务
  - [ ] egui 跨平台查看器
  - [ ] Release 预编译包 + 基础文档
- [ ] **2**：RK3588 MPP 后端接入
- [ ] **3**：Intel oneVPL + Wayland 捕获支持
- [ ] **4**：独立发布 codec crate，对接 rclrust，支持 ROS 图像话题输出

## 📜 License

Dual licensed under:

- Apache-2.0
- MIT

Consistent with the rclrust project.

## 💡 与同类项目的区别

- **不是简单封装 GStreamer**：核心目标是原生对接厂商硬件编码接口，这部分纯纯M属性大爆发。
- **不是通用 VNC**：主打 USB RNDIS 离线点对点，面向机器人开发板调试场景（其实是主包的校园网有AP隔离，小登时期深受其害）。

## 📮 欢迎贡献

Issues、PR、测试反馈都欢迎。
