// swift-tools-version: 6.0
import PackageDescription

let package = Package(
  name: "Windman",
  platforms: [.macOS(.v14)],
  products: [.executable(name: "Windman", targets: ["Windman"])],
  targets: [
    .target(name: "WindowRules"),
    .target(
      name: "WindowServer",
      linkerSettings: [
        .linkedFramework("ApplicationServices"),
        .linkedFramework("Carbon"),
      ]
    ),
    .executableTarget(name: "Windman", dependencies: ["WindowRules", "WindowServer"]),
    .testTarget(name: "WindowRulesTests", dependencies: ["WindowRules"]),
    .testTarget(name: "WindmanTests", dependencies: ["Windman"]),
  ]
)
