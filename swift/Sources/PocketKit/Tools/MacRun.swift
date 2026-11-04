import Foundation

/// AGT-13 stage 2: what the iPhone app supplies so the model can run code on
/// the paired Mac. The app owns the peer state (which Mac, is it connected).
public protocol MacCodeRunning: Sendable {
    /// A Mac is paired **and** currently connected. Read at the start of every
    /// reply, from a background thread.
    var isMacConnected: Bool { get }
    /// Sends the run to the Mac and waits. Never throws: failures are a
    /// `ToolRunResult` with plain wording for the model and the user.
    func run(language: String, code: String, timeoutMs: UInt32) async -> ToolRunResult
}

/// Turns the Mac's `POST /pocket/v1/run` replies into `ToolRunResult`s. The
/// wording never mentions licences or "Pro" (owner rule: none on the iPhone).
public enum MacRunReply {
    public static let settingOff = String(localized: "Your Mac isn't set up to run code from this iPhone. Turn on 'Let my paired iPhone run code' in PocketAI on your Mac.", bundle: .module)
    public static let notAvailable = String(localized: "This isn't available with your Mac yet.", bundle: .module)
    public static let unreachable = String(localized: "Your Mac isn't reachable.", bundle: .module)

    /// `language` is the Mac's name for it: python, javascript or shell.
    public static func request(language: String, code: String, timeoutMs: UInt32) -> [String: Any] {
        ["language": language, "code": code, "timeout_ms": Int(timeoutMs)]
    }

    public static func failure(_ message: String) -> ToolRunResult {
        ToolRunResult(ok: false, output: "", error: message, durationMs: 0, truncated: false)
    }

    public static func result(status: Int, body: Data) -> ToolRunResult {
        let json = (try? JSONSerialization.jsonObject(with: body)) as? [String: Any] ?? [:]
        if status == 200 {
            return ToolRunResult(
                ok: json["ok"] as? Bool ?? false,
                output: json["output"] as? String ?? "",
                error: json["error"] as? String ?? "",
                durationMs: (json["duration_ms"] as? NSNumber)?.uint64Value ?? 0,
                truncated: json["truncated"] as? Bool ?? false)
        }
        let err = json["error"] as? [String: Any]
        let code = err?["code"] as? String ?? ""
        let message = err?["message"] as? String ?? ""
        switch code {
        case "run_disabled": return failure(settingOff)
        case "not_entitled": return failure(notAvailable)
        case "unauthorized": return failure(String(localized: "Your Mac doesn't recognise this iPhone. Pair it again from the Mac tab.", bundle: .module))
        case "disconnected_by_host": return failure(String(localized: "Your Mac disconnected this iPhone. Connect it from the Mac tab.", bundle: .module))
        case "busy": return failure(String(localized: "Your Mac is already running something. Try again in a moment.", bundle: .module))
        case "rate_limited": return failure(String(localized: "Too many runs in a short time. Try again in a minute.", bundle: .module))
        case "code_too_large": return failure(String(localized: "That code is too large to send to your Mac.", bundle: .module))
        case "interpreter_missing": return failure(message.isEmpty ? String(localized: "Your Mac can't run that language.", bundle: .module) : message)
        default: return failure(String(localized: "Your Mac couldn't run that.", bundle: .module))
        }
    }
}

public extension PeerClient {
    /// Runs `code` on the paired Mac (`language` python, javascript or shell).
    func runOnMac(language: String, code: String, timeoutMs: UInt32) async -> ToolRunResult {
        do {
            let (data, status) = try await postJSONWithStatus(
                path: "/pocket/v1/run",
                body: MacRunReply.request(language: language, code: code, timeoutMs: timeoutMs))
            return MacRunReply.result(status: status, body: data)
        } catch is CancellationError {
            return MacRunReply.failure(String(localized: "Stopped.", bundle: .module))
        } catch let error as URLError where error.code == .cancelled {
            return MacRunReply.failure(String(localized: "Stopped.", bundle: .module))
        } catch PeerClientError.disconnected {
            return MacRunReply.failure(String(localized: "This Mac is disconnected. Connect it from the Mac tab.", bundle: .module))
        } catch {
            return MacRunReply.failure(MacRunReply.unreachable)
        }
    }
}
