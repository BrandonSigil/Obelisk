import Foundation

/// Any JSON a storefront returned, kept as it is: the library cache stores a
/// game's whole metadata without modelling every field.
enum JSONValue: Codable, Equatable, Sendable {
    case null
    case bool(Bool)
    case integer(Int64)
    case unsigned(UInt64)
    case double(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    /// The value `json` encodes, or nil when it is no JSON.
    init?(json: String?) {
        guard let json, let value = try? JSONDecoder().decode(JSONValue.self, from: Data(json.utf8)) else {
            return nil
        }
        self = value
    }

    /// The value as JSON text.
    var json: String? {
        let encoder = JSONEncoder()
        encoder.outputFormatting = .sortedKeys
        return (try? encoder.encode(self)).flatMap { String(data: $0, encoding: .utf8) }
    }

    subscript(key: String) -> JSONValue? {
        guard case let .object(object) = self else { return nil }
        return object[key]
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        // Each failed attempt throws, so the most common kinds come first:
        // storefront metadata is mostly strings and objects.
        if container.decodeNil() { self = .null }
        else if let value = try? container.decode(String.self) { self = .string(value) }
        else if let value = try? container.decode([String: JSONValue].self) { self = .object(value) }
        else if let value = try? container.decode([JSONValue].self) { self = .array(value) }
        else if let value = try? container.decode(Bool.self) { self = .bool(value) }
        // Integers first, so identifiers keep every digit.
        else if let value = try? container.decode(Int64.self) { self = .integer(value) }
        else if let value = try? container.decode(UInt64.self) { self = .unsigned(value) }
        else { self = .double(try container.decode(Double.self)) }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .null: try container.encodeNil()
        case let .bool(value): try container.encode(value)
        case let .integer(value): try container.encode(value)
        case let .unsigned(value): try container.encode(value)
        case let .double(value): try container.encode(value)
        case let .string(value): try container.encode(value)
        case let .array(value): try container.encode(value)
        case let .object(value): try container.encode(value)
        }
    }
}
