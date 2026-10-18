import AppKit
import Testing

@testable import Windman

@Test @MainActor func missingNotificationsOnlyCheckAtStartAndDeadline() async throws {
  let changes = WindowChanges()
  defer { changes.stop() }
  var checks = 0
  let result = try await changes.wait(for: .milliseconds(50)) {
    checks += 1
    return false
  }
  #expect(!result)
  #expect(checks == 2)
}

@Test @MainActor func notificationCompletesWaitWithoutWaitingForDeadline() async throws {
  let changes = WindowChanges()
  changes.start(processIDs: [], elements: [])
  defer { changes.stop() }
  var ready = false
  var checks = 0
  let notification = Task { @MainActor in
    try await Task.sleep(for: .milliseconds(10))
    ready = true
    NSWorkspace.shared.notificationCenter.post(
      name: NSWorkspace.didActivateApplicationNotification, object: nil)
  }
  let started = ContinuousClock.now
  let result = try await changes.wait(for: .seconds(5)) {
    checks += 1
    return ready
  }
  try await notification.value
  #expect(result)
  #expect(checks == 2)
  #expect(started.duration(to: .now) < .seconds(4))
}

@Test @MainActor func cancellationRemovesPendingVerification() async throws {
  let changes = WindowChanges()
  defer { changes.stop() }
  var checks = 0
  let (started, continuation) = AsyncStream<Void>.makeStream()
  let operation = Task { @MainActor in
    try await changes.wait(for: .seconds(5)) {
      checks += 1
      continuation.yield(())
      return false
    }
  }
  var iterator = started.makeAsyncIterator()
  _ = await iterator.next()
  continuation.finish()
  operation.cancel()
  do {
    _ = try await operation.value
    Issue.record("Cancelled verification completed successfully")
  } catch is CancellationError {
  }
  let countAfterCancellation = checks
  #expect(countAfterCancellation == 1)
  let result = try await changes.wait(for: .milliseconds(20)) { true }
  #expect(result)
  #expect(checks == countAfterCancellation)
}
