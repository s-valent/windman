public struct HoverTracker: Sendable {
  private var candidate: UInt32?
  private var previousInteraction: MouseInteraction?

  public init() {}

  public mutating func update(
    windowID: UInt32?, interaction: MouseInteraction = MouseInteraction(), suspended: Bool = false
  ) -> UInt32? {
    let clicked =
      previousInteraction.map { $0.buttonEventCount != interaction.buttonEventCount } ?? false
    previousInteraction = interaction
    if suspended || interaction.isPressed || clicked {
      candidate = windowID
      return nil
    }
    guard windowID != candidate else { return nil }
    candidate = windowID
    return windowID
  }

  public mutating func reset() {
    candidate = nil
  }
}
