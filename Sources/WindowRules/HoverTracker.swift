public enum HoverDecision: Equatable, Sendable {
  case unchanged
  case interrupted
  case waitForStop
  case waitForTransition
  case focus(UInt32)
}

public struct HoverTracker: Sendable {
  public private(set) var windowID: UInt32?
  private var previousInteraction: MouseInteraction?

  public init() {}

  public mutating func update(
    windowID: UInt32?, interaction: MouseInteraction = MouseInteraction(), suspended: Bool = false,
    buttonChanged: Bool = false, isFast: Bool = false, transitionInProgress: Bool = false
  ) -> HoverDecision {
    let clicked =
      previousInteraction.map { $0 != interaction } ?? false
    previousInteraction = interaction
    if suspended || buttonChanged || interaction.isPressed || clicked {
      self.windowID = windowID
      return .interrupted
    }
    guard windowID != self.windowID else { return .unchanged }
    if isFast { return .waitForStop }
    if transitionInProgress { return .waitForTransition }
    self.windowID = windowID
    return windowID.map(HoverDecision.focus) ?? .unchanged
  }

  public mutating func reset(windowID: UInt32? = nil, interaction: MouseInteraction? = nil) {
    self.windowID = windowID
    previousInteraction = interaction
  }
}
