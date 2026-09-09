// swift-tools-version:5.9
import Foundation
import PackageDescription

// third_party/sherpa-onnx 相对包根目录；用 #filePath 算绝对路径，避免 SPM 相对路径解析的坑
let packageRoot = URL(fileURLWithPath: #filePath).deletingLastPathComponent().path
let sherpaInclude = "\(packageRoot)/third_party/sherpa-onnx/include"
let sherpaLib = "\(packageRoot)/third_party/sherpa-onnx/lib"

let package = Package(
    name: "miremote",
    platforms: [.macOS(.v13)],
    targets: [
        .target(
            name: "SherpaBridge",
            path: "Sources/SherpaBridge",
            publicHeadersPath: "include",
            cSettings: [.unsafeFlags(["-I\(sherpaInclude)"])],
            linkerSettings: [.unsafeFlags([
                "-L\(sherpaLib)",
                "-lsherpa-onnx-c-api",
                "-lonnxruntime",
            ])]
        ),
        .executableTarget(
            name: "miremote",
            dependencies: ["SherpaBridge"],
            path: "Sources/miremote",
            linkerSettings: [.unsafeFlags([
                "-Xlinker", "-rpath", "-Xlinker", "@executable_path/../Frameworks"
            ])]
        )
    ]
)
