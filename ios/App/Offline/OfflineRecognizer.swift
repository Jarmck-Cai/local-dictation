import AVFoundation
import Foundation
import SherpaOnnx

/// Actor isolation keeps native sessions on a worker executor, never the UI executor.
actor OfflineRecognizer {
    private var recognizer: SherpaOnnxOfflineRecognizer?
    private var modelDirectory: URL?

    func transcribe(file: URL, models: URL) async throws -> String {
        try Task.checkCancellation()
        if recognizer == nil || modelDirectory != models {
            // Native constructors abort on invalid files; never call them before SHA checks.
            for asset in ModelAssets.all {
                guard try ModelAssets.verify(asset, in: models) else { throw OfflineError.invalidModel }
            }
            let voice = sherpaOnnxOfflineSenseVoiceModelConfig(
                model: models.appendingPathComponent("model.int8.onnx").path,
                language: "auto", useInverseTextNormalization: true)
            let model = sherpaOnnxOfflineModelConfig(
                tokens: models.appendingPathComponent("tokens.txt").path,
                numThreads: 2, provider: "cpu", debug: 0, senseVoice: voice)
            var configuration = sherpaOnnxOfflineRecognizerConfig(
                featConfig: sherpaOnnxFeatureConfig(sampleRate: 16_000, featureDim: 80), modelConfig: model)
            recognizer = SherpaOnnxOfflineRecognizer(config: &configuration)
            modelDirectory = models
        }
        guard let recognizer else { throw OfflineError.invalidModel }
        let samples = try loadSamples(file)
        guard !samples.isEmpty else { return "" }
        let silero = sherpaOnnxSileroVadModelConfig(
            model: models.appendingPathComponent("silero_vad_v4.onnx").path,
            threshold: 0.5, minSilenceDuration: 0.35, minSpeechDuration: 0.2,
            windowSize: 512, maxSpeechDuration: 12)
        var vadConfiguration = sherpaOnnxVadModelConfig(sileroVad: silero, sampleRate: 16_000, numThreads: 1)
        let vad = SherpaOnnxVoiceActivityDetectorWrapper(config: &vadConfiguration, buffer_size_in_seconds: 30)
        var pieces: [String] = []

        func drain() throws {
            while !vad.isEmpty() {
                try Task.checkCancellation()
                let segment = vad.front().samples
                vad.pop()
                guard !segment.isEmpty else { continue }
                let result = recognizer.decode(samples: segment, sampleRate: 16_000)
                let text = Self.clean(result.text)
                if !text.isEmpty { pieces.append(text) }
            }
        }

        for offset in stride(from: 0, to: samples.count, by: 512) {
            try Task.checkCancellation()
            let end = min(offset + 512, samples.count)
            var chunk = Array(samples[offset..<end])
            if chunk.count < 512 { chunk += Array(repeating: 0, count: 512 - chunk.count) }
            vad.acceptWaveform(samples: chunk)
            try drain()
        }
        vad.flush()
        try drain()
        try Task.checkCancellation()
        return pieces.joined(separator: " ")
    }

    private func loadSamples(_ url: URL) throws -> [Float] {
        let file = try AVAudioFile(forReading: url, commonFormat: .pcmFormatFloat32, interleaved: false)
        let format = file.processingFormat
        guard format.channelCount == 1, abs(format.sampleRate - 16_000) < 1,
              file.length > 0 else { throw OfflineError.invalidAudio }
        guard file.length <= 16_000 * 121 else { throw OfflineError.tooLong }
        guard let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(file.length)) else {
            throw OfflineError.invalidAudio
        }
        try file.read(into: buffer)
        guard let channel = buffer.floatChannelData?[0] else { throw OfflineError.invalidAudio }
        return Array(UnsafeBufferPointer(start: channel, count: Int(buffer.frameLength)))
    }

    private static func clean(_ text: String) -> String {
        text.replacingOccurrences(of: "<\\|[^|]*\\|>", with: "", options: .regularExpression)
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
