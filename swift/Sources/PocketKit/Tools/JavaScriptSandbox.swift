import Foundation
import JavaScriptCore

/// AGT-13: runs model-written JavaScript in a fresh `JSContext` (its own VM)
/// with nothing wired in but `console`/`print`: no network, no files, no
/// timers, no `require`. One run at a time per sandbox; call `run` off the
/// main thread.
///
/// The timeout is enforced by killing the script, not by asking it to stop:
/// JavaScriptCore has no public interrupt, so we use the VM's execution-time
/// watchdog (`JSContextGroupSetExecutionTimeLimit`, looked up at runtime), set
/// to tick every 50 ms. Each tick our callback returns true once the run is
/// past its deadline or cancelled, and JSC throws an uncatchable termination.
/// If that symbol is ever missing, the run still ends at the deadline for the
/// caller: the script keeps spinning on its own thread (abandoned) and
/// `abandonedRuns` goes up, but the answer is never held hostage.
public final class JavaScriptSandbox: @unchecked Sendable {
    public static let defaultTimeout: TimeInterval = 10
    public static let defaultOutputLimit = 20 * 1024

    private let timeout: TimeInterval
    private let outputLimit: Int
    private let lock = NSLock()
    private var current: RunState?
    private var _abandonedRuns = 0

    /// Runs whose thread could not be killed (watchdog symbol unavailable).
    public var abandonedRuns: Int { lock.withLock { _abandonedRuns } }

    public init(timeout: TimeInterval = JavaScriptSandbox.defaultTimeout,
                outputLimit: Int = JavaScriptSandbox.defaultOutputLimit) {
        self.timeout = timeout
        self.outputLimit = outputLimit
    }

    /// Stop: kills the run in progress, if any. The run returns an error
    /// result ("Stopped.").
    public func cancel() {
        lock.withLock { current?.cancelled = true }
    }

    /// Blocks until the script finishes or is killed. Never throws.
    public func run(_ code: String) -> ToolRunResult {
        let state = RunState(deadline: Date().addingTimeInterval(timeout), outputLimit: outputLimit)
        lock.withLock { current = state }
        defer { lock.withLock { if current === state { current = nil } } }

        let started = Date()
        let done = DispatchSemaphore(value: 0)
        let thread = Thread { [state] in
            Self.execute(code, state: state)
            done.signal()
        }
        thread.stackSize = 8 * 1024 * 1024  // deep recursion hits RangeError, not a crash
        thread.start()

        // The watchdog normally ends the run at `timeout`; the grace period
        // covers termination latency before we give up on the thread.
        let waited = done.wait(timeout: .now() + timeout + 2)
        if waited == .timedOut {
            state.lock.withLock { state.cancelled = true }  // keep trying to kill it
            lock.withLock { _abandonedRuns += 1 }
        }
        let ms = UInt64(max(0, Date().timeIntervalSince(started) * 1000))
        return state.result(durationMs: ms, timedOutAfter: timeout, abandoned: waited == .timedOut)
    }

    // MARK: - Running

    private static func execute(_ code: String, state: RunState) {
        guard let context = JSContext() else {
            state.lock.withLock { state.error = "JavaScript isn't available on this device." }
            return
        }
        context.exceptionHandler = { _, exception in
            state.lock.withLock { if state.error == nil { state.error = describe(exception) } }
        }
        let emit: @convention(block) (String) -> Void = { state.append($0) }
        // console + print, defined in JS so objects print as JSON like Node does.
        let prelude = """
        (function (emit) {
          function fmt(v) {
            if (typeof v === 'string') return v;
            if (v === undefined) return 'undefined';
            if (typeof v === 'function' || typeof v === 'symbol') return String(v);
            if (v instanceof Error) return String(v.stack || v);
            try { var s = JSON.stringify(v); return s === undefined ? String(v) : s; }
            catch (e) { return String(v); }
          }
          function log() { emit(Array.prototype.map.call(arguments, fmt).join(' ')); }
          globalThis.console = { log: log, info: log, debug: log, warn: log, error: log };
          globalThis.print = log;
          return fmt;
        })
        """
        guard let bind = context.evaluateScript(prelude),
              let fmt = bind.call(withArguments: [unsafeBitCast(emit, to: AnyObject.self)]),
              !fmt.isUndefined
        else {
            state.lock.withLock { state.error = "JavaScript isn't available on this device." }
            return
        }

        state.armWatchdog(on: context)
        let value = context.evaluateScript(code)
        // Anything the script queued as microtasks (promises) has run by now.
        if state.error == nil, let value, !value.isUndefined,
           let text = fmt.call(withArguments: [value])?.toString() {
            state.append("=> " + text)
        }
        // A kill is not a script error; the caller words it.
        if state.killed { state.lock.withLock { state.error = nil } }
    }

    /// "ReferenceError: Can't find variable: x (line 3)".
    static func describe(_ exception: JSValue?) -> String {
        guard let exception else { return "The script failed." }
        var text = exception.toString() ?? "The script failed."
        if let line = exception.objectForKeyedSubscript("line"), line.isNumber {
            text += " (line \(Int(line.toInt32())))"
        }
        return text
    }
}

/// One run's shared state: output buffer, flags, and the watchdog callback.
private final class RunState: @unchecked Sendable {
    let lock = NSLock()
    let deadline: Date
    let outputLimit: Int
    var cancelled = false
    private(set) var killed = false
    private var timedOut = false
    var error: String?
    private var output = ""
    private var bytes = 0
    private var truncated = false

    init(deadline: Date, outputLimit: Int) {
        self.deadline = deadline
        self.outputLimit = outputLimit
    }

    /// Appends a line, capped at `outputLimit` UTF-8 bytes. Once full,
    /// further output is dropped (the script may keep printing until killed).
    func append(_ line: String) {
        lock.withLock {
            guard !truncated else { return }
            let text = (output.isEmpty ? "" : "\n") + line
            let room = outputLimit - bytes
            if text.utf8.count <= room {
                output += text
                bytes += text.utf8.count
                return
            }
            var kept = ""
            var used = 0
            for ch in text {
                let n = String(ch).utf8.count
                if used + n > room { break }
                kept.append(ch)
                used += n
            }
            output += kept
            bytes += used
            truncated = true
        }
    }

    func result(durationMs: UInt64, timedOutAfter: TimeInterval, abandoned: Bool) -> ToolRunResult {
        lock.withLock {
            var error = self.error ?? ""
            if timedOut || abandoned {
                error = "Stopped after \(Int(timedOutAfter)) seconds: the script ran too long."
            } else if cancelled && killed {
                error = "Stopped."
            }
            return ToolRunResult(ok: error.isEmpty, output: output, error: error,
                                 durationMs: durationMs, truncated: truncated)
        }
    }

    // MARK: Watchdog

    private typealias ShouldTerminate = @convention(c) (OpaquePointer?, UnsafeMutableRawPointer?) -> Bool
    private typealias SetLimit = @convention(c) (OpaquePointer?, Double, ShouldTerminate?, UnsafeMutableRawPointer?) -> Void

    /// JSC's watchdog: private SPI, so looked up at runtime. Nil if absent.
    private static let setLimit: SetLimit? = {
        // RTLD_DEFAULT
        guard let symbol = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "JSContextGroupSetExecutionTimeLimit") else { return nil }
        return unsafeBitCast(symbol, to: SetLimit.self)
    }()
    private static let tick = 0.05

    /// Called on the script's thread by JSC when the limit elapses. True kills
    /// the script; false must re-arm, because JSC fires the callback once.
    private static let shouldTerminate: ShouldTerminate = { ctx, info in
        let state = Unmanaged<RunState>.fromOpaque(info!).takeUnretainedValue()
        let kill = state.lock.withLock {
            if state.cancelled {
                state.killed = true
            } else if Date() >= state.deadline {
                state.killed = true
                state.timedOut = true
            }
            return state.killed
        }
        if !kill, let ctx { setLimit?(JSContextGetGroup(ctx), tick, shouldTerminate, info) }
        return kill
    }

    func armWatchdog(on context: JSContext) {
        guard let setLimit = Self.setLimit else { return }
        // The unretained pointer is safe: `RunState` outlives the script's thread.
        setLimit(JSContextGetGroup(context.jsGlobalContextRef), Self.tick, Self.shouldTerminate,
                 Unmanaged.passUnretained(self).toOpaque())
    }
}
