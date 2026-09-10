//! 前台 App 识别（对应 Swift `frontmostApp()`）。
//! 用于按前台应用匹配映射方案（profile）。

use objc2_app_kit::NSWorkspace;

/// 返回 (bundle_id, app_name)
pub fn frontmost_app() -> (Option<String>, Option<String>) {
    let ws = NSWorkspace::sharedWorkspace();
    let Some(app) = ws.frontmostApplication() else {
        return (None, None);
    };
    let bundle_id = app.bundleIdentifier().map(|s| s.to_string());
    let name = app.localizedName().map(|s| s.to_string());
    (bundle_id, name)
}
