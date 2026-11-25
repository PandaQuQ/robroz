// Desktop and iOS entry. Android enters through `android_main` in the lib.
fn main() -> Result<(), slint::PlatformError> {
    robroz_mobile::run()
}
