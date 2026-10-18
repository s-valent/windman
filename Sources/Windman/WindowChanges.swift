import AppKit
import ApplicationServices
import OSLog

@MainActor
final class WindowChanges: NSObject {
  private let logger = Logger(subsystem: "local.windman", category: "notifications")
  private var observers: [AXObserver] = []
  private var subscriptions: [(AXObserver, AXUIElement, CFString)] = []
  private var continuation: CheckedContinuation<Bool, Error>?
  private var predicate: (() throws -> Bool)?
  private var timeout: Task<Void, Never>?
  private var waitID: UUID?

  func start(processIDs: Set<Int32>, elements: [AXUIElement]) {
    processIDs.forEach { processID in
      var result: AXObserver?
      let error = AXObserverCreate(
        processID,
        { _, _, _, context in
          guard let context else { return }
          MainActor.assumeIsolated {
            Unmanaged<WindowChanges>.fromOpaque(context).takeUnretainedValue().changed()
          }
        }, &result)
      guard error == .success, let observer = result else {
        logger.debug("Window observer unavailable: \(error.rawValue)")
        return
      }
      observers.append(observer)
      CFRunLoopAddSource(CFRunLoopGetMain(), AXObserverGetRunLoopSource(observer), .commonModes)
      let application = AXUIElementCreateApplication(processID)
      [
        kAXFocusedWindowChangedNotification, kAXMainWindowChangedNotification,
        kAXWindowCreatedNotification,
      ].forEach { subscribe(observer, element: application, notification: $0) }
      elements.filter {
        var owner: pid_t = 0
        return AXUIElementGetPid($0, &owner) == .success && owner == processID
      }.forEach { element in
        [
          kAXUIElementDestroyedNotification, kAXMovedNotification, kAXResizedNotification,
          kAXWindowMiniaturizedNotification, kAXWindowDeminiaturizedNotification,
        ].forEach { subscribe(observer, element: element, notification: $0) }
      }
    }
    NSWorkspace.shared.notificationCenter.addObserver(
      self, selector: #selector(workspaceChanged),
      name: NSWorkspace.didActivateApplicationNotification, object: nil)
    NSWorkspace.shared.notificationCenter.addObserver(
      self, selector: #selector(workspaceChanged),
      name: NSWorkspace.activeSpaceDidChangeNotification, object: nil)
  }

  func stop() {
    finish(.failure(CancellationError()))
    subscriptions.forEach { AXObserverRemoveNotification($0.0, $0.1, $0.2) }
    subscriptions.removeAll()
    observers.forEach {
      CFRunLoopRemoveSource(CFRunLoopGetMain(), AXObserverGetRunLoopSource($0), .commonModes)
    }
    observers.removeAll()
    NSWorkspace.shared.notificationCenter.removeObserver(self)
  }

  func wait(for duration: Duration, until predicate: @escaping () throws -> Bool) async throws
    -> Bool
  {
    try Task.checkCancellation()
    if try predicate() { return true }
    let id = UUID()
    return try await withTaskCancellationHandler {
      try await withCheckedThrowingContinuation { continuation in
        self.continuation = continuation
        self.predicate = predicate
        waitID = id
        timeout = Task { [weak self] in
          do {
            try await Task.sleep(for: duration)
          } catch {
            return
          }
          self?.changed(expired: true)
        }
        if Task.isCancelled { finish(.failure(CancellationError())) }
      }
    } onCancel: {
      Task { @MainActor [weak self] in
        guard let self, waitID == id else { return }
        finish(.failure(CancellationError()))
      }
    }
  }

  private func subscribe(_ observer: AXObserver, element: AXUIElement, notification: String) {
    let name = notification as CFString
    let result = AXObserverAddNotification(
      observer, element, name, Unmanaged.passUnretained(self).toOpaque())
    if result == .success {
      subscriptions.append((observer, element, name))
    } else if result != .notificationUnsupported && result != .notImplemented {
      logger.debug("Subscribe \(notification): \(result.rawValue)")
    }
  }

  @objc private func workspaceChanged(_ notification: Notification) {
    changed()
  }

  private func changed(expired: Bool = false) {
    guard let predicate else { return }
    do {
      let satisfied = try predicate()
      if satisfied || expired { finish(.success(satisfied)) }
    } catch {
      finish(.failure(error))
    }
  }

  private func finish(_ result: Result<Bool, Error>) {
    timeout?.cancel()
    timeout = nil
    predicate = nil
    waitID = nil
    let pending = continuation
    continuation = nil
    pending?.resume(with: result)
  }
}
