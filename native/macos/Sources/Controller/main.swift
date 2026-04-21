import Darwin
import Foundation
import NetworkExtension

enum ExitCodes {
    static let success = Int32(0)
    static let unavailable = Int32(2)
    static let usage = Int32(64)
}

enum ControllerError: LocalizedError {
    case invalidUsage(String)
    case preferences(String)
    case connection(String)

    var errorDescription: String? {
        switch self {
        case let .invalidUsage(message), let .preferences(message), let .connection(message):
            return message
        }
    }

    var exitCode: Int32 {
        switch self {
        case .invalidUsage:
            return ExitCodes.usage
        case .preferences, .connection:
            return ExitCodes.unavailable
        }
    }
}

enum Command: String {
    case status
    case install
    case uninstall
    case enable
    case disable
    case apply
    case clear
}

enum Preset: String {
    case unthrottled
    case edge
    case threeG = "3g"
    case offline

    var label: String {
        switch self {
        case .unthrottled:
            return "Unthrottled"
        case .edge:
            return "Edge"
        case .threeG:
            return "3G"
        case .offline:
            return "Offline"
        }
    }

    static func parse(_ value: String) throws -> Self {
        switch value.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
        case "unthrottled", "full", "none":
            return .unthrottled
        case "edge":
            return .edge
        case "3g", "three-g", "three_g", "umts":
            return .threeG
        case "offline", "airplane", "airplane-mode", "airplane_mode":
            return .offline
        default:
            throw ControllerError.invalidUsage(
                "Unsupported preset '\(value)'. Expected one of: unthrottled, edge, 3g, offline."
            )
        }
    }
}

struct ParsedCommand {
    let command: Command?
    let preset: Preset?
    let udid: String?
    let showHelp: Bool
}

final class SyncWaiter<Value> {
    private let lock = NSLock()
    private var value: Value?

    func resolve(_ value: Value) {
        lock.lock()
        self.value = value
        lock.unlock()
    }

    func wait() -> Value {
        while true {
            lock.lock()
            if let value {
                lock.unlock()
                return value
            }
            lock.unlock()
            RunLoop.current.run(mode: .default, before: Date(timeIntervalSinceNow: 0.05))
        }
    }
}

func main() {
    do {
        let arguments = Array(CommandLine.arguments.dropFirst())
        if shouldLaunchEmbeddedApp(arguments: arguments) {
            try launchEmbeddedApp(arguments: arguments)
            return
        }

        let parsed = try parseCommandLine(arguments)
        if parsed.showHelp {
            printHelp()
            exit(ExitCodes.success)
        }

        guard let command = parsed.command else {
            printHelp()
            exit(ExitCodes.success)
        }

        let output = try run(command: command, preset: parsed.preset, udid: parsed.udid)
        print(output)
        exit(ExitCodes.success)
    } catch let error as ControllerError {
        fputs("\(error.localizedDescription)\n", stderr)
        exit(error.exitCode)
    } catch {
        fputs("\(error.localizedDescription)\n", stderr)
        exit(ExitCodes.unavailable)
    }
}

func shouldLaunchEmbeddedApp(arguments: [String]) -> Bool {
    let executableName = URL(fileURLWithPath: CommandLine.arguments[0]).lastPathComponent
    let helperNames = ["CatPaneThrottlingController", "catpane-network-ctl"]
    if helperNames.contains(executableName) {
        return false
    }
    guard let first = arguments.first else {
        return true
    }
    return Command(rawValue: first) == nil
}

func launchEmbeddedApp(arguments: [String]) throws {
    let executable = URL(fileURLWithPath: CommandLine.arguments[0]).resolvingSymlinksInPath()
    let rustExecutableName = (Bundle.main.object(forInfoDictionaryKey: "CatPaneRustExecutable") as? String)?
        .trimmingCharacters(in: .whitespacesAndNewlines)
    let fallback = executable.lastPathComponent == "catpane" ? "catpane-rust" : "catpane"
    let targetName = rustExecutableName?.isEmpty == false ? rustExecutableName! : fallback
    let target = executable.deletingLastPathComponent().appendingPathComponent(targetName)

    guard FileManager.default.isExecutableFile(atPath: target.path) else {
        throw ControllerError.connection("Bundled CatPane binary not found at \(target.path).")
    }

    let argv = [target.path] + arguments
    let cStrings = argv.map { strdup($0) } + [nil]
    defer {
        cStrings.dropLast().forEach { free($0) }
    }

    execv(target.path, cStrings)
    let message = String(cString: strerror(errno))
    throw ControllerError.connection("Failed to launch bundled CatPane binary: \(message).")
}

func parseCommandLine(_ arguments: [String]) throws -> ParsedCommand {
    if arguments.isEmpty {
        return ParsedCommand(command: .status, preset: nil, udid: nil, showHelp: false)
    }

    if ["-h", "--help"].contains(arguments[0]) {
        return ParsedCommand(command: nil, preset: nil, udid: nil, showHelp: true)
    }

    guard let command = Command(rawValue: arguments[0]) else {
        throw ControllerError.invalidUsage("Unknown command '\(arguments[0])'.")
    }

    var preset: Preset?
    var udid: String?
    var index = 1
    while index < arguments.count {
        switch arguments[index] {
        case "--preset":
            guard index + 1 < arguments.count else {
                throw ControllerError.invalidUsage("Missing value for --preset.")
            }
            preset = try Preset.parse(arguments[index + 1])
            index += 2
        case "--udid":
            guard index + 1 < arguments.count else {
                throw ControllerError.invalidUsage("Missing value for --udid.")
            }
            udid = arguments[index + 1]
            index += 2
        case "-h", "--help":
            return ParsedCommand(command: nil, preset: nil, udid: nil, showHelp: true)
        default:
            throw ControllerError.invalidUsage("Unknown option '\(arguments[index])'.")
        }
    }

    switch command {
    case .apply:
        if preset == nil {
            throw ControllerError.invalidUsage("The apply command requires --preset.")
        }
    default:
        break
    }

    return ParsedCommand(command: command, preset: preset, udid: udid, showHelp: false)
}

func printHelp() {
    print(
        """
        Usage: CatPaneThrottlingController <command> [--preset PRESET] [--udid SIMULATOR_UDID]

        Commands:
          status     Show the current CatPane simulator throttling state
          install    Install the per-app tunnel configuration without starting it
          uninstall  Remove the CatPane simulator throttling configuration
          enable     Enable and start the last-saved preset
          disable    Stop and disable the CatPane simulator throttling configuration
          apply      Apply a preset and start the per-app proxy
          clear      Stop the proxy and restore unthrottled simulator traffic

        Presets:
          unthrottled, edge, 3g, offline

        Notes:
          - Throttling is currently scoped to the Simulator host app on macOS, so it affects all booted iOS Simulators.
          - The --udid flag is stored for status and future per-simulator refinement, but current macOS app rules target the Simulator app as a whole.
        """
    )
}

func run(command: Command, preset: Preset?, udid: String?) throws -> String {
    return try runWithNetworkExtension(command: command, preset: preset, udid: udid)
}

func runWithNetworkExtension(command: Command, preset: Preset?, udid: String?) throws -> String {
    switch command {
    case .status:
        return try statusMessage()
    case .install:
        _ = try configureManager(preset: preset ?? .edge, udid: udid, enabled: true)
        return "Installed CatPane Simulator throttling profile targeting com.apple.iphonesimulator."
    case .uninstall:
        guard let manager = try loadManager() else {
            return "CatPane Simulator throttling profile is not installed."
        }
        stop(manager: manager)
        try remove(manager: manager)
        return "Removed CatPane Simulator throttling profile."
    case .enable:
        let manager = try ensureConfiguredManager()
        try startOrUpdate(manager: manager, preset: storedPreset(from: manager), udid: storedUDID(from: manager))
        return statusSummary(for: manager)
    case .disable:
        guard let manager = try loadManager() else {
            return "CatPane Simulator throttling profile is not installed."
        }
        stop(manager: manager)
        manager.isEnabled = false
        try save(manager: manager)
        try reload(manager: manager)
        return "Disabled CatPane Simulator throttling."
    case .apply:
        let preset = preset!
        if preset == .unthrottled {
            return try run(command: .clear, preset: nil, udid: udid)
        }
        let manager = try configureManager(preset: preset, udid: udid, enabled: true)
        try startOrUpdate(manager: manager, preset: preset, udid: udid)
        return "Applied \(preset.label) simulator network condition via CatPane app proxy. This currently affects all iOS Simulator traffic on macOS."
    case .clear:
        guard let manager = try loadManager() else {
            return "CatPane Simulator throttling is already clear."
        }
        stop(manager: manager)
        updateProviderConfiguration(on: manager, preset: .unthrottled, udid: udid)
        manager.isEnabled = false
        try save(manager: manager)
        try reload(manager: manager)
        return "Cleared CatPane simulator network throttling and restored unthrottled traffic."
    }
}

func statusMessage() throws -> String {
    guard let manager = try loadManager() else {
        return "CatPane Simulator throttling profile is not installed."
    }
    return statusSummary(for: manager)
}

func statusSummary(for manager: NEAppProxyProviderManager) -> String {
    let preset = storedPreset(from: manager).label
    let enabled = manager.isEnabled ? "enabled" : "disabled"
    let status = connectionStatusLabel(manager.connection.status)
    let udid = storedUDID(from: manager) ?? "all simulators"
    return "CatPane Simulator throttling is \(enabled); connection is \(status); preset is \(preset); target is \(udid)."
}

func loadManager() throws -> NEAppProxyProviderManager? {
    let waiter = SyncWaiter<Result<[NEAppProxyProviderManager], Error>>()
    NEAppProxyProviderManager.loadAllFromPreferences { managers, error in
        if let error {
            waiter.resolve(.failure(error))
        } else {
            waiter.resolve(.success(managers ?? []))
        }
    }

    let managers = try waiter.wait()
        .mapError {
            ControllerError.preferences("Failed to load CatPane tunnel preferences: \($0.localizedDescription)")
        }
        .get()

    return managers.first { manager in
        manager.localizedDescription == "CatPane Simulator Throttling"
            || (manager.protocolConfiguration as? NETunnelProviderProtocol)?.providerBundleIdentifier == extensionBundleIdentifier()
    }
}

func ensureConfiguredManager() throws -> NEAppProxyProviderManager {
    if let manager = try loadManager() {
        return manager
    }
    return try configureManager(preset: .edge, udid: nil, enabled: true)
}

@discardableResult
func configureManager(preset: Preset, udid: String?, enabled: Bool) throws -> NEAppProxyProviderManager {
    let manager = try loadManager() ?? NEAppProxyProviderManager.forPerAppVPN()
    let proto = (manager.protocolConfiguration as? NETunnelProviderProtocol) ?? NETunnelProviderProtocol()
    proto.providerBundleIdentifier = extensionBundleIdentifier()
    proto.serverAddress = "CatPane Simulator Throttling"
    proto.disconnectOnSleep = false
    manager.protocolConfiguration = proto
    manager.localizedDescription = "CatPane Simulator Throttling"
    manager.isEnabled = enabled
    manager.appRules = [simulatorAppRule()]
    updateProviderConfiguration(on: manager, preset: preset, udid: udid)
    try save(manager: manager)
    try reload(manager: manager)
    return manager
}

func updateProviderConfiguration(on manager: NEAppProxyProviderManager, preset: Preset, udid: String?) {
    guard let proto = manager.protocolConfiguration as? NETunnelProviderProtocol else {
        return
    }
    var configuration = proto.providerConfiguration ?? [:]
    configuration["preset"] = preset.rawValue
    if let udid, !udid.isEmpty {
        configuration["simulatorUDID"] = udid
    } else {
        configuration.removeValue(forKey: "simulatorUDID")
    }
    configuration["simulatorSigningIdentifier"] = "com.apple.iphonesimulator"
    proto.providerConfiguration = configuration
    manager.protocolConfiguration = proto
}

func storedPreset(from manager: NEAppProxyProviderManager) -> Preset {
    let value = (manager.protocolConfiguration as? NETunnelProviderProtocol)?
        .providerConfiguration?["preset"] as? String
    if let value, let preset = try? Preset.parse(value) {
        return preset
    }
    return .unthrottled
}

func storedUDID(from manager: NEAppProxyProviderManager) -> String? {
    (manager.protocolConfiguration as? NETunnelProviderProtocol)?
        .providerConfiguration?["simulatorUDID"] as? String
}

func save(manager: NEAppProxyProviderManager) throws {
    let waiter = SyncWaiter<Result<Void, Error>>()
    manager.saveToPreferences { error in
        if let error {
            waiter.resolve(.failure(error))
        } else {
            waiter.resolve(.success(()))
        }
    }
    _ = try waiter.wait()
        .mapError {
            ControllerError.preferences("Failed to save CatPane tunnel preferences: \($0.localizedDescription)")
        }
        .get()
}

func reload(manager: NEAppProxyProviderManager) throws {
    let waiter = SyncWaiter<Result<Void, Error>>()
    manager.loadFromPreferences { error in
        if let error {
            waiter.resolve(.failure(error))
        } else {
            waiter.resolve(.success(()))
        }
    }
    _ = try waiter.wait()
        .mapError {
            ControllerError.preferences("Failed to reload CatPane tunnel preferences: \($0.localizedDescription)")
        }
        .get()
}

func remove(manager: NEAppProxyProviderManager) throws {
    let waiter = SyncWaiter<Result<Void, Error>>()
    manager.removeFromPreferences { error in
        if let error {
            waiter.resolve(.failure(error))
        } else {
            waiter.resolve(.success(()))
        }
    }
    _ = try waiter.wait()
        .mapError {
            ControllerError.preferences("Failed to remove CatPane tunnel preferences: \($0.localizedDescription)")
        }
        .get()
}

func startOrUpdate(manager: NEAppProxyProviderManager, preset: Preset, udid: String?) throws {
    updateProviderConfiguration(on: manager, preset: preset, udid: udid)
    manager.isEnabled = true
    try save(manager: manager)
    try reload(manager: manager)

    if let session = manager.connection as? NETunnelProviderSession,
       manager.connection.status == .connected
            || manager.connection.status == .connecting
            || manager.connection.status == .reasserting
    {
        let response = try sendProviderMessage(
            session: session,
            message: ["command": "setPreset", "preset": preset.rawValue]
        )
        if response["ok"] as? Bool == true {
            return
        }
    }

    stop(manager: manager)
    usleep(250_000)

    guard let session = manager.connection as? NETunnelProviderSession else {
        throw ControllerError.connection("CatPane tunnel session is unavailable.")
    }
    do {
        try session.startTunnel(options: ["preset": preset.rawValue])
    } catch {
        throw ControllerError.connection(
            "Failed to start CatPane simulator throttling tunnel: \(error.localizedDescription)."
        )
    }
}

func stop(manager: NEAppProxyProviderManager) {
    if let session = manager.connection as? NETunnelProviderSession {
        session.stopTunnel()
    } else {
        manager.connection.stopVPNTunnel()
    }
}

func sendProviderMessage(
    session: NETunnelProviderSession,
    message: [String: String]
) throws -> [String: Any] {
    let data = try JSONSerialization.data(withJSONObject: message)
    let waiter = SyncWaiter<Result<Data?, Error>>()
    do {
        try session.sendProviderMessage(data) { response in
            waiter.resolve(.success(response))
        }
    } catch {
        throw ControllerError.connection(
            "Failed to send a message to the CatPane network provider: \(error.localizedDescription)."
        )
    }
    let responseData = try waiter.wait()
        .mapError {
            ControllerError.connection("Provider message failed: \($0.localizedDescription)")
        }
        .get()

    guard let responseData else {
        return [:]
    }
    let object = try JSONSerialization.jsonObject(with: responseData, options: [])
    return object as? [String: Any] ?? [:]
}

func extensionBundleIdentifier() -> String {
    "\(bundleIdentifierBase()).throttling-extension"
}

func bundleIdentifierBase() -> String {
    if let env = ProcessInfo.processInfo.environment["CATPANE_BUNDLE_ID_BASE"], !env.isEmpty {
        return env
    }

    let executable = URL(fileURLWithPath: CommandLine.arguments[0]).resolvingSymlinksInPath()
    let infoPlist = executable
        .deletingLastPathComponent() // Helpers
        .deletingLastPathComponent() // Contents
        .appendingPathComponent("Info.plist")
    if let data = try? Data(contentsOf: infoPlist),
       let plist = try? PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any],
       let identifier = plist["CFBundleIdentifier"] as? String,
       !identifier.isEmpty
    {
        return identifier
    }

    return "io.github.catpane"
}

func simulatorAppRule() -> NEAppRule {
    let requirement = #"identifier "com.apple.iphonesimulator" and anchor apple"#
    let rule = NEAppRule(
        signingIdentifier: "com.apple.iphonesimulator",
        designatedRequirement: requirement
    )
    rule.matchPath = simulatorAppPath()
    return rule
}

func simulatorAppPath() -> String? {
    let developer = ProcessInfo.processInfo.environment["DEVELOPER_DIR"] ?? "/Applications/Xcode.app/Contents/Developer"
    let candidate = "\(developer)/Applications/Simulator.app"
    return FileManager.default.fileExists(atPath: candidate) ? candidate : nil
}

func connectionStatusLabel(_ status: NEVPNStatus) -> String {
    switch status {
    case .invalid:
        return "invalid"
    case .disconnected:
        return "disconnected"
    case .connecting:
        return "connecting"
    case .connected:
        return "connected"
    case .reasserting:
        return "reasserting"
    case .disconnecting:
        return "disconnecting"
    @unknown default:
        return "unknown"
    }
}

main()
