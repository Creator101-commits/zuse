import Foundation

/// The one thing a computer row says about its connection.
public enum ComputerState: Sendable, Equatable {
  case connected
  case connecting
  case unreachable
  /// Saved but not the open connection.
  case idle
}

public enum ComputerStatusText {
  /// "Connected · Local network · 12 ms", "Connecting…",
  /// "Can’t reach · last connected 5 min ago".
  public static func line(
    _ state: ComputerState, route: String? = nil, rttMs: Int? = nil, lastConnected: Date? = nil,
    now: Date = Date()
  ) -> String {
    switch state {
    case .connected:
      let parts: [String?] = ["Connected", route, rttMs.map { "\($0) ms" }]
      return parts.compactMap { $0 }.joined(separator: " · ")
    case .connecting:
      return "Connecting…"
    case .unreachable:
      return lastConnected.map { "Can’t reach · last connected \(ago($0, now: now))" }
        ?? "Can’t reach"
    case .idle:
      return lastConnected.map { "Last connected \(ago($0, now: now))" } ?? "Not connected"
    }
  }

  public static func ago(_ date: Date, now: Date = Date()) -> String {
    let seconds = max(0, Int(now.timeIntervalSince(date)))
    switch seconds {
    case ..<60: return "just now"
    case ..<3600: return "\(seconds / 60) min ago"
    case ..<86400:
      let hours = seconds / 3600
      return hours == 1 ? "1 hour ago" : "\(hours) hours ago"
    default:
      let days = seconds / 86400
      return days == 1 ? "yesterday" : "\(days) days ago"
    }
  }

  /// A one-line suggestion for an unreachable computer, from the route it last used.
  public static func unreachableTip(route: RouteKind?, name: String, usesSSH: Bool = false)
    -> String?
  {
    if usesSSH {
      return "Check that \(name) is on and that SSH (Remote Login) is allowed, then reconnect."
    }
    switch route {
    case .tailnet: return "Turn on Tailscale on this phone (Headscale works too)."
    case .lan, .loopback:
      return "Join the same Wi‑Fi as \(name), or use Tailscale or SSH to reach it from anywhere."
    case .public, .relay: return "Check that \(name) is on and Zuse is running, then reconnect."
    case nil: return nil
    }
  }
}
