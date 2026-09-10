fn main() {
    // sherpa-onnx 预编译库存在时才启用本地 ASR（sherpa_available cfg）
    println!("cargo::rustc-check-cfg=cfg(sherpa_available)");
    sherpa_setup();
    embed_info_plist();

    // 本地交叉检查（cargo check --target …）时 tauri-winres 会因缺少
    // llvm-rc 恐慌；设 MOJO_SKIP_TAURI_BUILD=1 跳过资源嵌入做纯类型检查
    if std::env::var("MOJO_SKIP_TAURI_BUILD").is_err() {
        tauri_build::build()
    }
}

/// 把 Info.plist 嵌进裸二进制的 __TEXT,__info_plist 段。
/// `tauri dev` 直接跑未打包二进制时，macOS TCC 从这里读隐私用途描述 ——
/// 缺了 NSBluetoothAlwaysUsageDescription，CoreBluetooth 一初始化就 SIGABRT。
/// 打包成 .app 后系统优先用 bundle 里的 Info.plist，此段自动被忽略。
// 注意：build.rs 里的 cfg(target_os) 判定的是 HOST；目标平台要用 CARGO_CFG_TARGET_OS
fn embed_info_plist() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
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

/// 编译 SherpaBridge C 桥并链接 sherpa-onnx 预编译库（按目标平台选目录）。
/// 库不在（未下载）时跳过，sherpa 模块退化为 stub。
///
/// 目录约定（CI 按此下载）：
///   macOS   third_party/sherpa-onnx/{lib,include}
///   Windows third_party/sherpa-onnx-win-x64/{lib,include}（MD-Release 动态 CRT）
///   Linux   third_party/sherpa-onnx-linux-x64/{lib,include}
fn sherpa_setup() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let (sub, marker): (&str, &str) = match target_os.as_str() {
        "macos" => ("third_party/sherpa-onnx", "libsherpa-onnx-c-api.dylib"),
        "windows" => (
            "third_party/sherpa-onnx-win-x64",
            "sherpa-onnx-c-api.lib",
        ),
        "linux" => (
            "third_party/sherpa-onnx-linux-x64",
            "libsherpa-onnx-c-api.so",
        ),
        _ => return,
    };

    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.join("../.."); // 仓库根
    let sherpa = root.join(sub);
    let lib_dir = sherpa.join("lib");
    let bridge = manifest.join("native/sherpa-bridge");

    println!(
        "cargo:rerun-if-changed={}",
        bridge.join("bridge.c").display()
    );
    println!("cargo:rerun-if-changed={}", lib_dir.display());

    if !lib_dir.join(marker).exists() {
        println!(
            "cargo:warning=未找到 {}，sherpa 本地 ASR 将以 stub 编译（CI 会自动下载）",
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
    // Windows 的 .lib 是导入库，同样用 dylib kind 链接
    println!("cargo:rustc-link-lib=dylib=sherpa-onnx-c-api");
    println!("cargo:rustc-link-lib=dylib=onnxruntime");

    match target_os.as_str() {
        "macos" => {
            // 开发期绝对路径 rpath；打包后 dylib 收在 .app/Contents/Frameworks
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());
            println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
        }
        "linux" => {
            // 开发期绝对路径；部署时 .so 与可执行文件同目录（$ORIGIN）
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());
            println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
        }
        _ => {} // Windows 无 rpath：DLL 与 exe 同目录即可（见 CI 产物说明）
    }
    println!("cargo:rustc-cfg=sherpa_available");
}
