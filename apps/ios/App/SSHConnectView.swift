import SwiftUI
import ZuseCore

#if os(iOS)
  import UIKit
#endif

/// Signs in to a computer over SSH, makes sure Zuse runs there, asks it for an invitation and
/// connects. Loopback-only invitations are reached through an SSH port forward.
struct SSHConnectView: View {
  let workspace: WorkspaceModel
  let finished: () -> Void
  private enum Auth: Hashable { case key, password }
  @AppStorage("native.lastSSHTarget") private var address = ""
  @State private var auth: Auth = .key
  @State private var password = ""
  @State private var publicKey: String?
  @State private var keyProblem: String?
  @State private var steps: [CheckStep] = []
  @State private var running = false
  @State private var failure: String?
  @State private var needsStart = false
  @State private var keepRunning = false
  @State private var lease: SSHSession?
  @State private var flow: Task<Void, Never>?
  @State private var succeeded = 0
  @State private var failed = 0
  @FocusState private var focused: Bool

  private static var deviceKind: String {
    #if os(macOS)
      "Mac"
    #else
      UIDevice.current.userInterfaceIdiom == .pad ? "iPad" : "iPhone"
    #endif
  }

  var body: some View {
    Form {
      Section {
        TextField("user@host", text: $address)
          .verbatimInput().autocorrectionDisabled()
          .font(.system(.body, design: .monospaced)).focused($focused)
          .submitLabel(.go).onSubmit(start)
          .accessibilityLabel("SSH address")
      } header: {
        Text("Computer")
      } footer: {
        Text("Also accepts user@host:2222, ssh://user@host and user@[IPv6 address].")
      }
      Section {
        Picker("Sign in with", selection: $auth) {
          Text("This \(Self.deviceKind)’s key").tag(Auth.key)
          Text("Password").tag(Auth.password)
        }
        .pickerStyle(.segmented).labelsHidden()
        if auth == .key {
          if let publicKey {
            VStack(alignment: .leading, spacing: 8) {
              Text(publicKey).font(.system(.caption2, design: .monospaced))
                .foregroundStyle(.secondary).textSelection(.enabled).lineLimit(4)
              Button {
                NativePlatform.copy(publicKey)
              } label: {
                Label("Copy public key", systemImage: "doc.on.doc")
              }
              .font(.footnote)
            }.padding(.vertical, 2)
          } else if let keyProblem {
            Text(keyProblem).font(.footnote).foregroundStyle(.red)
          }
        } else {
          SecureField("Password", text: $password).textContentType(.password)
        }
      } header: {
        Text("Sign in with")
      } footer: {
        Text(
          auth == .key
            ? "Add it to ~/.ssh/authorized_keys on the computer."
            : "Used once to sign in and add this \(Self.deviceKind)’s key to ~/.ssh/authorized_keys. It is never stored."
        )
      }
      Section {
        Button(action: start) {
          HStack {
            Spacer()
            Text(running ? "Connecting…" : "Connect").fontWeight(.semibold)
            Spacer()
          }
        }
        .disabled(
          running || address.trimmingCharacters(in: .whitespaces).isEmpty
            || (auth == .password && password.isEmpty))
      }
      if !steps.isEmpty {
        Section("Progress") {
          ForEach(steps) { CheckStepRow(step: $0) }
        }
      }
      if needsStart {
        Section {
          Toggle("Keep running after restart", isOn: $keepRunning)
          Button {
            startZuse()
          } label: {
            Label("Start Zuse", systemImage: "play.fill")
          }
          .disabled(running)
        } footer: {
          Text(
            keepRunning
              ? "Installs a service with `zuse daemon install --serve`."
              : "Starts `zuse serve` in the background until the computer restarts.")
        }
      }
      if let failure {
        Section {
          Text((try? AttributedString(markdown: failure)) ?? AttributedString(failure))
            .font(.footnote).foregroundStyle(.red).textSelection(.enabled)
        }
      }
    }
    .settingsList()
    .navigationTitle("Connect with SSH").compactNavigationTitle()
    .sensoryFeedback(.success, trigger: succeeded)
    .sensoryFeedback(.error, trigger: failed)
    .task { loadKey() }
    .onDisappear {
      flow?.cancel()
      flow = nil
      password = ""
      releaseLease()
    }
  }

  private func loadKey() {
    do {
      publicKey = try SSHIdentity.publicKeyLine(deviceName: NativePlatform.deviceName)
      keyProblem = nil
    } catch {
      keyProblem = error.localizedDescription
    }
  }

  private func releaseLease() {
    if let lease {
      Task { await SSHSessionPool.shared.release(lease) }
    }
    lease = nil
  }

  private func update(_ id: String, _ change: (inout CheckStep) -> Void) {
    guard let index = steps.firstIndex(where: { $0.id == id }) else { return }
    change(&steps[index])
  }

  private func fail(_ id: String, _ message: String, fix: String? = nil) {
    update(id) {
      $0.state = .failed
      $0.detail = message
      $0.fix = fix
    }
    failed += 1
  }

  private func start() {
    guard !running else { return }
    focused = false
    flow?.cancel()
    releaseLease()
    failure = nil
    needsStart = false
    steps = [
      CheckStep(id: "connect", title: "Connecting to host"),
      CheckStep(id: "signin", title: "Signed in"),
      CheckStep(id: "zuse", title: "Found Zuse"),
      CheckStep(id: "running", title: "Zuse running"),
      CheckStep(id: "paired", title: "Paired"),
      CheckStep(id: "connected", title: "Connected"),
    ]
    let credential: SSHCredential = auth == .password ? .password(password) : .deviceKey
    flow = Task { await run(credential) }
  }

  private func run(_ credential: SSHCredential) async {
    running = true
    defer { running = false }
    let target: SSHTarget
    do {
      target = try SSHTarget.parse(address)
      address = target.display
    } catch {
      fail("connect", error.localizedDescription)
      return
    }
    update("connect") {
      $0.state = .running
      $0.detail = target.display
    }
    // A computer already saved for this address keeps its trusted host key.
    let known = workspace.savedConnections.first { $0.ssh?.target == target }?.ssh?.hostKey
    var session: SSHSession
    do {
      session = try await SSHSessionPool.shared.acquire(
        target, expectedHostKey: known, credential: credential)
    } catch {
      guard !Task.isCancelled else { return }
      let text = error.localizedDescription
      if text.contains("host key") {
        update("connect") { $0.state = .passed }
        fail("signin", text)
      } else if (error as? ClientError)?.code == "ssh-key-rejected"
        || text.contains("didn’t accept")
      {
        update("connect") { $0.state = .passed }
        fail(
          "signin", text,
          fix: auth == .key ? "Or choose Password to add the key for you." : nil)
      } else {
        fail("connect", text)
      }
      return
    }
    lease = session
    guard !Task.isCancelled else { return }
    update("connect") { $0.state = .passed }
    update("signin") {
      $0.state = .running
      $0.detail = session.hostKey + (known == nil ? " · new, now trusted" : "")
    }
    if case .password = credential {
      password = ""
      do {
        let line = try SSHIdentity.publicKeyLine(deviceName: NativePlatform.deviceName)
        let result = try await session.run(try RemoteZuse.installKey(line), timeout: .seconds(20))
        guard result.succeeded else {
          fail(
            "signin", "Signed in, but couldn’t add this \(Self.deviceKind)’s key.",
            fix: result.output)
          return
        }
        // Prove the key works, and keep the shared session on it from now on.
        let hostKey = session.hostKey
        await SSHSessionPool.shared.release(session)
        lease = nil
        await SSHSessionPool.shared.close(target)
        session = try await SSHSessionPool.shared.acquire(
          target, expectedHostKey: hostKey, credential: .deviceKey)
        lease = session
        auth = .key
      } catch {
        fail(
          "signin", error.localizedDescription,
          fix:
            "The key was added, but the computer didn’t accept it. Check that SSH allows public keys."
        )
        return
      }
    }
    update("signin") { $0.state = .passed }
    guard !Task.isCancelled else { return }

    update("zuse") { $0.state = .running }
    do {
      let version = try await session.run(RemoteZuse.version, timeout: .seconds(20))
      guard version.succeeded else {
        if RemoteZuse.classify(version) == .notInstalled {
          fail(
            "zuse", "Zuse isn’t installed on this computer.",
            fix: "Install Zuse there, then try again.")
        } else {
          let last = version.output.split(whereSeparator: \.isNewline).last.map(String.init)
          fail("zuse", last ?? "Zuse didn’t report its version.")
        }
        return
      }
      update("zuse") {
        $0.state = .passed
        $0.detail = RemoteZuse.versionNumber(version.output).map { "Version \($0)" }
      }
    } catch {
      fail("zuse", error.localizedDescription)
      return
    }
    await pair(session)
  }

  /// `zuse pair` also tells whether the engine runs.
  private func pair(_ session: SSHSession, attempts: Int = 1) async {
    update("running") { $0.state = .running }
    for attempt in 0..<attempts {
      if attempt > 0 {
        try? await Task.sleep(for: .seconds(2))
        guard !Task.isCancelled else { return }
      }
      let result: RemoteCommandResult
      do {
        result = try await session.run(
          RemoteZuse.pair(label: NativePlatform.deviceName), timeout: .seconds(45))
      } catch {
        fail("running", error.localizedDescription)
        return
      }
      if let link = RemoteZuse.invitation(fromPairOutput: result.output) {
        update("running") {
          $0.state = .passed
          $0.detail = nil
        }
        update("paired") { $0.state = .passed }
        await connect(link, session: session)
        return
      }
      switch RemoteZuse.classify(result) {
      case .notRunning:
        if attempt == attempts - 1 {
          update("running") {
            $0.state = .failed
            $0.detail = "Zuse isn’t running on this computer."
          }
          needsStart = true
          failed += 1
        }
      case .notInstalled:
        fail("running", "Zuse isn’t installed on this computer.")
        return
      case .other(let line):
        update("running") { $0.state = .passed }
        fail("paired", line, fix: "Update Zuse on the computer, then try again.")
        return
      }
    }
  }

  private func startZuse() {
    guard let session = lease, !running else { return }
    needsStart = false
    let keep = keepRunning
    flow = Task {
      running = true
      defer { running = false }
      update("running") {
        $0.state = .running
        $0.detail = keep ? "Installing the service…" : "Starting…"
      }
      do {
        let result = try await session.run(
          keep ? RemoteZuse.installService : RemoteZuse.startInBackground, timeout: .seconds(60))
        guard result.succeeded else {
          fail("running", "Couldn’t start Zuse.", fix: result.output)
          needsStart = true
          return
        }
      } catch {
        fail("running", error.localizedDescription)
        needsStart = true
        return
      }
      // The engine takes a moment to open its port.
      await pair(session, attempts: 10)
    }
  }

  private func connect(_ link: String, session: SSHSession) async {
    update("connected") { $0.state = .running }
    let invitation: InvitationLink
    do { invitation = try InvitationLink(link) } catch {
      fail("connected", error.localizedDescription)
      return
    }
    var access = SSHAccess(
      target: session.target, hostKey: session.hostKey, tunnel: invitation.isLoopback)
    if await open(link, access: access) { return }
    guard !Task.isCancelled else { return }
    // The direct address may not be reachable from here; the computer can still forward it.
    if !access.tunnel {
      update("connected") { $0.detail = "Trying through an SSH tunnel…" }
      access.tunnel = true
      if await open(link, access: access) { return }
    }
    guard !Task.isCancelled else { return }
    fail(
      "connected", workspace.connectionProblem ?? "Couldn’t open the connection.",
      fix: "Check that the computer is awake, then try again.")
  }

  private func open(_ link: String, access: SSHAccess) async -> Bool {
    update("connected") {
      $0.detail = access.tunnel ? "Through an SSH tunnel" : (try? InvitationLink(link))?.kind.label
    }
    do {
      try await workspace.pair(link, ssh: access)
    } catch {
      update("connected") { $0.detail = error.localizedDescription }
      return false
    }
    guard let id = workspace.activeConnectionID,
      await workspace.waitForConnection(id, timeout: .seconds(access.tunnel ? 45 : 20))
    else { return false }
    update("connected") { $0.state = .passed }
    succeeded += 1
    releaseLease()
    try? await Task.sleep(for: .milliseconds(700))
    if !Task.isCancelled { finished() }
    return true
  }
}
