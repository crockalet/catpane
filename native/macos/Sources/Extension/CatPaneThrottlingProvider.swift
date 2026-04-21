import Foundation
import Network
import NetworkExtension
import OSLog

private let providerQueue = DispatchQueue(label: "io.github.crockalet.catpane.throttling.provider")

private enum ProviderMessageError: LocalizedError {
    case invalidMessage

    var errorDescription: String? {
        switch self {
        case .invalidMessage:
            return "Invalid provider message."
        }
    }
}

private enum Preset: String {
    case unthrottled
    case edge
    case threeG = "3g"
    case offline

    var baseLatency: TimeInterval {
        switch self {
        case .unthrottled:
            return 0
        case .edge:
            return 0.35
        case .threeG:
            return 0.12
        case .offline:
            return 0
        }
    }

    var bytesPerSecond: Double? {
        switch self {
        case .unthrottled:
            return nil
        case .edge:
            return 30_000
        case .threeG:
            return 180_000
        case .offline:
            return nil
        }
    }

    func delay(for byteCount: Int) -> TimeInterval {
        guard let bytesPerSecond else {
            return baseLatency
        }
        return baseLatency + (Double(byteCount) / bytesPerSecond)
    }
}

private final class ProviderState {
    private let lock = NSLock()
    private var preset: Preset = .unthrottled

    func setPreset(_ preset: Preset) {
        lock.lock()
        self.preset = preset
        lock.unlock()
    }

    func getPreset() -> Preset {
        lock.lock()
        let value = preset
        lock.unlock()
        return value
    }
}

final class CatPaneThrottlingProvider: NEAppProxyProvider, NEAppProxyUDPFlowHandling {
    private let logger = Logger(subsystem: "io.github.crockalet.catpane.throttling", category: "provider")
    private let state = ProviderState()
    private let relayLock = NSLock()
    private var tcpRelays: [UUID: TCPRelay] = [:]
    private var udpRelays: [UUID: UDPRelay] = [:]

    override func startProxy(options: [String: Any]? = nil, completionHandler: @escaping (Error?) -> Void) {
        let preset = resolvePreset(options: options)
        state.setPreset(preset)
        logger.notice("CatPane app proxy started with preset \(preset.rawValue, privacy: .public).")
        completionHandler(nil)
    }

    override func stopProxy(with reason: NEProviderStopReason, completionHandler: @escaping () -> Void) {
        logger.notice("CatPane app proxy stopping with reason \(reason.rawValue, privacy: .public).")
        relayLock.lock()
        let tcp = Array(tcpRelays.values)
        let udp = Array(udpRelays.values)
        tcpRelays.removeAll()
        udpRelays.removeAll()
        relayLock.unlock()
        tcp.forEach { $0.cancel() }
        udp.forEach { $0.cancel() }
        completionHandler()
    }

    override func handleNewFlow(_ flow: NEAppProxyFlow) -> Bool {
        guard let tcpFlow = flow as? NEAppProxyTCPFlow else {
            return false
        }

        if state.getPreset() == .offline {
            reject(flow: tcpFlow)
            return true
        }

        let relay = TCPRelay(provider: self, flow: tcpFlow)
        store(relay: relay)
        relay.start()
        return true
    }

    func handleNewUDPFlow(
        _ flow: NEAppProxyUDPFlow,
        initialRemoteFlowEndpoint remoteEndpoint: Network.NWEndpoint
    ) -> Bool {
        if state.getPreset() == .offline {
            reject(flow: flow)
            return true
        }

        guard let session = makeUDPSession(for: remoteEndpoint, localEndpoint: flow.localEndpoint as? NWHostEndpoint) else {
            logger.error("Unsupported UDP endpoint for CatPane throttling relay.")
            return false
        }

        let relay = UDPRelay(
            provider: self,
            flow: flow,
            remoteEndpoint: remoteEndpoint,
            session: session
        )
        store(relay: relay)
        relay.start()
        return true
    }

    override func handleAppMessage(_ messageData: Data, completionHandler: ((Data?) -> Void)? = nil) {
        do {
            let object = try JSONSerialization.jsonObject(with: messageData) as? [String: String]
            guard let object, let command = object["command"] else {
                throw ProviderMessageError.invalidMessage
            }

            switch command {
            case "setPreset":
                let preset = try resolvePreset(rawValue: object["preset"])
                state.setPreset(preset)
                if preset == .offline {
                    cancelActiveRelays()
                }
                completionHandler?(encodeResponse(["ok": true, "preset": preset.rawValue]))
            case "status":
                completionHandler?(encodeResponse(["ok": true, "preset": state.getPreset().rawValue]))
            default:
                throw ProviderMessageError.invalidMessage
            }
        } catch {
            logger.error("Provider message failed: \(error.localizedDescription, privacy: .public)")
            completionHandler?(encodeResponse(["ok": false, "error": error.localizedDescription]))
        }
    }

    fileprivate func currentPreset() -> Preset {
        state.getPreset()
    }

    fileprivate func connectionDelay(for bytes: Int) -> TimeInterval {
        currentPreset().delay(for: bytes)
    }

    fileprivate func closeTCPRelay(_ relay: TCPRelay) {
        relayLock.lock()
        tcpRelays.removeValue(forKey: relay.id)
        relayLock.unlock()
    }

    fileprivate func closeUDPRelay(_ relay: UDPRelay) {
        relayLock.lock()
        udpRelays.removeValue(forKey: relay.id)
        relayLock.unlock()
    }

    private func store(relay: TCPRelay) {
        relayLock.lock()
        tcpRelays[relay.id] = relay
        relayLock.unlock()
    }

    private func store(relay: UDPRelay) {
        relayLock.lock()
        udpRelays[relay.id] = relay
        relayLock.unlock()
    }

    private func cancelActiveRelays() {
        relayLock.lock()
        let tcp = Array(tcpRelays.values)
        let udp = Array(udpRelays.values)
        relayLock.unlock()
        tcp.forEach { $0.cancel() }
        udp.forEach { $0.cancel() }
    }

    private func reject(flow: NEAppProxyFlow) {
        let error = NSError(
            domain: NEAppProxyErrorDomain,
            code: NEAppProxyFlowError.refused.rawValue,
            userInfo: [NSLocalizedDescriptionKey: "CatPane simulator throttling is set to offline."]
        )
        flow.closeReadWithError(error)
        flow.closeWriteWithError(error)
    }

    private func resolvePreset(options: [String: Any]?) -> Preset {
        if let raw = options?["preset"] as? String, let parsed = try? resolvePreset(rawValue: raw) {
            return parsed
        }
        if let proto = protocolConfiguration as? NETunnelProviderProtocol,
           let raw = proto.providerConfiguration?["preset"] as? String,
           let parsed = try? resolvePreset(rawValue: raw)
        {
            return parsed
        }
        return .unthrottled
    }

    private func resolvePreset(rawValue: String?) throws -> Preset {
        guard let rawValue, let preset = Preset(rawValue: rawValue) else {
            throw ProviderMessageError.invalidMessage
        }
        return preset
    }

    private func encodeResponse(_ payload: [String: Any]) -> Data? {
        try? JSONSerialization.data(withJSONObject: payload)
    }

    private func makeUDPSession(for endpoint: Network.NWEndpoint, localEndpoint: NWHostEndpoint?) -> NWUDPSession? {
        switch endpoint {
        case let .hostPort(host, port):
            return createUDPSession(
                to: NWHostEndpoint(hostname: host.debugDescription, port: String(port.rawValue)),
                from: localEndpoint
            )
        case let .service(name, type, domain, _):
            return createUDPSession(
                to: NWBonjourServiceEndpoint(name: name, type: type, domain: domain),
                from: localEndpoint
            )
        case .unix, .url, .opaque:
            return nil
        @unknown default:
            return nil
        }
    }
}

private final class TCPRelay {
    let id = UUID()
    private weak var provider: CatPaneThrottlingProvider?
    private let flow: NEAppProxyTCPFlow
    private let connection: NWTCPConnection
    private let closeLock = NSLock()
    private var closed = false
    private var clientReadClosed = false
    private var remoteReadClosed = false

    init(provider: CatPaneThrottlingProvider, flow: NEAppProxyTCPFlow) {
        self.provider = provider
        self.flow = flow
        self.connection = provider.createTCPConnection(
            to: flow.remoteEndpoint,
            enableTLS: false,
            tlsParameters: nil,
            delegate: nil
        )
    }

    func start() {
        flow.open(withLocalEndpoint: nil) { [weak self] error in
            guard let self else { return }
            if let error {
                self.cancel(with: error)
                return
            }
            self.pumpClientToRemote()
            self.pumpRemoteToClient()
        }
    }

    func cancel() {
        cancel(with: nil)
    }

    private func cancel(with error: Error?) {
        closeLock.lock()
        if closed {
            closeLock.unlock()
            return
        }
        closed = true
        closeLock.unlock()

        flow.closeReadWithError(error)
        flow.closeWriteWithError(error)
        connection.writeClose()
        connection.cancel()
        provider?.closeTCPRelay(self)
    }

    private func markClientClosed() {
        closeLock.lock()
        clientReadClosed = true
        let shouldFinish = clientReadClosed && remoteReadClosed
        closeLock.unlock()
        if shouldFinish {
            cancel()
        }
    }

    private func markRemoteClosed() {
        closeLock.lock()
        remoteReadClosed = true
        let shouldFinish = clientReadClosed && remoteReadClosed
        closeLock.unlock()
        if shouldFinish {
            cancel()
        }
    }

    private func pumpClientToRemote() {
        if isClosedOrOffline() { return }
        flow.readData { [weak self] data, error in
            guard let self else { return }
            if let error {
                self.cancel(with: error)
                return
            }
            guard let data else {
                self.markClientClosed()
                self.connection.writeClose()
                return
            }
            if data.isEmpty {
                self.markClientClosed()
                self.connection.writeClose()
                return
            }

            let delay = self.provider?.connectionDelay(for: data.count) ?? 0
            providerQueue.asyncAfter(deadline: .now() + delay) { [weak self] in
                guard let self else { return }
                if self.isClosedOrOffline() { return }
                self.connection.write(data) { error in
                    if let error {
                        self.cancel(with: error)
                    } else {
                        self.pumpClientToRemote()
                    }
                }
            }
        }
    }

    private func pumpRemoteToClient() {
        if isClosedOrOffline() { return }
        connection.readMinimumLength(1, maximumLength: 16 * 1024) { [weak self] data, error in
            guard let self else { return }
            if let error {
                self.cancel(with: error)
                return
            }
            guard let data else {
                self.markRemoteClosed()
                self.flow.closeWriteWithError(nil)
                return
            }
            if data.isEmpty {
                self.markRemoteClosed()
                self.flow.closeWriteWithError(nil)
                return
            }

            let delay = self.provider?.connectionDelay(for: data.count) ?? 0
            providerQueue.asyncAfter(deadline: .now() + delay) { [weak self] in
                guard let self else { return }
                if self.isClosedOrOffline() { return }
                self.flow.write(data) { error in
                    if let error {
                        self.cancel(with: error)
                    } else {
                        self.pumpRemoteToClient()
                    }
                }
            }
        }
    }

    private func isClosedOrOffline() -> Bool {
        closeLock.lock()
        let isClosed = closed
        closeLock.unlock()
        if isClosed {
            return true
        }
        if provider?.currentPreset() == .offline {
            cancel(with: NSError(
                domain: NEAppProxyErrorDomain,
                code: NEAppProxyFlowError.refused.rawValue,
                userInfo: [NSLocalizedDescriptionKey: "CatPane simulator throttling switched to offline."]
            ))
            return true
        }
        return false
    }
}

private final class UDPRelay {
    let id = UUID()
    private weak var provider: CatPaneThrottlingProvider?
    private let flow: NEAppProxyUDPFlow
    private let remoteEndpoint: Network.NWEndpoint
    private let session: NWUDPSession
    private let closeLock = NSLock()
    private var closed = false

    init(
        provider: CatPaneThrottlingProvider,
        flow: NEAppProxyUDPFlow,
        remoteEndpoint: Network.NWEndpoint,
        session: NWUDPSession
    ) {
        self.provider = provider
        self.flow = flow
        self.remoteEndpoint = remoteEndpoint
        self.session = session
    }

    func start() {
        flow.open(withLocalFlowEndpoint: nil) { [weak self] error in
            guard let self else { return }
            if let error {
                self.cancel(with: error)
                return
            }

            self.session.setReadHandler({ [weak self] datagrams, error in
                guard let self else { return }
                if let error {
                    self.cancel(with: error)
                    return
                }
                guard let datagrams, !datagrams.isEmpty else {
                    return
                }

                let delay = self.provider?.connectionDelay(for: datagrams.reduce(0) { $0 + $1.count }) ?? 0
                providerQueue.asyncAfter(deadline: .now() + delay) { [weak self] in
                    guard let self else { return }
                    if self.isClosedOrOffline() { return }
                    let items: [(Data, Network.NWEndpoint)] = datagrams.map { ($0, self.remoteEndpoint) }
                    self.flow.writeDatagrams(items) { error in
                        if let error {
                            self.cancel(with: error)
                        }
                    }
                }
            }, maxDatagrams: 32)

            self.pumpFlowToRemote()
        }
    }

    func cancel() {
        cancel(with: nil)
    }

    private func cancel(with error: Error?) {
        closeLock.lock()
        if closed {
            closeLock.unlock()
            return
        }
        closed = true
        closeLock.unlock()
        flow.closeReadWithError(error)
        flow.closeWriteWithError(error)
        session.cancel()
        provider?.closeUDPRelay(self)
    }

    private func pumpFlowToRemote() {
        if isClosedOrOffline() { return }
        flow.readDatagrams { [weak self] packets, error in
            guard let self else { return }
            if let error {
                self.cancel(with: error)
                return
            }
            guard let packets else {
                self.cancel()
                return
            }
            if packets.isEmpty {
                self.pumpFlowToRemote()
                return
            }

            let datagrams = packets.map(\.0)
            let totalBytes = datagrams.reduce(0) { $0 + $1.count }
            let delay = self.provider?.connectionDelay(for: totalBytes) ?? 0
            providerQueue.asyncAfter(deadline: .now() + delay) { [weak self] in
                guard let self else { return }
                if self.isClosedOrOffline() { return }
                self.session.writeMultipleDatagrams(datagrams) { error in
                    if let error {
                        self.cancel(with: error)
                    } else {
                        self.pumpFlowToRemote()
                    }
                }
            }
        }
    }

    private func isClosedOrOffline() -> Bool {
        closeLock.lock()
        let isClosed = closed
        closeLock.unlock()
        if isClosed {
            return true
        }
        if provider?.currentPreset() == .offline {
            cancel(with: NSError(
                domain: NEAppProxyErrorDomain,
                code: NEAppProxyFlowError.refused.rawValue,
                userInfo: [NSLocalizedDescriptionKey: "CatPane simulator throttling switched to offline."]
            ))
            return true
        }
        return false
    }
}
