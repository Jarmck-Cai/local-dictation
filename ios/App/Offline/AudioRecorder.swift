import AVFoundation
import Foundation

/// The host app is the only process that opens the microphone.
@MainActor
final class AudioRecorder: NSObject, AVAudioRecorderDelegate {
    private var recorder: AVAudioRecorder?
    private var recordingURL: URL?
    private var interruptedURL: URL?
    private var interruptionObserver: NSObjectProtocol?
    var onInterrupted: (() -> Void)?
    var isRecording: Bool { recorder?.isRecording == true }
    var hasCapturedAudio: Bool { recordingURL != nil || interruptedURL != nil }

    override init() {
        super.init()
        interruptionObserver = NotificationCenter.default.addObserver(
            forName: AVAudioSession.interruptionNotification, object: nil, queue: .main
        ) { [weak self] notification in
            guard let raw = notification.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
                  AVAudioSession.InterruptionType(rawValue: raw) == .began else { return }
            Task { @MainActor [weak self] in self?.finishInterruptedCapture() }
        }
    }

    deinit {
        if let interruptionObserver { NotificationCenter.default.removeObserver(interruptionObserver) }
    }

    func start(at url: URL) throws {
        guard recorder == nil else { throw OfflineError.recorderFailed }
        guard AVAudioSession.sharedInstance().recordPermission == .granted else {
            throw OfflineError.microphonePermission
        }
        let session = AVAudioSession.sharedInstance()
        do {
            interruptedURL = nil
            try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try session.setCategory(.record, mode: .measurement, options: [])
            try session.setPreferredSampleRate(16_000)
            try session.setActive(true)
            let created = try AVAudioRecorder(url: url, settings: [
                AVFormatIDKey: Int(kAudioFormatLinearPCM),
                AVSampleRateKey: 16_000,
                AVNumberOfChannelsKey: 1,
                AVLinearPCMBitDepthKey: 16,
                AVLinearPCMIsFloatKey: false,
                AVLinearPCMIsBigEndianKey: false
            ])
            created.delegate = self
            guard created.prepareToRecord(), created.record(forDuration: 120) else {
                throw OfflineError.recorderFailed
            }
            recorder = created
            recordingURL = url
        } catch {
            recorder?.stop()
            recorder = nil
            recordingURL = nil
            try? session.setActive(false, options: .notifyOthersOnDeactivation)
            throw error
        }
    }

    /// Always closes/deactivates the mic before returning the file for ASR.
    @discardableResult
    func stop() -> URL? {
        let url = recordingURL ?? interruptedURL
        let active = recorder
        recorder = nil
        recordingURL = nil
        interruptedURL = nil
        active?.stop()
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        return url
    }

    private func finishInterruptedCapture() {
        guard recorder != nil else { return }
        // Release the mic immediately. The coordinator's next stop() still receives this file.
        interruptedURL = stop()
        onInterrupted?()
    }

    func audioRecorderDidFinishRecording(_ recorder: AVAudioRecorder, successfully flag: Bool) {
        guard self.recorder === recorder else { return }
        // Covers the hard 120-second limit. Manual stop clears self.recorder first.
        finishInterruptedCapture()
    }

    func audioRecorderEncodeErrorDidOccur(_ recorder: AVAudioRecorder, error: Error?) {
        guard self.recorder === recorder else { return }
        finishInterruptedCapture()
    }
}
