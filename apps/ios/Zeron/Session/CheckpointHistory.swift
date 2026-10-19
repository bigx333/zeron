import UIKit

// History in the tasks sheet: the checkpoints each finished turn leaves
// (conversation, provider session and, when captured, the checkout), with
// restore in place. A restore keeps where you were as a recovery point, so
// the section offers Undo right after one, and keeps recovery points folded
// for inspection. Every word comes from what the core returned.

// MARK: Model

extension RunCheckpointView {
    /// What the provider does on restore, in plain words.
    var strategyText: String? {
        switch providerStrategy {
        case "native": "Resumes the provider session at this point."
        case "portable": "Continues from a copy of the conversation."
        default: nil
        }
    }

    /// What a restore does, said before it happens.
    var restoreBlurb: String {
        switch (recovery, filesAvailable) {
        case (true, true): "The conversation and the checkout return to where they were before the restore. Where you are now is saved as a recovery point."
        case (true, false): "The conversation returns to where it was before the restore. Where you are now is saved as a recovery point."
        case (false, true): "The conversation and the checkout go back to just after this turn. Where you are now is saved as a recovery point."
        case (false, false): "The conversation goes back to just after this turn; no files were saved here, so the checkout stays as it is. Where you are now is saved as a recovery point."
        }
    }

    /// Why Restore can't run right now, the core's word last.
    /// `sharedTasks`: a task of this chat still works in its checkout.
    func blocked(by chat: SessionRow?, sharedTasks: Bool) -> String? {
        guard let chat else { return "This chat isn't available." }
        if !chat.deviceOnline { return "Its device is offline." }
        if chat.indicator == .working || chat.indicator == .awaitingInput { return "Finish or stop the current turn first." }
        if sharedTasks { return "Stop tasks working in this checkout first." }
        if !restorable { return blockedReason ?? "This point can't be restored." }
        return nil
    }

    var filesFact: String {
        guard filesAvailable else { return "Conversation only" }
        switch filesChanged {
        case nil: return "Checkout saved"
        case 0: return "Checkout unchanged since"
        case 1: return "1 file differs"
        case let n?: return "\(n) files differ"
        }
    }

    var strategyFact: String? {
        switch providerStrategy {
        case "native": "Native resume"
        case "portable": "Portable"
        default: nil
        }
    }
}

enum HistoryAgo {
    static func phrase(_ ms: Int64, now: Date = Date()) -> String {
        let secs = max(0, Int(now.timeIntervalSince1970 - Double(ms) / 1000))
        if secs < 60 { return "just now" }
        return (secs < 3600 ? "\(secs / 60)m" : secs < 86400 ? "\(secs / 3600)h" : "\(secs / 86400)d") + " ago"
    }

    static func short(_ ms: Int64, now: Date = Date()) -> String {
        let secs = max(0, Int(now.timeIntervalSince1970 - Double(ms) / 1000))
        if secs < 60 { return "now" }
        return secs < 3600 ? "\(secs / 60)m" : secs < 86400 ? "\(secs / 3600)h" : "\(secs / 86400)d"
    }
}

extension AppModel {
    func checkpoints(_ chatId: String) async throws -> [RunCheckpointView] {
        guard let client else { throw CoreError.Closed }
        return try await client.chatCheckpoints(chatId: chatId)
    }

    func saveCheckpoint(_ chatId: String) async throws {
        guard let client else { throw CoreError.Closed }
        _ = try await client.createChatCheckpoint(chatId: chatId)
    }

    /// Restore; returns the recovery checkpoint the core kept.
    func rollback(_ chatId: String, to checkpointId: String, request: String) async throws -> String {
        guard let client else { throw CoreError.Closed }
        return try await client.rollbackChat(chatId: chatId, checkpointId: checkpointId, clientRequestId: request)
    }
}

// MARK: Section

/// Owns History's state and rows for the tasks sheet; `views` hands the
/// sheet what to stack, `onChange` asks it to restack.
final class HistorySection {
    private enum Load: Equatable {
        case loading
        case ready([RunCheckpointView])
        case failed(String)
        /// The host predates checkpoints: the section stays away.
        case unsupported
    }

    private struct Receipt {
        var title: String
        var recovery: String
        var undone = false
        /// When the restore landed; a turn finished after it retires Undo.
        var at = Int64(Date().timeIntervalSince1970 * 1000)
    }

    private let app: AppModel
    private let chatId: String
    private weak var host: UIViewController?
    private let onChange: () -> Void
    private var load: Load = .loading
    private var readFor: Int64?
    private var reading = false
    private var receipt: Receipt?
    private var restoring: String?
    private var errors: [String: String] = [:]
    private var requests: [String: String] = [:]
    private var saving = false
    private var saveError: String?
    private var expanded: Set<String> = []
    private var recoveryOpen = false
    private var shown = 5
    private var rows: [String: CheckpointRowView] = [:]

    private lazy var header: HistoryHeaderView = {
        let header = HistoryHeaderView()
        header.onSave = { [weak self] in self?.save() }
        return header
    }()
    private let receiptView = ReceiptView()
    private let status = HistoryStatusView()
    private lazy var recoveryFold: FoldRowView = {
        let fold = FoldRowView()
        fold.addAction(UIAction { [weak self] _ in
            guard let self else { return }
            self.recoveryOpen.toggle()
            self.onChange()
        }, for: .touchUpInside)
        return fold
    }()
    private lazy var more: LaneRowControl = {
        let more = LaneRowControl()
        let label = LaneType.label(.sans, 15)
        label.textColor = Palette.secondary
        label.tag = 1
        label.translatesAutoresizingMaskIntoConstraints = false
        label.isUserInteractionEnabled = false
        more.addSubview(label)
        NSLayoutConstraint.activate([
            more.heightAnchor.constraint(greaterThanOrEqualToConstant: 44),
            label.leadingAnchor.constraint(equalTo: more.leadingAnchor, constant: 38),
            label.centerYAnchor.constraint(equalTo: more.centerYAnchor),
        ])
        more.isAccessibilityElement = true
        more.accessibilityTraits = .button
        more.addAction(UIAction { [weak self] _ in
            self?.shown += 10
            self?.onChange()
        }, for: .touchUpInside)
        return more
    }()

    init(app: AppModel, chatId: String, host: UIViewController, onChange: @escaping () -> Void) {
        self.app = app
        self.chatId = chatId
        self.host = host
        self.onChange = onChange
        receiptView.onUndo = { [weak self] in
            guard let self, let receipt = self.receipt else { return }
            self.rollback(receipt.recovery, title: receipt.title, undo: true)
        }
        status.onRetry = { [weak self] in
            self?.load = .loading
            self?.readFor = nil
            self?.onChange()
        }
    }

    /// Read again once per finished turn (a new checkpoint is saved then) —
    /// not while the turn runs, whose activity ticks would poll.
    private func refreshIfNeeded(_ chat: SessionRow?) {
        guard let chat, load != .unsupported, !reading, readFor != chat.lastActivityMs else { return }
        if readFor != nil, chat.indicator == .working || chat.indicator == .awaitingInput { return }
        readFor = chat.lastActivityMs
        read()
    }

    private func read() {
        reading = true
        Task { @MainActor [weak self] in
            guard let self else { return }
            do {
                let list = try await self.app.checkpoints(self.chatId)
                self.load = .ready(list)
                // The chat moved on after the restore: Undo would now mean
                // something else, so the receipt retires.
                if let receipt = self.receipt, list.contains(where: { !$0.recovery && $0.createdAtMs > receipt.at }) {
                    self.receipt = nil
                }
            } catch CoreError.Unsupported, CoreError.NotImplemented {
                self.load = .unsupported
            } catch {
                if case .ready = self.load {} else { self.load = .failed(error.hostReason ?? "Try again.") }
            }
            self.reading = false
            self.onChange()
        }
    }

    private func reread() {
        readFor = nil
        onChange()
    }

    /// The rows to stack under the sheet's other sections; empty when the
    /// host can't keep checkpoints.
    func views(chat: SessionRow?, animated: Bool) -> [UIView] {
        refreshIfNeeded(chat)
        if load == .unsupported { return [] }
        let sharedTasks = app.children(of: chatId).contains { $0.task.map { !$0.terminal && $0.isolation == "shared" } ?? false }
        var views: [UIView] = [header]
        header.configure(saving: saving, enabled: chat?.deviceOnline ?? false)
        if let saveError { views.append(HistoryNoteView(saveError, color: Palette.danger)) }
        if let receipt {
            receiptView.configure(
                text: receipt.undone ? "Undone. The restore is kept as a recovery point." : "Restored to \u{201C}\(receipt.title)\u{201D}.",
                canUndo: !receipt.undone,
                undoing: restoring == receipt.recovery,
                error: errors[receipt.recovery]
            )
            views.append(receiptView)
        }
        switch load {
        case .loading, .unsupported:
            status.configure(.loading)
            views.append(status)
        case let .failed(message):
            status.configure(.failed("Couldn't load history. \(message)"))
            views.append(status)
        case let .ready(list):
            let points = list.filter { !$0.recovery }
            let recovery = list.filter(\.recovery)
            if list.isEmpty {
                status.configure(.empty)
                views.append(status)
            }
            for point in points.prefix(shown) { views.append(row(point, chat: chat, sharedTasks: sharedTasks, animated: animated)) }
            if points.count > shown {
                (more.viewWithTag(1) as? UILabel)?.text = "Show \(min(10, points.count - shown)) more"
                more.accessibilityLabel = "Show \(min(10, points.count - shown)) more checkpoints"
                views.append(more)
            }
            if !recovery.isEmpty {
                recoveryFold.set(count: recovery.count, open: recoveryOpen, noun: "recovery point")
                views.append(recoveryFold)
                if recoveryOpen { for point in recovery { views.append(row(point, chat: chat, sharedTasks: sharedTasks, animated: animated)) } }
            }
        }
        return views
    }

    private func row(_ point: RunCheckpointView, chat: SessionRow?, sharedTasks: Bool, animated: Bool) -> CheckpointRowView {
        let fresh = rows[point.id] == nil
        let view = rows[point.id] ?? {
            let view = CheckpointRowView()
            view.onToggle = { [weak self] in
                guard let self else { return }
                if !self.expanded.insert(point.id).inserted { self.expanded.remove(point.id) }
                self.onChange()
            }
            rows[point.id] = view
            return view
        }()
        view.onRestore = { [weak self, weak view] in
            guard let self, let view else { return }
            self.confirm(point, from: view.restoreButton)
        }
        view.configure(
            point: point,
            blocked: point.blocked(by: chat, sharedTasks: sharedTasks),
            restoring: restoring == point.id,
            busyElsewhere: restoring != nil && restoring != point.id,
            error: errors[point.id],
            expanded: expanded.contains(point.id),
            animated: animated
        )
        // A checkpoint that arrives while the sheet is up eases in once.
        if fresh, animated, !LaneMotion.reduced, readFor != nil, case .ready = load {
            view.alpha = 0
            view.transform = CGAffineTransform(translationX: 0, y: 6)
            UIView.animate(withDuration: 0.5, delay: 0.05, usingSpringWithDamping: 1, initialSpringVelocity: 0, options: [.allowUserInteraction]) {
                view.alpha = 1
                view.transform = .identity
            }
        }
        return view
    }

    private func confirm(_ point: RunCheckpointView, from anchor: UIView) {
        let alert = UIAlertController(title: point.recovery ? "Restore this recovery point?" : "Restore to this point?", message: point.restoreBlurb, preferredStyle: .alert)
        alert.addAction(UIAlertAction(title: "Keep current", style: .cancel))
        let restore = UIAlertAction(title: "Restore", style: .default) { [weak self] _ in
            self?.rollback(point.id, title: point.title, undo: false)
        }
        alert.addAction(restore)
        alert.preferredAction = restore
        host?.present(alert, animated: true)
    }

    private func rollback(_ id: String, title: String, undo: Bool) {
        guard restoring == nil else { return }
        restoring = id
        errors[id] = nil
        let request = requests[id] ?? UUID().uuidString
        requests[id] = request
        onChange()
        Task { @MainActor [weak self] in
            guard let self else { return }
            do {
                let recovery = try await self.app.rollback(self.chatId, to: id, request: request)
                self.requests[id] = nil
                self.expanded.remove(id)
                if undo, var previous = self.receipt {
                    previous.recovery = recovery
                    previous.undone = true
                    self.receipt = previous
                } else {
                    self.receipt = Receipt(title: title, recovery: recovery)
                }
                UINotificationFeedbackGenerator().notificationOccurred(.success)
                UIAccessibility.post(notification: .announcement, argument: undo ? "Restore undone" : "Restored to \(title)")
                self.restoring = nil
                self.reread()
            } catch {
                self.errors[id] = "Couldn't restore. " + (error.hostReason ?? "Try again.")
                UIAccessibility.post(notification: .announcement, argument: self.errors[id])
                self.restoring = nil
                self.onChange()
            }
        }
    }

    private func save() {
        guard !saving else { return }
        saving = true
        saveError = nil
        onChange()
        Task { @MainActor [weak self] in
            guard let self else { return }
            do {
                try await self.app.saveCheckpoint(self.chatId)
                self.saving = false
                self.reread()
            } catch {
                self.saveError = "Couldn't save a checkpoint. " + (error.hostReason ?? "Try again.")
                self.saving = false
                self.onChange()
            }
        }
    }
}

// MARK: Views

/// "History" with Save beside it.
private final class HistoryHeaderView: UIView {
    var onSave: (() -> Void)?
    private let save = UIButton(configuration: .plain())

    override init(frame: CGRect) {
        super.init(frame: frame)
        let label = LaneType.label(.sansMedium, 13)
        label.text = "History"
        label.textColor = Palette.secondary
        label.accessibilityTraits = .header
        save.configuration?.image = UIImage(systemName: "plus.circle", withConfiguration: UIImage.SymbolConfiguration(pointSize: 14, weight: .medium))
        save.configuration?.imagePadding = 6
        save.configuration?.baseForegroundColor = Palette.secondary
        save.configuration?.buttonSize = .small
        save.addAction(UIAction { [weak self] _ in self?.onSave?() }, for: .touchUpInside)
        let row = UIStackView(arrangedSubviews: [label, UIView(), save])
        row.alignment = .center
        row.translatesAutoresizingMaskIntoConstraints = false
        addSubview(row)
        NSLayoutConstraint.activate([
            row.topAnchor.constraint(equalTo: topAnchor, constant: 6),
            row.bottomAnchor.constraint(equalTo: bottomAnchor),
            row.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            row.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -4),
            save.heightAnchor.constraint(greaterThanOrEqualToConstant: 44),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    func configure(saving: Bool, enabled: Bool) {
        save.configuration?.title = saving ? "Saving…" : "Save now"
        save.configuration?.showsActivityIndicator = saving
        save.isEnabled = enabled && !saving
        save.accessibilityLabel = saving ? "Saving a checkpoint" : "Save a checkpoint now"
        save.accessibilityHint = enabled ? nil : "Its device is offline"
    }
}

/// One note line in the section's voice.
private final class HistoryNoteView: UIView {
    init(_ text: String, color: UIColor) {
        super.init(frame: .zero)
        let label = LaneType.label(.sans, 14)
        label.text = text
        label.textColor = color
        label.numberOfLines = 0
        label.translatesAutoresizingMaskIntoConstraints = false
        addSubview(label)
        NSLayoutConstraint.activate([
            label.topAnchor.constraint(equalTo: topAnchor, constant: 4),
            label.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -4),
            label.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            label.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }
}

/// Loading, a failed read with Retry, or the empty invitation.
private final class HistoryStatusView: UIView {
    enum State: Equatable { case loading, failed(String), empty }
    var onRetry: (() -> Void)?
    private let spinner = UIActivityIndicatorView(style: .medium)
    private let label = LaneType.label(.sans, 14)
    private let retry = UIButton(configuration: .gray())

    override init(frame: CGRect) {
        super.init(frame: frame)
        label.numberOfLines = 0
        retry.configuration?.title = "Retry"
        retry.configuration?.cornerStyle = .capsule
        retry.configuration?.buttonSize = .small
        retry.addAction(UIAction { [weak self] _ in self?.onRetry?() }, for: .touchUpInside)
        let line = UIStackView(arrangedSubviews: [spinner, label])
        line.spacing = 10
        line.alignment = .center
        let column = UIStackView(arrangedSubviews: [line, retry])
        column.axis = .vertical
        column.alignment = .leading
        column.spacing = 8
        column.translatesAutoresizingMaskIntoConstraints = false
        addSubview(column)
        NSLayoutConstraint.activate([
            column.topAnchor.constraint(equalTo: topAnchor, constant: 8),
            column.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -8),
            column.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            column.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    func configure(_ state: State) {
        switch state {
        case .loading:
            spinner.startAnimating()
            spinner.isHidden = false
            label.text = "Loading history…"
            label.textColor = Palette.secondary
            retry.isHidden = true
        case let .failed(message):
            spinner.stopAnimating()
            spinner.isHidden = true
            label.text = message
            label.textColor = Palette.danger
            retry.isHidden = false
        case .empty:
            spinner.stopAnimating()
            spinner.isHidden = true
            label.text = "A checkpoint is saved after each finished turn."
            label.textColor = Palette.secondary
            retry.isHidden = true
        }
    }
}

/// "Restored to … [Undo]" on a success wash.
private final class ReceiptView: UIView {
    var onUndo: (() -> Void)?
    private let label = LaneType.label(.sans, 14)
    private let undo = UIButton(configuration: .plain())
    private let error = LaneType.label(.sans, 14)

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = StatusTone.done.withAlphaComponent(0.1)
        layer.cornerRadius = 12
        layer.cornerCurve = .continuous
        let check = UIImageView(image: UIImage(systemName: "checkmark", withConfiguration: UIImage.SymbolConfiguration(pointSize: 12, weight: .semibold)))
        check.tintColor = Palette.success
        check.setContentHuggingPriority(.required, for: .horizontal)
        label.textColor = Palette.text
        label.numberOfLines = 0
        undo.configuration?.title = "Undo"
        undo.configuration?.buttonSize = .small
        undo.setContentHuggingPriority(.required, for: .horizontal)
        undo.addAction(UIAction { [weak self] _ in self?.onUndo?() }, for: .touchUpInside)
        error.textColor = Palette.danger
        error.numberOfLines = 0
        let line = UIStackView(arrangedSubviews: [check, label, undo])
        line.spacing = 8
        line.alignment = .center
        let column = UIStackView(arrangedSubviews: [line, error])
        column.axis = .vertical
        column.spacing = 4
        column.translatesAutoresizingMaskIntoConstraints = false
        addSubview(column)
        NSLayoutConstraint.activate([
            column.topAnchor.constraint(equalTo: topAnchor, constant: 6),
            column.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -6),
            column.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            column.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -6),
            undo.heightAnchor.constraint(greaterThanOrEqualToConstant: 44),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    func configure(text: String, canUndo: Bool, undoing: Bool, error message: String?) {
        label.text = text
        undo.isHidden = !canUndo
        undo.isEnabled = !undoing
        undo.configuration?.showsActivityIndicator = undoing
        undo.configuration?.title = undoing ? "Undoing…" : "Undo"
        error.text = message
        error.isHidden = message == nil
    }
}

/// One checkpoint: tap to inspect; Restore asks first.
private final class CheckpointRowView: UIView {
    var onToggle: (() -> Void)?
    var onRestore: (() -> Void)?
    private let header = LaneRowControl()
    private let glyph = UIImageView()
    private let title = FadingLabel()
    private let time = LaneType.label(.sans, 13)
    private let detail = UIStackView()
    private let saved = LaneType.label(.sans, 13)
    private let facts = LaneType.label(.sans, 13)
    private let strategy = LaneType.label(.sans, 14)
    private let blocked = LaneType.label(.sans, 14)
    private let error = LaneType.label(.sans, 14)
    let restoreButton = UIButton(configuration: .gray())
    private var expanded = false

    override init(frame: CGRect) {
        super.init(frame: frame)
        layer.cornerRadius = 14
        layer.cornerCurve = .continuous
        clipsToBounds = true
        glyph.tintColor = Palette.secondary
        glyph.contentMode = .center
        LaneType.apply(title, .sans, 16)
        title.textColor = Palette.text
        time.textColor = StatusTone.time
        time.setContentHuggingPriority(.required, for: .horizontal)
        time.setContentCompressionResistancePriority(.required, for: .horizontal)
        let line = UIStackView(arrangedSubviews: [glyph, title, time])
        line.spacing = 10
        line.alignment = .center
        line.isUserInteractionEnabled = false
        line.translatesAutoresizingMaskIntoConstraints = false
        header.addSubview(line)
        NSLayoutConstraint.activate([
            header.heightAnchor.constraint(greaterThanOrEqualToConstant: 48),
            glyph.widthAnchor.constraint(equalToConstant: 16),
            line.topAnchor.constraint(equalTo: header.topAnchor, constant: 8),
            line.bottomAnchor.constraint(equalTo: header.bottomAnchor, constant: -8),
            line.leadingAnchor.constraint(equalTo: header.leadingAnchor, constant: 12),
            line.trailingAnchor.constraint(equalTo: header.trailingAnchor, constant: -12),
        ])
        header.isAccessibilityElement = true
        header.accessibilityTraits = .button
        header.accessibilityHint = "Double-tap to show details"
        header.addAction(UIAction { [weak self] _ in self?.onToggle?() }, for: .touchUpInside)

        saved.textColor = Palette.tertiary
        facts.textColor = Palette.secondary
        facts.numberOfLines = 0
        strategy.textColor = Palette.secondary
        strategy.numberOfLines = 0
        blocked.textColor = Palette.warning
        blocked.numberOfLines = 0
        error.textColor = Palette.danger
        error.numberOfLines = 0
        restoreButton.configuration?.cornerStyle = .capsule
        restoreButton.configuration?.buttonSize = .small
        restoreButton.addAction(UIAction { [weak self] _ in self?.onRestore?() }, for: .touchUpInside)
        let actions = UIStackView(arrangedSubviews: [restoreButton, UIView()])
        for v in [saved, facts, strategy, blocked, error, actions] { detail.addArrangedSubview(v) }
        detail.axis = .vertical
        detail.spacing = 8
        detail.isLayoutMarginsRelativeArrangement = true
        detail.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 0, leading: 38, bottom: 14, trailing: 12)
        detail.isHidden = true
        detail.alpha = 0

        let column = UIStackView(arrangedSubviews: [header, detail])
        column.axis = .vertical
        column.translatesAutoresizingMaskIntoConstraints = false
        addSubview(column)
        NSLayoutConstraint.activate([
            column.topAnchor.constraint(equalTo: topAnchor),
            column.bottomAnchor.constraint(equalTo: bottomAnchor),
            column.leadingAnchor.constraint(equalTo: leadingAnchor),
            column.trailingAnchor.constraint(equalTo: trailingAnchor),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    func configure(point: RunCheckpointView, blocked reason: String?, restoring: Bool, busyElsewhere: Bool, error message: String?, expanded open: Bool, animated: Bool) {
        glyph.image = UIImage(systemName: point.recovery ? "arrow.uturn.backward.circle" : "clock", withConfiguration: UIImage.SymbolConfiguration(pointSize: 14, weight: .regular))
        title.text = point.title
        time.text = HistoryAgo.short(point.createdAtMs)
        saved.text = point.recovery ? "Kept \(HistoryAgo.phrase(point.createdAtMs)), before a restore" : "Saved \(HistoryAgo.phrase(point.createdAtMs))"
        facts.text = [point.filesFact, point.strategyFact].compactMap { $0 }.joined(separator: "   ")
        strategy.text = point.strategyText
        strategy.isHidden = point.strategyText == nil
        blocked.text = reason
        blocked.isHidden = reason == nil
        error.text = message
        error.isHidden = message == nil
        restoreButton.configuration?.title = restoring ? "Restoring…" : message != nil ? "Try again" : point.recovery ? "Restore this point" : "Restore to here"
        restoreButton.configuration?.showsActivityIndicator = restoring
        restoreButton.isEnabled = reason == nil && !restoring && !busyElsewhere
        header.accessibilityLabel = "\(point.title), saved \(HistoryAgo.phrase(point.createdAtMs))"
        header.accessibilityValue = open ? "Expanded" : "Collapsed"
        guard open != expanded else { return }
        expanded = open
        let change = {
            self.detail.isHidden = !open
            self.detail.alpha = open ? 1 : 0
            self.backgroundColor = open ? Palette.controlFill : .clear
            self.superview?.layoutIfNeeded()
        }
        if animated, window != nil { LaneMotion.disclose(change) } else { change() }
    }
}
