import Foundation

/// Commands that share a computer with this app's other devices. The CLI owns all state; these
/// only build its literal arguments and read what it prints.
public enum ShareComputer {
  /// `zuse serve` with the chosen sharing, as a person would type it.
  public static func serveCommand(lan: Bool, tailscaleServe: Bool = false) -> String {
    (["zuse"] + serveArguments(lan: lan, tailscaleServe: tailscaleServe)).joined(separator: " ")
  }
  public static func serveArguments(lan: Bool, tailscaleServe: Bool = false) -> [String] {
    var arguments = ["serve"]
    if lan {
      arguments.append("--lan")
      // The CLI only accepts --tailscale-serve together with --lan.
      if tailscaleServe { arguments.append("--tailscale-serve") }
    }
    return arguments
  }
  /// Keeps `zuse serve` running across restarts, with the same sharing flags.
  public static func serviceArguments(lan: Bool, tailscaleServe: Bool = false) -> [String] {
    ["daemon", "install", "--serve"]
      + Array(serveArguments(lan: lan, tailscaleServe: tailscaleServe).dropFirst())
  }
  public static let removeServiceArguments = ["daemon", "uninstall"]
  public static let statusArguments = ["status"]
  public static func pairArguments(label: String) -> [String] {
    ["pair", "--json", "--label", label]
  }
  public static let pairCommand = "zuse pair --app"
  public static let linkCommand = "zuse link"
}

/// One line from `zuse pair --json`.
public struct PairJSON: Sendable, Equatable {
  public let app: String?
  public let browser: String?
  public let selfSigned: Bool
  public let localOnly: Bool
  /// The link an app should open.
  public var invitation: String? { app ?? browser }

  public init(output: String) throws {
    for line in output.split(whereSeparator: \.isNewline).reversed() {
      let trimmed = line.trimmingCharacters(in: .whitespaces)
      guard trimmed.hasPrefix("{"),
        let value = try? JSONDecoder().decode(JSONValue.self, from: Data(trimmed.utf8)),
        case .object = value
      else { continue }
      app = value["app"].string
      browser = value["browser"].string
      selfSigned = value["selfSigned"].bool
      localOnly = value["localOnly"].bool
      return
    }
    throw ClientError("Zuse did not print an invitation. Update Zuse on this computer.")
  }
}
