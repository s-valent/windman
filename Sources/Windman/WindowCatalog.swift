import AppKit
import ApplicationServices
import WindowRules
import WindowServer

enum WindowFailure: LocalizedError {
  case unavailable(UInt32)
  case accessibility(String, Int32)
  case focus(Int32)
  case orderNotApplied(UInt32)
  case focusNotApplied(UInt32)
  case desktopChanged
  case unsupported(UInt32)
  case interactionChanged

  var errorDescription: String? {
    switch self {
    case .unavailable(let id): "Window \(id) is no longer accessible."
    case .accessibility(let action, let code): "\(action) failed (Accessibility \(code))."
    case .focus(let code): "Private window focus failed (\(code))."
    case .orderNotApplied(let id): "Could not preserve window order around \(id)."
    case .focusNotApplied(let id): "Window \(id) did not receive keyboard focus."
    case .desktopChanged: "Waiting for the desktop to settle"
    case .unsupported(let id): "Window \(id) cannot be raised or activated."
    case .interactionChanged: "Automatic focus interrupted by mouse interaction."
    }
  }
}

@MainActor
struct WindowCatalog {
  func mouseInteraction() -> MouseInteraction {
    let events: [CGEventType] = [
      .leftMouseDown, .leftMouseUp, .rightMouseDown, .rightMouseUp, .otherMouseDown, .otherMouseUp,
    ]
    let count = events.reduce(UInt64(0)) {
      $0 + UInt64(CGEventSource.counterForEventType(.combinedSessionState, eventType: $1))
    }
    return MouseInteraction(buttonEventCount: count, isPressed: NSEvent.pressedMouseButtons != 0)
  }

  func desktopState() -> DesktopState {
    var animating = true
    let spaces = wm_copy_active_spaces(&animating) as? [NSNumber] ?? []
    return DesktopState(spaceIDs: Set(spaces.map(\.uint64Value)), isAnimating: animating)
  }

  func visibleWindows() -> [WindowSnapshot] {
    let desktop = desktopState()
    guard desktop.isStable else { return [] }
    let displays = displayBounds()
    guard
      let entries = CGWindowListCopyWindowInfo(
        [.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID
      ) as? [NSDictionary]
    else { return [] }

    return entries.compactMap { entry in
      guard let id = entry[kCGWindowNumber] as? NSNumber,
        let process = entry[kCGWindowOwnerPID] as? NSNumber,
        let layer = entry[kCGWindowLayer] as? NSNumber,
        let alpha = entry[kCGWindowAlpha] as? NSNumber,
        let bounds = entry[kCGWindowBounds] as? NSDictionary,
        let rectangle = CGRect(dictionaryRepresentation: bounds),
        layer.intValue == 0, alpha.doubleValue > 0, !rectangle.isEmpty,
        process.int32Value != ProcessInfo.processInfo.processIdentifier,
        displays.contains(where: { $0.intersects(rectangle) }),
        let spaces = wm_copy_window_spaces(id.uint32Value) as? [NSNumber],
        desktop.includes(windowSpaces: Set(spaces.map(\.uint64Value)))
      else { return nil }

      return WindowSnapshot(
        id: id.uint32Value, processID: process.int32Value, bounds: rectangle)
    }
  }

  func resolve(_ windows: [WindowSnapshot]) throws -> [UInt32: AXUIElement] {
    let groups = Dictionary(grouping: windows, by: \.processID)
    return try groups.reduce(into: [:]) { result, group in
      let application = AXUIElementCreateApplication(group.key)
      try check(
        AXUIElementSetMessagingTimeout(application, 0.25), action: "Set messaging timeout")
      var value: CFTypeRef?
      try check(
        AXUIElementCopyAttributeValue(application, kAXWindowsAttribute as CFString, &value),
        action: "Read application windows")
      guard let elements = value as? [AXUIElement] else {
        throw WindowFailure.unavailable(group.value[0].id)
      }
      let wanted = Set(group.value.map(\.id))
      elements.forEach { element in
        let id = wm_accessibility_window_id(element)
        guard wanted.contains(id) else { return }
        result[id] = element
      }
      if let missing = group.value.first(where: { result[$0.id] == nil }) {
        throw WindowFailure.unavailable(missing.id)
      }
    }
  }

  func desktopWindow(at point: CGPoint, hitWindowID: UInt32) -> WindowSnapshot? {
    guard
      let finder = NSRunningApplication.runningApplications(
        withBundleIdentifier: "com.apple.finder"
      ).first,
      let entries = CGWindowListCopyWindowInfo(.optionOnScreenOnly, kCGNullWindowID)
        as? [NSDictionary]
    else { return nil }
    let underPointer = entries.filter { entry in
      guard let bounds = entry[kCGWindowBounds] as? NSDictionary,
        let rectangle = CGRect(dictionaryRepresentation: bounds),
        let alpha = entry[kCGWindowAlpha] as? NSNumber
      else { return false }
      return alpha.doubleValue > 0 && rectangle.contains(point)
    }
    let hit =
      hitWindowID == 0
      ? underPointer.first
      : underPointer.first {
        ($0[kCGWindowNumber] as? NSNumber)?.uint32Value == hitWindowID
      }
    let desktopLevel = CGWindowLevelForKey(.desktopIconWindow)
    guard let level = hit?[kCGWindowLayer] as? NSNumber,
      level.int32Value <= desktopLevel
    else { return nil }
    let desktop = underPointer.first {
      ($0[kCGWindowOwnerPID] as? NSNumber)?.int32Value == finder.processIdentifier
        && ($0[kCGWindowLayer] as? NSNumber)?.int32Value == desktopLevel
    }
    return WindowSnapshot(
      id: (desktop?[kCGWindowNumber] as? NSNumber)?.uint32Value ?? 0,
      processID: finder.processIdentifier, bounds: .zero)
  }

  func focusDesktop(
    _ window: WindowSnapshot, expectedInteraction: MouseInteraction
  ) async throws {
    let desktop = desktopState()
    guard desktop.isStable else { throw WindowFailure.desktopChanged }
    guard !expectedInteraction.isPressed else { throw WindowFailure.interactionChanged }
    try requireContext(desktop, interaction: expectedInteraction)
    let previous = focusedWindowID()
    if NSWorkspace.shared.frontmostApplication?.processIdentifier == window.processID,
      window.id == 0 || previous == window.id
    {
      return
    }
    let changes = WindowChanges()
    changes.start(processIDs: [window.processID], elements: [])
    defer { changes.stop() }
    try requireContext(desktop, interaction: expectedInteraction)
    let result =
      window.id == 0
      ? wm_focus_process(window.processID) : wm_focus_window(window.id, previous)
    guard result == 0 else { throw WindowFailure.focus(result) }
    let focused = try await changes.wait(for: .milliseconds(300)) {
      try requireContext(desktop, interaction: expectedInteraction)
      return NSWorkspace.shared.frontmostApplication?.processIdentifier == window.processID
    }
    guard focused else { throw WindowFailure.focusNotApplied(window.id) }
    trace("Focused desktop")
  }

  func isNormalWindow(_ element: AXUIElement) -> Bool {
    var value: CFTypeRef?
    guard
      AXUIElementCopyAttributeValue(
        element, kAXSubroleAttribute as CFString, &value) == .success
    else { return false }
    let subrole = value as? String
    return subrole == kAXStandardWindowSubrole || subrole == kAXDialogSubrole
  }

  func focusedWindowID() -> UInt32 {
    guard let processID = NSWorkspace.shared.frontmostApplication?.processIdentifier else {
      return 0
    }
    let application = AXUIElementCreateApplication(processID)
    guard AXUIElementSetMessagingTimeout(application, 0.25) == .success else { return 0 }
    var value: CFTypeRef?
    guard
      AXUIElementCopyAttributeValue(
        application, kAXFocusedWindowAttribute as CFString, &value) == .success,
      let value, CFGetTypeID(value) == AXUIElementGetTypeID()
    else { return 0 }
    let element = value as! AXUIElement
    return wm_accessibility_window_id(element)
  }

  func apply(
    _ plan: RaisePlan, elements: [UInt32: AXUIElement],
    expectedInteraction: MouseInteraction? = nil
  ) async throws {
    let desktop = desktopState()
    let interaction = expectedInteraction ?? mouseInteraction()
    guard desktop.isStable else { throw WindowFailure.desktopChanged }
    guard !interaction.isPressed else { throw WindowFailure.interactionChanged }
    let changes = WindowChanges()
    changes.start(
      processIDs: Set(plan.backToFront.map(\.processID)), elements: Array(elements.values))
    defer { changes.stop() }
    var resolved = elements
    try await WindowTransition.perform(
      plan,
      currentOrder: { visibleWindows().map(\.id) },
      prepare: { window in
        try requireContext(desktop, interaction: interaction)
        if resolved[window.id] == nil {
          resolved.merge(try resolve([window])) { _, fresh in fresh }
        }
        guard let element = resolved[window.id] else {
          throw WindowFailure.unavailable(window.id)
        }
        try prepare(window, element: element)
      },
      raise: { window in
        try requireContext(desktop, interaction: interaction)
        guard let element = resolved[window.id] else {
          throw WindowFailure.unavailable(window.id)
        }
        try await raise(
          window, element: element, desktop: desktop, interaction: interaction, changes: changes,
          focusTarget: window.id == plan.target.id)
      },
      focus: { window in
        try await focus(window, desktop: desktop, interaction: interaction, changes: changes)
      }
    )
    try requireContext(desktop, interaction: interaction)
    let expected = plan.protectedWindows.map(\.id) + [plan.target.id]
    let actual = visibleWindows().prefix(expected.count).map(\.id)
    guard actual == expected else { throw WindowFailure.orderNotApplied(plan.target.id) }
    guard focusedWindowID() == plan.target.id else {
      throw WindowFailure.focusNotApplied(plan.target.id)
    }
  }

  private func prepare(_ window: WindowSnapshot, element: AXUIElement) throws {
    var actions: CFArray?
    try check(AXUIElementCopyActionNames(element, &actions), action: "Read window actions")
    if let names = actions as? [String], names.contains(kAXRaiseAction) { return }
    var settable = DarwinBoolean(false)
    try check(
      AXUIElementIsAttributeSettable(element, kAXMainAttribute as CFString, &settable),
      action: "Check main window support")
    guard settable.boolValue else { throw WindowFailure.unsupported(window.id) }
  }

  private func raise(
    _ window: WindowSnapshot, element: AXUIElement, desktop: DesktopState,
    interaction: MouseInteraction, changes: WindowChanges, focusTarget: Bool
  ) async throws {
    try requireContext(desktop, interaction: interaction)
    guard visibleWindows().first?.id != window.id else { return }
    try requireContext(desktop, interaction: interaction)
    var result = AXUIElementPerformAction(element, kAXRaiseAction as CFString)
    trace("AXRaise \(window.id): \(result.rawValue)")
    if result == .success {
      if focusTarget {
        try requestFocus(window, desktop: desktop, interaction: interaction)
      } else if try await waitForFront(
        window.id, desktop: desktop, interaction: interaction, changes: changes,
        timeout: .milliseconds(40))
      {
        return
      }
      if !focusTarget {
        try requestFocus(window, desktop: desktop, interaction: interaction)
      }
      try requireContext(desktop, interaction: interaction)
      result = AXUIElementPerformAction(element, kAXRaiseAction as CFString)
    }
    if result != .success {
      guard
        result == .attributeUnsupported || result == .actionUnsupported
          || result == .notImplemented
      else {
        throw WindowFailure.accessibility("Raise window \(window.id)", result.rawValue)
      }
      try check(
        AXUIElementSetAttributeValue(element, kAXMainAttribute as CFString, kCFBooleanTrue),
        action: "Select main window \(window.id)")
      try requireContext(desktop, interaction: interaction)
      guard let application = NSRunningApplication(processIdentifier: window.processID),
        application.activate(options: [])
      else { throw WindowFailure.unavailable(window.id) }
    }
    guard
      try await waitForFront(
        window.id, desktop: desktop, interaction: interaction, changes: changes,
        timeout: .milliseconds(400))
    else {
      throw WindowFailure.orderNotApplied(window.id)
    }
    trace("Raised \(window.id)")
  }

  private func focus(
    _ window: WindowSnapshot, desktop: DesktopState, interaction: MouseInteraction,
    changes: WindowChanges
  ) async throws {
    try requireContext(desktop, interaction: interaction)
    guard focusedWindowID() != window.id else { return }
    try requestFocus(window, desktop: desktop, interaction: interaction)
    let focused = try await changes.wait(for: .milliseconds(300)) {
      try requireContext(desktop, interaction: interaction)
      return focusedWindowID() == window.id
    }
    guard focused else {
      throw WindowFailure.focusNotApplied(window.id)
    }
    trace("Focused \(window.id)")
  }

  private func requestFocus(
    _ window: WindowSnapshot, desktop: DesktopState, interaction: MouseInteraction
  ) throws {
    try requireContext(desktop, interaction: interaction)
    let previous = focusedWindowID()
    guard previous != window.id else { return }
    try requireContext(desktop, interaction: interaction)
    let result = wm_focus_window(window.id, previous)
    guard result == 0 else { throw WindowFailure.focus(result) }
    trace("Requested focus \(window.id)")
  }

  private func waitForFront(
    _ windowID: UInt32, desktop: DesktopState, interaction: MouseInteraction,
    changes: WindowChanges, timeout: Duration
  ) async throws -> Bool {
    try await changes.wait(for: timeout) {
      try requireContext(desktop, interaction: interaction)
      return visibleWindows().first?.id == windowID
    }
  }

  private func requireContext(_ expected: DesktopState, interaction: MouseInteraction) throws {
    try Task.checkCancellation()
    guard desktopState() == expected else { throw WindowFailure.desktopChanged }
    guard mouseInteraction() == interaction else { throw WindowFailure.interactionChanged }
  }

  private func displayBounds() -> [CGRect] {
    var count: UInt32 = 0
    guard CGGetActiveDisplayList(0, nil, &count) == .success else { return [] }
    var displays = [CGDirectDisplayID](repeating: 0, count: Int(count))
    guard CGGetActiveDisplayList(count, &displays, &count) == .success else { return [] }
    return displays.prefix(Int(count)).map { CGDisplayBounds($0) }
  }

  private func trace(_ action: String) {
    guard CommandLine.arguments.contains("--trace") else { return }
    print("\(action); focus: \(focusedWindowID()); order: \(visibleWindows().map(\.id))")
  }

  private func check(_ error: AXError, action: String) throws {
    guard error == .success else {
      throw WindowFailure.accessibility(action, error.rawValue)
    }
  }
}
