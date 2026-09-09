import Foundation

enum Log {
    nonisolated(unsafe) static var verbose = false

    private static let fmt: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "HH:mm:ss.SSS"
        return f
    }()

    static func info(_ s: String) {
        print("\(fmt.string(from: Date())) \(s)")
        fflush(stdout)
    }

    static func debug(_ s: String) {
        guard verbose else { return }
        print("\(fmt.string(from: Date())) · \(s)")
        fflush(stdout)
    }

    static func warn(_ s: String) {
        let line = "\(fmt.string(from: Date())) 警告: \(s)\n"
        FileHandle.standardOutput.write(Data(line.utf8))
        FileHandle.standardError.write(Data(line.utf8))
    }

    static func error(_ s: String) {
        let line = "\(fmt.string(from: Date())) 错误: \(s)\n"
        FileHandle.standardOutput.write(Data(line.utf8))
        FileHandle.standardError.write(Data(line.utf8))
    }
}
