import SwiftUI
import ZuseCore

#if os(iOS)
  import AVFoundation
  import UIKit
  import VisionKit
#endif

enum ConnectionMethod: String, Identifiable {
  case add
  var id: String { rawValue }
}

extension View {
  /// Presents Add computer: scan or paste an invitation, or set the computer up over SSH.
  func connectionFlow(_ method: Binding<ConnectionMethod?>, workspace: WorkspaceModel)
    -> some View
  {
    sheet(item: method) { _ in AddComputerSheet(workspace: workspace) }
  }
}

struct AddComputerSheet: View {
  let workspace: WorkspaceModel
  @Environment(\.dismiss) private var dismiss
  var body: some View {
    NavigationStack {
      AddComputerView(workspace: workspace) { dismiss() }
        .navigationTitle("Add computer").compactNavigationTitle()
        .toolbar {
          ToolbarItem(placement: .sheetClose) { SheetCloseButton { dismiss() } }
        }
    }.nativeSheetSize(width: 480, height: 640).tint(.primary)
  }
}

struct AddComputerView: View {
  let workspace: WorkspaceModel
  let finished: () -> Void
  @State private var link = ""
  @State private var error: String?
  @State private var busy = false
  @State private var request = UUID()
  @State private var scanGeneration = 0
  @State private var succeeded = 0
  @State private var failed = 0
  @State private var showWhere = false
  @State private var showAnywhere = false

  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 28) {
        #if os(iOS)
          CameraCard(generation: scanGeneration, paused: busy) { submit($0) }
        #endif
        pasteSection
        NavigationLink {
          SSHConnectView(workspace: workspace, finished: finished)
        } label: {
          HStack(spacing: 12) {
            SettingsIcon(symbol: "terminal")
            VStack(alignment: .leading, spacing: 2) {
              Text("Connect with SSH").foregroundStyle(.primary)
              Text("Sign in to the computer and let Zuse set it up").font(.footnote)
                .foregroundStyle(.secondary)
            }
            Spacer(minLength: 8)
            Image(systemName: "chevron.right").font(.footnote.weight(.semibold))
              .foregroundStyle(.tertiary)
          }
          .padding(14)
          .background(AppTheme.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
          .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        VStack(alignment: .leading, spacing: 0) {
          HelpDisclosure(title: "Where do I find the link?", expanded: $showWhere) {
            Text("**Desktop:** open Zuse, then Settings → Devices → Connect a device.")
            Text(
              "**Headless or server:** run these on the computer, then scan or paste the link it prints."
            )
            CopyableCommand(command: ShareComputer.serveCommand(lan: true))
            CopyableCommand(command: ShareComputer.pairCommand)
          }
          Divider().padding(.vertical, 4)
          HelpDisclosure(title: "Reach it from anywhere", expanded: $showAnywhere) {
            Text(
              "Install Tailscale (or use your own Headscale server) on both devices, and turn on Direct connections in Zuse on the computer. The link then works wherever you are."
            )
            Text("Or link the computer to your Zuse account and reach it after signing in here:")
            CopyableCommand(command: ShareComputer.linkCommand)
          }
        }
      }
      .padding(20).frame(maxWidth: 560).frame(maxWidth: .infinity)
    }
    .background(AppTheme.surface)
    .sensoryFeedback(.success, trigger: succeeded)
    .sensoryFeedback(.error, trigger: failed)
    .onDisappear {
      request = UUID()
      busy = false
    }
  }

  private var pasteSection: some View {
    VStack(alignment: .leading, spacing: 8) {
      Text("Or paste a link").font(.subheadline.weight(.semibold))
      HStack(spacing: 8) {
        TextField("http://…/#pair=…", text: $link)
          .verbatimInput().autocorrectionDisabled()
          .font(.system(.body, design: .monospaced)).lineLimit(1)
          .submitLabel(.go).onSubmit { submit(link) }
          .accessibilityLabel("Invitation link")
        PasteButton(payloadType: String.self) { values in
          guard let value = values.first else { return }
          Task { @MainActor in
            link = value
            submit(value)
          }
        }
        .labelStyle(.titleOnly).buttonBorderShape(.capsule).controlSize(.small)
      }
      .padding(.leading, 12).padding(.trailing, 6).frame(minHeight: 46)
      .background(AppTheme.card, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
      .overlay {
        RoundedRectangle(cornerRadius: 12, style: .continuous)
          .strokeBorder(error == nil ? Color.primary.opacity(0.1) : Color.red, lineWidth: 0.5)
      }
      if let error {
        Text((try? AttributedString(markdown: error)) ?? AttributedString(error))
          .font(.footnote).foregroundStyle(.red).textSelection(.enabled)
          .fixedSize(horizontal: false, vertical: true)
      }
      if busy {
        HStack(spacing: 8) {
          ProgressView().controlSize(.small)
          Text("Connecting…").font(.footnote).foregroundStyle(.secondary)
        }
      } else if !link.isEmpty {
        Button("Connect") { submit(link) }.buttonStyle(.borderedProminent)
          .tint(AppTheme.accent).buttonBorderShape(.capsule)
      }
    }
  }

  private func submit(_ value: String) {
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !busy, !trimmed.isEmpty else { return }
    let current = UUID()
    request = current
    busy = true
    error = nil
    Task {
      defer { if request == current { busy = false } }
      do {
        try await workspace.pair(trimmed)
        guard request == current else { return }
        succeeded += 1
        finished()
      } catch is CancellationError {
      } catch {
        guard request == current else { return }
        self.error = error.localizedDescription
        failed += 1
        scanGeneration += 1
      }
    }
  }
}

private struct HelpDisclosure<Content: View>: View {
  let title: String
  @Binding var expanded: Bool
  @ViewBuilder let content: Content
  var body: some View {
    VStack(alignment: .leading, spacing: 10) {
      Button {
        withAnimation(.snappy(duration: 0.2)) { expanded.toggle() }
      } label: {
        HStack {
          Text(title).font(.subheadline.weight(.medium))
          Spacer()
          Image(systemName: "chevron.right").font(.footnote.weight(.semibold))
            .foregroundStyle(.tertiary).rotationEffect(.degrees(expanded ? 90 : 0))
        }
        .frame(minHeight: 40).contentShape(Rectangle())
      }
      .buttonStyle(.plain)
      .accessibilityValue(expanded ? "Expanded" : "Collapsed")
      if expanded {
        VStack(alignment: .leading, spacing: 10) { content }
          .font(.footnote).foregroundStyle(.secondary)
          .fixedSize(horizontal: false, vertical: true)
          .padding(.bottom, 8)
      }
    }
  }
}

/// A shell command with a copy button.
struct CopyableCommand: View {
  let command: String
  @State private var copied = false
  var body: some View {
    HStack(spacing: 8) {
      Text(command).font(.system(.footnote, design: .monospaced)).foregroundStyle(.primary)
        .textSelection(.enabled).lineLimit(2)
      Spacer(minLength: 4)
      Button {
        NativePlatform.copy(command)
        copied = true
        Task {
          try? await Task.sleep(for: .seconds(1.5))
          copied = false
        }
      } label: {
        Image(systemName: copied ? "checkmark" : "doc.on.doc").frame(width: 28, height: 28)
          .contentShape(Rectangle())
      }
      .buttonStyle(.plain).foregroundStyle(.secondary)
      .accessibilityLabel(copied ? "Copied" : "Copy \(command)")
    }
    .padding(.leading, 10).padding(.trailing, 4).padding(.vertical, 4)
    .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: 8))
  }
}

#if os(iOS)
  /// The scanner inside a rounded card; it asks for the camera only when shown.
  private struct CameraCard: View {
    let generation: Int
    let paused: Bool
    let scanned: (String) -> Void
    @Environment(\.openURL) private var openURL
    private enum Phase: Equatable {
      case checking, denied, unavailable, scanning
      case failed(String)
    }
    @State private var phase: Phase = .checking
    var body: some View {
      ZStack {
        Color.black
        switch phase {
        case .checking:
          ProgressView().tint(.white)
        case .denied:
          VStack(spacing: 10) {
            Image(systemName: "camera").font(.system(size: 28)).foregroundStyle(.white.opacity(0.7))
            Text("Allow camera access to scan the code on your computer.")
              .font(.footnote).foregroundStyle(.white.opacity(0.8)).multilineTextAlignment(.center)
            Button("Open Settings") {
              if let url = URL(string: UIApplication.openSettingsURLString) { openURL(url) }
            }
            .buttonStyle(.borderedProminent).tint(.white).foregroundStyle(.black)
            .buttonBorderShape(.capsule).controlSize(.small)
          }.padding(24)
        case .unavailable:
          Text("This device can’t scan codes. Paste the link instead.")
            .font(.footnote).foregroundStyle(.white.opacity(0.8)).multilineTextAlignment(.center)
            .padding(24)
        case .failed(let message):
          VStack(spacing: 10) {
            Text(message).font(.footnote).foregroundStyle(.white.opacity(0.8))
              .multilineTextAlignment(.center)
            Button("Try again") { phase = .scanning }.buttonStyle(.bordered).tint(.white)
              .buttonBorderShape(.capsule).controlSize(.small)
          }.padding(24)
        case .scanning:
          QRScanner(scanned: scanned, onError: { phase = .failed($0) })
            .id(generation)
            .opacity(paused ? 0.4 : 1)
          RoundedRectangle(cornerRadius: 18, style: .continuous)
            .strokeBorder(.white.opacity(0.85), lineWidth: 1.5)
            .frame(width: 168, height: 168).allowsHitTesting(false)
        }
      }
      .frame(height: 280)
      .clipShape(RoundedRectangle(cornerRadius: 22, style: .continuous))
      .overlay(alignment: .bottom) {
        if phase == .scanning {
          Text("Point at the QR code on your computer")
            .font(.caption.weight(.medium)).foregroundStyle(.white)
            .padding(.horizontal, 10).padding(.vertical, 5)
            .background(.black.opacity(0.45), in: Capsule()).padding(.bottom, 12)
        }
      }
      .task { await checkCamera() }
      .accessibilityElement(children: .contain)
      .accessibilityLabel("QR code scanner")
    }
    private func checkCamera() async {
      switch AVCaptureDevice.authorizationStatus(for: .video) {
      case .authorized: break
      case .notDetermined:
        guard await AVCaptureDevice.requestAccess(for: .video) else {
          phase = .denied
          return
        }
      default:
        phase = .denied
        return
      }
      phase =
        DataScannerViewController.isSupported && DataScannerViewController.isAvailable
        ? .scanning : .unavailable
    }
  }
#endif
