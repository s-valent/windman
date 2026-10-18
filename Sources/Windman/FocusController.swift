import AppKit
import ApplicationServices
import OSLog
import WindowRules
import WindowServer

private struct PendingHover {
  let windowID: UInt32
  let point: CGPoint
  let interaction: MouseInteraction
}

@MainActor
final class FocusController {
  var enabled = true {
    didSet {
      resetHover()
      refreshPermission()
    }
  }
  var suspended = false {
    didSet { resetHover() }
  }
  var onStatus: ((String) -> Void)? {
    didSet {
      if !lastStatus.isEmpty { onStatus?(lastStatus) }
    }
  }

  private let catalog = WindowCatalog()
  private let events = DesktopEvents()
  private let logger = Logger(subsystem: "local.windman", category: "focus")
  private var tracker = HoverTracker()
  private var started = false
  private var trusted = false
  private var pendingMovement = false
  private var pointerIsFast = false
  private var stopCheck: Timer?
  private var pendingHover: PendingHover?
  private var transition: Task<Void, Never>?
  private var hoveredWindow: UInt32?
  private var desktop: DesktopState?
  private var mouseInteraction: MouseInteraction?
  private var lastStatus = ""
  private let startupError = wm_initialize().map { String(cString: $0) }

  func start() {
    guard !started else { return }
    started = true
    events.onPointer = { [weak self] button, isFast in
      self?.pointerChanged(button: button, isFast: isFast)
    }
    events.onDesktop = { [weak self] in self?.resetHover() }
    events.onActivation = { [weak self] in
      guard let self, !trusted else { return }
      refreshPermission()
    }
    guard events.start() else {
      started = false
      report(
        "Mouse event monitoring unavailable; reopen Windman after granting Accessibility access")
      return
    }
    resetHover()
    refreshPermission()
  }

  func stop() {
    cancelStopCheck()
    transition?.cancel()
    pendingMovement = false
    events.stop()
    started = false
  }

  func refreshPermission() {
    trusted = AXIsProcessTrusted()
    if let startupError {
      report(startupError)
    } else if !trusted {
      resetHover()
      report("Accessibility permission required")
    } else {
      report(enabled ? "Following mouse" : "Paused")
    }
  }

  private func pointerChanged(button: Bool, isFast: Bool) {
    pointerIsFast = isFast
    evaluatePointer(button: button)
  }

  private func evaluatePointer(button: Bool) {
    guard started, enabled, trusted, startupError == nil else { return }
    guard let point = CGEvent(source: nil)?.location else { return }
    let id = wm_window_at_point(point)
    let hovered: UInt32? = id
    if !button && !suspended && hovered == hoveredWindow {
      cancelStopCheck()
      return
    }
    let interaction = catalog.mouseInteraction()
    let interacted =
      button || interaction.isPressed || (mouseInteraction.map { $0 != interaction } ?? false)
    mouseInteraction = interaction
    if suspended || interacted {
      cancelStopCheck()
      transition?.cancel()
      pendingMovement = false
      hoveredWindow = hovered
      _ = tracker.update(windowID: hovered, interaction: interaction, suspended: true)
      return
    }
    if pointerIsFast {
      scheduleStopCheck(windowID: id, point: point, interaction: interaction)
      return
    }
    cancelStopCheck()
    if transition != nil {
      pendingMovement = true
      return
    }
    guard AXIsProcessTrusted() else {
      refreshPermission()
      return
    }
    let currentDesktop = catalog.desktopState()
    if currentDesktop != desktop {
      desktop = currentDesktop
      resetHover()
      return
    }
    guard currentDesktop.isStable else {
      resetHover()
      report("Waiting for the desktop to settle")
      return
    }
    hoveredWindow = hovered
    guard
      let targetID = tracker.update(
        windowID: hoveredWindow, interaction: interaction, suspended: suspended || interacted)
    else {
      if lastStatus.isEmpty || lastStatus == "Paused"
        || lastStatus == "Accessibility permission required"
        || lastStatus == "Waiting for the desktop to settle"
      {
        report("Following mouse")
      }
      return
    }

    do {
      let operation: @MainActor () async throws -> String
      if let finderDesktop = catalog.desktopWindow(at: point, hitWindowID: targetID) {
        operation = { [catalog] in
          try await catalog.focusDesktop(finderDesktop, expectedInteraction: interaction)
          return "Following mouse · Desktop"
        }
      } else {
        guard let plan = RaisePlan(targetID: targetID, frontToBack: catalog.visibleWindows())
        else { return }
        let elements = try catalog.resolve([plan.target])
        guard let target = elements[targetID], catalog.isNormalWindow(target) else { return }
        operation = { [catalog] in
          try await catalog.apply(plan, elements: elements, expectedInteraction: interaction)
          return "Following mouse · \(plan.protectedWindows.count) protected"
        }
      }
      guard catalog.mouseInteraction() == interaction,
        let currentPoint = CGEvent(source: nil)?.location,
        wm_window_at_point(currentPoint) == targetID
      else {
        resetHover()
        return
      }
      transition = Task { [weak self] in
        guard let self else { return }
        defer {
          transition = nil
          if pendingMovement {
            pendingMovement = false
            if stopCheck == nil { evaluatePointer(button: false) }
          }
        }
        do {
          report(try await operation())
        } catch is CancellationError {
          report("Following mouse")
        } catch WindowFailure.interactionChanged {
          report("Following mouse")
        } catch {
          logger.error("\(error.localizedDescription, privacy: .public)")
          report(error.localizedDescription)
        }
      }
    } catch {
      logger.error("\(error.localizedDescription, privacy: .public)")
      report(error.localizedDescription)
    }
  }

  private func resetHover() {
    cancelStopCheck()
    pointerIsFast = false
    events.resetMotion()
    transition?.cancel()
    pendingMovement = false
    desktop = catalog.desktopState()
    tracker.reset()
    let interaction = catalog.mouseInteraction()
    mouseInteraction = interaction
    let point = CGEvent(source: nil)?.location
    let id = point.map { wm_window_at_point($0) } ?? 0
    hoveredWindow = id
    _ = tracker.update(windowID: hoveredWindow, interaction: interaction, suspended: true)
  }

  private func scheduleStopCheck(windowID: UInt32, point: CGPoint, interaction: MouseInteraction) {
    pendingHover = PendingHover(windowID: windowID, point: point, interaction: interaction)
    if let stopCheck {
      stopCheck.fireDate = Date(timeIntervalSinceNow: 0.07)
      return
    }
    let timer = Timer(timeInterval: 0.07, repeats: false) { [weak self] _ in
      MainActor.assumeIsolated { self?.pointerStopped() }
    }
    stopCheck = timer
    RunLoop.main.add(timer, forMode: .common)
  }

  private func pointerStopped() {
    let pending = pendingHover
    cancelStopCheck()
    guard let pending, started, enabled, !suspended,
      CGEvent(source: nil)?.location == pending.point,
      catalog.mouseInteraction() == pending.interaction,
      wm_window_at_point(pending.point) == pending.windowID
    else { return }
    pointerIsFast = false
    events.resetMotion()
    evaluatePointer(button: false)
  }

  private func cancelStopCheck() {
    stopCheck?.invalidate()
    stopCheck = nil
    pendingHover = nil
  }

  private func report(_ status: String) {
    guard status != lastStatus else { return }
    lastStatus = status
    onStatus?(status)
  }
}
