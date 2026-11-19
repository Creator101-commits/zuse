import CryptoKit
import Foundation
import ZuseCore

#if canImport(Citadel)
  import Citadel
  import NIOCore
  import NIOPosix
  import NIOSSH
#endif

/// This device's SSH key: created once, private half kept in the Keychain.
enum SSHIdentity {
  private static let keychainKey = "ssh-p256-key"
  static func privateKey() throws -> P256.Signing.PrivateKey {
    if let raw = try Vault.read(keychainKey) {
      return try P256.Signing.PrivateKey(rawRepresentation: raw)
    }
    let key = P256.Signing.PrivateKey()
    try Vault.write(key.rawRepresentation, key: keychainKey)
    return key
  }
  /// The `authorized_keys` line for this device.
  static func publicKeyLine(deviceName: String) throws -> String {
    SSHKeyFormat.p256OpenSSH(
      x963: try privateKey().publicKey.x963Representation,
      comment: "zuse-" + deviceName.replacingOccurrences(of: " ", with: "-"))
  }
}

enum SSHCredential: Sendable {
  case deviceKey
  /// Used once to sign in and install the device key. Never stored.
  case password(String)
}

/// One signed-in SSH connection to a computer: runs commands and forwards local ports.
final class SSHSession: @unchecked Sendable {
  let target: SSHTarget
  /// The host key presented on this connection, `SHA256:…`.
  let hostKey: String

  #if canImport(Citadel)
    private let client: SSHClient
    private let lock = NSLock()
    private var listeners: [Channel] = []

    private init(target: SSHTarget, hostKey: String, client: SSHClient) {
      self.target = target
      self.hostKey = hostKey
      self.client = client
    }

    var isConnected: Bool { client.isConnected }

    static func connect(
      _ target: SSHTarget, expectedHostKey: String?, credential: SSHCredential
    ) async throws -> SSHSession {
      let recorder = HostKeyRecorder(expected: expectedHostKey)
      let method: SSHAuthenticationMethod
      switch credential {
      case .deviceKey:
        method = .p256(username: target.user, privateKey: try SSHIdentity.privateKey())
      case .password(let password):
        method = .passwordBased(username: target.user, password: password)
      }
      do {
        let client = try await SSHClient.connect(
          host: target.host, port: target.port, authenticationMethod: method,
          hostKeyValidator: .custom(recorder), reconnect: .never,
          connectTimeout: .seconds(15))
        guard let presented = recorder.presented else {
          try? await client.close()
          throw ClientError("The computer did not present an SSH host key.")
        }
        return SSHSession(target: target, hostKey: presented, client: client)
      } catch {
        if recorder.changed {
          throw ClientError(SSHHostKeyCheck.changedMessage(target), code: "identity-changed")
        }
        if let error = error as? ClientError { throw error }
        throw describe(
          error, target: target, credential: credential, reached: recorder.presented != nil)
      }
    }

    private static func describe(
      _ error: Error, target: SSHTarget, credential: SSHCredential, reached: Bool
    ) -> ClientError {
      let text = String(describing: error).lowercased()
      if reached || text.contains("auth") {
        switch credential {
        case .deviceKey:
          return ClientError(
            "\(target.display) didn’t accept this device’s key. Add it to ~/.ssh/authorized_keys on the computer, or sign in once with your password.",
            code: "ssh-key-rejected")
        case .password:
          return ClientError("\(target.display) didn’t accept that password.")
        }
      }
      return ClientError(
        "Couldn’t reach \(target.host) on port \(target.port). Check the address, that the computer is on, and that SSH (Remote Login) is allowed."
      )
    }

    /// Runs one POSIX `sh` body. Output and exit status always come back; only transport
    /// failures throw.
    func run(_ body: String, timeout: Duration = .seconds(45)) async throws -> RemoteCommandResult {
      let command = RemoteZuse.exec(body)
      let work = Task { () throws -> String in
        let buffer = try await self.client.executeCommand(
          command, maxResponseSize: 1 << 20, mergeStreams: true)
        return String(buffer: buffer)
      }
      // Cancelling ends the output stream early; the missing exit status marks the timeout.
      let timer = Task {
        try? await Task.sleep(for: timeout)
        work.cancel()
      }
      defer { timer.cancel() }
      let text = try await work.value
      let result = RemoteZuse.parse(text)
      if result.status == nil && work.isCancelled {
        throw ClientError("The computer did not answer in time.")
      }
      return result
    }

    /// Listens on 127.0.0.1 and pipes each connection to `host:port` as the computer sees it.
    func forward(toHost host: String, port: Int) async throws -> Int {
      let originator = try SocketAddress(ipAddress: "127.0.0.1", port: 0)
      let bootstrap = ServerBootstrap(group: MultiThreadedEventLoopGroup.singleton)
        .serverChannelOption(ChannelOptions.socketOption(.so_reuseaddr), value: 1)
        .childChannelOption(ChannelOptions.autoRead, value: false)
        .childChannelInitializer { local in
          let promise = local.eventLoop.makePromise(of: Void.self)
          promise.completeWithTask {
            let (localGlue, remoteGlue) = PipeHandler.pair()
            _ = try await self.client.createDirectTCPIPChannel(
              using: SSHChannelType.DirectTCPIP(
                targetHost: host, targetPort: port, originatorAddress: originator)
            ) { remote in
              remote.pipeline.addHandler(remoteGlue)
            }
            try await local.pipeline.addHandler(localGlue).get()
            local.read()
          }
          promise.futureResult.whenFailure { _ in local.close(promise: nil) }
          return promise.futureResult
        }
      let listener = try await bootstrap.bind(host: "127.0.0.1", port: 0).get()
      guard let local = listener.localAddress?.port else {
        try? await listener.close()
        throw ClientError("Could not open a local port for the SSH tunnel.")
      }
      lock.withLock { listeners.append(listener) }
      return local
    }

    func stopForwarding(_ port: Int) async {
      let matching = lock.withLock {
        let found = listeners.filter { $0.localAddress?.port == port }
        listeners.removeAll { $0.localAddress?.port == port }
        return found
      }
      for listener in matching { try? await listener.close() }
    }

    func close() async {
      let open = lock.withLock {
        let current = listeners
        listeners = []
        return current
      }
      for listener in open { try? await listener.close() }
      try? await client.close()
    }
  #else
    private init(target: SSHTarget, hostKey: String) {
      self.target = target
      self.hostKey = hostKey
    }
    static let unavailable = ClientError(
      "SSH isn’t available in this build. Add the Citadel package to the app target.")
    var isConnected: Bool { false }
    static func connect(
      _ target: SSHTarget, expectedHostKey: String?, credential: SSHCredential
    ) async throws -> SSHSession { throw unavailable }
    func run(_ body: String, timeout: Duration = .seconds(45)) async throws -> RemoteCommandResult {
      throw Self.unavailable
    }
    func forward(toHost host: String, port: Int) async throws -> Int { throw Self.unavailable }
    func stopForwarding(_ port: Int) async {}
    func close() async {}
  #endif
}

#if canImport(Citadel)
  /// Trust on first use: records what the computer presents and refuses a changed key.
  private final class HostKeyRecorder: NIOSSHClientServerAuthenticationDelegate,
    @unchecked Sendable
  {
    private let expected: String?
    private let lock = NSLock()
    private var seen: String?
    private var mismatch = false
    init(expected: String?) { self.expected = expected }
    var presented: String? { lock.withLock { seen } }
    var changed: Bool { lock.withLock { mismatch } }

    func validateHostKey(
      hostKey: NIOSSHPublicKey, validationCompletePromise: EventLoopPromise<Void>
    ) {
      let line = String(openSSHPublicKey: hostKey)
      guard let fingerprint = SSHFingerprint.sha256(openSSHPublicKey: line) else {
        validationCompletePromise.fail(
          ClientError("The computer presented an unreadable host key."))
        return
      }
      let check = SSHHostKeyCheck.verify(saved: expected, presented: fingerprint)
      lock.withLock {
        seen = fingerprint
        mismatch = check == .changed
      }
      if check == .changed {
        validationCompletePromise.fail(ClientError("The SSH host key changed."))
      } else {
        validationCompletePromise.succeed(())
      }
    }
  }

  /// Joins two channels byte for byte, with backpressure, until either side closes.
  private final class PipeHandler: ChannelDuplexHandler, @unchecked Sendable {
    typealias InboundIn = NIOAny
    typealias OutboundIn = NIOAny
    typealias OutboundOut = NIOAny

    private var partner: PipeHandler?
    private var context: ChannelHandlerContext?
    private var pendingRead = false

    static func pair() -> (PipeHandler, PipeHandler) {
      let first = PipeHandler()
      let second = PipeHandler()
      first.partner = second
      second.partner = first
      return (first, second)
    }

    private func partnerWrite(_ data: NIOAny) {
      context?.write(data, promise: nil)
    }
    private func partnerFlush() { context?.flush() }
    private func partnerClose() { context?.close(promise: nil) }
    private func partnerBecameWritable() {
      if pendingRead {
        pendingRead = false
        context?.read()
      }
    }
    private var partnerWritable: Bool { context?.channel.isWritable ?? false }

    func handlerAdded(context: ChannelHandlerContext) {
      self.context = context
      if context.channel.isWritable { partner?.partnerBecameWritable() }
    }
    func handlerRemoved(context: ChannelHandlerContext) {
      self.context = nil
      partner = nil
    }
    func channelRead(context: ChannelHandlerContext, data: NIOAny) { partner?.partnerWrite(data) }
    func channelReadComplete(context: ChannelHandlerContext) { partner?.partnerFlush() }
    func channelInactive(context: ChannelHandlerContext) { partner?.partnerClose() }
    func errorCaught(context: ChannelHandlerContext, error: Error) {
      partner?.partnerClose()
      context.close(promise: nil)
    }
    func channelWritabilityChanged(context: ChannelHandlerContext) {
      if context.channel.isWritable { partner?.partnerBecameWritable() }
    }
    func read(context: ChannelHandlerContext) {
      if let partner, partner.partnerWritable {
        context.read()
      } else {
        pendingRead = true
      }
    }
  }
#endif

/// One SSH session per computer, shared by the open connection, connection checks and the
/// add flow. A session closes after a minute with nobody using it.
actor SSHSessionPool {
  static let shared = SSHSessionPool()
  private struct Entry {
    let session: SSHSession
    var users: Int
    var idle: Task<Void, Never>?
  }
  private var entries: [SSHTarget: Entry] = [:]
  private var connecting: [SSHTarget: Task<SSHSession, Error>] = [:]

  /// The caller must `release` the session when it no longer needs it.
  func acquire(
    _ target: SSHTarget, expectedHostKey: String?, credential: SSHCredential = .deviceKey
  ) async throws -> SSHSession {
    if var entry = entries[target] {
      if entry.session.isConnected {
        entry.idle?.cancel()
        entry.idle = nil
        entry.users += 1
        entries[target] = entry
        return entry.session
      }
      entries[target] = nil
      let stale = entry.session
      Task { await stale.close() }
    }
    let task: Task<SSHSession, Error>
    if let pending = connecting[target] {
      task = pending
    } else {
      task = Task {
        try await SSHSession.connect(
          target, expectedHostKey: expectedHostKey, credential: credential)
      }
      connecting[target] = task
    }
    let session: SSHSession
    do {
      session = try await task.value
    } catch {
      if connecting[target] == task { connecting[target] = nil }
      throw error
    }
    if connecting[target] == task { connecting[target] = nil }
    if var entry = entries[target], entry.session === session {
      entry.users += 1
      entry.idle?.cancel()
      entry.idle = nil
      entries[target] = entry
    } else {
      entries[target] = Entry(session: session, users: 1, idle: nil)
    }
    return session
  }

  func release(_ session: SSHSession) {
    guard var entry = entries[session.target], entry.session === session else { return }
    entry.users = max(0, entry.users - 1)
    if entry.users == 0 {
      let target = session.target
      entry.idle = Task { [weak self] in
        try? await Task.sleep(for: .seconds(60))
        guard !Task.isCancelled else { return }
        await self?.closeIdle(target, session: session)
      }
    }
    entries[session.target] = entry
  }

  /// Ends a session now, for example after its computer is forgotten.
  func close(_ target: SSHTarget) async {
    guard let entry = entries.removeValue(forKey: target) else { return }
    entry.idle?.cancel()
    await entry.session.close()
  }

  private func closeIdle(_ target: SSHTarget, session: SSHSession) async {
    guard let entry = entries[target], entry.session === session, entry.users == 0 else { return }
    entries[target] = nil
    await session.close()
  }
}
