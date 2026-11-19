import SwiftUI
import ZuseCore

/// macOS: share this Mac with `zuse serve` and pair it directly.
/// iOS: the commands to run on a desktop or server.
struct ServeSettingsView: View {
  let workspace: WorkspaceModel
  @State private var lan = true
  @State private var tailscaleServe = false
  @State private var busy = false
  @State private var status: String?
  @State private var notice: String?
  @State private var failure: String?
  @AppStorage("native.serveExecutable") private var executable = "zuse"

  var body: some View {
    Form {
      #if os(macOS)
        Section {
          Text(
            "Keep Zuse running on this Mac so your phone and other devices can connect to it."
          )
          .font(.callout).foregroundStyle(.secondary)
          Toggle("Allow devices on this network", isOn: $lan)
          Toggle("Also share over Tailscale HTTPS", isOn: $tailscaleServe).disabled(!lan)
          LabeledContent("Command") {
            Text(ShareComputer.serveCommand(lan: lan, tailscaleServe: tailscaleServe))
              .font(.system(.callout, design: .monospaced)).textSelection(.enabled)
          }
          HStack {
            Button("Stop sharing", role: .destructive) {
              Task { await run(ShareComputer.removeServiceArguments, done: "Sharing stopped.") }
            }
            Spacer()
            Button("Keep running") {
              Task {
                await run(
                  ShareComputer.serviceArguments(lan: lan, tailscaleServe: tailscaleServe),
                  done: "Zuse will keep running, including after a restart.")
              }
            }.buttonStyle(.borderedProminent)
          }
          Button("Pair this Mac", systemImage: "laptopcomputer") {
            Task { await pairThisMac() }
          }
        } header: {
          Text("Share this Mac")
        } footer: {
          Text("Runs `zuse daemon install --serve` with the options above.")
        }
        Section("Status") {
          if let status {
            Text(status).font(.system(.caption, design: .monospaced)).textSelection(.enabled)
          } else {
            Text("No status yet.").font(.caption).foregroundStyle(.secondary)
          }
          Button("Refresh", systemImage: "arrow.clockwise") { Task { await refresh() } }
        }
        Section("Zuse command line") {
          TextField("Executable", text: $executable).verbatimInput()
        }
      #else
        Section {
          Text("On a desktop, open Zuse and choose Settings → Devices → Connect a device.")
            .font(.callout).foregroundStyle(.secondary)
        } header: {
          Text("Desktop")
        }
        Section {
          Text("Run this on the computer to accept connections from your network:")
            .font(.callout).foregroundStyle(.secondary)
          CopyableCommand(command: ShareComputer.serveCommand(lan: true))
          Text("Then print a link to scan or paste here:").font(.callout)
            .foregroundStyle(.secondary)
          CopyableCommand(command: ShareComputer.pairCommand)
          Text("To keep it running after a restart:").font(.callout).foregroundStyle(.secondary)
          CopyableCommand(
            command: "zuse " + ShareComputer.serviceArguments(lan: true).joined(separator: " "))
        } header: {
          Text("Headless or server")
        } footer: {
          Text("Or skip the commands: Add computer → Connect with SSH sets it up for you.")
        }
        Section {
          Text(
            "Use Tailscale (or your own Headscale server) on both devices, or link the computer to your Zuse account:"
          )
          .font(.callout).foregroundStyle(.secondary)
          CopyableCommand(command: ShareComputer.linkCommand)
        } header: {
          Text("Reach it from anywhere")
        }
      #endif
      if let notice { Text(notice).font(.footnote).foregroundStyle(.secondary) }
      if let failure { Text(failure).font(.footnote).foregroundStyle(.red) }
    }
    .settingsList()
    #if os(macOS)
      .navigationTitle("Share this Mac")
      .disabled(busy)
      .overlay { if busy { ProgressView() } }
      .task { await refresh() }
    #else
      .navigationTitle("Set up a computer").compactNavigationTitle()
    #endif
  }

  #if os(macOS)
    private func refresh() async {
      do {
        status = try await ServeCommand.run(
          executable: executable, arguments: ShareComputer.statusArguments, timeout: .seconds(20)
        ).trimmingCharacters(in: .whitespacesAndNewlines)
      } catch {
        status = nil
        failure = error.localizedDescription
      }
    }

    private func run(_ arguments: [String], done: String) async {
      guard !busy else { return }
      busy = true
      failure = nil
      notice = nil
      defer { busy = false }
      do {
        // Output can include invitations; show only a fixed confirmation.
        _ = try await ServeCommand.run(executable: executable, arguments: arguments)
        notice = done
      } catch {
        failure = error.localizedDescription
      }
      await refresh()
    }

    private func pairThisMac() async {
      guard !busy else { return }
      busy = true
      failure = nil
      notice = nil
      defer { busy = false }
      do {
        let output = try await ServeCommand.run(
          executable: executable,
          arguments: ShareComputer.pairArguments(label: NativePlatform.deviceName),
          timeout: .seconds(30))
        guard let link = try PairJSON(output: output).invitation else {
          throw ClientError("Zuse did not return a link. Start sharing first, then try again.")
        }
        // A loopback link is fine here: it points at this Mac.
        try await workspace.pair(link, allowLoopback: true)
        notice = "This Mac is connected."
      } catch {
        failure =
          (error as? ClientError)?.message
          ?? "Couldn’t pair this Mac. Start sharing first, then try again."
      }
    }
  #endif
}
