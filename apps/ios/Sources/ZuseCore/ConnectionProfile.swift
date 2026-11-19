import Foundation

/// Credentials are encoded only into the device's Keychain by the application.
public struct RemoteConnection: Codable, Identifiable, Equatable, Sendable {
  public var address: String
  public var token: String
  public var pairingLink: String?
  public var certificatePin: String?
  public var environmentID: String?
  public var label: String?
  /// Set when the computer was added over SSH. Older saved entries decode without it.
  public var ssh: SSHAccess?
  /// When this device last had a working connection, for "last connected 5 min ago".
  public var lastConnectedAt: Date?
  /// The route the last connection used, for status and reachability tips.
  public var lastRoute: RouteKind?
  public var id: String {
    // The endpoint distinguishes separate transports to the same environment.
    // Tokens never participate in display names, identities, or archive paths.
    guard var url = URLComponents(string: address) else { return address }
    url.query = nil
    url.fragment = nil
    url.user = nil
    url.password = nil
    return url.string ?? address
  }
  public var displayName: String {
    if let label, !label.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return label }
    return URLComponents(string: address)?.host ?? "Computer"
  }
  public var transportName: String {
    if ssh?.tunnel == true { return "SSH tunnel" }
    if let lastRoute { return lastRoute.label }
    let host = URLComponents(string: address)?.host?.lowercased() ?? ""
    if RouteKind.isTailnet(host) { return "Tailscale" }
    if PairingLink.isPrivateHost(host) { return "Local network" }
    return "Remote"
  }
  /// The route a connection most likely uses, from what it used last or its address.
  public var expectedRoute: RouteKind? {
    if let lastRoute { return lastRoute }
    guard let host = URLComponents(string: address)?.host else { return nil }
    return RouteKind.classify(host: host)
  }
  /// How this device was introduced to the computer.
  public var connectedThrough: String {
    if let ssh { return "SSH " + ssh.target.display }
    if pairingLink != nil { return "Pairing link" }
    return token.isEmpty ? "Local connection" : "Connection token"
  }
  public init(
    address: String, token: String, certificatePin: String? = nil,
    environmentID: String? = nil, label: String? = nil, pairingLink: String? = nil,
    ssh: SSHAccess? = nil
  ) {
    self.ssh = ssh
    self.address = address
    self.token = token
    self.pairingLink = pairingLink
    self.certificatePin = certificatePin
    self.environmentID = environmentID
    self.label = label
  }
  public func validated() throws -> RemoteConnection {
    guard let url = URL(string: address), url.fragment == nil,
      URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems?.contains(where: {
        $0.name == "token"
      }) != true
    else { throw ClientError("Enter the endpoint without a token or pairing code in the address.") }
    try PairingLink.validate(url, webSocket: true)
    if certificatePin != nil && url.scheme != "wss" {
      throw ClientError("A pinned computer requires a secure WebSocket connection.")
    }
    return self
  }
}

public enum ConnectionProfiles {
  public static func saving(_ connection: RemoteConnection, in profiles: [RemoteConnection])
    -> [RemoteConnection]
  {
    // Pairing the same SSH computer again replaces its older invitation.
    var result = profiles.filter {
      $0.id != connection.id
        && (connection.ssh == nil || $0.ssh?.target != connection.ssh?.target)
    }
    result.insert(connection, at: 0)
    return result
  }
}
