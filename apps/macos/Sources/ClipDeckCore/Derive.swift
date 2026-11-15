import Foundation

/// Preview/title derivation per spec/behavior.md §4. Truncation counts
/// Unicode scalar values and appends U+2026 only when truncation occurs.
public enum Derive {
    public static let titleCap = 60
    public static let previewCap = 120
    public static let linkFallbackCap = 80

    /// Truncate to `cap` Unicode scalars, appending `…` when shortened.
    public static func truncateScalars(_ s: String, cap: Int) -> String {
        let scalars = Array(s.unicodeScalars)
        if scalars.count <= cap { return s }
        return String(String.UnicodeScalarView(scalars[0..<cap])) + ellipsis
    }

    /// (title, preview) for a stored item.
    public static func pair(_ type: ItemType, content: String, dims: (UInt32, UInt32)?) -> (String, String) {
        switch type {
        case .link:
            let host = linkHost(content) ?? truncateScalars(content, cap: linkFallbackCap)
            return (host, host)
        case .color:
            let t = content.trimmingCharacters(in: .whitespacesAndNewlines)
            return (t, t)
        case .image:
            let (w, h) = dims ?? (0, 0)
            return ("Image", "\(w)\u{d7}\(h) image")
        case .file:
            let lines = content.split(separator: "\n", omittingEmptySubsequences: false)
            let label = lines.count == 1
                ? String(lines[0].split(separator: "/").last ?? lines[0])
                : "\(lines.count) files"
            return (label, label)
        case .text, .code, .html:
            let first = String(content.split(separator: "\n", omittingEmptySubsequences: false).first ?? "")
                .trimmingCharacters(in: .whitespacesAndNewlines)
            var title = truncateScalars(first, cap: titleCap)
            if title.isEmpty { title = type.rawValue }
            return (title, truncateScalars(first, cap: previewCap))
        }
    }

    /// Hostname of a URL with leading `www.` removed (lowercased, URL
    /// semantics). `nil` on parse failure selects the 80-scalar fallback.
    static func linkHost(_ content: String) -> String? {
        let lower = content.lowercased()
        let afterScheme: Substring
        if lower.hasPrefix("https://") { afterScheme = content.dropFirst(8) }
        else if lower.hasPrefix("http://") { afterScheme = content.dropFirst(7) }
        else { return nil }
        let authority = afterScheme.split(whereSeparator: { "/" .contains($0) || "?" .contains($0) || "#" .contains($0) }).first
        guard let authority, !authority.isEmpty else { return nil }
        let hostPort = authority.split(separator: "@").last ?? authority[...]
        guard let host = hostPort.split(separator: ":").first, !host.isEmpty else { return nil }
        let hostStr = String(host).lowercased()
        return hostStr.hasPrefix("www.") ? String(hostStr.dropFirst(4)) : hostStr
    }
}
