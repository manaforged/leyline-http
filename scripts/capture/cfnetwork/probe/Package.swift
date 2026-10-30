// swift-tools-version: 5.9
import PackageDescription

// CFNetwork capture probe — URLSession GET against the capture server.
// Builds for macOS (host triple) and iOS Simulator:
//
//   swift build                                   # macOS
//   swift build --triple arm64-apple-ios-simulator \
//       --sdk "$(xcrun --sdk iphonesimulator --show-sdk-path)"   # iOS sim
//
// The probe is deliberately tiny and dependency-free: the ClientHello and H2
// frame shapes are a property of the OS framework (CFNetwork/URLSession
// version), not of the probe — any URLSession client on the same OS build
// produces the same handshake.
let package = Package(
    name: "cfnetwork-probe",
    platforms: [
        .macOS(.v13),
        .iOS(.v16),
    ],
    targets: [
        .executableTarget(name: "cfnetwork-probe", path: "Sources/probe"),
    ]
)
