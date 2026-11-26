import XCTest

@testable import Stint

/// Calls through the generated bindings into the Rust core, so CI covers the link.
final class MenuTests: XCTestCase {
    func testEveryMockScenarioHasAMenu() throws {
        let scenarios = mockScenarios()
        XCTAssertEqual(scenarios.first, "default")
        for scenario in scenarios {
            XCTAssertNotNil(mockMenu(scenario: scenario), scenario)
        }
    }

    func testDefaultMenuStartsWithTheHeader() throws {
        let menu = try XCTUnwrap(mockMenu(scenario: "default"))
        XCTAssertEqual(menu.entries.first, .header(status: "6 accounts · 2 need attention", dot: .attention))
        XCTAssertFalse(menu.offline)
    }

    func testCommandsHaveTitlesAndKeys() {
        XCTAssertEqual(commandTitle(command: .settings), "Settings…")
        XCTAssertEqual(commandKey(command: .quit), "q")
    }
}
