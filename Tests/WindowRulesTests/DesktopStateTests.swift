import Testing
import WindowRules

@Test func excludesWindowsFromOtherDesktopsEvenWhenTheirPreviewsAreVisible() {
  let desktop = DesktopState(spaceIDs: [30], isAnimating: false)

  #expect(desktop.includes(windowSpaces: [30]))
  #expect(!desktop.includes(windowSpaces: [7]))
  #expect(!desktop.includes(windowSpaces: [29]))
  #expect(!desktop.includes(windowSpaces: []))
}

@Test func suspendsDuringDesktopAnimations() {
  let desktop = DesktopState(spaceIDs: [30], isAnimating: true)

  #expect(!desktop.isStable)
  #expect(!desktop.includes(windowSpaces: [30]))
}

@Test func includesWindowsOnEitherActiveDisplayAndAllDesktopsWindows() {
  let desktop = DesktopState(spaceIDs: [7, 30], isAnimating: false)

  #expect(desktop.includes(windowSpaces: [7]))
  #expect(desktop.includes(windowSpaces: [30]))
  #expect(desktop.includes(windowSpaces: [28, 29, 30]))
  #expect(!desktop.includes(windowSpaces: [29]))
}

@Test func missingDesktopInformationDoesNotEnableFocusChanges() {
  #expect(!DesktopState(spaceIDs: [], isAnimating: false).isStable)
}
