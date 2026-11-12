import Foundation

public indirect enum NativeBridgeParameter: Codable, Equatable, Sendable {
    case null
    case string(String)
    case integer(Int64)
    case unsigned(UInt64)
    case boolean(Bool)
    case object([String: NativeBridgeParameter])
    case array([NativeBridgeParameter])

    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Bool.self) {
            self = .boolean(value)
        } else if let value = try? container.decode(Int64.self) {
            self = .integer(value)
        } else if let value = try? container.decode(UInt64.self) {
            self = .unsigned(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([NativeBridgeParameter].self) {
            self = .array(value)
        } else if let value = try? container.decode([String: NativeBridgeParameter].self) {
            self = .object(value)
        } else {
            throw DecodingError.dataCorruptedError(
                in: container,
                debugDescription: "Unsupported native bridge JSON value"
            )
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .null: try container.encodeNil()
        case let .string(value): try container.encode(value)
        case let .integer(value): try container.encode(value)
        case let .unsigned(value): try container.encode(value)
        case let .boolean(value): try container.encode(value)
        case let .object(value): try container.encode(value)
        case let .array(value): try container.encode(value)
        }
    }
}

#if os(macOS)
import Darwin

public actor NativeBridgeClient {
    public static let protocolVersion = 1
    public static let snapshotSchemaVersion = 1
    public static let mutationSchemaVersion = 1
    public static let maximumMessageBytes = 1024 * 1024

    private let socketURL: URL
    private var descriptor: Int32 = -1
    private var serverMutationSchemaVersion: Int?
    private let encoder = JSONEncoder()
    private let decoder = JSONDecoder()

    public init(socketURL: URL = NativeBridgeClient.defaultSocketURL()) {
        self.socketURL = socketURL
    }

    deinit {
        if descriptor >= 0 { Darwin.close(descriptor) }
    }

    public static func defaultSocketURL(
        homeDirectory: URL = FileManager.default.homeDirectoryForCurrentUser
    ) -> URL {
        runtimeDirectory(homeDirectory: homeDirectory)
            .appendingPathComponent("native-bridge-v1.sock", isDirectory: false)
    }

    public static func defaultSnapshotURL(
        homeDirectory: URL = FileManager.default.homeDirectoryForCurrentUser
    ) -> URL {
        runtimeDirectory(homeDirectory: homeDirectory)
            .appendingPathComponent("tray-snapshot-v1.json", isDirectory: false)
    }

    public func call<Result: Decodable & Sendable>(
        _ method: String,
        params: [String: NativeBridgeParameter]? = nil,
        as resultType: Result.Type
    ) throws -> Result {
        do {
            if descriptor < 0 { try connectAndHello() }
            return try send(method, params: params, as: resultType)
        } catch {
            disconnect()
            throw error
        }
    }

    public func callMutation<Result: Decodable & Sendable>(
        _ method: String,
        params: [String: NativeBridgeParameter],
        as resultType: Result.Type
    ) throws -> Result {
        if descriptor < 0 { try connectAndHello() }
        guard serverMutationSchemaVersion == Self.mutationSchemaVersion else {
            throw UsageRepositoryError.mutationUnavailable(
                schemaVersion: serverMutationSchemaVersion
            )
        }
        do {
            return try send(method, params: params, as: resultType)
        } catch {
            disconnect()
            throw error
        }
    }

    public func disconnect() {
        if descriptor >= 0 { Darwin.close(descriptor) }
        descriptor = -1
        serverMutationSchemaVersion = nil
    }

    public nonisolated static func isSocketReachable(
        _ socketURL: URL = NativeBridgeClient.defaultSocketURL()
    ) -> Bool {
        let descriptor = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard descriptor >= 0 else { return false }
        defer { Darwin.close(descriptor) }
        return connect(descriptor: descriptor, to: socketURL.path) == 0
    }

    private static func runtimeDirectory(homeDirectory: URL) -> URL {
        homeDirectory
            .appendingPathComponent(".llm-usage-bar", isDirectory: true)
            .appendingPathComponent("runtime", isDirectory: true)
    }

    private func connectAndHello() throws {
        descriptor = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard descriptor >= 0 else { throw UsageRepositoryError.bridgeUnavailable }
        var timeout = timeval(tv_sec: 30, tv_usec: 0)
        _ = withUnsafePointer(to: &timeout) { pointer in
            Darwin.setsockopt(
                descriptor,
                SOL_SOCKET,
                SO_RCVTIMEO,
                pointer,
                socklen_t(MemoryLayout<timeval>.size)
            )
        }
        _ = withUnsafePointer(to: &timeout) { pointer in
            Darwin.setsockopt(
                descriptor,
                SOL_SOCKET,
                SO_SNDTIMEO,
                pointer,
                socklen_t(MemoryLayout<timeval>.size)
            )
        }
        do {
            try connectDescriptor(to: socketURL.path)
            let hello: HelloResponse = try send("hello", params: nil, as: HelloResponse.self)
            guard hello.protocolVersion == Self.protocolVersion else {
                throw UsageRepositoryError.unsupportedProtocol(hello.protocolVersion)
            }
            guard hello.server == "llm-usage-bar-rust" else {
                throw UsageRepositoryError.malformedResponse
            }
            guard hello.snapshotSchemaVersion == Self.snapshotSchemaVersion else {
                throw UsageRepositoryError.unsupportedSnapshotSchema(hello.snapshotSchemaVersion)
            }
            serverMutationSchemaVersion = hello.mutationSchemaVersion
        } catch {
            disconnect()
            throw error
        }
    }

    private func connectDescriptor(to path: String) throws {
        guard Self.connect(descriptor: descriptor, to: path) == 0 else {
            throw UsageRepositoryError.bridgeUnavailable
        }
    }

    private nonisolated static func connect(descriptor: Int32, to path: String) -> Int32 {
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        let pathBytes = Array(path.utf8CString)
        let maximumPathBytes = MemoryLayout.size(ofValue: address.sun_path)
        guard pathBytes.count <= maximumPathBytes else { return -1 }
        withUnsafeMutableBytes(of: &address.sun_path) { buffer in
            buffer.initializeMemory(as: UInt8.self, repeating: 0)
            buffer.copyBytes(from: pathBytes.map { UInt8(bitPattern: $0) })
        }
        let length = socklen_t(MemoryLayout<sa_family_t>.size + pathBytes.count)
        return withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.connect(descriptor, $0, length)
            }
        }
    }

    private func send<Result: Decodable & Sendable>(
        _ method: String,
        params: [String: NativeBridgeParameter]?,
        as resultType: Result.Type
    ) throws -> Result {
        let request = BridgeRequest(
            protocolVersion: Self.protocolVersion,
            id: UUID().uuidString,
            method: method,
            params: params
        )
        var data = try encoder.encode(request)
        data.append(0x0A)
        try writeAll(data)
        let responseData = try readLine()
        let response = try decoder.decode(BridgeResponse<Result>.self, from: responseData)
        guard response.protocolVersion == Self.protocolVersion else {
            throw UsageRepositoryError.unsupportedProtocol(response.protocolVersion)
        }
        guard response.id == request.id else { throw UsageRepositoryError.malformedResponse }
        if let error = response.error {
            throw UsageRepositoryError.remote(
                code: error.code,
                message: error.message,
                data: error.data
            )
        }
        guard let result = response.result else { throw UsageRepositoryError.malformedResponse }
        return result
    }

    private func writeAll(_ data: Data) throws {
        try data.withUnsafeBytes { rawBuffer in
            guard var pointer = rawBuffer.baseAddress else { return }
            var remaining = rawBuffer.count
            while remaining > 0 {
                let written = Darwin.write(descriptor, pointer, remaining)
                guard written > 0 else { throw UsageRepositoryError.bridgeUnavailable }
                remaining -= written
                pointer = pointer.advanced(by: written)
            }
        }
    }

    private func readLine() throws -> Data {
        var result = Data()
        var byte: UInt8 = 0
        while result.count < Self.maximumMessageBytes {
            let count = Darwin.read(descriptor, &byte, 1)
            guard count == 1 else { throw UsageRepositoryError.bridgeUnavailable }
            if byte == 0x0A { return result }
            result.append(byte)
        }
        throw UsageRepositoryError.requestTooLarge
    }

    private struct BridgeRequest: Encodable {
        let protocolVersion: Int
        let id: String
        let method: String
        let params: [String: NativeBridgeParameter]?
    }

    private struct BridgeResponse<Result: Decodable>: Decodable {
        let protocolVersion: Int
        let id: String
        let result: Result?
        let error: BridgeError?
    }

    private struct BridgeError: Decodable {
        let code: String
        let message: String
        let data: NativeBridgeParameter?
    }

    private struct HelloResponse: Decodable, Sendable {
        let protocolVersion: Int
        let server: String
        let readOnly: Bool?
        let snapshotSchemaVersion: Int
        let mutationSchemaVersion: Int?
    }
}
#else
public actor NativeBridgeClient {
    public static let protocolVersion = 1
    public static let snapshotSchemaVersion = 1
    public static let mutationSchemaVersion = 1
    public static let maximumMessageBytes = 1024 * 1024
    public init(socketURL: URL = URL(fileURLWithPath: "/dev/null")) {}
    public static func defaultSocketURL(homeDirectory: URL = .init(fileURLWithPath: "/")) -> URL { homeDirectory }
    public static func defaultSnapshotURL(homeDirectory: URL = .init(fileURLWithPath: "/")) -> URL { homeDirectory }
    public func call<Result: Decodable & Sendable>(
        _ method: String,
        params: [String: NativeBridgeParameter]? = nil,
        as: Result.Type
    ) throws -> Result {
        throw UsageRepositoryError.bridgeUnavailable
    }
    public func callMutation<Result: Decodable & Sendable>(
        _ method: String,
        params: [String: NativeBridgeParameter],
        as: Result.Type
    ) throws -> Result {
        throw UsageRepositoryError.mutationUnavailable(schemaVersion: nil)
    }
    public nonisolated static func isSocketReachable(_ socketURL: URL = .init(fileURLWithPath: "/dev/null")) -> Bool { false }
    public func disconnect() {}
}
#endif
