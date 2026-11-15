import Foundation

/// Text classification per spec/behavior.md §3. The indentation signal is
/// tested against raw text; all other signals against trimmed text.
public enum Classifier {
    private static let keywordSignals = [
        "const ", "let ", "var ", "function ", "def ", "class ", "import ",
        "#include", "</", "/>",
    ]

    public static func classify(_ raw: String) -> ItemType {
        let t = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if isHexColor(t) || t.hasPrefix("rgb(") || t.hasPrefix("rgba(") {
            return .color
        }
        if isLink(t) {
            return .link
        }
        var score = 0
        if t.contains(where: { ";{}()[]".contains($0) }) {
            score += 1
        }
        if keywordSignals.contains(where: { t.contains($0) }) {
            score += 1
        }
        if t.contains("\n") && (raw.contains("\t") || raw.contains("  ")) {
            score += 1
        }
        return score >= 2 ? .code : .text
    }

    /// `^#[0-9a-fA-F]{3,8}$` on trimmed text.
    static func isHexColor(_ t: String) -> Bool {
        guard t.hasPrefix("#") else { return false }
        let rest = String(t.dropFirst())
        let n = rest.unicodeScalars.count
        return (3...8).contains(n) && rest.unicodeScalars.allSatisfy {
            CharacterSet.hexadecimal.contains($0)
        }
    }

    /// `^(https?:\/\/|www\.)\S+$` on trimmed text.
    static func isLink(_ t: String) -> Bool {
        let rest: Substring
        if t.hasPrefix("https://") { rest = t.dropFirst(8) }
        else if t.hasPrefix("http://") { rest = t.dropFirst(7) }
        else if t.hasPrefix("www.") { rest = t.dropFirst(4) }
        else { return false }
        return !rest.isEmpty && !rest.contains(where: { $0.isWhitespace })
    }
}

extension CharacterSet {
    static let hexadecimal = CharacterSet(charactersIn: "0123456789abcdefABCDEF")
}
