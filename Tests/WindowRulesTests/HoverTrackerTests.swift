import Testing
import WindowRules

@Test func emptyDesktopHitTriggersOnceAndAllowsReturningToWindow() {
  var tracker = HoverTracker()
  #expect(tracker.update(windowID: 1) == .focus(1))
  #expect(tracker.update(windowID: 0) == .focus(0))
  #expect(tracker.update(windowID: 0) == .unchanged)
  #expect(tracker.update(windowID: 1) == .focus(1))
}

@Test func releasingClickOnDesktopDoesNotRefocusFinder() {
  var tracker = HoverTracker()
  #expect(tracker.update(windowID: 1) == .focus(1))
  #expect(
    tracker.update(
      windowID: 0, interaction: MouseInteraction(buttonEventCount: 1, isPressed: true))
      == .interrupted)
  #expect(
    tracker.update(
      windowID: 0, interaction: MouseInteraction(buttonEventCount: 2)) == .interrupted)
  #expect(
    tracker.update(
      windowID: 0, interaction: MouseInteraction(buttonEventCount: 2)) == .unchanged)
}

@Test func triggersImmediatelyAndOnlyOncePerEntry() {
  var tracker = HoverTracker()
  #expect(tracker.update(windowID: 1) == .focus(1))
  #expect(tracker.update(windowID: 1) == .unchanged)
  #expect(tracker.update(windowID: 1) == .unchanged)
}

@Test func enteringAnotherWindowTriggersImmediately() {
  var tracker = HoverTracker()
  #expect(tracker.update(windowID: 1) == .focus(1))
  #expect(tracker.update(windowID: 2) == .focus(2))
  #expect(tracker.update(windowID: 2) == .unchanged)
}

@Test func leavingOrSuspendingAllowsReentry() {
  var tracker = HoverTracker()
  #expect(tracker.update(windowID: 1) == .focus(1))
  #expect(tracker.update(windowID: nil) == .unchanged)
  #expect(tracker.update(windowID: 1) == .focus(1))
  tracker.reset()
  #expect(tracker.update(windowID: 1) == .focus(1))
}

@Test func releasingClickDoesNotRefocusWindowUnderNewPopup() {
  var tracker = HoverTracker()
  #expect(tracker.update(windowID: 1) == .focus(1))
  #expect(
    tracker.update(
      windowID: 1, interaction: MouseInteraction(buttonEventCount: 1, isPressed: true))
      == .interrupted)
  #expect(
    tracker.update(
      windowID: 1, interaction: MouseInteraction(buttonEventCount: 2)) == .interrupted)
  #expect(
    tracker.update(
      windowID: 1, interaction: MouseInteraction(buttonEventCount: 2)) == .unchanged)
  #expect(
    tracker.update(
      windowID: 2, interaction: MouseInteraction(buttonEventCount: 2)) == .focus(2))
}

@Test func completeClickBetweenPointerSamplesDoesNotTriggerFocus() {
  var tracker = HoverTracker()
  #expect(tracker.update(windowID: 1) == .focus(1))
  #expect(
    tracker.update(
      windowID: 2, interaction: MouseInteraction(buttonEventCount: 2)) == .interrupted)
  #expect(
    tracker.update(
      windowID: 2, interaction: MouseInteraction(buttonEventCount: 2)) == .unchanged)
  #expect(
    tracker.update(
      windowID: 1, interaction: MouseInteraction(buttonEventCount: 2)) == .focus(1))
}

@Test func endingDragDoesNotActivateWindowUnderReleasePoint() {
  var tracker = HoverTracker()
  #expect(tracker.update(windowID: 1) == .focus(1))
  #expect(
    tracker.update(
      windowID: 1, interaction: MouseInteraction(buttonEventCount: 1, isPressed: true))
      == .interrupted)
  #expect(
    tracker.update(
      windowID: 2, interaction: MouseInteraction(buttonEventCount: 1, isPressed: true))
      == .interrupted)
  #expect(
    tracker.update(
      windowID: 2, interaction: MouseInteraction(buttonEventCount: 2)) == .interrupted)
  #expect(
    tracker.update(
      windowID: 2, interaction: MouseInteraction(buttonEventCount: 2)) == .unchanged)
  #expect(
    tracker.update(
      windowID: 3, interaction: MouseInteraction(buttonEventCount: 2)) == .focus(3))
}

@Test func closingMenuDoesNotCreateSyntheticWindowEntry() {
  var tracker = HoverTracker()
  #expect(tracker.update(windowID: 1) == .focus(1))
  #expect(tracker.update(windowID: 2, suspended: true) == .interrupted)
  #expect(tracker.update(windowID: 2) == .unchanged)
  #expect(tracker.update(windowID: 1) == .focus(1))
}

@Test func armingTrackingDoesNotRefocusParentOfAlreadyOpenPopup() {
  var tracker = HoverTracker()
  #expect(tracker.update(windowID: 1, suspended: true) == .interrupted)
  #expect(tracker.update(windowID: 1) == .unchanged)
  #expect(tracker.update(windowID: 2) == .focus(2))
}

@Test func passingThroughWindowsDoesNotConsumeTheirEntry() {
  var tracker = HoverTracker()
  tracker.reset(windowID: 1)
  #expect(tracker.update(windowID: 2, isFast: true) == .waitForStop)
  #expect(tracker.update(windowID: 3, isFast: true) == .waitForStop)
  #expect(tracker.windowID == 1)
  #expect(tracker.update(windowID: 1, isFast: true) == .unchanged)
  #expect(tracker.update(windowID: 2, isFast: true) == .waitForStop)
  #expect(tracker.update(windowID: 2) == .focus(2))
}

@Test func latestHoverWaitsForProtectedWindowRestoration() {
  var tracker = HoverTracker()
  tracker.reset(windowID: 1)
  #expect(tracker.update(windowID: 2, transitionInProgress: true) == .waitForTransition)
  #expect(tracker.update(windowID: 3, transitionInProgress: true) == .waitForTransition)
  #expect(tracker.windowID == 1)
  #expect(tracker.update(windowID: 3) == .focus(3))
}

@Test func clickSuppressesPendingFastHoverEvenDuringTransition() {
  var tracker = HoverTracker()
  tracker.reset(windowID: 1)
  #expect(tracker.update(windowID: 2, isFast: true) == .waitForStop)
  #expect(
    tracker.update(
      windowID: 2, buttonChanged: true, isFast: true, transitionInProgress: true) == .interrupted)
  #expect(tracker.update(windowID: 2) == .unchanged)
}

@Test func fastMovementStillNeedsStopCheckWhenTransitionCompletes() {
  var tracker = HoverTracker()
  tracker.reset(windowID: 1)
  #expect(
    tracker.update(
      windowID: 2, isFast: true, transitionInProgress: true) == .waitForStop)
  #expect(tracker.update(windowID: 2, isFast: true) == .waitForStop)
  #expect(tracker.update(windowID: 2) == .focus(2))
}
