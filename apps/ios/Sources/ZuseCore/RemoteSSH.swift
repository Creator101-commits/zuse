import Foundation

#if canImport(CryptoKit)
  import CryptoKit
#else
  import Crypto
#endif

/// `user@host[:port]` for an SSH sign-in. Holds no credentials.
public struct SSHTarget: Codable, Hashable, Sendable {
  public let user: String
  public let host: String
  public let port: Int

  public init(user: String, host: String, port: Int = 22) {
    self.user = user
    self.host = host
    self.port = port
  }

  /// Accepts `user@host`, `user@host:2222`, `ssh://user@host:2222` and `user@[ipv6]:port`.
  public static func parse(_ value: String) throws -> SSHTarget {
    var text = value.trimmingCharacters(in: .whitespacesAndNewlines)
    if text.lowercased().hasPrefix("ssh://") { text = String(text.dropFirst(6)) }
    while text.hasSuffix("/") { text.removeLast() }
    guard let at = text.lastIndex(of: "@") else {
      throw ClientError("Enter the computer as user@host, for example me@studio.local.")
    }
    let user = String(text[..<at])
    var rest = String(text[text.index(after: at)...])
    guard !user.isEmpty, user.count <= 64,
      user.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || "._-".contains($0)) })
    else { throw ClientError("Enter a valid user name before the @.") }
    var port = 22
    var host: String
    if rest.hasPrefix("[") {
      guard let close = rest.firstIndex(of: "]") else {
        throw ClientError("Close the IPv6 address with ].")
      }
      host = String(rest[rest.index(after: rest.startIndex)..<close])
      rest = String(rest[rest.index(after: close)...])
      if !rest.isEmpty {
        guard rest.hasPrefix(":"), let value = Int(rest.dropFirst()) else {
          throw ClientError("Enter the port as a number, for example user@[::1]:2222.")
        }
        port = value
      }
    } else if rest.filter({ $0 == ":" }).count == 1, let colon = rest.firstIndex(of: ":") {
      host = String(rest[..<colon])
      guard let value = Int(rest[rest.index(after: colon)...]) else {
        throw ClientError("Enter the port as a number, for example user@host:2222.")
      }
      port = value
    } else {
      // A bare IPv6 address without brackets has no port.
      host = rest
    }
    host = host.trimmingCharacters(in: .whitespaces)
    guard !host.isEmpty, host.count <= 253,
      host.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || ".-:%_".contains($0)) })
    else { throw ClientError("Enter a host name or address after the @.") }
    guard (1...65535).contains(port) else { throw ClientError("SSH ports are 1 to 65535.") }
    return SSHTarget(user: user, host: host, port: port)
  }

  /// What people type: `user@host`, with the port only when it is not 22.
  public var display: String {
    let address = host.contains(":") ? "[\(host)]" : host
    return port == 22 ? "\(user)@\(address)" : "\(user)@\(address):\(port)"
  }
}

/// How a saved computer is reached over SSH. The private key stays in the Keychain;
/// a password is never stored.
public struct SSHAccess: Codable, Equatable, Sendable {
  public var target: SSHTarget
  /// `SHA256:…`, trusted on first use.
  public var hostKey: String?
  /// The invitation only answers on the computer itself, so every connection runs through a
  /// local port forward over SSH.
  public var tunnel: Bool

  public init(target: SSHTarget, hostKey: String?, tunnel: Bool) {
    self.target = target
    self.hostKey = hostKey
    self.tunnel = tunnel
  }
}

public enum SSHFingerprint {
  /// `SHA256:<base64 without padding>` of the key blob, as `ssh-keygen -l` prints it.
  public static func sha256(openSSHPublicKey line: String) -> String? {
    let fields = line.split(whereSeparator: \.isWhitespace)
    guard fields.count >= 2, let blob = Data(base64Encoded: String(fields[1])), !blob.isEmpty
    else { return nil }
    return sha256(blob: blob)
  }
  public static func sha256(blob: Data) -> String {
    let digest = Data(SHA256.hash(data: blob)).base64EncodedString()
    return "SHA256:" + digest.replacingOccurrences(of: "=", with: "")
  }
}

public enum SSHHostKeyCheck: Equatable, Sendable {
  case firstUse
  case trusted
  case changed

  public static func verify(saved: String?, presented: String) -> SSHHostKeyCheck {
    guard let saved, !saved.isEmpty else { return .firstUse }
    return saved == presented ? .trusted : .changed
  }

  public static func changedMessage(_ target: SSHTarget) -> String {
    "\(target.host) is presenting a different host key than before, so Zuse stopped. If you reinstalled the computer or reset SSH, forget it here and add it again. Otherwise someone may be intercepting the connection."
  }
}

public enum SSHKeyFormat {
  /// An `authorized_keys` line for a P-256 public key in X9.63 form (65 bytes).
  public static func p256OpenSSH(x963 point: Data, comment: String) -> String {
    let name = "ecdsa-sha2-nistp256"
    var blob = Data()
    for field in [Data(name.utf8), Data("nistp256".utf8), point] {
      var length = UInt32(field.count).bigEndian
      blob.append(Data(bytes: &length, count: 4))
      blob.append(field)
    }
    let safe = comment.filter { $0.isASCII && ($0.isLetter || $0.isNumber || "@._-".contains($0)) }
    return "\(name) \(blob.base64EncodedString())" + (safe.isEmpty ? "" : " \(safe)")
  }
}

/// The result of one remote command, with output and exit status combined in one stream.
public struct RemoteCommandResult: Sendable, Equatable {
  public let output: String
  public let status: Int?
  public var succeeded: Bool { status == 0 }
  public init(output: String, status: Int?) {
    self.output = output
    self.status = status
  }
}

/// Commands the app runs on a computer over SSH. Bodies are POSIX `sh`; each one runs through
/// `sh -c` so the login shell's own syntax does not matter.
public enum RemoteZuse {
  /// Non-interactive SSH shells skip profiles, so add the usual install locations.
  public static let pathPrefix =
    #"PATH="$HOME/.local/bin:$HOME/.zuse/bin:$HOME/bin:/usr/local/bin:/opt/homebrew/bin:/Applications/Zuse.app/Contents/MacOS:$PATH"; export PATH; "#
  static let exitMarker = "__ZUSE_EXIT__"

  public static let version = "zuse --version"
  public static let startInBackground =
    #"nohup zuse serve >"$HOME/.zuse-serve.log" 2>&1 </dev/null &"#
  public static let installService = "zuse daemon install --serve"

  /// Falls back to the plain form for computers with a CLI from before `--json`.
  public static func pair(label: String) -> String {
    let name = quote(safeLabel(label))
    return "ZUSE_LANGUAGE=en zuse pair --json --label \(name)"
      + " || ZUSE_LANGUAGE=en zuse pair --label \(name)"
  }

  /// Adds this device's key to `authorized_keys` once.
  public static func installKey(_ line: String) throws -> String {
    guard !line.contains("'"), !line.contains("\n"),
      line.hasPrefix("ecdsa-sha2-") || line.hasPrefix("ssh-")
    else { throw ClientError("This device's SSH key is invalid.") }
    let key = quote(line)
    return #"umask 077; mkdir -p "$HOME/.ssh" && touch "$HOME/.ssh/authorized_keys" && "#
      + #"(grep -qxF "# + key + #" "$HOME/.ssh/authorized_keys" || printf '%s\n' "# + key
      + #" >> "$HOME/.ssh/authorized_keys")"#
  }

  /// The full remote command: profile-free PATH, merged output and a trailing exit status, so
  /// the session always sees success and the status travels in the output.
  public static func exec(_ body: String) -> String {
    let script = pathPrefix + "{\n" + body + "\n} 2>&1\necho \"" + exitMarker + "$?\""
    return "sh -c " + quote(script) + " 2>&1"
  }

  public static func parse(_ text: String) -> RemoteCommandResult {
    guard let marker = text.range(of: exitMarker, options: .backwards) else {
      return RemoteCommandResult(
        output: text.trimmingCharacters(in: .whitespacesAndNewlines), status: nil)
    }
    let code = text[marker.upperBound...].prefix { $0.isNumber }
    return RemoteCommandResult(
      output: String(text[..<marker.lowerBound]).trimmingCharacters(in: .whitespacesAndNewlines),
      status: Int(code))
  }

  /// POSIX single quoting.
  public static func quote(_ value: String) -> String {
    "'" + value.replacingOccurrences(of: "'", with: #"'\''"#) + "'"
  }

  /// Device names travel through two shells; keep them to plain, quote-free characters.
  public static func safeLabel(_ value: String) -> String {
    let kept = value.map { character -> Character in
      character.isLetter || character.isNumber || " ._-()".contains(character) ? character : " "
    }
    let collapsed = String(kept).split(separator: " ").joined(separator: " ")
    return collapsed.isEmpty ? "Phone" : String(collapsed.prefix(64))
  }

  /// The invitation `zuse pair --json` printed, or the last link in plain output.
  public static func invitation(fromPairOutput output: String) -> String? {
    for line in output.split(whereSeparator: \.isNewline).reversed() {
      let trimmed = line.trimmingCharacters(in: .whitespaces)
      guard trimmed.hasPrefix("{"),
        let value = try? JSONDecoder().decode(JSONValue.self, from: Data(trimmed.utf8))
      else { continue }
      if let app = value["app"].string, PairingLink.isNativeInvitation(app) { return app }
      if let browser = value["browser"].string, PairingLink.isNativeInvitation(browser) {
        return browser
      }
    }
    return InvitationLink.extract(from: output)
  }

  public enum Failure: Equatable, Sendable {
    case notInstalled
    case notRunning
    case other(String)
  }

  public static func classify(_ result: RemoteCommandResult) -> Failure {
    let text = result.output.lowercased()
    if result.status == 127 || text.contains("command not found")
      || text.contains("zuse: not found")
    {
      return .notInstalled
    }
    if text.contains("no zuse server is running") || text.contains("start zuse serve") {
      return .notRunning
    }
    let last = result.output.split(whereSeparator: \.isNewline).last.map(String.init)
    return .other(last ?? "The command failed.")
  }

  /// The `zuse --version` line, without the program name.
  public static func versionNumber(_ output: String) -> String? {
    guard let line = output.split(whereSeparator: \.isNewline).first else { return nil }
    let words = line.split(separator: " ")
    return words.last { $0.first?.isNumber == true }.map(String.init) ?? String(line)
  }
}
