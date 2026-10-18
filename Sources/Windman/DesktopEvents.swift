import AppKit
import WindowRules

enum PointerEvent {
  case button
  case movement(isFast: Bool)
}

@MainActor
final class DesktopEvents: NSObject {
  var onPointer: ((PointerEvent) -> Void)?
  var onDesktop: (() -> Void)?
  var onActivation: (() -> Void)?

  private var removeMonitors: [() -> Void] = []
  private var movementQueued = false
  private var running = false
  private var motion = PointerMotion()

  func start() -> Bool {
    guard !running else { return true }
    running = true
    let mask: NSEvent.EventTypeMask = [
      .mouseMoved, .leftMouseDragged, .rightMouseDragged, .otherMouseDragged,
      .leftMouseDown, .leftMouseUp, .rightMouseDown, .rightMouseUp,
      .otherMouseDown, .otherMouseUp,
    ]
    guard
      let globalMonitor = NSEvent.addGlobalMonitorForEvents(
        matching: mask,
        handler: { [weak self] event in
          MainActor.assumeIsolated { self?.receive(event) }
        })
    else {
      running = false
      return false
    }
    removeMonitors.append { NSEvent.removeMonitor(globalMonitor) }
    guard
      let localMonitor = NSEvent.addLocalMonitorForEvents(
        matching: mask,
        handler: { [weak self] event in
          MainActor.assumeIsolated { self?.receive(event) }
          return event
        })
    else {
      stop()
      return false
    }
    removeMonitors.append { NSEvent.removeMonitor(localMonitor) }
    let workspace = NSWorkspace.shared.notificationCenter
    [
      NSWorkspace.activeSpaceDidChangeNotification, NSWorkspace.didWakeNotification,
      NSWorkspace.willSleepNotification, NSWorkspace.sessionDidResignActiveNotification,
      NSWorkspace.sessionDidBecomeActiveNotification,
    ].forEach {
      workspace.addObserver(self, selector: #selector(desktopChanged), name: $0, object: nil)
    }
    workspace.addObserver(
      self, selector: #selector(applicationActivated),
      name: NSWorkspace.didActivateApplicationNotification, object: nil)
    NotificationCenter.default.addObserver(
      self, selector: #selector(desktopChanged),
      name: NSApplication.didChangeScreenParametersNotification, object: nil)
    return true
  }

  func stop() {
    running = false
    removeMonitors.forEach { $0() }
    removeMonitors.removeAll()
    NSWorkspace.shared.notificationCenter.removeObserver(self)
    NotificationCenter.default.removeObserver(self)
  }

  func resetMotion() {
    motion.reset(
      position: CGEvent(source: nil)?.location, timestamp: ProcessInfo.processInfo.systemUptime)
  }

  private func receive(_ event: NSEvent) {
    switch event.type {
    case .leftMouseDown, .leftMouseUp, .rightMouseDown, .rightMouseUp,
      .otherMouseDown, .otherMouseUp:
      resetMotion()
      onPointer?(.button)
    default:
      if let point = event.cgEvent?.location {
        motion.update(position: point, timestamp: event.timestamp)
      }
      guard !movementQueued else { return }
      movementQueued = true
      DispatchQueue.main.async { [weak self] in
        guard let self else { return }
        movementQueued = false
        guard running else { return }
        onPointer?(.movement(isFast: motion.isFast))
      }
    }
  }

  @objc private func desktopChanged(_ notification: Notification) {
    onDesktop?()
  }

  @objc private func applicationActivated(_ notification: Notification) {
    onActivation?()
  }
}
