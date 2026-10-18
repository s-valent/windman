import CoreGraphics
import Foundation

public struct WindowSnapshot: Equatable, Sendable {
  public let id: UInt32
  public let processID: Int32
  public let bounds: CGRect

  public init(id: UInt32, processID: Int32, bounds: CGRect) {
    self.id = id
    self.processID = processID
    self.bounds = bounds
  }
}

public struct RaisePlan: Equatable, Sendable {
  public let target: WindowSnapshot
  public let protectedWindows: [WindowSnapshot]

  public var backToFront: [WindowSnapshot] {
    [target] + protectedWindows.reversed()
  }

  public func windowsToRaise(in currentOrder: [UInt32]) -> [WindowSnapshot] {
    let desired = protectedWindows + [target]
    let count =
      (0...desired.count).first { count in
        let moved = Set(desired.prefix(count).map(\.id))
        let remaining = currentOrder.filter { !moved.contains($0) }
        return Array(remaining.prefix(desired.count - count)) == desired.dropFirst(count).map(\.id)
      } ?? desired.count
    return Array(desired.prefix(count).reversed())
  }

  public init?(targetID: UInt32, frontToBack: [WindowSnapshot]) {
    guard let index = frontToBack.firstIndex(where: { $0.id == targetID }) else {
      return nil
    }
    let target = frontToBack[index]
    self.target = target
    protectedWindows = frontToBack.prefix(index).filter {
      !$0.bounds.isEmpty && target.bounds.contains($0.bounds)
    }
  }
}
