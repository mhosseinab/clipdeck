import Foundation

/// Scripting hooks per spec/behavior.md §8.
public enum Hooks {
    /// Parse one UI line `event=command` (split at the FIRST `=`, both trimmed).
    public static func parseLine(_ line: String) -> Hook? {
        guard let eq = line.firstIndex(of: "="), eq != line.startIndex else { return nil }
        let event = String(line[..<eq]).trimmingCharacters(in: .whitespaces)
        let command = String(line[line.index(after: eq)...]).trimmingCharacters(in: .whitespaces)
        guard (event == "copy" || event == "paste"), !command.isEmpty else { return nil }
        return Hook(event: event, command: command)
    }

    /// Run all hooks for `event` concurrently, feeding `content` to stdin.
    /// Silent failures; each command is killed after 5 s.
    public static func run(event: String, content: String, hooks: [Hook]) {
        for hook in hooks where hook.event == event && !hook.command.trimmingCharacters(in: .whitespaces).isEmpty {
            let p = Process()
            p.executableURL = URL(fileURLWithPath: "/bin/sh")
            p.arguments = ["-c", hook.command]
            let stdin = Pipe()
            p.standardInput = stdin
            p.standardOutput = Pipe()
            p.standardError = Pipe()
            do {
                try p.run()
                stdin.fileHandleForWriting.write(Data(content.utf8))
                try? stdin.fileHandleForWriting.close()
                DispatchQueue.global().async {
                    let deadline = Date().addingTimeInterval(5)
                    while p.isRunning && Date() < deadline {
                        usleep(20_000)
                    }
                    if p.isRunning {
                        p.terminate()
                    }
                }
            } catch {
                // silent per spec
            }
        }
    }
}
