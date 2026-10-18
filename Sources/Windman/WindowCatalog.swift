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

enum FocusTarget {
  case window(RaisePlan, AXUIElement)
  case desktop(processID: Int32, windowID: UInt32?)

  var status: String {
    switch self {
    case .window(let plan, _): "Following mouse · \(plan.protectedWindows.count) protected"
    case .desktop: "Following mouse · Desktop"
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
    visibleWindows(in: windowEntries())
  }

  func resolve(_ window: WindowSnapshot) throws -> AXUIElement {
    let application = AXUIElementCreateApplication(window.processID)
    try check(AXUIElementSetMessagingTimeout(application, 0.25), action: "Set messaging timeout")
    var value: CFTypeRef?
    try check(
      AXUIElementCopyAttributeValue(application, kAXWindowsAttribute as CFString, &value),
      action: "Read application windows")
    guard let elements = value as? [AXUIElement],
      let element = elements.first(where: { wm_accessibility_window_id($0) == window.id })
    else { throw WindowFailure.unavailable(window.id) }
    return element
  }

  func target(at point: CGPoint, windowID: UInt32) throws -> FocusTarget? {
    let entries = windowEntries()
    let underPointer = entries.filter { $0.window.bounds.contains(point) }
    let hit =
      windowID == 0 ? underPointer.first : underPointer.first { $0.window.id == windowID }
    let desktopLevel = CGWindowLevelForKey(.desktopIconWindow)
    if let hit, hit.level <= desktopLevel {
      guard
        let finder = NSRunningApplication.runningApplications(
          withBundleIdentifier: "com.apple.finder"
        ).first
      else { return nil }
      let desktop = underPointer.first {
        $0.window.processID == finder.processIdentifier && $0.level == desktopLevel
      }
      return .desktop(processID: finder.processIdentifier, windowID: desktop?.window.id)
    }
    guard let plan = RaisePlan(targetID: windowID, frontToBack: visibleWindows(in: entries))
    else { return nil }
    let element = try resolve(plan.target)
    guard isNormalWindow(element) else { return nil }
    return .window(plan, element)
  }

  func apply(_ target: FocusTarget, expectedInteraction: MouseInteraction) async throws {
    switch target {
    case .window(let plan, let element):
      try await apply(
        plan, element: element, expectedInteraction: expectedInteraction)
    case .desktop(let processID, let windowID):
      try await focusDesktop(
        processID: processID, windowID: windowID, interaction: expectedInteraction)
    }
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
    _ plan: RaisePlan, element: AXUIElement,
    expectedInteraction: MouseInteraction? = nil
  ) async throws {
    let context = try TransitionContext(interaction: expectedInteraction)
    context.changes.start(
      processIDs: Set(plan.backToFront.map(\.processID)), elements: [element])
    defer { context.changes.stop() }
    var resolved = [plan.target.id: element]
    try await WindowTransition.perform(
      plan,
      currentOrder: { visibleWindows().map(\.id) },
      prepare: { window in
        try context.validate()
        if resolved[window.id] == nil {
          resolved[window.id] = try resolve(window)
        }
        guard let element = resolved[window.id] else {
          throw WindowFailure.unavailable(window.id)
        }
        try prepare(window, element: element)
      },
      raise: { window in
        try context.validate()
        guard let element = resolved[window.id] else {
          throw WindowFailure.unavailable(window.id)
        }
        try await raise(
          window, element: element, context: context,
          focusTarget: window.id == plan.target.id)
      },
      focus: { window in
        try await focus(window, context: context)
      }
    )
    try context.validate()
    let expected = plan.protectedWindows.map(\.id) + [plan.target.id]
    let actual = visibleWindows().prefix(expected.count).map(\.id)
    guard actual == expected else { throw WindowFailure.orderNotApplied(plan.target.id) }
    guard focusedWindowID() == plan.target.id else {
      throw WindowFailure.focusNotApplied(plan.target.id)
    }
  }

  private func windowEntries() -> [WindowEntry] {
    let entries =
      CGWindowListCopyWindowInfo(.optionOnScreenOnly, kCGNullWindowID)
      as? [NSDictionary] ?? []
    return entries.compactMap(WindowEntry.init)
  }

  private func visibleWindows(in entries: [WindowEntry]) -> [WindowSnapshot] {
    let desktop = desktopState()
    guard desktop.isStable else { return [] }
    let displays = displayBounds()
    return entries.compactMap { entry in
      let window = entry.window
      guard entry.level == 0,
        window.processID != ProcessInfo.processInfo.processIdentifier,
        displays.contains(where: { $0.intersects(window.bounds) }),
        let spaces = wm_copy_window_spaces(window.id) as? [NSNumber],
        desktop.includes(windowSpaces: Set(spaces.map(\.uint64Value)))
      else { return nil }
      return window
    }
  }

  private func isNormalWindow(_ element: AXUIElement) -> Bool {
    var value: CFTypeRef?
    guard
      AXUIElementCopyAttributeValue(
        element, kAXSubroleAttribute as CFString, &value) == .success
    else { return false }
    let subrole = value as? String
    return subrole == kAXStandardWindowSubrole || subrole == kAXDialogSubrole
  }

  private func focusDesktop(
    processID: Int32, windowID: UInt32?, interaction: MouseInteraction
  ) async throws {
    let context = try TransitionContext(interaction: interaction)
    let previous = focusedWindowID()
    if NSWorkspace.shared.frontmostApplication?.processIdentifier == processID,
      windowID == nil || previous == windowID
    {
      return
    }
    context.changes.start(processIDs: [processID], elements: [])
    defer { context.changes.stop() }
    try context.validate()
    let result = windowID.map { wm_focus_window($0, previous) } ?? wm_focus_process(processID)
    guard result == 0 else { throw WindowFailure.focus(result) }
    let focused = try await context.wait(for: .milliseconds(300)) {
      NSWorkspace.shared.frontmostApplication?.processIdentifier == processID
    }
    guard focused else { throw WindowFailure.focusNotApplied(windowID ?? 0) }
    trace("Focused desktop")
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
    _ window: WindowSnapshot, element: AXUIElement, context: TransitionContext, focusTarget: Bool
  ) async throws {
    try context.validate()
    guard visibleWindows().first?.id != window.id else { return }
    try context.validate()
    var result = AXUIElementPerformAction(element, kAXRaiseAction as CFString)
    trace("AXRaise \(window.id): \(result.rawValue)")
    if result == .success {
      if !focusTarget,
        try await waitForFront(
          window.id, context: context, timeout: .milliseconds(40))
      {
        return
      }
      try requestFocus(window, context: context)
      try context.validate()
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
      try context.validate()
      guard let application = NSRunningApplication(processIdentifier: window.processID),
        application.activate(options: [])
      else { throw WindowFailure.unavailable(window.id) }
    }
    guard
      try await waitForFront(
        window.id, context: context, timeout: .milliseconds(400))
    else {
      throw WindowFailure.orderNotApplied(window.id)
    }
    trace("Raised \(window.id)")
  }

  private func focus(_ window: WindowSnapshot, context: TransitionContext) async throws {
    guard try requestFocus(window, context: context) else { return }
    let focused = try await context.wait(for: .milliseconds(300)) {
      focusedWindowID() == window.id
    }
    guard focused else {
      throw WindowFailure.focusNotApplied(window.id)
    }
    trace("Focused \(window.id)")
  }

  @discardableResult
  private func requestFocus(_ window: WindowSnapshot, context: TransitionContext) throws -> Bool {
    try context.validate()
    let previous = focusedWindowID()
    guard previous != window.id else { return false }
    try context.validate()
    let result = wm_focus_window(window.id, previous)
    guard result == 0 else { throw WindowFailure.focus(result) }
    trace("Requested focus \(window.id)")
    return true
  }

  private func waitForFront(
    _ windowID: UInt32, context: TransitionContext, timeout: Duration
  ) async throws -> Bool {
    try await context.wait(for: timeout) {
      visibleWindows().first?.id == windowID
    }
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

private struct WindowEntry {
  let window: WindowSnapshot
  let level: Int32

  init?(_ entry: NSDictionary) {
    guard let id = entry[kCGWindowNumber] as? NSNumber,
      let process = entry[kCGWindowOwnerPID] as? NSNumber,
      let layer = entry[kCGWindowLayer] as? NSNumber,
      let alpha = entry[kCGWindowAlpha] as? NSNumber, alpha.doubleValue > 0,
      let bounds = entry[kCGWindowBounds] as? NSDictionary,
      let rectangle = CGRect(dictionaryRepresentation: bounds), !rectangle.isEmpty
    else { return nil }
    window = WindowSnapshot(id: id.uint32Value, processID: process.int32Value, bounds: rectangle)
    level = layer.int32Value
  }
}

@MainActor
private struct TransitionContext {
  let changes = WindowChanges()
  private let desktop: DesktopState
  private let interaction: MouseInteraction

  init(interaction: MouseInteraction?) throws {
    let catalog = WindowCatalog()
    desktop = catalog.desktopState()
    self.interaction = interaction ?? catalog.mouseInteraction()
    guard desktop.isStable else { throw WindowFailure.desktopChanged }
    guard !self.interaction.isPressed else { throw WindowFailure.interactionChanged }
    try validate()
  }

  func validate() throws {
    try Task.checkCancellation()
    let catalog = WindowCatalog()
    guard catalog.desktopState() == desktop else { throw WindowFailure.desktopChanged }
    guard catalog.mouseInteraction() == interaction else { throw WindowFailure.interactionChanged }
  }

  func wait(for duration: Duration, until predicate: @escaping () -> Bool) async throws -> Bool {
    try await changes.wait(for: duration) {
      try validate()
      return predicate()
    }
  }
}
