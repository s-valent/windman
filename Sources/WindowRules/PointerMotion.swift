import CoreGraphics
import Foundation

public struct PointerMotion: Sendable {
  public private(set) var isFast = false

  private var position: CGPoint?
  private var timestamp: TimeInterval?
  private var slowSince: TimeInterval?

  public init() {}

  public mutating func update(position: CGPoint, timestamp: TimeInterval) {
    guard let previousPosition = self.position, let previousTimestamp = self.timestamp else {
      reset(position: position, timestamp: timestamp)
      return
    }
    let elapsed = timestamp - previousTimestamp
    guard elapsed > 0, position != previousPosition else { return }
    self.position = position
    self.timestamp = timestamp
    let distance = hypot(position.x - previousPosition.x, position.y - previousPosition.y)
    let interval = elapsed > 0.1 ? 1.0 / 60 : elapsed
    let speed = distance / interval
    if elapsed > 0.1 {
      isFast = speed >= 700
      slowSince = nil
      return
    }
    if speed >= 700 {
      isFast = true
      slowSince = nil
    } else if isFast && speed <= 300 {
      if let slowSince, timestamp - slowSince >= 0.02 {
        isFast = false
        self.slowSince = nil
      } else if slowSince == nil {
        slowSince = timestamp
      }
    } else {
      slowSince = nil
    }
  }

  public mutating func reset(position: CGPoint?, timestamp: TimeInterval) {
    self.position = position
    self.timestamp = timestamp
    slowSince = nil
    isFast = false
  }
}
