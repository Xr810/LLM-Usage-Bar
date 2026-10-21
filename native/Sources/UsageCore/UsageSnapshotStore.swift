import Foundation

public actor UsageSnapshotStore {
    public enum StoreError: Error, Equatable {
        case unsupportedSchema(Int)
    }

    private struct Envelope: Codable {
        let schemaVersion: Int
        let snapshot: UsageSnapshot
    }

    public static let schemaVersion = 1

    private let fileURL: URL
    private let decoder: JSONDecoder
    private let encoder: JSONEncoder

    public init(fileURL: URL) {
        self.fileURL = fileURL
        decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
    }

    public func load() throws -> UsageSnapshot? {
        guard FileManager.default.fileExists(atPath: fileURL.path) else { return nil }
        let envelope = try decoder.decode(Envelope.self, from: Data(contentsOf: fileURL))
        guard envelope.schemaVersion == Self.schemaVersion else {
            throw StoreError.unsupportedSchema(envelope.schemaVersion)
        }
        return envelope.snapshot
    }

    public func save(_ snapshot: UsageSnapshot) throws {
        let directory = fileURL.deletingLastPathComponent()
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let data = try encoder.encode(Envelope(schemaVersion: Self.schemaVersion, snapshot: snapshot))
        try data.write(to: fileURL, options: .atomic)
    }
}
