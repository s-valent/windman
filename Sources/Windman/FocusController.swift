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
  private var desktop: DesktopState?
  private var lastStatus = ""
  private let startupError = wm_initialize().map { String(cString: $0) }

  func start() {
    guard !started else { return }
    started = true
    events.onPointer = { [weak self] event in self?.pointerChanged(event) }
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

  private func pointerChanged(_ event: PointerEvent) {
    switch event {
    case .button:
      pointerIsFast = false
      evaluatePointer(buttonChanged: true)
    case .movement(let isFast):
      pointerIsFast = isFast
      evaluatePointer()
    }
  }

  private func evaluatePointer(buttonChanged: Bool = false) {
    guard started, enabled, trusted, startupError == nil else { return }
    guard let point = CGEvent(source: nil)?.location else { return }
    let id = wm_window_at_point(point)
    if !buttonChanged && !suspended && id == tracker.windowID {
      cancelStopCheck()
      return
    }
    let interaction = catalog.mouseInteraction()
    let decision = tracker.update(
      windowID: id, interaction: interaction, suspended: suspended, buttonChanged: buttonChanged,
      isFast: pointerIsFast, transitionInProgress: transition != nil)
    switch decision {
    case .interrupted:
      cancelStopCheck()
      transition?.cancel()
      pendingMovement = false
      return
    case .waitForStop:
      scheduleStopCheck(windowID: id, point: point, interaction: interaction)
      return
    case .waitForTransition:
      cancelStopCheck()
      pendingMovement = true
      return
    case .unchanged:
      cancelStopCheck()
      return
    case .focus:
      cancelStopCheck()
    }
    guard AXIsProcessTrusted() else {
      refreshPermission()
      return
    }
    let currentDesktop = catalog.desktopState()
    if currentDesktop != desktop {
      resetHover()
      return
    }
    guard currentDesktop.isStable else {
      resetHover()
      report("Waiting for the desktop to settle")
      return
    }
    startTransition(windowID: id, point: point, interaction: interaction)
  }

  private func startTransition(windowID: UInt32, point: CGPoint, interaction: MouseInteraction) {
    transition = Task { [weak self] in
      guard let self else { return }
      defer {
        transition = nil
        if pendingMovement {
          pendingMovement = false
          if stopCheck == nil { evaluatePointer() }
        }
      }
      do {
        try Task.checkCancellation()
        guard let target = try catalog.target(at: point, windowID: windowID) else { return }
        guard catalog.mouseInteraction() == interaction,
          let currentPoint = CGEvent(source: nil)?.location,
          wm_window_at_point(currentPoint) == windowID
        else {
          resetHover()
          return
        }
        try await catalog.apply(target, expectedInteraction: interaction)
        report(target.status)
      } catch is CancellationError {
        report("Following mouse")
      } catch WindowFailure.interactionChanged {
        report("Following mouse")
      } catch {
        logger.error("\(error.localizedDescription, privacy: .public)")
        report(error.localizedDescription)
      }
    }
  }

  private func resetHover() {
    cancelStopCheck()
    pointerIsFast = false
    events.resetMotion()
    transition?.cancel()
    pendingMovement = false
    desktop = catalog.desktopState()
    let interaction = catalog.mouseInteraction()
    let point = CGEvent(source: nil)?.location
    let id = point.map { wm_window_at_point($0) } ?? 0
    tracker.reset(windowID: id, interaction: interaction)
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
    evaluatePointer()
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
