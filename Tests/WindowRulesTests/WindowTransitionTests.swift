import CoreGraphics
import Testing
import WindowRules

private enum SimulatedWindowError: Error {
  case unsupported
}

private enum WindowAction: Equatable {
  case raise(UInt32)
  case focus(UInt32)
}

@MainActor
private final class WindowScene {
  var order: [UInt32]
  var focused: UInt32?
  var unavailable: Set<UInt32> = []
  var focusOnRaise: Set<UInt32> = []
  var raiseCounts: [UInt32: Int] = [:]
  var prepareRequests: [UInt32] = []
  var focusRequests: [UInt32] = []
  var raiseOnFirstFocus = false
  var actions: [WindowAction] = []

  init(order: [UInt32]) {
    self.order = order
  }

  func prepare(_ window: WindowSnapshot) throws {
    prepareRequests.append(window.id)
    guard !unavailable.contains(window.id) else { throw SimulatedWindowError.unsupported }
  }

  func raise(_ window: WindowSnapshot) async throws {
    guard !unavailable.contains(window.id) else { throw SimulatedWindowError.unsupported }
    raiseCounts[window.id, default: 0] += 1
    actions.append(.raise(window.id))
    order.removeAll { $0 == window.id }
    order.insert(window.id, at: 0)
    if focusOnRaise.contains(window.id) {
      focused = window.id
    }
  }

  func focus(_ window: WindowSnapshot) async throws {
    focusRequests.append(window.id)
    guard focused != window.id else { return }
    actions.append(.focus(window.id))
    if focused == nil && raiseOnFirstFocus {
      try await raise(window)
    }
    focused = window.id
  }
}

@Test @MainActor func keepsProtectedWindowsAboveTargetAfterActivationRaisesIt() async throws {
  let bounds = CGRect(x: 0, y: 0, width: 100, height: 100)
  let protected = WindowSnapshot(
    id: 1, processID: 1, bounds: CGRect(x: 10, y: 10, width: 20, height: 20))
  let partial = WindowSnapshot(
    id: 2, processID: 2, bounds: CGRect(x: 80, y: 0, width: 100, height: 100))
  let target = WindowSnapshot(id: 3, processID: 3, bounds: bounds)
  let plan = try #require(RaisePlan(targetID: 3, frontToBack: [protected, partial, target]))
  let scene = WindowScene(order: [1, 2, 3])
  scene.raiseOnFirstFocus = true

  try await WindowTransition.perform(
    plan, currentOrder: { scene.order }, prepare: scene.prepare, raise: scene.raise,
    focus: scene.focus)

  #expect(scene.order == [1, 3, 2])
  #expect(scene.focused == 3)
  #expect(scene.raiseCounts[1] == 2)
}

@Test @MainActor func failedPreparationDoesNotCoverProtectedWindow() async throws {
  let bounds = CGRect(x: 0, y: 0, width: 100, height: 100)
  let windows = [1, 2].map { WindowSnapshot(id: UInt32($0), processID: Int32($0), bounds: bounds) }
  let partial = WindowSnapshot(
    id: 3, processID: 3, bounds: CGRect(x: 80, y: 0, width: 100, height: 100))
  let plan = try #require(RaisePlan(targetID: 2, frontToBack: [windows[0], partial, windows[1]]))
  let scene = WindowScene(order: [1, 3, 2])
  scene.unavailable = [1]

  await #expect(throws: SimulatedWindowError.self) {
    try await WindowTransition.perform(
      plan, currentOrder: { scene.order }, prepare: scene.prepare, raise: scene.raise,
      focus: scene.focus)
  }

  #expect(scene.order == [1, 3, 2])
  #expect(scene.focused == nil)
}

@Test @MainActor func preservesDialogAndRestoresTargetFocusAfterDialogActivation() async throws {
  let dialog = WindowSnapshot(
    id: 1, processID: 1, bounds: CGRect(x: 10, y: 10, width: 20, height: 20))
  let target = WindowSnapshot(
    id: 2, processID: 2, bounds: CGRect(x: 0, y: 0, width: 100, height: 100))
  let partial = WindowSnapshot(
    id: 3, processID: 3, bounds: CGRect(x: 80, y: 0, width: 100, height: 100))
  let plan = try #require(RaisePlan(targetID: 2, frontToBack: [dialog, partial, target]))
  let scene = WindowScene(order: [1, 3, 2])
  scene.focusOnRaise = [1]

  try await WindowTransition.perform(
    plan, currentOrder: { scene.order }, prepare: scene.prepare, raise: scene.raise,
    focus: scene.focus)

  #expect(scene.order == [1, 2, 3])
  #expect(scene.focused == 2)
  #expect(scene.actions == [.raise(2), .raise(1), .focus(2)])
}

@Test @MainActor func raisesBeforeFocusingWhenStackMustChange() async throws {
  let front = WindowSnapshot(
    id: 1, processID: 1, bounds: CGRect(x: 80, y: 0, width: 100, height: 100))
  let target = WindowSnapshot(
    id: 2, processID: 2, bounds: CGRect(x: 0, y: 0, width: 100, height: 100))
  let plan = try #require(RaisePlan(targetID: 2, frontToBack: [front, target]))
  let scene = WindowScene(order: [1, 2])
  scene.focused = 1

  try await WindowTransition.perform(
    plan, currentOrder: { scene.order }, prepare: scene.prepare, raise: scene.raise,
    focus: scene.focus)

  #expect(scene.order == [2, 1])
  #expect(scene.focused == 2)
  #expect(scene.actions == [.raise(2), .focus(2)])
}

@Test @MainActor func raisingCanSupplyFocusWithoutSeparateFocusAction() async throws {
  let front = WindowSnapshot(
    id: 1, processID: 1, bounds: CGRect(x: 80, y: 0, width: 100, height: 100))
  let target = WindowSnapshot(
    id: 2, processID: 2, bounds: CGRect(x: 0, y: 0, width: 100, height: 100))
  let plan = try #require(RaisePlan(targetID: 2, frontToBack: [front, target]))
  let scene = WindowScene(order: [1, 2])
  scene.focused = 1
  scene.focusOnRaise = [2]

  try await WindowTransition.perform(
    plan, currentOrder: { scene.order }, prepare: scene.prepare, raise: scene.raise,
    focus: scene.focus)

  #expect(scene.order == [2, 1])
  #expect(scene.focused == 2)
  #expect(scene.actions == [.raise(2)])
}

@Test @MainActor func correctStackOnlyChangesFocusEvenIfProtectedWindowCannotRaise() async throws {
  let dialog = WindowSnapshot(
    id: 1, processID: 1, bounds: CGRect(x: 10, y: 10, width: 20, height: 20))
  let target = WindowSnapshot(
    id: 2, processID: 2, bounds: CGRect(x: 0, y: 0, width: 100, height: 100))
  let plan = try #require(RaisePlan(targetID: 2, frontToBack: [dialog, target]))
  let scene = WindowScene(order: [1, 2])
  scene.focused = 1
  scene.unavailable = [1]

  try await WindowTransition.perform(
    plan, currentOrder: { scene.order }, prepare: scene.prepare, raise: scene.raise,
    focus: scene.focus)

  #expect(scene.order == [1, 2])
  #expect(scene.focused == 2)
  #expect(scene.raiseCounts.isEmpty)
  #expect(scene.prepareRequests.isEmpty)
  #expect(scene.focusRequests == [2])
}

@Test @MainActor func repairsOnlyProtectedWindowIfActivationUnexpectedlyRaisesTarget() async throws
{
  let bounds = CGRect(x: 0, y: 0, width: 100, height: 100)
  let windows = [1, 2].map { WindowSnapshot(id: UInt32($0), processID: Int32($0), bounds: bounds) }
  let plan = try #require(RaisePlan(targetID: 2, frontToBack: windows))
  let scene = WindowScene(order: [1, 2])
  scene.raiseOnFirstFocus = true

  try await WindowTransition.perform(
    plan, currentOrder: { scene.order }, prepare: scene.prepare, raise: scene.raise,
    focus: scene.focus)

  #expect(scene.order == [1, 2])
  #expect(scene.focused == 2)
  #expect(scene.prepareRequests == [1])
  #expect(scene.raiseCounts[1] == 1)
  #expect(scene.raiseCounts[2] == 1)
}
