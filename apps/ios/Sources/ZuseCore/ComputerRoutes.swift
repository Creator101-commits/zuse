import Foundation

/// How a paired connection reaches its computer, as the native link reports it.
/// Mirrors the Rust route classes so labels match every other client.
public enum RouteKind: String, Codable, Sendable, CaseIterable {
  case loopback
  case lan
  case tailnet
  case `public`
  case relay

  public var label: String {
    switch self {
    case .loopback: "This computer"
    case .lan: "Local network"
    case .tailnet: "Tailscale"
    case .public: "Direct"
    case .relay: "Relay"
    }
  }

  /// Classifies an address alone. A self-hosted control server (Headscale) hands out the same
  /// ranges as Tailscale, so those addresses are treated as a tailnet too.
  public static func classify(host value: String) -> RouteKind {
    let host = bare(value)
    if isLoopback(host) { return .loopback }
    if isTailnet(host) { return .tailnet }
    if PairingLink.isPrivateHost(host) || isSharedAddressSpace(host) { return .lan }
    return .public
  }

  public static func isLoopback(_ value: String) -> Bool {
    let host = bare(value)
    if host == "localhost" || host.hasSuffix(".localhost") || host == "::1" { return true }
    if let octets = ipv4(host) { return octets[0] == 127 }
    return expandedIPv6(host) == [0, 0, 0, 0, 0, 0, 0, 1]
  }

  public static func isTailnet(_ value: String) -> Bool {
    let host = bare(value)
    if host.hasSuffix(".ts.net") { return true }
    if ipv4(host) != nil { return isSharedAddressSpace(host) }
    guard let groups = expandedIPv6(host) else { return false }
    return groups[0] == 0xfd7a && groups[1] == 0x115c && groups[2] == 0xa1e0
  }

  /// 100.64.0.0/10, the carrier-grade NAT block tailnets use.
  static func isSharedAddressSpace(_ host: String) -> Bool {
    guard let octets = ipv4(host) else { return false }
    return octets[0] == 100 && (64..<128).contains(octets[1])
  }

  static func bare(_ value: String) -> String {
    var host = value.lowercased().trimmingCharacters(in: .whitespaces)
    if host.hasPrefix("[") && host.hasSuffix("]") { host = String(host.dropFirst().dropLast()) }
    if let zone = host.firstIndex(of: "%") { host = String(host[..<zone]) }
    while host.hasSuffix(".") { host.removeLast() }
    return host
  }

  static func ipv4(_ host: String) -> [Int]? {
    let parts = host.split(separator: ".", omittingEmptySubsequences: false)
    guard parts.count == 4 else { return nil }
    let values = parts.compactMap { part -> Int? in
      guard !part.isEmpty, part.count <= 3, part.allSatisfy(\.isASCII) else { return nil }
      return Int(part)
    }
    guard values.count == 4, values.allSatisfy({ (0...255).contains($0) }) else { return nil }
    return values
  }

  static func expandedIPv6(_ host: String) -> [Int]? {
    guard host.contains(":") else { return nil }
    let halves = host.components(separatedBy: "::")
    guard halves.count <= 2 else { return nil }
    func groups(_ text: String) -> [Int]? {
      if text.isEmpty { return [] }
      let parts = text.split(separator: ":", omittingEmptySubsequences: false)
      var values: [Int] = []
      for part in parts {
        guard (1...4).contains(part.count), let value = Int(part, radix: 16) else { return nil }
        values.append(value)
      }
      return values
    }
    guard let head = groups(halves[0]) else { return nil }
    if halves.count == 1 { return head.count == 8 ? head : nil }
    guard let tail = groups(halves[1]), head.count + tail.count < 8 else { return nil }
    return head + Array(repeating: 0, count: 8 - head.count - tail.count) + tail
  }
}

/// A native invitation link, `<origin>/#pair=<base64url json>`, and where it points.
public struct InvitationLink: Sendable, Equatable {
  public static let loopbackOnlyMessage =
    "This link only works on that computer. On it, turn on Settings → Devices → Direct connections, or run `zuse serve --lan`, then create a new link."

  public let link: String
  public let scheme: String
  /// The host as written in the origin, without IPv6 brackets.
  public let host: String
  public let port: Int
  public var kind: RouteKind { RouteKind.classify(host: host) }
  /// Only reachable on the computer that created it, unless something forwards the port.
  public var isLoopback: Bool { kind == .loopback }

  public init(_ value: String) throws {
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    guard PairingLink.isNativeInvitation(trimmed), let url = URLComponents(string: trimmed),
      let scheme = url.scheme?.lowercased(), scheme == "http" || scheme == "https",
      let host = url.host, !host.isEmpty, url.user == nil, url.password == nil
    else { throw ClientError("This is not a Zuse invitation link.") }
    let port = url.port ?? (scheme == "https" ? 443 : 80)
    guard (1...65535).contains(port) else {
      throw ClientError("This invitation has an invalid port.")
    }
    link = trimmed
    self.scheme = scheme
    self.host = RouteKind.bare(host)
    self.port = port
  }

  /// The same invitation through another origin, such as a local SSH forward.
  /// Noise authenticates the computer, so the route is only a pipe.
  public func rewritingOrigin(host newHost: String = "127.0.0.1", port newPort: Int) -> String {
    guard var url = URLComponents(string: link) else { return link }
    url.scheme = "http"
    url.host = newHost
    url.port = newPort
    url.path = "/"
    url.query = nil
    return url.string ?? link
  }

  /// The last invitation link in free text, such as pasted messages or command output.
  public static func extract(from text: String) -> String? {
    guard
      let expression = try? NSRegularExpression(
        pattern: #"https?://[^\s"'<>]+#pair=[A-Za-z0-9_\-]+"#)
    else { return nil }
    let range = NSRange(text.startIndex..<text.endIndex, in: text)
    guard let match = expression.matches(in: text, range: range).last,
      let found = Range(match.range, in: text)
    else { return nil }
    return String(text[found])
  }
}
