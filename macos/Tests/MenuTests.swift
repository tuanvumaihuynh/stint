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
        XCTAssertEqual(menu.entries.first, .header(status: "7 accounts · 2 need attention", dot: .attention))
        XCTAssertFalse(menu.offline)
        XCTAssertEqual(menu.statusText, "77%")
    }

    func testFirstAccountIsPinnedByDefault() throws {
        let menu = try XCTUnwrap(mockMenu(scenario: "default"))
        let pinned = menu.entries.compactMap { entry -> String? in
            if case let .account(_, name, _, _, pinned) = entry, pinned { return name }
            return nil
        }
        XCTAssertEqual(pinned, ["Personal"])
    }

    func testCommandsHaveTitlesAndKeys() {
        XCTAssertEqual(commandTitle(command: .settings), "Settings…")
        XCTAssertEqual(commandKey(command: .quit), "q")
    }
}
