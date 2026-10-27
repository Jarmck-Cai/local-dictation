import Foundation
import LocalDictationCore

enum SharedEnvironment {
    static var groupID: String { Bundle.main.object(forInfoDictionaryKey: "LDAppGroup") as? String ?? "" }
    static func store() throws -> SharedStateStore {
        let appOnly = Bundle.main.object(forInfoDictionaryKey: "LDAppOnly") as? Bool == true
        let container = appOnly ? FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first : FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: groupID)
        guard let url = container else {
            throw NSError(domain: "Dictation", code: 1, userInfo: [NSLocalizedDescriptionKey: "App Group 不可用，请检查三个目标的签名与能力。"])
        }
        let store = try SharedStateStore(directory: url.appendingPathComponent("Dictation", isDirectory: true))
        try FileManager.default.setAttributes([.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication], ofItemAtPath: store.directory.path)
        var excluded = URLResourceValues(); excluded.isExcludedFromBackup = true
        var directory = store.directory; try directory.setResourceValues(excluded)
        return store
    }
}
