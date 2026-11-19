import Foundation
import Testing

@testable import ZuseCore

@Test func sharingUsesOnlyCommandsTheCLIAccepts() {
  #expect(ShareComputer.serveCommand(lan: false) == "zuse serve")
  #expect(ShareComputer.serveCommand(lan: true) == "zuse serve --lan")
  #expect(
    ShareComputer.serveCommand(lan: true, tailscaleServe: true)
      == "zuse serve --lan --tailscale-serve")
  // --tailscale-serve requires --lan, so it is dropped without it.
  #expect(ShareComputer.serveArguments(lan: false, tailscaleServe: true) == ["serve"])
  #expect(
    ShareComputer.serviceArguments(lan: true) == ["daemon", "install", "--serve", "--lan"])
  #expect(ShareComputer.serviceArguments(lan: false) == ["daemon", "install", "--serve"])
  #expect(ShareComputer.statusArguments == ["status"])
  #expect(
    ShareComputer.pairArguments(label: "Studio") == ["pair", "--json", "--label", "Studio"])
}

@Test func pairJSONGivesTheAppLinkAndFlags() throws {
  let output = """
    Starting…
    {"app":"http://192.168.1.20:27657/#pair=abc","browser":null,"selfSigned":false,"localOnly":false}
    """
  let parsed = try PairJSON(output: output)
  #expect(parsed.invitation == "http://192.168.1.20:27657/#pair=abc")
  #expect(!parsed.localOnly)
  let local = try PairJSON(
    output: #"{"app":null,"browser":"https://127.0.0.1:27659/#pair=x","#
      + #""selfSigned":true,"localOnly":true}"#)
  #expect(local.invitation == "https://127.0.0.1:27659/#pair=x")
  #expect(local.selfSigned && local.localOnly)
  #expect(throws: ClientError.self) { try PairJSON(output: "No Zuse server is running") }
}
