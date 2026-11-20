# 安卓探路（未完成，Panda 暂停项目后中止）

时间：2026-11-20 约 11:59–12:25（英国时间）。分支 `spike/android`，基于 main `08e95b1`，只在本地提交，没推送，没开 PR。

## 环境
- 盒子上有 Android SDK（build-tools 35/36、NDK 27.1、platforms 35/37）。本次补装了 JDK 21（Debian 13 没有 17）、emulator、`system-images;android-35;google_apis;x86_64`、Gradle 8.14.3（在 `/workspace/tools`）、cargo-ndk，以及 Rust 的 android 和 wasm32 target。trunk 没装完就中止了。
- `/dev/kvm` 存在，root 能打开。box 用户默认没权限，用 `sudo setfacl -m u:box:rw /dev/kvm` 加上以后能打开（KVM API 12）。第一次加的 ACL 没生效，模拟器在没有加速的情况下卡在 `offline` 十多分钟。第二次重启模拟器时又被残留的锁文件挡住，随后项目暂停。**模拟器始终没能启动完成，APK 没有装上去跑过。**

## 路线 A：iced 原生安卓
- iced 0.14 官方不支持移动端（iced#302，作者表示不维护）。可用的路子是社区 fork [Santuzius/iced_android](https://github.com/Santuzius/iced_android)（iced 分支 `android-0.14` 加 `IcedActivity`），桌面上的 `iced::application`、`Task`、`Subscription` 都能原样用。
- 做法见 `spike/android-native/`：单独的 Cargo workspace，`build.sh` 把**当前的 client 源码原样**拷进去（`main.rs` 改名为 `lib.rs`），再追加一个 `android_main`，它用环境变量设置 `ROBROZ_HOST=http://10.0.2.2:18790`，然后调用原来的 `main()`。`[patch.crates-io]` 让包括 robroz-ui 在内的所有 iced 依赖都走这个 fork；cargo-ndk 编出 x86_64 的 so，Gradle 打包；Manifest 里加了 INTERNET 权限。
- **结果：`cargo ndk` 和 `gradle assembleDebug` 第一次就通过了，产出 `app/build/outputs/apk/debug/app-debug.apk`**（debug 包 305 MB，正式版需要用 release 加 strip）。client 代码一行没改。
- 没验证的：连上主机、收 SSE、发消息。原因是模拟器没起来（见上）。预计的风险有：wgpu 在模拟器的 swiftshader 下可能起不来，要退到 tiny-skia；中文字体要带上或者读系统字体；软键盘和触摸（fork 只给部分控件补了触摸）；ureq 走明文 HTTP，原生 socket 不受 Android 明文策略限制，但连真机要换局域网或 TLS。
- 粗估工作量：跑通三项 0.5–1 天；做到能日常用（字体、键盘、手机布局、后台切换时 SSE 重连、release 签名）3–5 天。另外要长期跟着社区 fork 走，这是风险。

## 路线 B：网页版（PWA）
- 只做了第一步：直接用 `cargo build -p robroz-client --target wasm32-unknown-unknown` 编当前客户端，只有 1 个编译错误：
  ```
  error[E0433]: cannot find `time` in `tokio`
      --> client/src/main.rs:1576:12
       tokio::time::sleep(d).await;
  ```
- 编译能过不等于能跑。真正卡住的地方在运行时：ureq（同步 HTTP 和 `std::net`）、`std::thread::spawn`、`tokio::task::spawn_blocking` 在浏览器里都不能用，SSE 的读取（`source/stream.rs`）也是在阻塞线程里跑的。要把网络层换成 fetch 和 EventSource（web-sys 或 gloo），所有 API 调用改成 async；token 要存在 localStorage，而 EventSource 不能带 Authorization 头，所以 token 只能放进 query，或者改用 fetch 流式读取；另外 host 要加 CORS（本次没改 host）。
- 没做：trunk 打包、manifest、Playwright Pixel 7 模拟、对真实主机的三项测试。
- 粗估工作量：网络层改成 async 加上 wasm 的 cfg 分支 2–3 天，host 的 CORS 和 token 方案另算。

## 初步建议（数据不全）
原生路线用现在的代码几乎零改动就能打出 APK，代价最小，值得先在模拟器或真机上把三项跑通再定。PWA 要动整个网络层。恢复以后的下一步：修好 KVM 权限（建议建 kvm 组，而不是用 ACL），启动模拟器，`adb install` 后验证那三项并截图。

## 截图
无（模拟器没起来，PWA 没做到那一步）。
