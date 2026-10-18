import CoreGraphics
import Testing
import WindowRules

@Test func slowMovementDoesNotRequireStopCheck() {
  var motion = PointerMotion()
  motion.reset(position: .zero, timestamp: 0)
  motion.update(position: CGPoint(x: 2, y: 0), timestamp: 0.01)
  #expect(!motion.isFast)
  motion.update(position: CGPoint(x: 4, y: 0), timestamp: 0.02)
  #expect(!motion.isFast)
}

@Test func fastCrossingRemainsDeferredThroughOneSlowSample() {
  var motion = PointerMotion()
  motion.reset(position: .zero, timestamp: 0)
  motion.update(position: CGPoint(x: 20, y: 0), timestamp: 0.01)
  #expect(motion.isFast)
  motion.update(position: CGPoint(x: 21, y: 0), timestamp: 0.02)
  #expect(motion.isFast)
  motion.update(position: CGPoint(x: 41, y: 0), timestamp: 0.03)
  #expect(motion.isFast)
}

@Test func sustainedDecelerationAllowsFocusBeforeStopping() {
  var motion = PointerMotion()
  motion.reset(position: .zero, timestamp: 0)
  motion.update(position: CGPoint(x: 20, y: 0), timestamp: 0.01)
  motion.update(position: CGPoint(x: 21, y: 0), timestamp: 0.02)
  motion.update(position: CGPoint(x: 22, y: 0), timestamp: 0.035)
  #expect(motion.isFast)
  motion.update(position: CGPoint(x: 23, y: 0), timestamp: 0.05)
  #expect(!motion.isFast)
}

@Test func largeFirstMovementAfterIdleIsNotMistakenForSlowEntry() {
  var motion = PointerMotion()
  motion.reset(position: .zero, timestamp: 0)
  motion.update(position: CGPoint(x: 100, y: 0), timestamp: 10)
  #expect(motion.isFast)
}

@Test func repeatedPositionDoesNotPretendPointerDecelerated() {
  var motion = PointerMotion()
  motion.reset(position: .zero, timestamp: 0)
  motion.update(position: CGPoint(x: 20, y: 0), timestamp: 0.01)
  motion.update(position: CGPoint(x: 20, y: 0), timestamp: 0.02)
  motion.update(position: CGPoint(x: 20, y: 0), timestamp: 0.05)
  #expect(motion.isFast)
}

@Test func slowMovementAfterIdleDoesNotInheritPreviousFastMotion() {
  var motion = PointerMotion()
  motion.reset(position: .zero, timestamp: 0)
  motion.update(position: CGPoint(x: 20, y: 0), timestamp: 0.01)
  motion.update(position: CGPoint(x: 21, y: 0), timestamp: 10)
  #expect(!motion.isFast)
}

@Test func intermediateSpeedDoesNotOscillateBetweenFastAndSlow() {
  var motion = PointerMotion()
  motion.reset(position: .zero, timestamp: 0)
  motion.update(position: CGPoint(x: 10, y: 0), timestamp: 0.01)
  motion.update(position: CGPoint(x: 15, y: 0), timestamp: 0.02)
  motion.update(position: CGPoint(x: 20, y: 0), timestamp: 0.03)
  #expect(motion.isFast)
}

@Test func resettingAfterClickDiscardsPreviousFastMotion() {
  var motion = PointerMotion()
  motion.reset(position: .zero, timestamp: 0)
  motion.update(position: CGPoint(x: 20, y: 0), timestamp: 0.01)
  motion.reset(position: CGPoint(x: 20, y: 0), timestamp: 0.02)
  motion.update(position: CGPoint(x: 21, y: 0), timestamp: 0.03)
  #expect(!motion.isFast)
}
