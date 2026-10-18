public struct MouseInteraction: Equatable, Sendable {
  public let buttonEventCount: UInt64
  public let isPressed: Bool

  public init(buttonEventCount: UInt64 = 0, isPressed: Bool = false) {
    self.buttonEventCount = buttonEventCount
    self.isPressed = isPressed
  }
}
