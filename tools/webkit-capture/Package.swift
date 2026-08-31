// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "webkit-probe",
    platforms: [.macOS(.v13)],
    targets: [
        .executableTarget(name: "webkit-probe", path: "Sources"),
    ]
)
