import XCTest
@testable import PocketLiveShortly

final class LSLoginProblemTests: XCTestCase {
    private let refused = #"login failed: device start: Post "http://localhost:8080/auth/device/start": dial tcp [::1]:8080: connect: connection refused"#

    func testDeadLocalhostIsUnreachableAndSaysSo() {
        let p = LSLoginProblem(loginOutput: refused, server: "http://localhost:8080")
        XCTAssertEqual(p?.kind, .unreachable)
        XCTAssertTrue(p?.message.contains("local server that isn't running") == true)
        XCTAssertTrue(p?.message.contains("localhost:8080") == true)
    }

    func testMapping() {
        let prod = LSServer.production
        func kind(_ out: String) -> LSLoginProblem.Kind? { LSLoginProblem(loginOutput: out, server: prod)?.kind }
        XCTAssertEqual(kind("login failed: device start: dial tcp: lookup liveshortly.com: no such host"), .unreachable)
        XCTAssertEqual(kind("login failed: device start: tls: failed to verify certificate: x509: expired"), .serverTrouble)
        XCTAssertEqual(kind("login failed: device start: HTTP 502"), .serverTrouble)
        XCTAssertEqual(kind("login failed: device start: bad response (HTTP 404)"), .clientTooOld)
        XCTAssertEqual(kind("login failed: device start: HTTP 410"), .clientTooOld)
        XCTAssertEqual(kind("login failed: device login timed out — try again"), .other)
        XCTAssertNil(kind("✓ me@example.com (https://liveshortly.com)"))
        XCTAssertEqual(LSLoginProblem(kind: .unreachable, server: prod).message,
                       "Can't reach liveshortly.com. Check your connection.")
    }

    func testServerLabel() {
        XCTAssertEqual(LSServer.label("https://liveshortly.com/"), "liveshortly.com")
        XCTAssertEqual(LSServer.label("http://localhost:8080"), "localhost:8080 · custom")
    }

    func testResolveOrder() throws {
        let home = NSTemporaryDirectory() + UUID().uuidString
        XCTAssertEqual(LSServer.resolve(home: home, env: [:]), LSServer.production)
        XCTAssertEqual(LSServer.resolve(home: home, env: ["LIVESHORTLY_API_URL": "https://x.test/"]), "https://x.test")
        try FileManager.default.createDirectory(atPath: home + "/.liveshortly", withIntermediateDirectories: true)
        try Data(#"{"api_url":"http://localhost:8080"}"#.utf8).write(to: URL(fileURLWithPath: home + "/.liveshortly/host.json"))
        XCTAssertEqual(LSServer.resolve(home: home, env: ["LIVESHORTLY_API_URL": "https://x.test"]), "http://localhost:8080")
    }
}
