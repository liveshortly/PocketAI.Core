import SwiftUI

/// Lightweight, dependency-free syntax highlighter for fenced code blocks.
/// Regex-tokenizes a handful of common languages and returns an
/// `AttributedString` coloured from PocketUI tokens (no raw hex). Unknown
/// languages fall back to a generic keyword set; worst case is plain text.
///
/// Priority (first match wins per character): comments → strings → numbers →
/// keywords → types. This keeps keywords inside strings/comments uncoloured.
public enum SyntaxHighlighter {

    public static func highlight(_ code: String, language: String?) -> AttributedString {
        let ns = code as NSString
        let n = ns.length
        guard n > 0 else { return AttributedString(code) }

        var colors = [Color?](repeating: nil, count: n)

        func apply(_ pattern: String, _ color: Color, _ options: NSRegularExpression.Options = []) {
            guard let re = try? NSRegularExpression(pattern: pattern, options: options) else { return }
            re.enumerateMatches(in: code, options: [], range: NSRange(location: 0, length: n)) { match, _, _ in
                guard let r = match?.range, r.location != NSNotFound else { return }
                let end = min(r.location + r.length, n)
                var i = r.location
                while i < end {
                    if colors[i] == nil { colors[i] = color }
                    i += 1
                }
            }
        }

        let lang = (language ?? "").lowercased()

        // 1. Comments (highest priority so keywords inside them stay grey).
        apply(#"/\*[\s\S]*?\*/"#, PocketColor.textTertiary)          // block /* */
        apply(#"//[^\n]*"#, PocketColor.textTertiary)                // line //
        if ["python", "py", "bash", "sh", "shell", "zsh", "ruby", "rb", "yaml", "yml", "toml", "r"].contains(lang) {
            apply(##"#[^\n]*"##, PocketColor.textTertiary)           // line #
        }

        // 2. Strings.
        apply(#""(?:\\.|[^"\\\n])*""#, PocketColor.statusOk)         // "..."
        apply(#"'(?:\\.|[^'\\\n])*'"#, PocketColor.statusOk)         // '...'
        apply("`(?:\\\\.|[^`\\\\])*`", PocketColor.statusOk)         // `...` template

        // 3. Numbers.
        apply(#"\b0[xX][0-9a-fA-F]+\b"#, PocketColor.statusWarn)
        apply(#"\b\d[\d_]*(?:\.\d+)?(?:[eE][+-]?\d+)?\b"#, PocketColor.statusWarn)

        // 4. Keywords (language-aware).
        let kws = keywords(for: lang)
        if !kws.isEmpty {
            let alternation = kws.map { NSRegularExpression.escapedPattern(for: $0) }.joined(separator: "|")
            apply("\\b(?:\(alternation))\\b", PocketColor.accent)
        }

        // 5. Types / capitalised identifiers.
        apply(#"\b[A-Z][A-Za-z0-9_]*\b"#, PocketColor.statusInfo)

        // Coalesce runs of equal colour into one AttributedString each.
        var out = AttributedString()
        var i = 0
        while i < n {
            let c = colors[i]
            var j = i + 1
            while j < n && colors[j] == c { j += 1 }
            var run = AttributedString(ns.substring(with: NSRange(location: i, length: j - i)))
            run.foregroundColor = c ?? PocketColor.textPrimary
            out.append(run)
            i = j
        }
        return out
    }

    private static func keywords(for lang: String) -> [String] {
        switch lang {
        case "swift":
            return ["func", "let", "var", "if", "else", "guard", "return", "struct", "class", "enum", "protocol",
                    "extension", "import", "for", "in", "while", "switch", "case", "default", "break", "continue",
                    "do", "try", "catch", "throw", "throws", "async", "await", "public", "private", "internal",
                    "fileprivate", "static", "self", "nil", "true", "false", "init", "deinit", "some", "any",
                    "where", "as", "is", "weak", "unowned", "lazy", "mutating", "override", "final", "defer"]
        case "python", "py":
            return ["def", "class", "if", "elif", "else", "return", "import", "from", "as", "for", "in", "while",
                    "try", "except", "finally", "with", "lambda", "yield", "pass", "break", "continue", "and",
                    "or", "not", "is", "None", "True", "False", "self", "async", "await", "global", "nonlocal", "raise"]
        case "javascript", "js", "typescript", "ts", "jsx", "tsx":
            return ["function", "const", "let", "var", "if", "else", "return", "class", "extends", "import",
                    "from", "export", "default", "for", "while", "switch", "case", "break", "continue", "new",
                    "try", "catch", "finally", "throw", "async", "await", "yield", "typeof", "instanceof", "this",
                    "null", "undefined", "true", "false", "of", "in", "interface", "type", "enum", "public", "private"]
        case "rust", "rs":
            return ["fn", "let", "mut", "if", "else", "match", "return", "struct", "enum", "trait", "impl", "use",
                    "mod", "pub", "for", "in", "while", "loop", "break", "continue", "self", "Self", "async",
                    "await", "move", "ref", "as", "where", "const", "static", "dyn", "unsafe", "true", "false", "crate"]
        case "go":
            return ["func", "var", "const", "if", "else", "return", "struct", "interface", "type", "import",
                    "package", "for", "range", "switch", "case", "default", "break", "continue", "go", "defer",
                    "chan", "map", "nil", "true", "false", "select", "fallthrough"]
        case "json":
            return ["true", "false", "null"]
        case "bash", "sh", "shell", "zsh":
            return ["if", "then", "else", "elif", "fi", "for", "in", "do", "done", "while", "case", "esac",
                    "function", "return", "export", "local", "echo", "cd", "source"]
        case "":
            return []
        default:
            // Generic union for unknown languages.
            return ["function", "func", "def", "class", "struct", "enum", "if", "else", "return", "import",
                    "for", "while", "const", "let", "var", "true", "false", "null", "nil", "public", "private"]
        }
    }
}
