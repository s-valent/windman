public struct DesktopState: Equatable, Sendable {
  public let spaceIDs: Set<UInt64>
  public let isAnimating: Bool

  public var isStable: Bool {
    !isAnimating && !spaceIDs.isEmpty
  }

  public init(spaceIDs: Set<UInt64>, isAnimating: Bool) {
    self.spaceIDs = spaceIDs
    self.isAnimating = isAnimating
  }

  public func includes(windowSpaces: Set<UInt64>) -> Bool {
    isStable && !spaceIDs.isDisjoint(with: windowSpaces)
  }
}
