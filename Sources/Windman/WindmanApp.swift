import AppKit
import ApplicationServices
import WindowRules
import WindowServer

@main
@MainActor
enum WindmanApp {
  static func main() {
    if let option = CommandLine.arguments.firstIndex(of: "--focus-window"),
      CommandLine.arguments.indices.contains(option + 1),
      let windowID = UInt32(CommandLine.arguments[option + 1])
    {
      focusWindow(windowID)
      return
    }
    if CommandLine.arguments.contains("--diagnose") {
      let error = wm_initialize().map { String(cString: $0) }
      print("Private APIs: \(error ?? "available")")
      print("Accessibility (this process): \(AXIsProcessTrusted() ? "granted" : "not granted")")
      print("The menu-bar app's permission may differ; check its status menu.")
      let desktop = WindowCatalog().desktopState()
      print("Active Spaces: \(desktop.spaceIDs.sorted()); animating: \(desktop.isAnimating)")
      print("Visible normal-layer windows: \(WindowCatalog().visibleWindows().count)")
      WindowCatalog().visibleWindows().forEach { window in
        let name = NSRunningApplication(processIdentifier: window.processID)?.localizedName ?? "?"
        print("\(window.id) \(name) \(window.bounds)")
      }
      return
    }
    let application = NSApplication.shared
    let delegate = AppDelegate()
    application.setActivationPolicy(.accessory)
    application.delegate = delegate
    withExtendedLifetime(delegate) { application.run() }
  }

  private static func focusWindow(_ windowID: UInt32) {
    let application = NSApplication.shared
    application.setActivationPolicy(.accessory)
    Task {
      do {
        let catalog = WindowCatalog()
        guard let plan = RaisePlan(targetID: windowID, frontToBack: catalog.visibleWindows()) else {
          throw WindowFailure.unavailable(windowID)
        }
        print("Target: \(windowID); protected: \(plan.protectedWindows.map(\.id))")
        let started = ContinuousClock.now
        try await catalog.apply(plan, elements: catalog.resolve([plan.target]))
        print("Verified stacking order and keyboard focus in \(started.duration(to: .now))")
        exit(0)
      } catch {
        print(error.localizedDescription)
        exit(1)
      }
    }
    application.run()
  }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSMenuDelegate {
  private let controller = FocusController()
  private var statusItem: NSStatusItem?
  private let statusLine = NSMenuItem(title: "Starting…", action: nil, keyEquivalent: "")
  private let enabledItem = NSMenuItem(
    title: "Focus Follows Mouse", action: #selector(toggleEnabled), keyEquivalent: "")
  private let defaults = UserDefaults.standard

  func applicationDidFinishLaunching(_ notification: Notification) {
    defaults.register(defaults: ["enabled": true])
    controller.enabled = defaults.bool(forKey: "enabled")

    let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
    item.button?.image = NSImage(
      systemSymbolName: "cursorarrow.rays", accessibilityDescription: "Windman")
    item.button?.toolTip = "Windman — focus follows mouse"
    statusItem = item

    let menu = NSMenu()
    menu.delegate = self
    menu.addItem(statusLine)
    menu.addItem(.separator())
    enabledItem.target = self
    enabledItem.state = controller.enabled ? .on : .off
    menu.addItem(enabledItem)

    menu.addItem(.separator())

    let permissionItem = NSMenuItem(
      title: "Grant Accessibility Access…", action: #selector(openAccessibility), keyEquivalent: "")
    permissionItem.target = self
    menu.addItem(permissionItem)
    let quitItem = NSMenuItem(title: "Quit Windman", action: #selector(quit), keyEquivalent: "q")
    quitItem.target = self
    menu.addItem(quitItem)
    item.menu = menu

    controller.onStatus = { [weak self] status in
      self?.statusLine.title = status
      self?.statusItem?.button?.toolTip = "Windman — \(status)"
    }
    controller.start()
  }

  func applicationWillTerminate(_ notification: Notification) {
    controller.stop()
  }

  func menuWillOpen(_ menu: NSMenu) {
    controller.suspended = true
    controller.refreshPermission()
  }

  func menuDidClose(_ menu: NSMenu) {
    controller.suspended = false
  }

  @objc private func toggleEnabled() {
    controller.enabled.toggle()
    defaults.set(controller.enabled, forKey: "enabled")
    enabledItem.state = controller.enabled ? .on : .off
  }

  @objc private func openAccessibility() {
    if !wm_request_accessibility(),
      let url = URL(
        string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
    {
      NSWorkspace.shared.open(url)
    }
  }

  @objc private func quit() {
    NSApplication.shared.terminate(nil)
  }
}
