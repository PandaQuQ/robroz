
// ---- appended by spike/android-native/build.sh (not in client/) ----
/// Android entry: point the client at the host through env vars, then run
/// the unchanged desktop `main()` on iced_android's runtime.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: iced_android::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("robroz"),
    );
    std::panic::set_hook(Box::new(|info| log::error!("panic: {info}")));
    // 10.0.2.2 is the emulator's alias for the box's loopback.
    let host = option_env!("ROBROZ_ANDROID_HOST").unwrap_or("http://10.0.2.2:8790");
    std::env::set_var("ROBROZ_HOST", host);
    std::env::set_var("ROBROZ_USER_NAME", "安卓模拟器");
    std::env::set_var("ROBROZ_TOUCH", "1");
    if let Some(t) = option_env!("ROBROZ_ANDROID_TOKEN") {
        std::env::set_var("ROBROZ_TOKEN", t);
    }
    log::info!("robroz android start, host {host}");
    iced_android::init(app);
    if let Err(e) = main() {
        log::error!("iced stopped: {e}");
    }
}
