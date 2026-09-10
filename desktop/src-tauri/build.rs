fn main() {
    // sherpa-onnx 预编译库存在时才启用本地 ASR（sherpa_available cfg）
    println!("cargo::rustc-check-cfg=cfg(sherpa_available)");
    sherpa_setup();
    embed_info_plist();

    tauri_build::build()
}

/// 把 Info.plist 嵌进裸二进制的 __TEXT,__info_plist 段。
/// `tauri dev` 直接跑未打包二进制时，macOS TCC 从这里读隐私用途描述 ——
/// 缺了 NSBluetoothAlwaysUsageDescription，CoreBluetooth 一初始化就 SIGABRT。
/// 打包成 .app 后系统优先用 bundle 里的 Info.plist，此段自动被忽略。
#[cfg(target_os = "macos")]
fn embed_info_plist() {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let plist = manifest.join("Info.plist");
    println!("cargo:rerun-if-changed={}", plist.display());
    if plist.exists() {
        println!(
            "cargo:rustc-link-arg=-Wl,-sectcreate,__TEXT,__info_plist,{}",
            plist.display()
        );
    }
}

#[cfg(not(target_os = "macos"))]
fn embed_info_plist() {}

/// 编译 SherpaBridge C 桥并链接 third_party/sherpa-onnx 预编译动态库。
/// 与 Package.swift 的链接方式一致；库不在（Windows/Linux/未下载）时跳过，
/// sherpa 模块退化为 stub。
#[cfg(target_os = "macos")]
fn sherpa_setup() {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.join("../.."); // 仓库根
    let sherpa = root.join("third_party/sherpa-onnx");
    let lib_dir = sherpa.join("lib");
    let bridge = manifest.join("native/sherpa-bridge");

    // 依赖变化时重跑
    println!("cargo:rerun-if-changed={}", bridge.join("bridge.c").display());
    println!("cargo:rerun-if-changed={}", lib_dir.display());

    if !lib_dir.join("libsherpa-onnx-c-api.dylib").exists() {
        println!(
            "cargo:warning=未找到 {}，sherpa 本地 ASR 将以 stub 编译（运行 build-app.sh 或 CI 下载脚本可获得）",
            lib_dir.display()
        );
        return;
    }

    cc::Build::new()
        .file(bridge.join("bridge.c"))
        .include(bridge.join("include"))
        .include(sherpa.join("include"))
        .compile("sherpa_bridge");

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=dylib=sherpa-onnx-c-api");
    println!("cargo:rustc-link-lib=dylib=onnxruntime");
    // 开发期直接用绝对路径 rpath；打包时由 Tauri 把 dylib 收进 Frameworks 后再补 @executable_path rpath
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());
    // 打包后 dylib 收在 .app/Contents/Frameworks（tauri.conf.json 的 macOS.frameworks），
    // 补标准 rpath；dev 期该路径不存在，dyld 自动跳过，无副作用
    println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
    println!("cargo:rustc-cfg=sherpa_available");
}

#[cfg(not(target_os = "macos"))]
fn sherpa_setup() {}
