import Foundation
import Testing

@testable import ZuseCore

#if canImport(CryptoKit)
  import CryptoKit
#else
  import Crypto
#endif

private let fragment =
  "eyJ2IjoxLCJpZCI6IjAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwIiwic2VjcmV0IjoiQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQSJ9"

@Test func sshTargetsAcceptTheFormsPeopleType() throws {
  #expect(try SSHTarget.parse("me@studio.local") == SSHTarget(user: "me", host: "studio.local"))
  #expect(
    try SSHTarget.parse(" me@studio.local:2222 ")
      == SSHTarget(user: "me", host: "studio.local", port: 2222))
  #expect(
    try SSHTarget.parse("ssh://me@10.0.0.4:2200/")
      == SSHTarget(user: "me", host: "10.0.0.4", port: 2200))
  #expect(try SSHTarget.parse("me@[fd7a:115c:a1e0::1]:22").host == "fd7a:115c:a1e0::1")
  #expect(try SSHTarget.parse("me@fe80::1").host == "fe80::1")
  #expect(SSHTarget(user: "me", host: "::1", port: 2222).display == "me@[::1]:2222")
  #expect(SSHTarget(user: "me", host: "mac").display == "me@mac")
  for invalid in ["studio.local", "@host", "me@", "me@host:abc", "me@host:70000", "m e@host"] {
    #expect(throws: ClientError.self) { try SSHTarget.parse(invalid) }
  }
}

@Test func invitationsAreClassifiedAndRewrittenForATunnel() throws {
  let loopback = try InvitationLink("http://127.0.0.1:27656/#pair=" + fragment)
  #expect(loopback.isLoopback)
  #expect(loopback.port == 27656)
  #expect(loopback.rewritingOrigin(port: 50123) == "http://127.0.0.1:50123/#pair=" + fragment)
  #expect(try InvitationLink("http://localhost:1/#pair=" + fragment).isLoopback)
  #expect(try InvitationLink("http://192.168.1.20:27657/#pair=" + fragment).kind == .lan)
  #expect(try InvitationLink("http://100.101.1.2:27657/#pair=" + fragment).kind == .tailnet)
  #expect(try InvitationLink("http://[fd7a:115c:a1e0::5]:27657/#pair=" + fragment).kind == .tailnet)
  #expect(try InvitationLink("https://studio.tail1234.ts.net/#pair=" + fragment).kind == .tailnet)
  #expect(try InvitationLink("https://zuse.example.com/#pair=" + fragment).kind == .public)
  #expect(throws: ClientError.self) { try InvitationLink("https://example.com/#pair=nope") }
  #expect(!InvitationLink.loopbackOnlyMessage.isEmpty)
}

@Test func routeKindsMatchTheRustClassifier() {
  #expect(RouteKind.classify(host: "127.0.0.5") == .loopback)
  #expect(RouteKind.classify(host: "[::1]") == .loopback)
  #expect(RouteKind.classify(host: "10.0.0.8") == .lan)
  #expect(RouteKind.classify(host: "fe80::1%en0") == .lan)
  #expect(RouteKind.classify(host: "100.64.0.1") == .tailnet)
  #expect(RouteKind.classify(host: "100.128.0.1") == .public)
  #expect(RouteKind.classify(host: "box.headscale.example") == .public)
  #expect(RouteKind.relay.label == "Relay")
  #expect(RouteKind(rawValue: "public") == .public)
}

@Test func pairOutputYieldsTheInvitation() {
  let link = "http://127.0.0.1:27656/#pair=" + fragment
  let json = #"{"app":"\#(link)","browser":null,"selfSigned":false,"localOnly":true}"#
  #expect(RemoteZuse.invitation(fromPairOutput: "noise\n" + json + "\n") == link)
  let plain = """
    Open in a browser: https://old.example/#pair=\(fragment)
    Open in the app: \(link)
    """
  #expect(RemoteZuse.invitation(fromPairOutput: plain) == link)
  #expect(RemoteZuse.invitation(fromPairOutput: "nothing here") == nil)
}

@Test func remoteCommandsCarryTheirExitStatus() {
  let command = RemoteZuse.exec("zuse --version")
  #expect(command.hasPrefix("sh -c '"))
  #expect(command.contains("$HOME/.local/bin"))
  #expect(command.contains("/Applications/Zuse.app/Contents/MacOS"))
  let result = RemoteZuse.parse("zuse 0.4.2\n__ZUSE_EXIT__0\n")
  #expect(result == RemoteCommandResult(output: "zuse 0.4.2", status: 0))
  #expect(RemoteZuse.versionNumber(result.output) == "0.4.2")
  #expect(RemoteZuse.parse("garbage").status == nil)
  let missing = RemoteZuse.parse("sh: 1: zuse: not found\n__ZUSE_EXIT__127")
  #expect(RemoteZuse.classify(missing) == .notInstalled)
  let stopped = RemoteZuse.parse(
    "No Zuse server is running on agent port 27654. Start zuse serve first.\n__ZUSE_EXIT__1")
  #expect(RemoteZuse.classify(stopped) == .notRunning)
  #expect(RemoteZuse.classify(RemoteZuse.parse("boom\n__ZUSE_EXIT__2")) == .other("boom"))
}

@Test func remoteCommandStringsStayQuoteSafe() throws {
  #expect(RemoteZuse.safeLabel("Ann's iPhone; rm -rf ~") == "Ann s iPhone rm -rf")
  #expect(RemoteZuse.safeLabel("'''") == "Phone")
  #expect(RemoteZuse.quote("a'b") == #"'a'\''b'"#)
  #expect(RemoteZuse.pair(label: "Ann's iPhone").contains("--label 'Ann s iPhone'"))
  #expect(RemoteZuse.pair(label: "x").contains("zuse pair --json"))
  #expect(RemoteZuse.startInBackground.contains("nohup zuse serve"))
  #expect(RemoteZuse.installService == "zuse daemon install --serve")
  let install = try RemoteZuse.installKey("ecdsa-sha2-nistp256 AAAA phone")
  #expect(install.contains("authorized_keys"))
  #expect(throws: ClientError.self) { try RemoteZuse.installKey("ssh-ed25519 AAAA x'; rm") }
}

@Test func keysAndFingerprintsUseOpenSSHFormats() throws {
  let key = P256.Signing.PrivateKey()
  let line = SSHKeyFormat.p256OpenSSH(x963: key.publicKey.x963Representation, comment: "zuse@phone")
  let fields = line.split(separator: " ")
  #expect(fields.count == 3)
  #expect(fields[0] == "ecdsa-sha2-nistp256")
  let blob = try #require(Data(base64Encoded: String(fields[1])))
  #expect(blob.count == 4 + 19 + 4 + 8 + 4 + 65)
  // "AAAA" decodes to three zero bytes; the fingerprint is SHA-256 of that blob.
  #expect(
    SSHFingerprint.sha256(openSSHPublicKey: "ssh-ed25519 AAAA")
      == "SHA256:cJ6AyISHokEeHuTfufIqhhSS0gxHZRUMDHlKvXD4FHw")
  #expect(SSHFingerprint.sha256(openSSHPublicKey: "garbage") == nil)
  #expect(SSHHostKeyCheck.verify(saved: nil, presented: "SHA256:a") == .firstUse)
  #expect(SSHHostKeyCheck.verify(saved: "SHA256:a", presented: "SHA256:a") == .trusted)
  #expect(SSHHostKeyCheck.verify(saved: "SHA256:a", presented: "SHA256:b") == .changed)
}

@Test func computerStatusLinesReadNaturally() {
  let now = Date(timeIntervalSince1970: 10_000)
  #expect(
    ComputerStatusText.line(.connected, route: "Local network", rttMs: 12)
      == "Connected · Local network · 12 ms")
  #expect(ComputerStatusText.line(.connected) == "Connected")
  #expect(ComputerStatusText.line(.connecting) == "Connecting…")
  #expect(
    ComputerStatusText.line(.unreachable, lastConnected: now.addingTimeInterval(-300), now: now)
      == "Can’t reach · last connected 5 min ago")
  #expect(ComputerStatusText.line(.unreachable) == "Can’t reach")
  #expect(ComputerStatusText.line(.idle) == "Not connected")
  #expect(ComputerStatusText.ago(now.addingTimeInterval(-7200), now: now) == "2 hours ago")
  #expect(
    ComputerStatusText.unreachableTip(route: .tailnet, name: "Studio")
      == "Turn on Tailscale on this phone (Headscale works too).")
  #expect(
    ComputerStatusText.unreachableTip(route: .lan, name: "Studio")
      == "Join the same Wi‑Fi as Studio, or use Tailscale or SSH to reach it from anywhere.")
  #expect(ComputerStatusText.unreachableTip(route: nil, name: "Studio") == nil)
}
