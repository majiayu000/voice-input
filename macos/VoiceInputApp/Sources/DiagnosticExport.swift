import Foundation

enum DiagnosticExport {
    private static let redactedValue = "<redacted>"
    private static let sensitiveKeys: Set<String> = [
        "api_key",
        "api_key_value",
        "authorization",
        "last_text",
        "secret",
        "token",
    ]

    static func sanitizeJSON(_ data: Data) throws -> String {
        let object = try JSONSerialization.jsonObject(with: data)
        let sanitized = sanitize(object)
        let output = try JSONSerialization.data(
            withJSONObject: sanitized,
            options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
        )
        return String(decoding: output, as: UTF8.self)
    }

    private static func sanitize(_ value: Any) -> Any {
        if let dictionary = value as? [String: Any] {
            return dictionary.reduce(into: [String: Any]()) { result, pair in
                result[pair.key] = sensitiveKeys.contains(pair.key.lowercased())
                    ? redactedValue
                    : sanitize(pair.value)
            }
        }
        if let array = value as? [Any] {
            return array.map(sanitize)
        }
        return value
    }
}
