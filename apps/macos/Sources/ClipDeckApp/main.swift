import AppKit
import Carbon.HIToolbox
import ClipDeckCore
import SwiftUI

// MARK: - Shared model

final class AppModel: ObservableObject {
    let store: Store
    let sync: SyncStore
    @Published var items: [ClipItem] = []
    @Published var pinboards: [Pinboard] = []
    @Published var query = ""
    @Published var selected = 0
    @Published var filter: String? = nil // nil=all, "pinned", or pinboard id
    @Published var showSettings = false
    @Published var stackOpen = false
    var lastApp: String?
    private var changeCount: Int = NSPasteboard.general.changeCount
    private var lastImageHash = ""

    static let shared = AppModel()

    private init() {
        let dir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("ClipDeck", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        store = Store(userData: dir)
        sync = SyncStore(syncDirectory: store.settings.syncDirectory)
        refresh()
    }

    func refresh() {
        var list = store.items
        if filter == "pinned" {
            list = list.filter { $0.pinned }
        } else if let pb = filter {
            list = list.filter { $0.pinboardId == pb }
        }
        if !query.isEmpty {
            let q = query.lowercased()
            list = list.filter {
                $0.title.lowercased().contains(q) || $0.preview.lowercased().contains(q)
                    || $0.content.lowercased().contains(q)
            }
        }
        items = Array(list.prefix(300))
        pinboards = store.pinboards
        if selected >= items.count { selected = 0 }
    }

    // MARK: capture (§2) — NSPasteboard.changeCount polling (≈400 ms)

    func startWatcher() {
        Timer.scheduledTimer(withTimeInterval: 0.4, repeats: true) { [weak self] _ in
            self?.poll()
        }
        Timer.scheduledTimer(withTimeInterval: 30, repeats: true) { [weak self] _ in
            guard let self else { return }
            let counts = self.sync.merge(into: self.store)
            if counts.imported > 0 {
                self.store.save()
                DispatchQueue.main.async { self.refresh() }
            }
        }
    }

    private func poll() {
        let pb = NSPasteboard.general
        guard pb.changeCount != changeCount else { return }
        changeCount = pb.changeCount
        let s = store.settings
        if s.ignoreTransient,
           let types = pb.types,
           ["org.nspasteboard.TransientType", "de.petermaurer.TransientPasteboardType",
            "com.agilebits.onepassword"].contains(where: { types.contains(NSPasteboard.PasteboardType($0)) }) {
            return
        }
        lastApp = NSWorkspace.shared.frontmostApplication?.localizedName
        if skipByRules() { return }
        if let urls = pb.readObjects(forClasses: [NSURL.self], options: [:]) as? [URL], !urls.isEmpty {
            record(.file, content: urls.map { $0.path }.joined(separator: "\n"))
            return
        }
        if let img = NSImage(pasteboard: pb), let tiff = img.tiffRepresentation,
           let rep = NSBitmapImageRep(data: tiff), let png = rep.representation(using: .png, properties: [:]) {
            let hash = sha256hex(png)
            if hash != lastImageHash {
                lastImageHash = hash
                let name = hash + ".png"
                try? png.write(to: store.imageDir.appendingPathComponent(name))
                record(.image, content: name, imageFile: name, dims: (UInt32(rep.pixelsWide), UInt32(rep.pixelsHigh)))
            }
            return
        }
        let raw = pb.string(forType: .string) ?? ""
        guard !raw.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        record(Classifier.classify(raw), content: raw)
    }

    private func skipByRules() -> Bool {
        guard let app = lastApp?.lowercased() else { return false }
        let s = store.settings
        let blocked = s.excludedApps.contains { !$0.isEmpty && app.contains($0.lowercased()) }
        let confidential = s.ignoreConfidential
            && ["keychain", "password", "1password", "bitwarden", "keepass", "lastpass"]
                .contains { app.contains($0) }
        return blocked || confidential
    }

    private func record(_ type: ItemType, content: String, imageFile: String? = nil,
                        dims: (UInt32, UInt32)? = nil) {
        let (title, preview) = Derive.pair(type, content: content, dims: dims)
        let item = ClipItem(
            id: UUID().uuidString.lowercased(), itemType: type, content: content,
            preview: preview, title: title, sourceApp: lastApp,
            createdAt: Int64(Date().timeIntervalSince1970 * 1000),
            sizeBytes: UInt64(content.utf8.count), imageFile: imageFile
        )
        store.addItem(item)
        sync.append(item)
        if store.settings.soundCopy { NSSound.beep() }
        store.save()
        DispatchQueue.main.async { self.refresh() }
    }

    // MARK: paste (§7)

    func pasteItem(_ item: ClipItem, plain: Bool? = nil) {
        let s = store.settings
        let rulePlain = lastApp.map { app in
            s.plainTextApps.contains { !$0.isEmpty && app.lowercased().contains($0.lowercased()) }
        } ?? false
        let usePlain = plain ?? (s.pasteAsPlainText || rulePlain)
        writeToPasteboard(item, plain: usePlain)
        Hooks.run(event: "paste", content: item.content, hooks: s.hooks)
        hideOverlay()
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.06) { simulatePaste() }
    }

    func copyItem(_ item: ClipItem) {
        writeToPasteboard(item, plain: false)
        Hooks.run(event: "copy", content: item.content, hooks: store.settings.hooks)
    }

    private func writeToPasteboard(_ item: ClipItem, plain: Bool) {
        let pb = NSPasteboard.general
        pb.clearContents()
        if !plain, item.itemType == .image, let name = item.imageFile,
           let img = NSImage(contentsOf: store.imageDir.appendingPathComponent(name)) {
            pb.writeObjects([img])
            return
        }
        pb.setString(item.content, forType: .string)
    }

    func popStack() {
        let dir: StackDir = store.settings.stackDirection == "lifo" ? .lifo : .fifo
        guard let item = store.popStack(dir) else { return }
        writeToPasteboard(item, plain: store.settings.pasteAsPlainText)
        Hooks.run(event: "paste", content: item.content, hooks: store.settings.hooks)
        store.remove(item.id)
        store.save()
        hideOverlay()
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.06) { simulatePaste() }
    }

    // MARK: overlay window control

    func showOverlay() {
        lastApp = NSWorkspace.shared.frontmostApplication?.localizedName
        if let win = NSApp.windows.first(where: { $0.identifier?.rawValue == "overlay" }) {
            win.makeKeyAndOrderFront(nil)
            NSApp.activate(ignoringOtherApps: true)
        }
        refresh()
    }

    func hideOverlay() {
        NSApp.windows.first { $0.identifier?.rawValue == "overlay" }?.orderOut(nil)
    }

    func toggleOverlay() {
        if let win = NSApp.windows.first(where: { $0.identifier?.rawValue == "overlay" }), win.isVisible {
            hideOverlay()
        } else {
            showOverlay()
        }
    }
}

// MARK: - paste simulation (§7.4)

func simulatePaste() {
    let src = CGEventSource(stateID: .combinedSessionState)
    let keyDown = CGEvent(keyboardEventSource: src, virtualKey: CGKeyCode(kVK_ANSI_V), keyDown: true)
    keyDown?.flags = .maskCommand
    let keyUp = CGEvent(keyboardEventSource: src, virtualKey: CGKeyCode(kVK_ANSI_V), keyDown: false)
    keyUp?.flags = .maskCommand
    keyDown?.post(tap: .cghidEventTap)
    keyUp?.post(tap: .cghidEventTap)
}

func sha256hex(_ data: Data) -> String {
    // simple FNV-1a based digest stand-in for filenames (not cryptographic claim)
    var h: UInt64 = 0xcbf29ce484222325
    for b in data {
        h ^= UInt64(b)
        h = h &* 0x100000001b3
    }
    return String(format: "%016llx", h)
}

// MARK: - global shortcut (§10) via Carbon

final class HotKey {
    private var hotKeyRef: EventHotKeyRef?
    private var handlerRef: EventHandlerRef?

    func register(accel: String, action: @escaping () -> Void) {
        unregister()
        guard let (mods, key) = carbonBinding(accel) else { return }
        var eventType = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        let boxed = Unmanaged.passRetained(action as AnyObject)
        InstallEventHandler(GetApplicationEventTarget(), { _, _, userData in
            guard let userData else { return noErr }
            let action = Unmanaged<AnyObject>.fromOpaque(userData).takeUnretainedValue() as! () -> Void
            action()
            return noErr
        }, 1, &eventType, UnsafeMutableRawPointer(boxed.toOpaque()), &handlerRef)
        let hotKeyID = EventHotKeyID(signature: OSType(0x43444b31), id: 1)
        RegisterEventHotKey(key, mods, hotKeyID, GetApplicationEventTarget(), 0, &hotKeyRef)
    }

    func unregister() {
        if let hotKeyRef { UnregisterEventHotKey(hotKeyRef) }
        if let handlerRef { RemoveEventHandler(handlerRef) }
        hotKeyRef = nil
        handlerRef = nil
    }

    /// "CommandOrControl+Shift+V" → (cmd|shift, kVK_ANSI_V). Primary modifier
    /// maps to Command on macOS (the platform half of CommandOrControl).
    private func carbonBinding(_ accel: String) -> (UInt32, UInt32)? {
        var mods: UInt32 = 0
        var keyCode: UInt32?
        for part in accel.split(separator: "+") {
            switch part {
            case "CommandOrControl", "Command": mods |= UInt32(cmdKey)
            case "Control": mods |= UInt32(controlKey)
            case "Alt": mods |= UInt32(optionKey)
            case "Shift": mods |= UInt32(shiftKey)
            default:
                guard let k = keyCodeMap[String(part)] else { return nil }
                keyCode = k
            }
        }
        guard let keyCode else { return nil }
        return (mods, keyCode)
    }

    private let keyCodeMap: [String: UInt32] = {
        var m: [String: UInt32] = [
            "Space": UInt32(kVK_Space), "Enter": UInt32(kVK_Return),
            "Escape": UInt32(kVK_Escape), "Tab": UInt32(kVK_Tab),
            "Backspace": UInt32(kVK_Delete), "Delete": UInt32(kVK_ForwardDelete),
            "Up": UInt32(kVK_UpArrow), "Down": UInt32(kVK_DownArrow),
            "Left": UInt32(kVK_LeftArrow), "Right": UInt32(kVK_RightArrow),
        ]
        let letters = Array("ABCDEFGHIJKLMNOPQRSTUVWXYZ")
        let vkLetters: [UInt32] = [
            kVK_ANSI_A, kVK_ANSI_B, kVK_ANSI_C, kVK_ANSI_D, kVK_ANSI_E, kVK_ANSI_F,
            kVK_ANSI_G, kVK_ANSI_H, kVK_ANSI_I, kVK_ANSI_J, kVK_ANSI_K, kVK_ANSI_L,
            kVK_ANSI_M, kVK_ANSI_N, kVK_ANSI_O, kVK_ANSI_P, kVK_ANSI_Q, kVK_ANSI_R,
            kVK_ANSI_S, kVK_ANSI_T, kVK_ANSI_U, kVK_ANSI_V, kVK_ANSI_W, kVK_ANSI_X,
            kVK_ANSI_Y, kVK_ANSI_Z,
        ].map { UInt32($0) }
        for (l, v) in zip(letters, vkLetters) { m[String(l)] = v }
        let digits = Array("0123456789")
        let vkDigits: [UInt32] = [
            kVK_ANSI_0, kVK_ANSI_1, kVK_ANSI_2, kVK_ANSI_3, kVK_ANSI_4,
            kVK_ANSI_5, kVK_ANSI_6, kVK_ANSI_7, kVK_ANSI_8, kVK_ANSI_9,
        ].map { UInt32($0) }
        for (d, v) in zip(digits, vkDigits) { m[String(d)] = v }
        let fkeys: [(String, Int)] = [
            ("F1", kVK_F1), ("F2", kVK_F2), ("F3", kVK_F3), ("F4", kVK_F4),
            ("F5", kVK_F5), ("F6", kVK_F6), ("F7", kVK_F7), ("F8", kVK_F8),
            ("F9", kVK_F9), ("F10", kVK_F10), ("F11", kVK_F11), ("F12", kVK_F12),
        ]
        for (n, v) in fkeys { m[n] = UInt32(v) }
        return m
    }()
}

// MARK: - login item (§11)

enum LoginItem {
    static func apply(_ enable: Bool) {
        if enable {
            try? SMAppService.mainApp.register()
        } else {
            try? SMAppService.mainApp.unregister()
        }
    }
}

// MARK: - SwiftUI scenes

@main
struct ClipDeckApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) var delegate
    @StateObject private var model = AppModel.shared

    var body: some Scene {
        MenuBarExtra("ClipDeck", systemImage: "doc.on.clipboard") {
            Button("Open ClipDeck") { model.showOverlay() }
            Button("Settings…") { model.showSettings = true; model.showOverlay() }
            Divider()
            Button("Quit") { NSApp.terminate(nil) }
        }
        Window("ClipDeck", id: "overlay") {
            OverlayView()
                .environmentObject(model)
                .frame(minWidth: 1240, minHeight: 372)
                .background(WindowConfigurator())
        }
        .windowStyle(.hiddenTitleBar)
        .windowResizability(.contentSize)
    }
}

struct WindowConfigurator: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView {
        let view = NSView()
        DispatchQueue.main.async {
            if let win = view.window {
                win.identifier = NSUserInterfaceItemIdentifier("overlay")
                win.isOpaque = false
                win.backgroundColor = .clear
                win.level = .floating
                win.styleMask = [.borderless, .fullSizeContentView]
                win.isMovableByWindowBackground = true
            }
        }
        return view
    }
    func updateNSView(_ nsView: NSView, context: Context) {}
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    let hotKey = HotKey()

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        let model = AppModel.shared
        model.startWatcher()
        hotKey.register(accel: model.store.settings.shortcut) { model.toggleOverlay() }
        if model.store.settings.launchAtLogin {
            LoginItem.apply(true)
        }
    }
}

// MARK: - overlay view

struct OverlayView: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        ZStack(alignment: .topLeading) {
            RoundedRectangle(cornerRadius: 22)
                .fill(Color(red: 0.055, green: 0.063, blue: 0.102).opacity(0.94))
            HStack(spacing: 14) {
                sidebar
                VStack(spacing: 12) {
                    topbar
                    cardStrip
                    hints
                }
            }
            .padding(18)
            if model.showSettings {
                SettingsView().environmentObject(model)
            }
        }
    }

    var sidebar: some View {
        VStack(alignment: .leading, spacing: 4) {
            railButton("⏱ History", active: model.filter == nil) { model.filter = nil; model.refresh() }
            railButton("★ Pinned", active: model.filter == "pinned") { model.filter = "pinned"; model.refresh() }
            Text("PINBOARDS").font(.system(size: 9.5, weight: .semibold)).foregroundStyle(.secondary)
                .padding(.top, 10).padding(.horizontal, 11)
            ForEach(model.pinboards, id: \.id) { pb in
                railButton("▦ \(pb.name)", active: model.filter == pb.id) {
                    model.filter = pb.id
                    model.refresh()
                }
            }
            Spacer()
        }
        .frame(width: 158)
    }

    func railButton(_ title: String, active: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title).font(.system(size: 12.5)).frame(maxWidth: .infinity, alignment: .leading)
                .padding(.vertical, 8).padding(.horizontal, 11)
                .background(active ? Color.accentColor.opacity(0.18) : .clear)
                .clipShape(RoundedRectangle(cornerRadius: 10))
        }
        .buttonStyle(.plain)
    }

    var topbar: some View {
        HStack {
            TextField("Search history…", text: $model.query)
                .textFieldStyle(.roundedBorder)
                .onChange(of: model.query) { _ in model.refresh() }
            if !model.stackItemsEmpty {
                Button("Stack") { model.stackOpen.toggle() }
            }
            Button("⚙") { model.showSettings = true }
        }
    }

    var cardStrip: some View {
        ScrollView(.horizontal, showsIndicators: true) {
            HStack(spacing: 13) {
                ForEach(Array(model.items.enumerated()), id: \.element.id) { idx, item in
                    CardView(item: item, selected: idx == model.selected)
                        .onTapGesture { model.selected = idx }
                        .onTapGesture(count: 2) { model.pasteItem(item) }
                        .contextMenu {
                            Button("Copy") { model.copyItem(item) }
                            Button("Paste") { model.pasteItem(item) }
                            Button(item.pinned ? "Unpin" : "Pin") {
                                model.store.update(item.id) { $0.pinned.toggle() }
                                model.store.save(); model.refresh()
                            }
                            Button("Delete") {
                                model.store.remove(item.id)
                                model.store.save(); model.refresh()
                            }
                        }
                }
            }
        }
    }

    var hints: some View {
        Text("← → navigate   ⏎ paste   ⇧⏎ plain   ⌘C copy   ⌘S stack   ⌫ delete   esc close")
            .font(.system(size: 10.5)).foregroundStyle(.secondary)
    }
}

extension AppModel {
    var stackItemsEmpty: Bool { store.stackItems.isEmpty }
}

struct CardView: View {
    let item: ClipItem
    let selected: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Spacer()
                Text(item.pinned ? "★" : "☆").font(.system(size: 11))
            }
            .frame(height: 22)
            preview
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            HStack {
                Text(label).font(.system(size: 9.5, weight: .semibold))
                    .padding(.horizontal, 8).padding(.vertical, 3)
                    .background(Color.accentColor.opacity(0.15))
                    .clipShape(RoundedRectangle(cornerRadius: 7))
                if let n = item.stackOrder { Text("#\(n)").font(.system(size: 9.5, weight: .bold)) }
                Spacer()
                if let app = item.sourceApp {
                    Text(app).font(.system(size: 10)).foregroundStyle(.secondary).lineLimit(1)
                }
            }
        }
        .padding(10)
        .frame(width: 228, height: 252)
        .background(Color.white.opacity(0.055))
        .clipShape(RoundedRectangle(cornerRadius: 15))
        .overlay(
            RoundedRectangle(cornerRadius: 15)
                .stroke(selected ? Color.accentColor : Color.white.opacity(0.08), lineWidth: selected ? 1.5 : 1)
        )
    }

    var label: String {
        switch item.itemType {
        case .text: return "Text"
        case .link: return "Link"
        case .code: return "Code"
        case .color: return "Color"
        case .image: return "Image"
        case .file: return "Files"
        case .html: return "Rich text"
        }
    }

    @ViewBuilder var preview: some View {
        switch item.itemType {
        case .color:
            Rectangle().fill(Color(hexString: item.content.trimmingCharacters(in: .whitespaces)))
        case .code:
            ScrollView([.vertical, .horizontal]) {
                Text(item.content.prefix(600))
                    .font(.system(size: 11.5, design: .monospace))
                    .foregroundStyle(Color(red: 0.72, green: 0.88, blue: 0.82))
            }
        case .link:
            VStack(alignment: .leading, spacing: 9) {
                RoundedRectangle(cornerRadius: 11)
                    .fill(LinearGradient(colors: [.blue, .purple], startPoint: .topLeading, endPoint: .bottomTrailing))
                    .frame(width: 42, height: 42)
                    .overlay(Text(item.preview.prefix(1).uppercased()).font(.system(size: 19, weight: .semibold)).foregroundColor(.white))
                Text(item.title).font(.system(size: 13.5, weight: .semibold)).lineLimit(3)
                Text(item.preview).font(.system(size: 11.5)).foregroundStyle(.secondary)
            }
        case .file:
            VStack(alignment: .leading, spacing: 7) {
                ForEach(item.content.split(separator: "\n").prefix(4), id: \.self) { f in
                    Text("📄 " + (f.split(separator: "/").last.map(String.init) ?? String(f)))
                        .font(.system(size: 12))
                        .padding(7).frame(maxWidth: .infinity, alignment: .leading)
                        .background(Color.white.opacity(0.05))
                        .clipShape(RoundedRectangle(cornerRadius: 8))
                }
            }
        case .image:
            if let name = item.imageFile, let img = NSImage(contentsOf: AppModel.shared.store.imageDir.appendingPathComponent(name)) {
                Image(nsImage: img).resizable().scaledToFill()
            }
        default:
            Text(item.content.prefix(600)).font(.system(size: 12.5)).lineLimit(12)
        }
    }
}

extension Color {
    init(hexString: String) {
        var v: UInt64 = 0
        Scanner(string: hexString.hasPrefix("#") ? String(hexString.dropFirst()) : hexString).scanHexInt64(&v)
        self.init(red: Double((v >> 16) & 0xFF) / 255, green: Double((v >> 8) & 0xFF) / 255, blue: Double(v & 0xFF) / 255)
    }
}

struct SettingsView: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("ClipDeck Settings").font(.system(size: 15, weight: .semibold))
            toggle("Watch clipboard", \.watchEnabled)
            toggle("Play sound on copy", \.soundCopy)
            toggle("Play sound on paste", \.soundPaste)
            toggle("Ignore transient content", \.ignoreTransient)
            toggle("Ignore confidential content", \.ignoreConfidential)
            toggle("Generate link previews", \.generateLinkPreviews)
            toggle("Always paste as plain text", \.pasteAsPlainText)
            toggle("Start ClipDeck at login", \.launchAtLogin) {
                LoginItem.apply(model.store.settings.launchAtLogin)
            }
            toggle("Show overlay during screen sharing", \.showInScreenShare)
            Spacer()
            HStack {
                Button("Export…") { exportJSON() }
                Button("Import…") { importJSON() }
                Spacer()
                Button("Close") { model.showSettings = false }
            }
        }
        .padding(28)
        .background(Color(red: 0.07, green: 0.08, blue: 0.11))
    }

    func toggle(_ title: String, _ keyPath: WritableKeyPath<Settings, Bool>, after: (() -> Void)? = nil) -> some View {
        Toggle(title, isOn: Binding(
            get: { model.store.settings[keyPath: keyPath] },
            set: { v in
                model.store.settings[keyPath: keyPath] = v
                model.store.save()
                if keyPath == \.launchAtLogin { LoginItem.apply(v) }
                after?()
            }
        ))
    }

    func exportJSON() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "clipdeck-export.json"
        if panel.runModal() == .OK, let url = panel.url {
            try? model.store.exportJSON().data(using: .utf8)?.write(to: url)
        }
    }

    func importJSON() {
        let panel = NSOpenPanel()
        if panel.runModal() == .OK, let url = panel.url,
           let raw = try? String(contentsOf: url) {
            model.store.importJSON(raw)
            model.store.save()
            model.refresh()
        }
    }
}
