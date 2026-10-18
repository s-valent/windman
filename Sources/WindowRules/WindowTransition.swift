import Foundation

public enum WindowTransition {
  @MainActor
  public static func perform(
    _ plan: RaisePlan,
    currentOrder: () -> [UInt32],
    prepare: (WindowSnapshot) throws -> Void,
    raise: (WindowSnapshot) async throws -> Void,
    focus: (WindowSnapshot) async throws -> Void
  ) async throws {
    let initialMoves = plan.windowsToRaise(in: currentOrder())
    try initialMoves.forEach(prepare)
    for window in initialMoves {
      try await raise(window)
    }
    try await focus(plan.target)
    let moves = plan.windowsToRaise(in: currentOrder())
    let prepared = Set(initialMoves.map(\.id))
    try moves.filter { !prepared.contains($0.id) }.forEach(prepare)
    for window in moves {
      try await raise(window)
    }
    if !moves.isEmpty {
      try await focus(plan.target)
    }
  }
}
