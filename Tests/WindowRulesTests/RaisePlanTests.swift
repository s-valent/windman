import CoreGraphics
import Foundation
import Testing
import WindowRules

@Test func preservesContainedWindowsAndPassesPartialOverlaps() throws {
  let contained = WindowSnapshot(
    id: 1, processID: 1, bounds: CGRect(x: 20, y: 20, width: 40, height: 40))
  let partial = WindowSnapshot(
    id: 2, processID: 2, bounds: CGRect(x: 80, y: 0, width: 100, height: 100))
  let target = WindowSnapshot(
    id: 3, processID: 3, bounds: CGRect(x: 0, y: 0, width: 100, height: 100))
  let behind = WindowSnapshot(
    id: 4, processID: 4, bounds: CGRect(x: 10, y: 10, width: 20, height: 20))
  let plan = try #require(
    RaisePlan(targetID: 3, frontToBack: [contained, partial, target, behind]))

  #expect(plan.backToFront.map(\.id) == [3, 1])
}

@Test func preservesRelativeOrderIncludingEqualBounds() throws {
  let bounds = CGRect(x: -500, y: 100, width: 400, height: 300)
  let windows = (1...3).map {
    WindowSnapshot(id: UInt32($0), processID: Int32($0), bounds: bounds)
  }
  let plan = try #require(RaisePlan(targetID: 3, frontToBack: windows))

  #expect(plan.backToFront.map(\.id) == [3, 2, 1])
}

@Test func missingTargetDoesNotProducePlan() {
  #expect(RaisePlan(targetID: 1, frontToBack: []) == nil)
}

@Test func computesOnlyNecessaryRaisesForCurrentOrder() throws {
  let bounds = CGRect(x: 0, y: 0, width: 100, height: 100)
  let windows = [1, 2, 3].map {
    WindowSnapshot(id: UInt32($0), processID: Int32($0), bounds: bounds)
  }
  let plan = try #require(RaisePlan(targetID: 3, frontToBack: windows))

  #expect(plan.windowsToRaise(in: [1, 2, 3, 4]).isEmpty)
  #expect(plan.windowsToRaise(in: [2, 1, 3, 4]).map(\.id) == [1])
  #expect(plan.windowsToRaise(in: [3, 1, 2, 4]).map(\.id) == [2, 1])
  #expect(plan.windowsToRaise(in: [4, 1, 2, 3]).map(\.id) == [3, 2, 1])
}
