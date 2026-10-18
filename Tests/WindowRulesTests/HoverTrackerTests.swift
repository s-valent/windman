import Testing
import WindowRules

@Test func emptyDesktopHitTriggersOnceAndAllowsReturningToWindow() {
  var tracker = HoverTracker()
  #expect(tracker.update(windowID: 1) == 1)
  #expect(tracker.update(windowID: 0) == 0)
  #expect(tracker.update(windowID: 0) == nil)
  #expect(tracker.update(windowID: 1) == 1)
}

@Test func releasingClickOnDesktopDoesNotRefocusFinder() {
  var tracker = HoverTracker()
  #expect(tracker.update(windowID: 1) == 1)
  #expect(
    tracker.update(
      windowID: 0, interaction: MouseInteraction(buttonEventCount: 1, isPressed: true)) == nil)
  #expect(
    tracker.update(
      windowID: 0, interaction: MouseInteraction(buttonEventCount: 2)) == nil)
  #expect(
    tracker.update(
      windowID: 0, interaction: MouseInteraction(buttonEventCount: 2)) == nil)
}

@Test func triggersImmediatelyAndOnlyOncePerEntry() {
  var tracker = HoverTracker()
  #expect(tracker.update(windowID: 1) == 1)
  #expect(tracker.update(windowID: 1) == nil)
  #expect(tracker.update(windowID: 1) == nil)
}

@Test func enteringAnotherWindowTriggersImmediately() {
  var tracker = HoverTracker()
  #expect(tracker.update(windowID: 1) == 1)
  #expect(tracker.update(windowID: 2) == 2)
  #expect(tracker.update(windowID: 2) == nil)
}

@Test func leavingOrSuspendingAllowsReentry() {
  var tracker = HoverTracker()
  #expect(tracker.update(windowID: 1) == 1)
  #expect(tracker.update(windowID: nil) == nil)
  #expect(tracker.update(windowID: 1) == 1)
  tracker.reset()
  #expect(tracker.update(windowID: 1) == 1)
}

@Test func releasingClickDoesNotRefocusWindowUnderNewPopup() {
  var tracker = HoverTracker()
  #expect(tracker.update(windowID: 1) == 1)
  #expect(
    tracker.update(
      windowID: 1, interaction: MouseInteraction(buttonEventCount: 1, isPressed: true)) == nil)
  #expect(
    tracker.update(
      windowID: 1, interaction: MouseInteraction(buttonEventCount: 2)) == nil)
  #expect(
    tracker.update(
      windowID: 1, interaction: MouseInteraction(buttonEventCount: 2)) == nil)
  #expect(
    tracker.update(
      windowID: 2, interaction: MouseInteraction(buttonEventCount: 2)) == 2)
}

@Test func completeClickBetweenPointerSamplesDoesNotTriggerFocus() {
  var tracker = HoverTracker()
  #expect(tracker.update(windowID: 1) == 1)
  #expect(
    tracker.update(
      windowID: 2, interaction: MouseInteraction(buttonEventCount: 2)) == nil)
  #expect(
    tracker.update(
      windowID: 2, interaction: MouseInteraction(buttonEventCount: 2)) == nil)
  #expect(
    tracker.update(
      windowID: 1, interaction: MouseInteraction(buttonEventCount: 2)) == 1)
}

@Test func endingDragDoesNotActivateWindowUnderReleasePoint() {
  var tracker = HoverTracker()
  #expect(tracker.update(windowID: 1) == 1)
  #expect(
    tracker.update(
      windowID: 1, interaction: MouseInteraction(buttonEventCount: 1, isPressed: true)) == nil)
  #expect(
    tracker.update(
      windowID: 2, interaction: MouseInteraction(buttonEventCount: 1, isPressed: true)) == nil)
  #expect(
    tracker.update(
      windowID: 2, interaction: MouseInteraction(buttonEventCount: 2)) == nil)
  #expect(
    tracker.update(
      windowID: 2, interaction: MouseInteraction(buttonEventCount: 2)) == nil)
  #expect(
    tracker.update(
      windowID: 3, interaction: MouseInteraction(buttonEventCount: 2)) == 3)
}

@Test func closingMenuDoesNotCreateSyntheticWindowEntry() {
  var tracker = HoverTracker()
  #expect(tracker.update(windowID: 1) == 1)
  #expect(tracker.update(windowID: 2, suspended: true) == nil)
  #expect(tracker.update(windowID: 2) == nil)
  #expect(tracker.update(windowID: 1) == 1)
}

@Test func armingTrackingDoesNotRefocusParentOfAlreadyOpenPopup() {
  var tracker = HoverTracker()
  #expect(tracker.update(windowID: 1, suspended: true) == nil)
  #expect(tracker.update(windowID: 1) == nil)
  #expect(tracker.update(windowID: 2) == 2)
}
