import Foundation
import Combine
import CryptoKit

/// Downloads public model files only. This object never sends recordings or transcripts.
@MainActor
final class ModelManager: ObservableObject {
    @Published private(set) var ready = false
    @Published private(set) var busy = false
    @Published private(set) var status = "首次使用需下载 SenseVoiceSmall 离线模型（约 241 MB）"

    let directory: URL

    init() {
        directory = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Models/sensevoice-int8-v1", isDirectory: true)
        // Existing files are checked without making a network request.
        Task { await checkExisting() }
    }

    private func checkExisting() async {
        guard !busy, !ready else { return }
        let location = directory
        let valid = await Task.detached(priority: .utility) {
            ModelAssets.all.allSatisfy { (try? ModelAssets.verify($0, in: location)) == true }
        }.value
        guard !busy, !ready else { return }
        ready = valid
        if valid { status = "SenseVoiceSmall 离线模型已就绪" }
    }

    func prepare() async throws {
        guard !busy else { return }
        busy = true
        ready = false
        defer { busy = false }
        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            var excluded = directory
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            try excluded.setResourceValues(values)

            for asset in ModelAssets.all {
                try Task.checkCancellation()
                let location = directory
                let verification = Task.detached(priority: .utility) {
                    try ModelAssets.verify(asset, in: location)
                }
                let valid = try await withTaskCancellationHandler {
                    try await verification.value
                } onCancel: {
                    verification.cancel()
                }
                if valid { continue }
                status = "正在下载 \(asset.label)…"
                let configuration = URLSessionConfiguration.ephemeral
                configuration.timeoutIntervalForRequest = 60
                configuration.timeoutIntervalForResource = 900
                let session = URLSession(configuration: configuration,
                    delegate: ModelDownloadLimiter(maximumBytes: asset.bytes), delegateQueue: nil)
                defer { session.invalidateAndCancel() }
                let (temporary, response) = try await session.download(from: asset.url)
                defer { try? FileManager.default.removeItem(at: temporary) }
                guard let http = response as? HTTPURLResponse, http.statusCode == 200 else {
                    throw OfflineError.downloadFailed
                }
                status = "正在校验 \(asset.label)…"
                let downloadedVerification = Task.detached(priority: .utility) {
                    try ModelAssets.verifyFile(temporary, asset: asset)
                }
                let matches = try await withTaskCancellationHandler {
                    try await downloadedVerification.value
                } onCancel: {
                    downloadedVerification.cancel()
                }
                guard matches else { throw OfflineError.invalidModel }
                try Task.checkCancellation()
                let target = directory.appendingPathComponent(asset.name)
                if FileManager.default.fileExists(atPath: target.path) {
                    try FileManager.default.removeItem(at: target)
                }
                try FileManager.default.moveItem(at: temporary, to: target)
            }
            ready = true
            status = "SenseVoiceSmall 离线模型已就绪"
        } catch {
            status = "模型未就绪，可重试下载：\(error.localizedDescription)"
            throw error
        }
    }
}

/// Enforces a disk budget during transfer, in addition to the final exact size/SHA check.
private final class ModelDownloadLimiter: NSObject, URLSessionDownloadDelegate, @unchecked Sendable {
    private let maximumBytes: Int64
    init(maximumBytes: Int64) { self.maximumBytes = maximumBytes }
    func urlSession(_ session: URLSession, downloadTask: URLSessionDownloadTask,
                    didWriteData bytesWritten: Int64, totalBytesWritten: Int64,
                    totalBytesExpectedToWrite: Int64) {
        if totalBytesWritten > maximumBytes || totalBytesExpectedToWrite > maximumBytes {
            downloadTask.cancel()
        }
    }
    func urlSession(_ session: URLSession, downloadTask: URLSessionDownloadTask,
                    didFinishDownloadingTo location: URL) {
        // The async URLSession download API receives and owns the temporary file.
    }
}

enum OfflineError: LocalizedError {
    case downloadFailed, invalidModel, microphonePermission, recorderFailed, invalidAudio, tooLong
    var errorDescription: String? {
        switch self {
        case .downloadFailed: return "模型下载失败，请检查网络。"
        case .invalidModel: return "模型校验失败，请重新下载。"
        case .microphonePermission: return "请在系统设置中允许麦克风。"
        case .recorderFailed: return "录音无法启动，麦克风可能正被其他应用使用。"
        case .invalidAudio: return "录音格式无效或没有完整音频。"
        case .tooLong: return "原型每次最多录音 120 秒。"
        }
    }
}

enum ModelAssets {
    struct Asset: Sendable {
        let name: String
        let label: String
        let url: URL
        let sha256: String
        let bytes: Int64
    }
    private static let base = "https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/2365baeacb507f821a0c8120fcee3d484dba7a07/"
    static let all: [Asset] = [
        Asset(name: "model.int8.onnx", label: "SenseVoiceSmall INT8", url: URL(string: base + "model.int8.onnx")!,
              sha256: "c71f0ce00bec95b07744e116345e33d8cbbe08cef896382cf907bf4b51a2cd51", bytes: 239233841),
        Asset(name: "tokens.txt", label: "多语言词表", url: URL(string: base + "tokens.txt")!,
              sha256: "f449eb28dc567533d7fa59be34e2abca8784f771850c78a47fb731a31429a1dc", bytes: 315894),
        Asset(name: "silero_vad_v4.onnx", label: "Silero VAD v4", url: URL(string: "https://blob.handy.computer/silero_vad_v4.onnx")!,
              sha256: "a35ebf52fd3ce5f1469b2a36158dba761bc47b973ea3382b3186ca15b1f5af28", bytes: 1807522)
    ]

    static func verify(_ asset: Asset, in directory: URL) throws -> Bool {
        let path = directory.appendingPathComponent(asset.name)
        guard FileManager.default.fileExists(atPath: path.path) else { return false }
        return try verifyFile(path, asset: asset)
    }

    static func verifyFile(_ path: URL, asset: Asset) throws -> Bool {
        let attributes = try FileManager.default.attributesOfItem(atPath: path.path)
        guard (attributes[.size] as? NSNumber)?.int64Value == asset.bytes else { return false }
        let handle = try FileHandle(forReadingFrom: path)
        defer { try? handle.close() }
        var hash = SHA256()
        while let chunk = try handle.read(upToCount: 1_048_576), !chunk.isEmpty {
            try Task.checkCancellation()
            hash.update(data: chunk)
        }
        return hash.finalize().map { String(format: "%02x", $0) }.joined() == asset.sha256
    }
}
